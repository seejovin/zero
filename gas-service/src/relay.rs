//! Separate opaque OHTTP relay. This module has no wallet methods, journal,
//! decryption keys, proof adapter or signing authority. Independent deployment
//! and logging policy are operational requirements, not configuration claims.
use crate::{Error, Result};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, header, service::service_fn, Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};
use std::{
    convert::Infallible,
    io::Read,
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    net::TcpListener,
    sync::{watch, Semaphore},
    task::JoinSet,
};

const MAX_CIPHERTEXT: usize = 540_672;
const OHTTP_REQUEST: &str = "message/ohttp-req";
const OHTTP_RESPONSE: &str = "message/ohttp-res";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayConfig {
    pub schema_version: u8,
    pub listen: SocketAddr,
    pub gateway_url: String,
    /// Optional exact browser origin. Android requires no CORS origin.
    pub cors_origin: Option<String>,
}
impl RelayConfig {
    pub fn read(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path).map_err(|_| Error::InvalidPolicy)?;
        let mut body = String::new();
        file.take(4097)
            .read_to_string(&mut body)
            .map_err(|_| Error::InvalidPolicy)?;
        if body.len() > 4096 {
            return Err(Error::InvalidPolicy);
        }
        let config: Self = crate::model::decode(&body)?;
        config.validate(false)?;
        Ok(config)
    }
    fn validate(&self, permit_loopback: bool) -> Result<Url> {
        if self.schema_version != 1 || !self.listen.ip().is_loopback() {
            return Err(Error::InvalidPolicy);
        }
        let url = Url::parse(&self.gateway_url).map_err(|_| Error::InvalidPolicy)?;
        let loopback = url
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        if !(url.scheme() == "https" || permit_loopback && loopback && url.scheme() == "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/ohttp"
        {
            return Err(Error::InvalidPolicy);
        }
        if let Some(origin) = &self.cors_origin {
            let value = Url::parse(origin).map_err(|_| Error::InvalidPolicy)?;
            if value.scheme() != "https"
                || value.host_str().is_none()
                || value.origin().ascii_serialization() != *origin
                || !value.username().is_empty()
                || value.password().is_some()
            {
                return Err(Error::InvalidPolicy);
            }
        }
        Ok(url)
    }
}

#[derive(Default)]
struct Circuit {
    failures: u8,
    blocked_until: Option<Instant>,
}
pub struct ObliviousRelay {
    client: Client,
    gateway: Url,
    cors_origin: Option<String>,
    processing: Arc<Semaphore>,
    circuit: Mutex<Circuit>,
}
impl ObliviousRelay {
    pub fn new(config: &RelayConfig) -> Result<Self> {
        Self::build(config, false)
    }
    #[cfg(test)]
    pub(crate) fn local_fixture(config: &RelayConfig) -> Result<Self> {
        Self::build(config, true)
    }
    fn build(config: &RelayConfig, permit_loopback: bool) -> Result<Self> {
        let gateway = config.validate(permit_loopback)?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(55))
            .pool_max_idle_per_host(2)
            .build()
            .map_err(|_| Error::Gateway)?;
        Ok(Self {
            client,
            gateway,
            cors_origin: config.cors_origin.clone(),
            processing: Arc::new(Semaphore::new(4)),
            circuit: Mutex::new(Circuit::default()),
        })
    }
    fn available(&self) -> bool {
        let Ok(mut circuit) = self.circuit.lock() else {
            return false;
        };
        match circuit.blocked_until {
            Some(until) if Instant::now() < until => false,
            Some(_) => {
                circuit.blocked_until = None;
                circuit.failures = 0;
                true
            }
            None => true,
        }
    }
    fn record_health(&self, success: bool) {
        if let Ok(mut circuit) = self.circuit.lock() {
            if success {
                circuit.failures = 0;
                circuit.blocked_until = None;
            } else {
                circuit.failures = circuit.failures.saturating_add(1);
                if circuit.failures >= 5 {
                    circuit.blocked_until = Some(Instant::now() + Duration::from_secs(30));
                }
            }
        }
    }
    fn forward(&self, ciphertext: Vec<u8>) -> std::result::Result<(StatusCode, Vec<u8>), ()> {
        // Build fresh constant headers. Original cookies, authorization, IP,
        // Forwarded, User-Agent, origin and request IDs are never forwarded.
        let result = (|| {
            let response = self
                .client
                .post(self.gateway.clone())
                .header("content-type", OHTTP_REQUEST)
                .header("accept", OHTTP_RESPONSE)
                .body(ciphertext)
                .send()
                .map_err(|_| ())?;
            let status = response.status();
            if status.is_client_error() {
                return Ok((
                    StatusCode::from_u16(status.as_u16()).map_err(|_| ())?,
                    Vec::new(),
                ));
            }
            if status.as_u16() != 200
                || response.headers().get_all("content-type").iter().count() != 1
                || response
                    .headers()
                    .get("content-type")
                    .and_then(|value| value.to_str().ok())
                    != Some(OHTTP_RESPONSE)
                || response
                    .content_length()
                    .is_some_and(|n| n > MAX_CIPHERTEXT as u64)
            {
                return Err(());
            }
            let mut body = Vec::new();
            response
                .take((MAX_CIPHERTEXT + 1) as u64)
                .read_to_end(&mut body)
                .map_err(|_| ())?;
            if body.is_empty() || body.len() > MAX_CIPHERTEXT {
                return Err(());
            }
            Ok((StatusCode::OK, body))
        })();
        self.record_health(result.is_ok());
        result
    }
    async fn request(
        self: Arc<Self>,
        request: Request<Incoming>,
    ) -> std::result::Result<Response<Full<Bytes>>, Infallible> {
        let origin = request
            .headers()
            .get("origin")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let result = self.clone().checked(request).await;
        let (status, body) = result.unwrap_or_else(|status| (status, Vec::new()));
        let mut response = Response::new(Full::new(Bytes::from(body)));
        *response.status_mut() = status;
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static(OHTTP_RESPONSE),
        );
        headers.insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-store"),
        );
        headers.insert(
            header::CONNECTION,
            header::HeaderValue::from_static("close"),
        );
        if let Some(origin) = origin.filter(|origin| self.cors_origin.as_ref() == Some(origin)) {
            if let Ok(value) = header::HeaderValue::from_str(&origin) {
                headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
            }
            headers.insert(header::VARY, header::HeaderValue::from_static("Origin"));
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                header::HeaderValue::from_static("POST"),
            );
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                header::HeaderValue::from_static("content-type, accept"),
            );
        }
        Ok(response)
    }
    async fn checked(
        self: Arc<Self>,
        request: Request<Incoming>,
    ) -> std::result::Result<(StatusCode, Vec<u8>), StatusCode> {
        if request.uri().scheme().is_some()
            || request.uri().authority().is_some()
            || request.uri().query().is_some()
            || request.uri().path() != "/ohttp"
        {
            return Err(StatusCode::NOT_FOUND);
        }
        let headers = request.headers();
        if headers.get_all("origin").iter().count() > 1 {
            return Err(StatusCode::FORBIDDEN);
        }
        if let Some(origin) = headers.get("origin") {
            if self.cors_origin.is_none() || origin.to_str().ok() != self.cors_origin.as_deref() {
                return Err(StatusCode::FORBIDDEN);
            }
        }
        if request.method() == Method::OPTIONS {
            if self.cors_origin.is_none()
                || !headers.contains_key("origin")
                || headers
                    .get("access-control-request-method")
                    .and_then(|value| value.to_str().ok())
                    != Some("POST")
                || headers.contains_key(header::TRANSFER_ENCODING)
                || headers
                    .get(header::CONTENT_LENGTH)
                    .is_some_and(|value| value != "0")
            {
                return Err(StatusCode::FORBIDDEN);
            }
            if let Some(value) = headers.get("access-control-request-headers") {
                if !value
                    .to_str()
                    .map_err(|_| StatusCode::BAD_REQUEST)?
                    .split(',')
                    .map(str::trim)
                    .all(|name| {
                        name.eq_ignore_ascii_case("content-type")
                            || name.eq_ignore_ascii_case("accept")
                    })
                {
                    return Err(StatusCode::FORBIDDEN);
                }
            }
            return Ok((StatusCode::NO_CONTENT, Vec::new()));
        }
        if request.method() != Method::POST {
            return Err(StatusCode::METHOD_NOT_ALLOWED);
        }
        if headers.get_all(header::CONTENT_TYPE).iter().count() != 1
            || headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                != Some(OHTTP_REQUEST)
            || headers.get_all(header::CONTENT_LENGTH).iter().count() != 1
            || headers.contains_key(header::TRANSFER_ENCODING)
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        let length = headers
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or(StatusCode::BAD_REQUEST)?;
        if length == 0 || length > MAX_CIPHERTEXT {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        if !self.available() {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let permit = self
            .processing
            .clone()
            .try_acquire_owned()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let mut incoming = request.into_body();
        let mut body = Vec::with_capacity(length);
        while let Some(frame) = incoming.frame().await {
            let data = frame
                .map_err(|_| StatusCode::BAD_REQUEST)?
                .into_data()
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            if body
                .len()
                .checked_add(data.len())
                .filter(|n| *n <= length)
                .is_none()
            {
                return Err(StatusCode::PAYLOAD_TOO_LARGE);
            }
            body.extend_from_slice(&data);
        }
        if body.len() != length {
            return Err(StatusCode::BAD_REQUEST);
        }
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            self.forward(body)
        })
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
    }
    pub async fn serve(
        self: Arc<Self>,
        listener: TcpListener,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<()> {
        if !listener
            .local_addr()
            .map_err(|_| Error::Gateway)?
            .ip()
            .is_loopback()
        {
            return Err(Error::InvalidPolicy);
        }
        let capacity = Arc::new(Semaphore::new(32));
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {
                changed=shutdown.changed()=>{if changed.is_err()||*shutdown.borrow(){break;}},
                accepted=listener.accept()=>{
                    let(stream,_peer)=accepted.map_err(|_|Error::Gateway)?;let Ok(permit)=capacity.clone().try_acquire_owned() else{drop(stream);continue;};let relay=self.clone();
                    tasks.spawn(async move {
                        let _permit=permit;let mut server=hyper::server::conn::http1::Builder::new();server.keep_alive(false).max_headers(24).max_buf_size(8192).timer(TokioTimer::new()).header_read_timeout(Duration::from_secs(3));
                        let _=tokio::time::timeout(Duration::from_secs(60),server.serve_connection(TokioIo::new(stream),service_fn(move|request|relay.clone().request(request)))).await;
                    });
                },
                _=tasks.join_next(),if !tasks.is_empty()=>{},
            }
        }
        while tasks.join_next().await.is_some() {}
        Ok(())
    }
}

pub fn run(config: RelayConfig) -> Result<()> {
    let relay = Arc::new(ObliviousRelay::new(&config)?);
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).max_blocking_threads(4).enable_all().build().map_err(|_|Error::Gateway)?.block_on(async move {
        let listener=TcpListener::bind(config.listen).await.map_err(|_|Error::Gateway)?;
        println!("{}",serde_json::json!({"opaque_relay":true,"listen":listener.local_addr().map_err(|_|Error::Gateway)?.to_string(),"independent_operator_verified":false}));
        let(stop,receiver)=watch::channel(false);let serving=relay.serve(listener,receiver);tokio::pin!(serving);
        tokio::select!{result=&mut serving=>result,signal=tokio::signal::ctrl_c()=>{signal.map_err(|_|Error::Gateway)?;let _=stop.send(true);serving.await}}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_has_no_secrets_arbitrary_gateway_or_credential_forwarding() {
        let mut config = RelayConfig {
            schema_version: 1,
            listen: "127.0.0.1:8081".parse().unwrap(),
            gateway_url: "https://gateway.example.org/ohttp".into(),
            cors_origin: Some("https://wallet.example.org".into()),
        };
        assert!(config.validate(false).is_ok());
        for url in [
            "http://gateway.example.org/ohttp",
            "https://user:pass@gateway.example.org/ohttp",
            "https://gateway.example.org/ohttp?target=other",
            "https://gateway.example.org/arbitrary",
        ] {
            config.gateway_url = url.into();
            assert!(config.validate(false).is_err());
        }
        assert!(serde_json::from_str::<RelayConfig>(r#"{"schema_version":1,"listen":"127.0.0.1:8081","gateway_url":"https://gateway.example.org/ohttp","cors_origin":null,"treasury_secret":"forbidden"}"#).is_err());
    }
    #[test]
    fn circuit_breaker_bounds_repeat_gateway_failures() {
        let config = RelayConfig {
            schema_version: 1,
            listen: "127.0.0.1:0".parse().unwrap(),
            gateway_url: "https://gateway.example.org/ohttp".into(),
            cors_origin: None,
        };
        let relay = ObliviousRelay::new(&config).unwrap();
        for _ in 0..4 {
            relay.record_health(false);
            assert!(relay.available());
        }
        relay.record_health(false);
        assert!(!relay.available());
        relay.circuit.lock().unwrap().blocked_until = Some(Instant::now() - Duration::from_secs(1));
        assert!(relay.available());
        assert_eq!(relay.processing.available_permits(), 4);
    }
}
