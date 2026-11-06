//! Operator-visible types. No customer address, principal, mnemonic or witness
//! field is accepted by this crate. Unknown fields and duplicate struct fields
//! are rejected by serde, including nested public envelope types.
use primitive_types::U256;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use wallet_core::private_payment::{PrivateDomain, PrivateEnvelope, MAX_FEE_SCHEDULE_SECONDS};

use crate::{Error, Result};

pub(crate) const MAX_JSON: usize = 1_048_576;
pub(crate) const MAX_CALLDATA: usize = 262_144;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdmissionPolicy {
    pub schema_version: u8,
    pub domain: PrivateDomain,
    pub broadcaster_address: String,
    pub schedule_id: String,
    pub fee_recipient_commitment: String,
    pub fee_units: String,
    pub gas_limit: u64,
    pub max_native_fee_wei: String,
    /// Inclusive beginning and exclusive end of a SHARED fee schedule. This is
    /// admission policy, not a claim that an already released proof expires.
    pub valid_from: u64,
    pub valid_until: u64,
    pub max_pending_operations: u32,
    pub max_pending_native_wei: String,
    pub max_total_native_spend_wei: String,
    pub max_envelope_bytes: u32,
    /// Exact selectors from the chosen reviewed adapter, never arbitrary calls.
    pub allowed_selectors: Vec<String>,
}

impl AdmissionPolicy {
    pub fn parse(input: &str) -> Result<Self> {
        let policy: Self = decode(input)?;
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<()> {
        wallet_core::private_payment::validate_domain(&self.domain)
            .map_err(|_| Error::InvalidPolicy)?;
        if self.schema_version != 1
            || self.domain.protocol.is_empty()
            || self.domain.protocol.len() > 96
            || self.domain.chain_id == 0
            || self.domain.token_decimals > 36
            || self.valid_from == 0
            || self.valid_until <= self.valid_from
            || self.valid_until > i64::MAX as u64
            || self.valid_until - self.valid_from > MAX_FEE_SCHEDULE_SECONDS
            || self.max_pending_operations == 0
            || self.max_pending_operations > 1000
            || self.max_envelope_bytes == 0
            || self.max_envelope_bytes as usize > MAX_JSON
            || !(21_000..=10_000_000).contains(&self.gas_limit)
            || self.allowed_selectors.is_empty()
            || self.allowed_selectors.len() > 8
        {
            return Err(Error::InvalidPolicy);
        }
        for address in [
            &self.domain.pool_contract,
            &self.domain.token_contract,
            &self.broadcaster_address,
        ] {
            fixed_hex(address, 20)?;
        }
        for digest in [
            &self.domain.verifier_digest,
            &self.schedule_id,
            &self.fee_recipient_commitment,
        ] {
            fixed_hex(digest, 32)?;
        }
        let mut selectors = std::collections::HashSet::new();
        for selector in &self.allowed_selectors {
            fixed_hex(selector, 4)?;
            if !selectors.insert(selector) {
                return Err(Error::InvalidPolicy);
            }
        }
        let fee = decimal(&self.fee_units)?;
        let operation_cap = decimal(&self.max_native_fee_wei)?;
        let pending_cap = decimal(&self.max_pending_native_wei)?;
        let total_cap = decimal(&self.max_total_native_spend_wei)?;
        if fee.is_zero()
            || operation_cap.is_zero()
            || operation_cap > U256::from(1_000_000_000_000_000_000u64)
            || pending_cap < operation_cap
            || total_cap < operation_cap
        {
            return Err(Error::InvalidPolicy);
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String> {
        self.validate()?;
        Ok(digest(b"local-wallet/gas-policy/v1\0", &encode(self)?))
    }

    pub(crate) fn matches(&self, envelope: &PrivateEnvelope, now: u64) -> Result<()> {
        if envelope.domain != self.domain
            || envelope.fee.schedule_id != self.schedule_id
            || envelope.fee.fee_recipient_commitment != self.fee_recipient_commitment
            || envelope.fee.fee_units != self.fee_units
            || envelope.fee.gas_limit != self.gas_limit
            || envelope.fee.max_native_fee_wei != self.max_native_fee_wei
            || envelope.fee.quote_valid_until != self.valid_until
            || now < self.valid_from
            || now >= self.valid_until
        {
            return Err(Error::PolicyMismatch);
        }
        Ok(())
    }
}

/// Produced by a reviewed private-protocol adapter. Producing this value is NOT
/// proof verification: the adapter must actually verify the proof, its expected
/// state transition, the fee opening and all ciphertext/commitment bindings.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VerifiedOperation {
    pub operation_digest: String,
    pub tree_number: u64,
    pub call_data: String,
    pub fee_output_commitment: String,
    /// L1 data/operator charges not included in gas_limit * max_fee_per_gas.
    pub additional_native_fee_cap_wei: String,
}

impl std::fmt::Debug for VerifiedOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedOperation")
            .field("operation_digest", &self.operation_digest)
            .field("call_data", &"[REDACTED]")
            .field(
                "additional_native_fee_cap_wei",
                &self.additional_native_fee_cap_wei,
            )
            .finish()
    }
}

impl VerifiedOperation {
    pub(crate) fn validate(
        &self,
        envelope: &PrivateEnvelope,
        policy: &AdmissionPolicy,
    ) -> Result<()> {
        let bytes = bounded_hex(&self.call_data, MAX_CALLDATA)?;
        if bytes.len() < 4
            || self.tree_number > 65_535
            || self.tree_number != u64::from(envelope.tree_number)
            || !policy
                .allowed_selectors
                .iter()
                .any(|s| s == &self.call_data[..10])
            || self.operation_digest != envelope.operation_digest
            || self.call_data != envelope.call_data
            || self.fee_output_commitment != envelope.fee.fee_output_commitment
            || decimal(&self.additional_native_fee_cap_wei)?
                >= decimal(&envelope.fee.max_native_fee_wei)?
        {
            return Err(Error::VerificationFailed);
        }
        Ok(())
    }
}

/// An opening native treasury balance, checked by the chain adapter. There is
/// deliberately no silent balance-adjustment or arbitrary funding-credit API.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TreasuryAnchor {
    pub chain_id: u64,
    pub broadcaster_address: String,
    pub balance_wei: String,
    pub block_number: u64,
    pub block_hash: String,
}

impl TreasuryAnchor {
    pub(crate) fn validate(&self, policy: &AdmissionPolicy) -> Result<()> {
        if self.chain_id != policy.domain.chain_id
            || self.broadcaster_address != policy.broadcaster_address
            || self.block_number == 0
            || decimal(&self.balance_wei)?.is_zero()
        {
            return Err(Error::InvalidEvidence);
        }
        fixed_hex(&self.block_hash, 32)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum PrivateExecution {
    Accepted {
        operation_digest: String,
        tree_number: u64,
        nullifiers: Vec<String>,
        output_commitments: Vec<String>,
        fee_output_commitment: String,
    },
    Reverted,
}

/// Receipt normalization is performed by a reviewed chain/protocol adapter. The
/// service checks exact operation identity, costs and state-machine consistency.
/// These fields alone do not constitute consensus or receipt-trie verification.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReceiptEvidence {
    pub request_id: String,
    pub transaction_hash: String,
    pub chain_id: u64,
    pub block_number: u64,
    pub block_hash: String,
    pub canonical_block_hash: String,
    pub finalized_height: u64,
    pub gas_used: u64,
    pub effective_gas_price_wei: String,
    pub additional_native_fee_wei: String,
    pub execution: PrivateExecution,
}

impl ReceiptEvidence {
    pub(crate) fn native_charge(&self) -> Result<U256> {
        U256::from(self.gas_used)
            .checked_mul(decimal(&self.effective_gas_price_wei)?)
            .and_then(|value| value.checked_add(decimal(&self.additional_native_fee_wei).ok()?))
            .ok_or(Error::Arithmetic)
    }

    pub(crate) fn is_finalized(&self) -> bool {
        self.finalized_height >= self.block_number
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReorgEvidence {
    pub request_id: String,
    pub chain_id: u64,
    pub previous_block_number: u64,
    pub previous_block_hash: String,
    pub canonical_block_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobPhase {
    Admitted,
    Signed,
    Unknown,
    Pending,
    Included,
    IncludedRevert,
    Finalized,
    FinalizedRevert,
    RetryExhausted,
    Reorged,
    Incident,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JobView {
    pub request_id: String,
    pub operation_digest: String,
    pub phase: JobPhase,
    pub transaction_hash: Option<String>,
    pub nonce: Option<u64>,
    pub reserved_native_wei: String,
    pub finalized_native_charge_wei: String,
    pub earned_fee_units: String,
    pub event_count: u64,
    pub broadcast_attempts: u32,
    pub last_broadcast_release_at: Option<u64>,
    pub attempt_number: u8,
    pub maximum_attempts: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OperatorSummary {
    pub schema_version: u8,
    pub chain_id: u64,
    pub jobs: u64,
    pub pending_jobs: u64,
    pub opening_balance_wei: String,
    pub reserved_native_wei: String,
    pub finalized_native_charge_wei: String,
    pub available_native_wei: String,
    pub earned_fee_units: String,
    pub halted: bool,
    pub production_sponsorship_available: bool,
}

/// Contains an already persisted outer transaction. Calling the release method
/// records UNKNOWN before returning this ticket; its receipt is never success.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BroadcastTicket {
    pub request_id: String,
    pub operation_digest: String,
    pub transaction_hash: String,
    pub raw_transaction: String,
}

impl std::fmt::Debug for BroadcastTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BroadcastTicket")
            .field("request_id", &self.request_id)
            .field("operation_digest", &self.operation_digest)
            .field("transaction_hash", &self.transaction_hash)
            .field("raw_transaction", &"[REDACTED]")
            .finish()
    }
}

pub(crate) fn decimal(input: &str) -> Result<U256> {
    if input.is_empty()
        || input.len() > 78
        || (input.len() > 1 && input.starts_with('0'))
        || !input.bytes().all(|c| c.is_ascii_digit())
    {
        return Err(Error::InvalidQuantity);
    }
    U256::from_dec_str(input).map_err(|_| Error::InvalidQuantity)
}

pub(crate) fn bounded_hex(input: &str, max_bytes: usize) -> Result<Vec<u8>> {
    if !input.starts_with("0x")
        || input.len() < 4
        || !input.len().is_multiple_of(2)
        || input.len() > max_bytes.saturating_mul(2).saturating_add(2)
        || !input[2..]
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(Error::InvalidEncoding);
    }
    hex::decode(&input[2..]).map_err(|_| Error::InvalidEncoding)
}

pub(crate) fn fixed_hex(input: &str, length: usize) -> Result<()> {
    let bytes = bounded_hex(input, length)?;
    if bytes.len() != length || bytes.iter().all(|b| *b == 0) {
        return Err(Error::InvalidEncoding);
    }
    Ok(())
}

pub(crate) fn request_id(input: &str) -> Result<()> {
    // A fresh random 256-bit identifier per logical request, never a user ID.
    fixed_hex(input, 32)
}

pub(crate) fn digest(domain: &[u8], text: &str) -> String {
    let mut hash = Keccak256::new();
    hash.update(domain);
    hash.update(text.as_bytes());
    format!("0x{}", hex::encode(hash.finalize()))
}

pub(crate) fn encode<T: Serialize>(input: &T) -> Result<String> {
    serde_json::to_string(input).map_err(|_| Error::Storage)
}

pub(crate) fn decode<T: for<'de> Deserialize<'de>>(input: &str) -> Result<T> {
    if input.len() > MAX_JSON {
        return Err(Error::InvalidEncoding);
    }
    serde_json::from_str(input).map_err(|_| Error::InvalidEncoding)
}
