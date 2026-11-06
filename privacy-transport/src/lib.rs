//! RFC 9458 / RFC 9292 encryption boundary for private wallet operations.
//! This crate never opens a socket and never falls back to a direct request.
//! The caller supplies a previously authenticated, shared gateway configuration.
//! An independent relay and reviewed configuration distribution remain deployment gates.

#![forbid(unsafe_code)]

use bhttp::{Message, Mode};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use zeroize::Zeroizing;

pub const OHTTP_REQUEST_CONTENT_TYPE: &str = "message/ohttp-req";
pub const OHTTP_RESPONSE_CONTENT_TYPE: &str = "message/ohttp-res";
pub const MAX_BODY_BYTES: usize = 524_288;
pub const MAX_ENVELOPE_BYTES: usize = MAX_BODY_BYTES + 16_384;
const PAD_BLOCK: usize = 4096;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TransportError {
    #[error("Untrusted or unsupported privacy gateway configuration")]
    Configuration,
    #[error("Private request exceeds its size limit")]
    Size,
    #[error("Privacy transport cryptographic authentication failed")]
    Authentication,
    #[error("Invalid privacy gateway response")]
    Response,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    FeeSchedule,
    Submit,
    Status,
    Pool,
}

impl Operation {
    pub fn path(self) -> &'static str {
        match self {
            Self::FeeSchedule => "/v1/private/fees",
            Self::Submit => "/v1/private/submit",
            Self::Status => "/v1/private/status",
            Self::Pool => "/v1/private/pool",
        }
    }
}

/// A pin authenticates a shared configuration only when the caller obtained the
/// pin through its reviewed application manifest; hashing an arbitrary fetched
/// configuration and trusting that hash is NOT an authentication mechanism.
pub struct GatewayConfig {
    authority: String,
    encoded_key: Vec<u8>,
}

impl GatewayConfig {
    pub fn from_pinned_key(
        authority: String,
        encoded_key: Vec<u8>,
        trusted_sha256: [u8; 32],
    ) -> Result<Self, TransportError> {
        let valid_authority = authority.len() <= 253
            && authority.contains('.')
            && authority.split('.').all(|part| {
                !part.is_empty()
                    && part.len() <= 63
                    && part.as_bytes()[0].is_ascii_alphanumeric()
                    && part.as_bytes()[part.len() - 1].is_ascii_alphanumeric()
                    && part
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            });
        if !valid_authority || encoded_key.is_empty() || encoded_key.len() > 4096 {
            return Err(TransportError::Configuration);
        }
        let digest: [u8; 32] = Sha256::digest(&encoded_key).into();
        if digest != trusted_sha256 {
            return Err(TransportError::Configuration);
        }
        ohttp::ClientRequest::from_encoded_config(&encoded_key)
            .map_err(|_| TransportError::Configuration)?;
        Ok(Self {
            authority,
            encoded_key,
        })
    }

    pub fn seal(
        &self,
        operation: Operation,
        public_envelope_json: &[u8],
    ) -> Result<SealedRequest, TransportError> {
        if public_envelope_json.is_empty() || public_envelope_json.len() > MAX_BODY_BYTES {
            return Err(TransportError::Size);
        }
        // The upper layer validates its closed public schema before calling us.
        // Fixed headers prevent callers from adding user-agent, cookies or IDs.
        let mut message = Message::request(
            b"POST".to_vec(),
            b"https".to_vec(),
            self.authority.as_bytes().to_vec(),
            operation.path().as_bytes().to_vec(),
        );
        message.put_header(b"content-type".to_vec(), b"application/json".to_vec());
        message.put_header(b"accept".to_vec(), b"application/json".to_vec());
        message.write_content(public_envelope_json);
        let mut plain = Zeroizing::new(Vec::new());
        message
            .write_bhttp(Mode::KnownLength, &mut *plain)
            .map_err(|_| TransportError::Response)?;
        let length = plain
            .len()
            .checked_add(PAD_BLOCK - 1)
            .ok_or(TransportError::Size)?
            / PAD_BLOCK
            * PAD_BLOCK;
        if length > MAX_ENVELOPE_BYTES - 512 {
            return Err(TransportError::Size);
        }
        plain.resize(length, 0);
        let client = ohttp::ClientRequest::from_encoded_config(&self.encoded_key)
            .map_err(|_| TransportError::Configuration)?;
        let (ciphertext, response) = client
            .encapsulate(&plain)
            .map_err(|_| TransportError::Authentication)?;
        Ok(SealedRequest {
            ciphertext,
            response,
        })
    }
}

/// Intentionally not Clone or Debug. The response context is single-use and
/// never serialized into a log, database or HTTP request.
pub struct SealedRequest {
    ciphertext: Vec<u8>,
    response: ohttp::ClientResponse,
}

impl SealedRequest {
    pub fn ciphertext(&self) -> &[u8] {
        &self.ciphertext
    }

    /// Consumes the response key context even if authentication fails. Retry a
    /// logical operation with a fresh OHTTP envelope and the SAME payment body.
    pub fn open_response(
        self,
        outer_status: u16,
        content_type: &str,
        ciphertext: &[u8],
    ) -> Result<PrivateResponse, TransportError> {
        if outer_status != 200 || content_type != OHTTP_RESPONSE_CONTENT_TYPE {
            return Err(TransportError::Response);
        }
        if ciphertext.is_empty() || ciphertext.len() > MAX_ENVELOPE_BYTES {
            return Err(TransportError::Size);
        }
        let plain = Zeroizing::new(
            self.response
                .decapsulate(ciphertext)
                .map_err(|_| TransportError::Authentication)?,
        );
        if plain.len() > MAX_ENVELOPE_BYTES {
            return Err(TransportError::Size);
        }
        let mut cursor = Cursor::new(plain.as_slice());
        let response = Message::read_bhttp(&mut cursor).map_err(|_| TransportError::Response)?;
        let status = response
            .control()
            .status()
            .ok_or(TransportError::Response)?
            .code();
        if !matches!(status, 200 | 202 | 400 | 409 | 422 | 429 | 503)
            || !response.informational().is_empty()
            || !response.trailer().is_empty()
            || response.content().len() > MAX_BODY_BYTES
        {
            return Err(TransportError::Response);
        }
        let fields = response.header().fields();
        if fields.len() != 1
            || fields[0].name() != b"content-type"
            || fields[0].value() != b"application/json"
        {
            return Err(TransportError::Response);
        }
        let used = usize::try_from(cursor.position()).map_err(|_| TransportError::Response)?;
        if plain
            .get(used..)
            .ok_or(TransportError::Response)?
            .iter()
            .any(|b| *b != 0)
        {
            return Err(TransportError::Response);
        }
        Ok(PrivateResponse {
            status,
            body: response.content().to_vec(),
        })
    }
}

pub struct PrivateResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohttp::hpke::{Aead, Kdf, Kem};
    use ohttp::{KeyConfig, Server, SymmetricSuite};

    fn gateway() -> (GatewayConfig, Server) {
        let config = KeyConfig::new(
            1,
            Kem::X25519Sha256,
            vec![SymmetricSuite::new(Kdf::HkdfSha256, Aead::Aes128Gcm)],
        )
        .unwrap();
        let server = Server::new(config).unwrap();
        let key = server.config().encode().unwrap();
        let pin = Sha256::digest(&key).into();
        (
            GatewayConfig::from_pinned_key("gas.example.org".into(), key, pin).unwrap(),
            server,
        )
    }

    fn response_bytes(status: u16) -> Vec<u8> {
        let mut m = Message::response(bhttp::StatusCode::try_from(status).unwrap());
        m.put_header(b"content-type".to_vec(), b"application/json".to_vec());
        m.write_content(b"{\"state\":\"unknown\"}");
        let mut b = Vec::new();
        m.write_bhttp(Mode::KnownLength, &mut b).unwrap();
        b
    }

    #[test]
    fn real_hpke_roundtrip_has_fixed_headers_padding_and_private_body() {
        let (config, server) = gateway();
        let body = br#"{"operation_digest":"opaque-fixture"}"#;
        let sealed = config.seal(Operation::Submit, body).unwrap();
        assert!(!sealed.ciphertext().windows(body.len()).any(|w| w == body));
        let (plain, reply) = server.decapsulate(sealed.ciphertext()).unwrap();
        assert_eq!(plain.len() % PAD_BLOCK, 0);
        let message = Message::read_bhttp(&mut Cursor::new(plain)).unwrap();
        assert_eq!(message.control().method(), Some(b"POST".as_slice()));
        assert_eq!(
            message.control().authority(),
            Some(b"gas.example.org".as_slice())
        );
        assert_eq!(
            message.control().path(),
            Some(b"/v1/private/submit".as_slice())
        );
        assert_eq!(message.header().fields().len(), 2);
        assert_eq!(message.content(), body);
        let response = reply.encapsulate(&response_bytes(202)).unwrap();
        let result = sealed
            .open_response(200, OHTTP_RESPONSE_CONTENT_TYPE, &response)
            .unwrap();
        assert_eq!(result.status, 202);
        assert_eq!(result.body, br#"{"state":"unknown"}"#);
    }

    #[test]
    fn pin_and_authority_are_mandatory() {
        let (_, server) = gateway();
        let key = server.config().encode().unwrap();
        assert!(
            GatewayConfig::from_pinned_key("gas.example.org".into(), key.clone(), [0; 32]).is_err()
        );
        for host in [
            "user@gas.example.org",
            "gas.example.org:443",
            "gas.example.org/",
            "-a.example.org",
            "GAS.example.org",
        ] {
            assert!(GatewayConfig::from_pinned_key(
                host.into(),
                key.clone(),
                Sha256::digest(&key).into()
            )
            .is_err());
        }
    }

    #[test]
    fn every_attempt_has_new_encryption_but_same_payment() {
        let (config, server) = gateway();
        let a = config.seal(Operation::Status, b"{}").unwrap();
        let b = config.seal(Operation::Status, b"{}").unwrap();
        assert_ne!(a.ciphertext(), b.ciphertext());
        let (pa, _) = server.decapsulate(a.ciphertext()).unwrap();
        let (pb, _) = server.decapsulate(b.ciphertext()).unwrap();
        assert_eq!(pa, pb);
    }

    #[test]
    fn tampering_and_cross_request_responses_are_rejected() {
        let (config, server) = gateway();
        let a = config.seal(Operation::Submit, b"{}").unwrap();
        let b = config.seal(Operation::Submit, b"{}").unwrap();
        let (_, reply) = server.decapsulate(a.ciphertext()).unwrap();
        let response = reply.encapsulate(&response_bytes(200)).unwrap();
        assert!(b
            .open_response(200, OHTTP_RESPONSE_CONTENT_TYPE, &response)
            .is_err());
        let mut tampered = response;
        let n = tampered.len();
        tampered[n - 1] ^= 1;
        assert!(a
            .open_response(200, OHTTP_RESPONSE_CONTENT_TYPE, &tampered)
            .is_err());
    }

    #[test]
    fn no_redirect_plaintext_or_oversized_fallback() {
        let (config, _) = gateway();
        assert!(config
            .seal(Operation::Submit, &vec![0; MAX_BODY_BYTES + 1])
            .is_err());
        for (status, content) in [
            (302, OHTTP_RESPONSE_CONTENT_TYPE),
            (200, "application/json"),
        ] {
            let request = config.seal(Operation::Submit, b"{}").unwrap();
            assert!(request.open_response(status, content, b"{}").is_err());
        }
    }
}
