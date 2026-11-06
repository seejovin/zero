//! RFC 9458 gateway and bounded loopback HTTP ingress. Only OHTTP ciphertext is
//! accepted over HTTP. Deployment must place an independent relay and reviewed
//! TLS termination in front of this listener; this module keeps no request logs.
use bhttp::{Message, Mode};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, header, service::service_fn, Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use ohttp::hpke::{Aead, Kdf, Kem};
use ohttp::{KeyConfig, Server, SymmetricSuite};
use serde::{Deserialize, Serialize};
use std::{
    convert::Infallible,
    io::Cursor,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::TcpListener,
    sync::{watch, Semaphore},
    task::JoinSet,
};
use wallet_core::private_payment::{
    validate_envelope_json, CANDIDATE_CHAIN_ID, CANDIDATE_PROTOCOL,
};
use zeroize::Zeroizing;

use crate::fee_schedule::SignedFeeSchedule;
use crate::model::{decode, encode, fixed_hex};
use crate::rpc::{ArbitrumRpc, PoolRequest};
use crate::secrets::SecretSource;
use crate::{Error, GasService, JobPhase, Result};

pub const REQUEST_CONTENT_TYPE: &str = "message/ohttp-req";
pub const RESPONSE_CONTENT_TYPE: &str = "message/ohttp-res";
pub const MAX_ENVELOPE_BYTES: usize = 540_672;
const MAX_BODY_BYTES: usize = 524_288;
const PAD_BLOCK: usize = 4096;
const MAX_CONNECTIONS: usize = 32;
const MAX_PROCESSING: usize = 4;
// Allow the separately bounded proof (20s) and RPC (20s) stages to finish while
// retaining a hard connection cap. Cancelled clients still cannot free an active
// blocking-work permit early or turn an admitted request into "failed".
const CONNECTION_DEADLINE: Duration = Duration::from_secs(55);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeeRequest {
    schema_version: u8,
    protocol: String,
    chain_id: u64,
    asset: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusRequest {
    schema_version: u8,
    operation_digest: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SubmissionResponse {
    pub operation_digest: String,
    /// Durable admission only. This never represents inclusion or finality.
    pub accepted: bool,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdvisoryStatus {
    pub schema_version: u8,
    pub operation_digest: String,
    pub state: String,
    pub transaction_hash: Option<String>,
}

pub struct ObliviousGateway {
    server: Server,
    authority: String,
    service: Arc<GasService>,
    schedule: SignedFeeSchedule,
    processing: Arc<Semaphore>,
    pool_rpc: Option<Arc<ArbitrumRpc>>,
}

impl ObliviousGateway {
    /// Derive only the public transport configuration. No journal is opened and
    /// no proof adapter, RPC client or treasury signer is constructed.
    pub fn public_key_from_secret(key_id: u8, secret: &SecretSource) -> Result<Vec<u8>> {
        let seed = secret.read()?;
        Self::key_config(key_id, &seed)?
            .encode()
            .map_err(|_| Error::Gateway)
    }

    fn key_config(key_id: u8, seed: &[u8; 32]) -> Result<KeyConfig> {
        KeyConfig::derive(
            key_id,
            Kem::X25519Sha256,
            vec![SymmetricSuite::new(Kdf::HkdfSha256, Aead::Aes128Gcm)],
            seed,
        )
        .map_err(|_| Error::Gateway)
    }

    pub fn from_secret(
        authority: String,
        key_id: u8,
        secret: &SecretSource,
        service: Arc<GasService>,
        schedule: SignedFeeSchedule,
        pinned_fee_public_key: [u8; 32],
    ) -> Result<Self> {
        Self::from_seed(
            authority,
            key_id,
            secret.read()?,
            service,
            schedule,
            pinned_fee_public_key,
        )
    }

    pub(crate) fn from_seed(
        authority: String,
        key_id: u8,
        seed: Zeroizing<[u8; 32]>,
        service: Arc<GasService>,
        schedule: SignedFeeSchedule,
        pinned_fee_public_key: [u8; 32],
    ) -> Result<Self> {
        validate_authority(&authority)?;
        service.install_fee_schedule(&schedule, pinned_fee_public_key)?;
        let config = Self::key_config(key_id, &seed)?;
        let server = Server::new(config).map_err(|_| Error::Gateway)?;
        Ok(Self {
            server,
            authority,
            service,
            schedule,
            processing: Arc::new(Semaphore::new(MAX_PROCESSING)),
            pool_rpc: None,
        })
    }

    pub fn with_pool_rpc(mut self, rpc: Arc<ArbitrumRpc>) -> Self {
        self.pool_rpc = Some(rpc);
        self
    }

    /// Public encoded HPKE key configuration, suitable for an independently
    /// authenticated release manifest. This method never exports the seed.
    pub fn public_configuration(&self) -> Result<Vec<u8>> {
        self.server.config().encode().map_err(|_| Error::Gateway)
    }

    /// Genuine OHTTP authenticated decryption and response encryption. Invalid
    /// ciphertext cannot be used to obtain plaintext errors about its contents.
    pub fn handle_ciphertext(&self, ciphertext: &[u8], now: u64) -> Result<Vec<u8>> {
        if ciphertext.is_empty() || ciphertext.len() > MAX_ENVELOPE_BYTES {
            return Err(Error::Gateway);
        }
        let (plain, response_context) = self
            .server
            .decapsulate(ciphertext)
            .map_err(|_| Error::Gateway)?;
        let plain = Zeroizing::new(plain);
        if plain.len() > MAX_ENVELOPE_BYTES {
            return Err(Error::Gateway);
        }
        let decoded = self.decode_inner(&plain);
        let (status, body) = match decoded {
            Ok((path, body)) => self.dispatch(&path, &body, now),
            Err(_) => (400, br#"{"code":"invalid_request"}"#.to_vec()),
        };
        let mut message =
            Message::response(bhttp::StatusCode::try_from(status).map_err(|_| Error::Gateway)?);
        message.put_header(b"content-type".to_vec(), b"application/json".to_vec());
        message.write_content(&body);
        let mut response = Zeroizing::new(Vec::new());
        message
            .write_bhttp(Mode::KnownLength, &mut *response)
            .map_err(|_| Error::Gateway)?;
        let padded = response
            .len()
            .checked_add(PAD_BLOCK - 1)
            .ok_or(Error::Gateway)?
            / PAD_BLOCK
            * PAD_BLOCK;
        if padded > MAX_ENVELOPE_BYTES - 512 {
            return Err(Error::Gateway);
        }
        response.resize(padded, 0);
        response_context
            .encapsulate(&response)
            .map_err(|_| Error::Gateway)
    }

    fn decode_inner(&self, plain: &[u8]) -> Result<(String, Vec<u8>)> {
        let mut cursor = Cursor::new(plain);
        let message = Message::read_bhttp(&mut cursor).map_err(|_| Error::Gateway)?;
        let control = message.control();
        if control.method() != Some(b"POST".as_slice())
            || control.scheme() != Some(b"https".as_slice())
            || control.authority() != Some(self.authority.as_bytes())
            || !message.informational().is_empty()
            || !message.trailer().is_empty()
            || message.content().is_empty()
            || message.content().len() > MAX_BODY_BYTES
        {
            return Err(Error::Gateway);
        }
        let path = control.path().ok_or(Error::Gateway)?;
        if ![
            b"/v1/private/fees".as_slice(),
            b"/v1/private/submit",
            b"/v1/private/status",
            b"/v1/private/pool",
        ]
        .contains(&path)
        {
            return Err(Error::Gateway);
        }
        let headers = message.header().fields();
        if headers.len() != 2
            || headers
                .iter()
                .filter(|f| f.name() == b"content-type" && f.value() == b"application/json")
                .count()
                != 1
            || headers
                .iter()
                .filter(|f| f.name() == b"accept" && f.value() == b"application/json")
                .count()
                != 1
        {
            return Err(Error::Gateway);
        }
        let consumed = usize::try_from(cursor.position()).map_err(|_| Error::Gateway)?;
        if plain
            .get(consumed..)
            .ok_or(Error::Gateway)?
            .iter()
            .any(|b| *b != 0)
        {
            return Err(Error::Gateway);
        }
        Ok((
            String::from_utf8(path.to_vec()).map_err(|_| Error::Gateway)?,
            message.content().to_vec(),
        ))
    }

    fn dispatch(&self, path: &str, body: &[u8], now: u64) -> (u16, Vec<u8>) {
        let result = self.dispatch_checked(path, body, now);
        match result {
            Ok(response) => response,
            Err(Error::ReleaseUnavailable | Error::IntegrationUnavailable | Error::Halted) => {
                (503, br#"{"code":"unavailable"}"#.to_vec())
            }
            Err(Error::Conflict) => (409, br#"{"code":"conflict"}"#.to_vec()),
            Err(Error::BudgetExceeded) => (429, br#"{"code":"capacity"}"#.to_vec()),
            Err(_) => (422, br#"{"code":"rejected"}"#.to_vec()),
        }
    }

    fn dispatch_checked(&self, path: &str, body: &[u8], now: u64) -> Result<(u16, Vec<u8>)> {
        let text = std::str::from_utf8(body).map_err(|_| Error::Gateway)?;
        match path {
            "/v1/private/submit" => {
                let envelope = validate_envelope_json(text).map_err(|_| Error::InvalidEncoding)?;
                let digest = envelope.operation_digest;
                match self.service.admit(&digest, text, now) {
                    Ok(_) => Ok((
                        202,
                        encode(&SubmissionResponse {
                            operation_digest: digest,
                            accepted: true,
                        })?
                        .into_bytes(),
                    )),
                    Err(
                        Error::ReleaseUnavailable | Error::IntegrationUnavailable | Error::Halted,
                    ) => Ok((
                        503,
                        encode(&SubmissionResponse {
                            operation_digest: digest,
                            accepted: false,
                        })?
                        .into_bytes(),
                    )),
                    Err(error) => Err(error),
                }
            }
            "/v1/private/fees" => {
                let request: FeeRequest = decode(text)?;
                if request.schema_version != 1
                    || request.protocol != CANDIDATE_PROTOCOL
                    || request.chain_id != CANDIDATE_CHAIN_ID
                    || request.asset != "USDC"
                {
                    return Err(Error::Gateway);
                }
                self.service.require_active()?;
                if now < self.schedule.schedule.valid_from
                    || now >= self.schedule.schedule.valid_until
                {
                    return Err(Error::PolicyMismatch);
                }
                Ok((200, encode(&self.schedule)?.into_bytes()))
            }
            "/v1/private/status" => {
                let request: StatusRequest = decode(text)?;
                if request.schema_version != 1 {
                    return Err(Error::Gateway);
                }
                fixed_hex(&request.operation_digest, 32)?;
                let (state, transaction_hash) =
                    match self.service.get_operation(&request.operation_digest) {
                        Ok(view) => (phase_name(view.phase).to_string(), view.transaction_hash),
                        Err(Error::NotFound) => ("not_found".into(), None),
                        Err(error) => return Err(error),
                    };
                Ok((
                    200,
                    encode(&AdvisoryStatus {
                        schema_version: 1,
                        operation_digest: request.operation_digest,
                        state,
                        transaction_hash,
                    })?
                    .into_bytes(),
                ))
            }
            "/v1/private/pool" => {
                let request: PoolRequest = decode(text)?;
                request.validate()?;
                let rpc = self
                    .pool_rpc
                    .as_ref()
                    .ok_or(Error::IntegrationUnavailable)?;
                Ok((200, encode(&rpc.pool_read(&request)?)?.into_bytes()))
            }
            _ => Err(Error::Gateway),
        }
    }

    /// Run only on loopback. No socket address, HTTP header, body or operation
    /// identifier is logged. Limits apply before HPKE and proof processing.
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
            return Err(Error::Gateway);
        }
        let connections = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {
                changed = shutdown.changed() => { if changed.is_err() || *shutdown.borrow() { break; } }
                accepted = listener.accept() => {
                    let (stream, _peer) = accepted.map_err(|_| Error::Gateway)?;
                    let Ok(permit) = connections.clone().try_acquire_owned() else { drop(stream); continue; };
                    let gateway = self.clone();
                    tasks.spawn(async move {
                        let _permit = permit;
                        let service = service_fn(move |request| gateway.clone().handle_http(request));
                        let mut builder = hyper::server::conn::http1::Builder::new();
                        builder.keep_alive(false).max_headers(12).max_buf_size(8192)
                            .timer(TokioTimer::new()).header_read_timeout(Duration::from_secs(3));
                        let _ = tokio::time::timeout(CONNECTION_DEADLINE,
                            builder.serve_connection(TokioIo::new(stream), service)).await;
                    });
                }
                _ = tasks.join_next(), if !tasks.is_empty() => {}
            }
        }
        while tasks.join_next().await.is_some() {}
        Ok(())
    }

    async fn handle_http(
        self: Arc<Self>,
        request: Request<Incoming>,
    ) -> std::result::Result<Response<Full<Bytes>>, Infallible> {
        let response = match self.http_checked(request).await {
            Ok(body) => http_response(StatusCode::OK, body),
            Err(status) => http_response(status, Vec::new()),
        };
        Ok(response)
    }

    async fn http_checked(
        self: Arc<Self>,
        request: Request<Incoming>,
    ) -> std::result::Result<Vec<u8>, StatusCode> {
        if request.method() != Method::POST
            || request.uri().path() != "/ohttp"
            || request.uri().query().is_some()
            || request.uri().scheme().is_some()
            || request.uri().authority().is_some()
        {
            return Err(StatusCode::NOT_FOUND);
        }
        let headers = request.headers();
        if headers.get_all(header::CONTENT_TYPE).iter().count() != 1
            || headers
                .get(header::CONTENT_TYPE)
                .and_then(|h| h.to_str().ok())
                != Some(REQUEST_CONTENT_TYPE)
            || headers.get_all(header::CONTENT_LENGTH).iter().count() != 1
            || headers.contains_key(header::TRANSFER_ENCODING)
            || headers.keys().any(|name| {
                !matches!(
                    name.as_str(),
                    "content-type" | "content-length" | "host" | "accept" | "connection"
                )
            })
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        let length = headers
            .get(header::CONTENT_LENGTH)
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or(StatusCode::BAD_REQUEST)?;
        if length == 0 || length > MAX_ENVELOPE_BYTES {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        let permit = self
            .processing
            .clone()
            .try_acquire_owned()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let mut incoming = request.into_body();
        let mut body = Vec::with_capacity(length);
        while let Some(frame) = incoming.frame().await {
            let frame = frame.map_err(|_| StatusCode::BAD_REQUEST)?;
            let data = frame.into_data().map_err(|_| StatusCode::BAD_REQUEST)?;
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
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .as_secs();
        tokio::task::spawn_blocking(move || {
            // Holding the permit inside this closure bounds work even if the
            // HTTP connection times out while an adapter is still running.
            let _permit = permit;
            self.handle_ciphertext(&body, now)
        })
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .map_err(|_| StatusCode::BAD_REQUEST)
    }
}

fn http_response(status: StatusCode, body: Vec<u8>) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(RESPONSE_CONTENT_TYPE),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::CONNECTION,
        header::HeaderValue::from_static("close"),
    );
    response
}

pub(crate) fn validate_authority(authority: &str) -> Result<()> {
    if authority.len() > 253
        || !authority.contains('.')
        || !authority.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
    {
        return Err(Error::Gateway);
    }
    Ok(())
}

fn phase_name(phase: JobPhase) -> &'static str {
    match phase {
        JobPhase::Admitted => "admitted",
        JobPhase::Signed => "signed",
        JobPhase::Unknown => "unknown",
        JobPhase::Pending => "pending",
        JobPhase::Included => "included",
        JobPhase::IncludedRevert => "included_revert",
        JobPhase::Finalized => "finalized",
        JobPhase::FinalizedRevert => "finalized_revert",
        JobPhase::RetryExhausted => "retry_exhausted",
        JobPhase::Reorged => "reorged",
        JobPhase::Incident => "incident",
    }
}
