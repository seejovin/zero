//! Private-transfer intent and transport-envelope correctness.
//!
//! These functions validate a deliberately narrow candidate's structure,
//! integer arithmetic, and local approval binding. They DO NOT verify a ZK
//! proof, decrypt a fee output, authenticate chain state, or establish that
//! ciphertext contains a private payment. A separately reviewed protocol
//! adapter must do that work. No deployment is approved for production here.
//! `require_private_release` is intentionally closed in this source candidate.

use crate::{crypto, validation, WalletError};
use primitive_types::U256;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const SCHEMA_VERSION: u8 = 2;
pub const CANDIDATE_PROTOCOL: &str = "railgun-v2-candidate";
pub const CANDIDATE_CHAIN_ID: u64 = 42161;
pub const CANDIDATE_POOL: &str = "0xfa7093cdd9ee6932b4eb2c9e1cde7ce00b1fa4b9";
pub const MAX_INPUTS: usize = 16;
pub const MAX_OUTPUTS: usize = 16;
pub const MAX_JSON_BYTES: usize = 524_288;
pub const MAX_PROOF_BYTES: usize = 65_536;
pub const MAX_CIPHERTEXT_BYTES: usize = 8_192;
pub const MAX_CALL_DATA_BYTES: usize = 131_072;
pub const MAX_GAS_LIMIT: u64 = 10_000_000;
pub const MAX_NATIVE_FEE_WEI: u64 = 1_000_000_000_000_000_000;
/// Shared fee publications and the exact local intent must use the same window.
/// This is an admission deadline, never an expiry of an already released proof.
pub const MAX_FEE_SCHEDULE_SECONDS: u64 = 86_400;

const ENVELOPE_DOMAIN: &[u8] = b"LocalWallet/private-envelope/v2\0";
const INTENT_DOMAIN: &[u8] = b"LocalWallet/private-intent/v2\0";

fn invalid(reason: &str) -> WalletError {
    WalletError::InvalidInput {
        reason: reason.into(),
    }
}
fn policy(reason: &str) -> WalletError {
    WalletError::Policy {
        reason: reason.into(),
    }
}

/// This is a compilation/review gate, not a remotely configurable preference.
pub fn require_private_release() -> Result<(), WalletError> {
    Err(policy(
        "Private transfers are unavailable: the protocol adapter, deployed verifier, privacy transport and Android release have not passed release review",
    ))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateDomain {
    pub protocol: String,
    pub chain_id: u64,
    pub pool_contract: String,
    pub token_contract: String,
    pub token_decimals: u8,
    /// Digest of the candidate's verifier/artifact configuration. This field
    /// binds records, but a caller-selected digest is NOT release approval.
    pub verifier_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeeTerms {
    pub schedule_id: String,
    pub fee_units: String,
    pub fee_recipient_commitment: String,
    pub fee_output_commitment: String,
    pub max_native_fee_wei: String,
    pub gas_limit: u64,
    /// Admission/quote deadline only. It is NOT a cryptographic cancellation
    /// or a claim that an already released operation expires on chain.
    pub quote_valid_until: u64,
}

/// An operator-facing envelope. No private recipient, principal, input-note
/// commitment, viewing key, mnemonic, wallet ID, or memo belongs in this type.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateEnvelope {
    pub schema_version: u8,
    pub domain: PrivateDomain,
    /// Protocol nullifiers are scoped to the input Merkle tree.
    pub tree_number: u16,
    pub state_root: String,
    pub nullifiers: Vec<String>,
    pub output_commitments: Vec<String>,
    pub encrypted_outputs: Vec<String>,
    pub proof: String,
    /// Exact contract calldata. The reviewed adapter/verifier MUST decode the
    /// permitted private-transfer selector and bind every field, reject public
    /// calls/unshielding/native value, and verify the proof. Shape acceptance
    /// here is never permission to sign or broadcast generic calldata.
    pub call_data: String,
    pub fee: FeeTerms,
    pub operation_digest: String,
}

impl std::fmt::Debug for PrivateEnvelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrivateEnvelope")
            .field("operation_digest", &self.operation_digest)
            .field("chain_id", &self.domain.chain_id)
            .field("payload", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub struct PrivateInputNote {
    pub commitment: String,
    pub nullifier: String,
    pub units: String,
}

impl std::fmt::Debug for PrivateInputNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrivateInputNote([REDACTED])")
    }
}

/// Local-only authorization. The journal encrypts the entire value. It must
/// never be sent to a broadcaster, RPC, fee endpoint, or crash-report service.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub struct PrivateIntent {
    pub schema_version: u8,
    pub intent_id: String,
    #[zeroize(skip)]
    pub domain: PrivateDomain,
    pub tree_number: u16,
    pub recipient: String,
    pub amount_units: String,
    #[zeroize(skip)]
    pub fee: FeeTerms,
    pub input_notes: Vec<PrivateInputNote>,
    pub state_root: String,
    pub output_commitments: Vec<String>,
    pub recipient_output_commitment: String,
    pub change_units: String,
    pub change_output_commitment: Option<String>,
    /// Zero-value padding outputs, if required by the reviewed circuit. Their
    /// zero value and encryption must be checked by the private engine.
    pub padding_output_commitments: Vec<String>,
    pub created_at: u64,
    /// Local approval binding. This digest itself is not a signature or proof.
    pub approval_digest: String,
}

impl std::fmt::Debug for PrivateIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrivateIntent")
            .field("intent_id", &self.intent_id)
            .field("private_fields", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateMaxRequest {
    pub input_notes: Vec<PrivateInputNote>,
    pub fee_units: String,
    pub max_inputs: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateMax {
    pub input_count: u16,
    pub total_input_units: String,
    pub fee_units: String,
    pub recipient_units: String,
    pub change_units: String,
}

/// Direct typed deserialization rejects duplicate fields at every level, as
/// well as unknown fields; do not first parse untrusted input into `Value`.
pub fn decode<T: for<'a> Deserialize<'a>>(text: &str) -> Result<T, WalletError> {
    if text.is_empty() || text.len() > MAX_JSON_BYTES {
        return Err(invalid("Private-operation JSON exceeds its size limit"));
    }
    serde_json::from_str(text)
        .map_err(|_| invalid("Malformed, ambiguous or unsupported private-operation JSON"))
}

pub(crate) fn canonical_hash(value: &str) -> Result<(), WalletError> {
    let normalized = validation::fixed_hex(value, 32)?;
    if normalized != value || value[2..].bytes().all(|b| b == b'0') {
        return Err(invalid(
            "Expected a nonzero canonical lowercase 32-byte hash",
        ));
    }
    Ok(())
}

fn canonical_hex(value: &str, minimum: usize, maximum: usize) -> Result<(), WalletError> {
    if !value.starts_with("0x")
        || !value.len().is_multiple_of(2)
        || value.len() < minimum * 2 + 2
        || value.len() > maximum * 2 + 2
        || !value[2..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("Invalid or excessive canonical encoded bytes"));
    }
    Ok(())
}

pub fn validate_domain(domain: &PrivateDomain) -> Result<(), WalletError> {
    if domain.protocol != CANDIDATE_PROTOCOL
        || domain.chain_id != CANDIDATE_CHAIN_ID
        || domain.pool_contract != CANDIDATE_POOL
        || domain.token_contract != crypto::ARBITRUM_USDC_CONTRACT.to_ascii_lowercase()
        || domain.token_decimals != 6
    {
        return Err(policy("Unsupported private-payment candidate domain"));
    }
    canonical_hash(&domain.verifier_digest)
}

pub fn validate_fee(fee: &FeeTerms) -> Result<(), WalletError> {
    canonical_hash(&fee.schedule_id)?;
    canonical_hash(&fee.fee_recipient_commitment)?;
    canonical_hash(&fee.fee_output_commitment)?;
    let units = validation::decimal(&fee.fee_units)?;
    let native = validation::decimal(&fee.max_native_fee_wei)?;
    if units.is_zero()
        || native.is_zero()
        || native > U256::from(MAX_NATIVE_FEE_WEI)
        || fee.gas_limit == 0
        || fee.gas_limit > MAX_GAS_LIMIT
        || fee.quote_valid_until == 0
        || fee.quote_valid_until > i64::MAX as u64
    {
        return Err(policy("Invalid private-transfer fee or gas envelope"));
    }
    Ok(())
}

fn validate_hashes(values: &[String], minimum: usize, maximum: usize) -> Result<(), WalletError> {
    if values.len() < minimum || values.len() > maximum {
        return Err(policy(
            "Unsupported private-operation input or output count",
        ));
    }
    let mut seen = HashSet::new();
    for value in values {
        canonical_hash(value)?;
        if !seen.insert(value) {
            return Err(invalid(
                "Duplicate private-operation commitment or nullifier",
            ));
        }
    }
    Ok(())
}

pub fn validate_envelope(envelope: &PrivateEnvelope) -> Result<(), WalletError> {
    validate_envelope_body(envelope)?;
    canonical_hash(&envelope.operation_digest)?;
    if envelope.operation_digest != envelope_digest(envelope)? {
        return Err(invalid(
            "Private envelope digest does not match its exact fields",
        ));
    }
    Ok(())
}

fn validate_envelope_body(envelope: &PrivateEnvelope) -> Result<(), WalletError> {
    if envelope.schema_version != SCHEMA_VERSION {
        return Err(policy("Unsupported private envelope schema"));
    }
    validate_domain(&envelope.domain)?;
    canonical_hash(&envelope.state_root)?;
    validate_hashes(&envelope.nullifiers, 1, MAX_INPUTS)?;
    validate_hashes(&envelope.output_commitments, 2, MAX_OUTPUTS)?;
    validate_fee(&envelope.fee)?;
    if !envelope
        .output_commitments
        .contains(&envelope.fee.fee_output_commitment)
        || envelope.encrypted_outputs.len() != envelope.output_commitments.len()
    {
        return Err(invalid(
            "Private output and fee commitments are inconsistent",
        ));
    }
    for ciphertext in &envelope.encrypted_outputs {
        canonical_hex(ciphertext, 32, MAX_CIPHERTEXT_BYTES)?;
    }
    canonical_hex(&envelope.proof, 32, MAX_PROOF_BYTES)?;
    canonical_hex(&envelope.call_data, 4, MAX_CALL_DATA_BYTES)
}

#[derive(Serialize)]
struct EnvelopeDigestBody<'a> {
    schema_version: u8,
    domain: &'a PrivateDomain,
    tree_number: u16,
    state_root: &'a str,
    nullifiers: &'a [String],
    output_commitments: &'a [String],
    encrypted_outputs: &'a [String],
    proof: &'a str,
    call_data: &'a str,
    fee: &'a FeeTerms,
}

pub fn envelope_digest(envelope: &PrivateEnvelope) -> Result<String, WalletError> {
    validate_envelope_body(envelope)?;
    let body = EnvelopeDigestBody {
        schema_version: envelope.schema_version,
        domain: &envelope.domain,
        tree_number: envelope.tree_number,
        state_root: &envelope.state_root,
        nullifiers: &envelope.nullifiers,
        output_commitments: &envelope.output_commitments,
        encrypted_outputs: &envelope.encrypted_outputs,
        proof: &envelope.proof,
        call_data: &envelope.call_data,
        fee: &envelope.fee,
    };
    domain_digest(ENVELOPE_DOMAIN, &body)
}

pub fn validate_envelope_json(text: &str) -> Result<PrivateEnvelope, WalletError> {
    let envelope: PrivateEnvelope = decode(text)?;
    validate_envelope(&envelope)?;
    Ok(envelope)
}

pub fn canonical_envelope(envelope: &PrivateEnvelope) -> Result<String, WalletError> {
    validate_envelope(envelope)?;
    encode(envelope)
}

pub fn seal_envelope_json(text: &str) -> Result<String, WalletError> {
    let mut envelope: PrivateEnvelope = decode(text)?;
    if !envelope.operation_digest.is_empty() {
        return canonical_envelope(&envelope);
    }
    envelope.operation_digest = envelope_digest(&envelope)?;
    canonical_envelope(&envelope)
}

fn input_value(notes: &[PrivateInputNote], max_inputs: usize) -> Result<U256, WalletError> {
    if notes.is_empty() || notes.len() > max_inputs || max_inputs == 0 || max_inputs > MAX_INPUTS {
        return Err(policy(
            "The selected notes require a different spending or consolidation plan",
        ));
    }
    let mut nullifiers = HashSet::new();
    let mut total = U256::zero();
    for note in notes {
        canonical_hash(&note.commitment)?;
        canonical_hash(&note.nullifier)?;
        // A commitment can be funded at more than one leaf position. Actual
        // spend identity is the nullifier within the intent's input tree; the
        // reviewed adapter/proof must bind its opening and leaf position.
        if !nullifiers.insert(&note.nullifier) {
            return Err(invalid("A private input note is duplicated"));
        }
        let units = validation::decimal(&note.units)?;
        if units.is_zero() {
            return Err(invalid("A spendable input note must contain value"));
        }
        total = total
            .checked_add(units)
            .ok_or_else(|| invalid("Private input amount overflow"))?;
    }
    Ok(total)
}

/// Exact local accounting over an explicitly selected note plan. U256 avoids
/// decimal rounding; success does NOT assert that these values fit the actual
/// candidate circuit's per-note value bounds (including its uint120 fields),
/// input/output shape, or witness constraints. The reviewed protocol adapter
/// must enforce those bounds before approval or proof construction. It must
/// reject an oversized plan, never silently truncate a note or output amount.
pub fn maximum_transfer(request: &PrivateMaxRequest) -> Result<PrivateMax, WalletError> {
    let total = input_value(&request.input_notes, request.max_inputs as usize)?;
    let fee = validation::decimal(&request.fee_units)?;
    if fee.is_zero() || total <= fee {
        return Err(policy(
            "Spendable private balance does not cover the approved fee and transfer",
        ));
    }
    Ok(PrivateMax {
        input_count: request.input_notes.len() as u16,
        total_input_units: total.to_string(),
        fee_units: fee.to_string(),
        recipient_units: (total - fee).to_string(),
        change_units: "0".into(),
    })
}

pub fn maximum_transfer_json(text: &str) -> Result<String, WalletError> {
    encode(&maximum_transfer(&decode::<PrivateMaxRequest>(text)?)?)
}

pub fn validate_intent(intent: &PrivateIntent) -> Result<(), WalletError> {
    validate_intent_body(intent)?;
    canonical_hash(&intent.approval_digest)?;
    if intent.approval_digest != intent_digest(intent)? {
        return Err(invalid("Private intent does not match its approval digest"));
    }
    Ok(())
}

fn validate_intent_body(intent: &PrivateIntent) -> Result<(), WalletError> {
    if intent.schema_version != SCHEMA_VERSION {
        return Err(policy("Unsupported private intent schema"));
    }
    canonical_hash(&intent.intent_id)?;
    validate_domain(&intent.domain)?;
    validate_fee(&intent.fee)?;
    canonical_hash(&intent.state_root)?;
    validate_private_recipient(&intent.recipient)?;
    // Full private-address validation belongs to the exact reviewed protocol
    // adapter, not this shape validator.
    if intent.created_at == 0
        || intent.created_at > i64::MAX as u64
        || intent.fee.quote_valid_until < intent.created_at
        || intent.fee.quote_valid_until - intent.created_at > MAX_FEE_SCHEDULE_SECONDS
    {
        return Err(invalid("Invalid private-transfer quote window"));
    }
    validate_hashes(&intent.output_commitments, 2, MAX_OUTPUTS)?;
    let mut expected = vec![
        intent.recipient_output_commitment.clone(),
        intent.fee.fee_output_commitment.clone(),
    ];
    let change = validation::decimal(&intent.change_units)?;
    match (&intent.change_output_commitment, change.is_zero()) {
        (None, true) => {}
        (Some(commitment), false) => expected.push(commitment.clone()),
        _ => return Err(invalid("Private change value and output are inconsistent")),
    }
    expected.extend(intent.padding_output_commitments.iter().cloned());
    validate_hashes(&expected, 2, MAX_OUTPUTS)?;
    let expected_set: HashSet<_> = expected.iter().collect();
    if expected_set != intent.output_commitments.iter().collect() {
        return Err(invalid(
            "Private intent contains an unapproved output commitment",
        ));
    }
    let amount = validation::decimal(&intent.amount_units)?;
    let fee = validation::decimal(&intent.fee.fee_units)?;
    if amount.is_zero() {
        return Err(invalid("Private transfer amount must be positive"));
    }
    let used = amount
        .checked_add(fee)
        .and_then(|v| v.checked_add(change))
        .ok_or_else(|| invalid("Private output amount overflow"))?;
    if input_value(&intent.input_notes, MAX_INPUTS)? != used {
        return Err(invalid(
            "Private transfer inputs and approved outputs do not conserve value",
        ));
    }
    Ok(())
}

pub(crate) fn validate_private_recipient(recipient: &str) -> Result<(), WalletError> {
    if recipient.len() < 8
        || recipient.len() > 256
        || !recipient.is_ascii()
        || recipient
            .bytes()
            .any(|c| c.is_ascii_whitespace() || c.is_ascii_control())
        || recipient.starts_with("0x")
        || recipient.starts_with("0X")
    {
        return Err(invalid(
            "Expected a protocol-private recipient, not a public EVM address",
        ));
    }
    Ok(())
}

/// Local review before the SDK has selected notes and randomized commitments.
/// This is never an operator envelope or a signable/releasable operation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub struct PrivateTransferApproval {
    pub schema_version: u8,
    pub intent_id: String,
    #[zeroize(skip)]
    pub domain: PrivateDomain,
    pub recipient: String,
    pub amount_units: String,
    pub schedule_id: String,
    pub fee_units: String,
    pub fee_recipient_commitment: String,
    pub max_native_fee_wei: String,
    pub gas_limit: u64,
    pub quote_valid_until: u64,
    pub created_at: u64,
    pub approval_digest: String,
}
impl std::fmt::Debug for PrivateTransferApproval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrivateTransferApproval([PRIVATE LOCAL REVIEW])")
    }
}
fn validate_transfer_approval_body(approval: &PrivateTransferApproval) -> Result<(), WalletError> {
    canonical_hash(&approval.intent_id)?;
    validate_domain(&approval.domain)?;
    validate_private_recipient(&approval.recipient)?;
    // The fee output does not exist before local note construction. Reuse the
    // common fee-policy validator with the already validated payee commitment;
    // this temporary value never becomes an output or leaves this function.
    validate_fee(&FeeTerms {
        schedule_id: approval.schedule_id.clone(),
        fee_units: approval.fee_units.clone(),
        fee_recipient_commitment: approval.fee_recipient_commitment.clone(),
        fee_output_commitment: approval.fee_recipient_commitment.clone(),
        max_native_fee_wei: approval.max_native_fee_wei.clone(),
        gas_limit: approval.gas_limit,
        quote_valid_until: approval.quote_valid_until,
    })?;
    if approval.schema_version != SCHEMA_VERSION
        || validation::decimal(&approval.amount_units)?.is_zero()
        || approval.created_at == 0
        || approval.quote_valid_until < approval.created_at
        || approval.quote_valid_until - approval.created_at > MAX_FEE_SCHEDULE_SECONDS
    {
        return Err(invalid("Invalid private transfer review"));
    }
    Ok(())
}
pub fn transfer_approval_digest(approval: &PrivateTransferApproval) -> Result<String, WalletError> {
    validate_transfer_approval_body(approval)?;
    let mut unsigned = approval.clone();
    unsigned.approval_digest.clear();
    domain_digest(b"LocalWallet/private-transfer-review/v2\0", &unsigned)
}
pub fn validate_transfer_approval(approval: &PrivateTransferApproval) -> Result<(), WalletError> {
    canonical_hash(&approval.approval_digest)?;
    if approval.approval_digest != transfer_approval_digest(approval)? {
        return Err(invalid("Private transfer review digest does not match"));
    }
    Ok(())
}
pub fn seal_transfer_approval_json(text: &str) -> Result<String, WalletError> {
    let mut approval: PrivateTransferApproval = decode(text)?;
    if approval.approval_digest.is_empty() {
        approval.approval_digest = transfer_approval_digest(&approval)?;
    }
    validate_transfer_approval(&approval)?;
    encode(&approval)
}
/// Call only after the reviewed SDK has locally checked the real private note
/// openings against the recipient and values. This binds that concrete intent
/// to the earlier review; it cannot itself verify encrypted note semantics.
pub fn validate_intent_for_approval(
    approval: &PrivateTransferApproval,
    intent: &PrivateIntent,
) -> Result<(), WalletError> {
    validate_transfer_approval(approval)?;
    validate_intent(intent)?;
    if approval.intent_id != intent.intent_id
        || approval.domain != intent.domain
        || approval.recipient != intent.recipient
        || approval.amount_units != intent.amount_units
        || approval.created_at != intent.created_at
        || approval.schedule_id != intent.fee.schedule_id
        || approval.fee_units != intent.fee.fee_units
        || approval.fee_recipient_commitment != intent.fee.fee_recipient_commitment
        || approval.max_native_fee_wei != intent.fee.max_native_fee_wei
        || approval.gas_limit != intent.fee.gas_limit
        || approval.quote_valid_until != intent.fee.quote_valid_until
    {
        return Err(policy("The concrete private operation differs from the user-approved recipient, amount or fee policy"));
    }
    Ok(())
}

pub fn intent_digest(intent: &PrivateIntent) -> Result<String, WalletError> {
    validate_intent_body(intent)?;
    let mut unsigned = intent.clone();
    unsigned.approval_digest.clear();
    domain_digest(INTENT_DOMAIN, &unsigned)
}

pub fn validate_intent_json(text: &str) -> Result<PrivateIntent, WalletError> {
    let intent = decode(text)?;
    validate_intent(&intent)?;
    Ok(intent)
}

pub fn seal_intent_json(text: &str) -> Result<String, WalletError> {
    let mut intent: PrivateIntent = decode(text)?;
    if intent.approval_digest.is_empty() {
        intent.approval_digest = intent_digest(&intent)?;
    }
    validate_intent(&intent)?;
    encode(&intent)
}

pub fn validate_envelope_for_intent(
    intent: &PrivateIntent,
    envelope: &PrivateEnvelope,
) -> Result<(), WalletError> {
    validate_intent(intent)?;
    validate_envelope(envelope)?;
    let nullifiers: Vec<_> = intent
        .input_notes
        .iter()
        .map(|n| n.nullifier.clone())
        .collect();
    if envelope.domain != intent.domain
        || envelope.tree_number != intent.tree_number
        || envelope.state_root != intent.state_root
        || envelope.nullifiers != nullifiers
        || envelope.output_commitments != intent.output_commitments
        || envelope.fee != intent.fee
    {
        return Err(policy(
            "Private envelope differs from the locally approved operation",
        ));
    }
    Ok(())
}

pub(crate) fn encode<T: Serialize>(value: &T) -> Result<String, WalletError> {
    serde_json::to_string(value).map_err(|_| WalletError::Crypto)
}

fn domain_digest<T: Serialize>(domain: &[u8], value: &T) -> Result<String, WalletError> {
    let bytes =
        zeroize::Zeroizing::new(serde_json::to_vec(value).map_err(|_| WalletError::Crypto)?);
    if bytes.len() > MAX_JSON_BYTES {
        return Err(invalid("Private operation exceeds its size limit"));
    }
    let mut digest = Sha256::new();
    digest.update(domain);
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(&*bytes);
    Ok(format!("0x{}", hex::encode(digest.finalize())))
}
