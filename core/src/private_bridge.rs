//! Native bridge for the separately encrypted private-payment journal and OHTTP.
//!
//! There is no private signing/proving or network API in this module. These
//! primitives do not open the production release gate or attest a ZK proof.

use crate::{private_journal, private_payment, WalletError};
use std::sync::{Arc, Mutex};
use wallet_privacy_transport::{GatewayConfig, Operation, SealedRequest, TransportError};

fn json<T: serde::Serialize>(value: T) -> Result<String, WalletError> {
    serde_json::to_string(&value).map_err(|_| WalletError::Crypto)
}

#[uniffi::export]
pub fn private_capabilities() -> String {
    serde_json::json!({
        "schema_version": 1,
        "enabled": false,
        "protocol": private_payment::CANDIDATE_PROTOCOL,
        "candidate_chain_id": private_payment::CANDIDATE_CHAIN_ID,
        "candidate_asset": "USDC",
        "journal_schema": 2,
        "ohttp_client_library": true,
        "proof_verification": false,
        "android_prover": false,
        "deployed_privacy_transport": false,
        "release_review": false,
        "public_fallback": false
    })
    .to_string()
}

/// Seal local approval fields. This does not prove or authorize an on-chain spend.
#[uniffi::export]
pub fn private_prepare_intent(input_json: String) -> Result<String, WalletError> {
    private_payment::seal_intent_json(&input_json)
}

/// Validates only closed-schema shape and digest binding, never a ZK proof.
#[uniffi::export]
pub fn private_validate_envelope(envelope_json: String) -> Result<String, WalletError> {
    let envelope = private_payment::validate_envelope_json(&envelope_json)?;
    private_payment::canonical_envelope(&envelope)
}

/// Bind exact public proof/call bytes to one operation digest. This is hashing
/// and shape validation only, never cryptographic proof acceptance.
#[uniffi::export]
pub fn private_seal_envelope(envelope_json: String) -> Result<String, WalletError> {
    private_payment::seal_envelope_json(&envelope_json)
}

#[uniffi::export]
pub fn private_maximum_transfer(input_json: String) -> Result<String, WalletError> {
    private_payment::maximum_transfer_json(&input_json)
}

/// Open only a new, private-journal namespace with a device-protected independent
/// key. Opening this object does not migrate the existing public wallet database.
#[derive(uniffi::Object)]
pub struct PrivatePaymentJournal {
    pub(crate) inner: private_journal::Journal,
}

#[uniffi::export]
impl PrivatePaymentJournal {
    #[uniffi::constructor]
    pub fn new(path: String, identity: String, key: Vec<u8>) -> Result<Arc<Self>, WalletError> {
        Ok(Arc::new(Self {
            inner: private_journal::Journal::open(path, identity, key)?,
        }))
    }

    pub fn record_approved(&self, intent_json: String) -> Result<String, WalletError> {
        self.inner.record_approved(intent_json)
    }
    pub fn record_ready(
        &self,
        intent_id: String,
        envelope_json: String,
    ) -> Result<String, WalletError> {
        self.inner.record_ready(intent_id, envelope_json)
    }
    pub fn mark_released(&self, intent_id: String) -> Result<String, WalletError> {
        self.inner.mark_released(intent_id)
    }
    pub fn mark_unknown(&self, intent_id: String) -> Result<String, WalletError> {
        self.inner.mark_unknown(intent_id)
    }
    pub fn observe_receipt(
        &self,
        intent_id: String,
        observation_json: String,
    ) -> Result<String, WalletError> {
        self.inner.observe_receipt(intent_id, observation_json)
    }
    pub fn mark_reorg(
        &self,
        intent_id: String,
        observation_json: String,
    ) -> Result<String, WalletError> {
        self.inner.mark_reorg(intent_id, observation_json)
    }
    pub fn cancel_approved(&self, intent_id: String) -> Result<String, WalletError> {
        self.inner.cancel_approved(intent_id)
    }
    pub fn get_payment(&self, intent_id: String) -> Result<String, WalletError> {
        self.inner.get_payment(intent_id)
    }
    pub fn list_payments(&self) -> Result<String, WalletError> {
        self.inner.list_payments()
    }
    pub fn list_events(&self, intent_id: String) -> Result<String, WalletError> {
        self.inner.list_events(intent_id)
    }
}

fn transport_error(_: TransportError) -> WalletError {
    // Do not propagate request bodies, provider messages, secrets or ciphertext.
    WalletError::Policy {
        reason: "Privacy transport authentication or response validation failed".into(),
    }
}

/// The pin must come from the reviewed shared release manifest, not from the
/// same untrusted response as the key. No live configuration ships in this build.
#[derive(uniffi::Object)]
pub struct PrivateOhttpGateway {
    inner: GatewayConfig,
}

#[uniffi::export]
impl PrivateOhttpGateway {
    #[uniffi::constructor]
    pub fn new(
        authority: String,
        encoded_key: Vec<u8>,
        trusted_sha256: Vec<u8>,
    ) -> Result<Arc<Self>, WalletError> {
        let pin: [u8; 32] = trusted_sha256.try_into().map_err(|_| WalletError::Policy {
            reason: "Invalid shared privacy configuration pin".into(),
        })?;
        Ok(Arc::new(Self {
            inner: GatewayConfig::from_pinned_key(authority, encoded_key, pin)
                .map_err(transport_error)?,
        }))
    }

    /// Intentionally accepts no wallet, account, address, balance or amount.
    pub fn seal_pool_request(
        &self,
        request_json: String,
    ) -> Result<Arc<PrivateOhttpRequest>, WalletError> {
        let body = crate::private_pool::validate(&request_json)?;
        self.seal(Operation::Pool, &body)
    }

    /// Intentionally accepts no wallet, account, address, balance or amount.
    pub fn seal_fee_schedule(&self) -> Result<Arc<PrivateOhttpRequest>, WalletError> {
        let body = serde_json::json!({
            "schema_version": 1,
            "protocol": private_payment::CANDIDATE_PROTOCOL,
            "chain_id": private_payment::CANDIDATE_CHAIN_ID,
            "asset": "USDC"
        })
        .to_string();
        self.seal(Operation::FeeSchedule, &body)
    }

    /// A strict public envelope is the only accepted submission schema. The
    /// caller still needs local proof/ciphertext verification and durable release.
    pub fn seal_submission(
        &self,
        envelope_json: String,
    ) -> Result<Arc<PrivateOhttpRequest>, WalletError> {
        let body = private_validate_envelope(envelope_json)?;
        self.seal(Operation::Submit, &body)
    }

    pub fn seal_status(
        &self,
        operation_digest: String,
    ) -> Result<Arc<PrivateOhttpRequest>, WalletError> {
        private_payment::canonical_hash(&operation_digest)?;
        let body = serde_json::json!({ "schema_version": 1, "operation_digest": operation_digest })
            .to_string();
        self.seal(Operation::Status, &body)
    }
}

impl PrivateOhttpGateway {
    fn seal(
        &self,
        operation: Operation,
        body: &str,
    ) -> Result<Arc<PrivateOhttpRequest>, WalletError> {
        let request = self
            .inner
            .seal(operation, body.as_bytes())
            .map_err(transport_error)?;
        Ok(Arc::new(PrivateOhttpRequest {
            inner: Mutex::new(Some(request)),
        }))
    }
}

/// Opaque, single-response HPKE context. It contains no socket or fallback URL.
#[derive(uniffi::Object)]
pub struct PrivateOhttpRequest {
    inner: Mutex<Option<SealedRequest>>,
}

#[uniffi::export]
impl PrivateOhttpRequest {
    pub fn ciphertext(&self) -> Result<Vec<u8>, WalletError> {
        self.inner
            .lock()
            .map_err(|_| WalletError::Crypto)?
            .as_ref()
            .map(|request| request.ciphertext().to_vec())
            .ok_or(WalletError::Conflict {
                reason: "Privacy response context was already consumed".into(),
            })
    }

    /// Consumes this context even when the response is rejected. An application
    /// retry must encrypt the SAME journaled operation using a fresh context.
    pub fn open_response(
        &self,
        outer_status: u16,
        content_type: String,
        ciphertext: Vec<u8>,
    ) -> Result<String, WalletError> {
        let request = self
            .inner
            .lock()
            .map_err(|_| WalletError::Crypto)?
            .take()
            .ok_or(WalletError::Conflict {
                reason: "Privacy response context was already consumed".into(),
            })?;
        let response = request
            .open_response(outer_status, &content_type, &ciphertext)
            .map_err(transport_error)?;
        let body = String::from_utf8(response.body).map_err(|_| WalletError::Policy {
            reason: "Invalid privacy response encoding".into(),
        })?;
        json(serde_json::json!({ "status": response.status, "body_json": body }))
    }
}
