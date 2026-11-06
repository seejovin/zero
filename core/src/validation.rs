//! Pure validation shared by Android's SQLite journal and the browser journal.
//! RPC evidence is checked against the actual signed transaction. RPC responses
//! are observations, not consensus proofs; independent providers remain needed.
use crate::crypto::{self, InspectedPayment, PreparedPayment, SignedPayment};
use crate::WalletError;
use primitive_types::U256;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha3::{Digest, Keccak256};
use std::collections::HashSet;
pub(crate) const MAX_PAYMENT_JSON: usize = 16_384;
pub(crate) const MAX_RECEIPT_JSON: usize = 1_048_576;
const MAX_RECEIPT_LOGS: usize = 1_024;
fn invalid(reason: &str) -> WalletError {
    WalletError::InvalidInput {
        reason: reason.into(),
    }
}
fn conflict(reason: &str) -> WalletError {
    WalletError::Conflict {
        reason: reason.into(),
    }
}
fn decode<T: for<'a> Deserialize<'a>>(input: &str, maximum: usize) -> Result<T, WalletError> {
    if input.len() > maximum {
        return Err(invalid("Input exceeds the size limit"));
    }
    serde_json::from_str(input)
        .map_err(|_| invalid("JSON is malformed or contains unsupported fields"))
}
/// Receipt fields which participate in the decision. Additional RPC extension
/// fields are ignored; critical duplicate fields fail serde deserialization.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RpcReceipt {
    transaction_hash: String,
    block_hash: String,
    block_number: String,
    from: String,
    to: String,
    status: String,
    gas_used: String,
    effective_gas_price: String,
    logs: Vec<RpcLog>,
    #[serde(default, rename = "type")]
    transaction_type: Option<String>,
    #[serde(default)]
    contract_address: Option<String>,
    #[serde(default)]
    l1_fee: Option<String>,
    #[serde(default)]
    operator_fee: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RpcLog {
    address: String,
    topics: Vec<String>,
    data: String,
    transaction_hash: String,
    block_hash: String,
    block_number: String,
    log_index: String,
    #[serde(default)]
    removed: bool,
}

#[derive(Serialize)]
pub(crate) struct NormalizedLog {
    address: String,
    topics: Vec<String>,
    data: String,
    log_index: u64,
}

#[derive(Serialize)]
pub(crate) struct VerifiedReceipt {
    pub(crate) transaction_hash: String,
    pub(crate) block_hash: String,
    pub(crate) block_number: u64,
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) status: u8,
    pub(crate) gas_used: String,
    pub(crate) effective_gas_price: String,
    pub(crate) actual_fee_wei: String,
    pub(crate) actual_execution_fee_wei: String,
    pub(crate) actual_l1_fee_wei: String,
    pub(crate) actual_operator_fee_wei: String,
    pub(crate) logs: Vec<NormalizedLog>,
    pub(crate) expected_transfer_observed: bool,
}

pub(crate) fn fixed_hex(input: &str, bytes: usize) -> Result<String, WalletError> {
    if input.len() != bytes * 2 + 2
        || !input.starts_with("0x")
        || !input[2..].bytes().all(|c| c.is_ascii_hexdigit())
    {
        return Err(invalid("Invalid fixed-length hexadecimal value"));
    }
    Ok(input.to_ascii_lowercase())
}

pub(crate) fn address(input: &str) -> Result<String, WalletError> {
    let lower = fixed_hex(input, 20)?;
    if lower[2..].bytes().all(|c| c == b'0') {
        return Err(invalid("Zero address is not an account"));
    }
    let body = &input[2..];
    if body.bytes().any(|c| c.is_ascii_lowercase()) && body.bytes().any(|c| c.is_ascii_uppercase())
    {
        let hash = Keccak256::digest(&lower.as_bytes()[2..]);
        for (index, byte) in body.bytes().enumerate() {
            if byte.is_ascii_alphabetic() {
                let nibble = if index.is_multiple_of(2) {
                    hash[index / 2] >> 4
                } else {
                    hash[index / 2] & 15
                };
                if byte.is_ascii_uppercase() != (nibble >= 8) {
                    return Err(invalid("Invalid Ethereum address checksum"));
                }
            }
        }
    }
    Ok(lower)
}

pub(crate) fn decimal(input: &str) -> Result<U256, WalletError> {
    if input.is_empty()
        || input.len() > 78
        || !input.bytes().all(|c| c.is_ascii_digit())
        || (input.len() > 1 && input.starts_with('0'))
    {
        return Err(invalid(
            "Amounts must be canonical unsigned decimal integers",
        ));
    }
    U256::from_dec_str(input).map_err(|_| invalid("Amount exceeds 256 bits"))
}

pub(crate) fn quantity(input: &str) -> Result<U256, WalletError> {
    if !input.starts_with("0x")
        || input.len() < 3
        || input.len() > 66
        || !input[2..].bytes().all(|c| c.is_ascii_hexdigit())
        || (input.len() > 3 && input.as_bytes()[2] == b'0')
    {
        return Err(invalid("Invalid Ethereum RPC quantity"));
    }
    U256::from_str_radix(&input[2..], 16).map_err(|_| invalid("Invalid Ethereum RPC quantity"))
}

pub(crate) fn quantity_u64(input: &str) -> Result<u64, WalletError> {
    let number = quantity(input)?;
    if number > U256::from(u64::MAX) {
        return Err(invalid("RPC quantity exceeds 64 bits"));
    }
    Ok(number.low_u64())
}

pub(crate) fn digest(input: &str) -> String {
    format!("0x{}", hex::encode(Keccak256::digest(input.as_bytes())))
}

pub(crate) fn inspected(signed: &SignedPayment) -> Result<InspectedPayment, WalletError> {
    let actual: InspectedPayment = decode(
        &crypto::inspect_signed(signed.raw_transaction.clone())?,
        MAX_PAYMENT_JSON,
    )?;
    if actual.chain_id != signed.chain_id
        || actual.from != signed.from
        || actual.to != signed.to
        || actual.asset != signed.asset
        || actual.amount != signed.amount
        || actual.amount_units != signed.amount_units
        || actual.nonce != signed.nonce
        || actual.max_fee_wei != signed.max_fee_wei
        || actual.hash != signed.hash
        || actual.raw_transaction != signed.raw_transaction
    {
        return Err(conflict(
            "Signed metadata does not match its transaction bytes",
        ));
    }
    let extra = decimal(&signed.l1_fee_estimate_wei)?
        .checked_add(decimal(&signed.operator_fee_estimate_wei)?)
        .ok_or_else(|| invalid("Additional fee estimate overflow"))?;
    let execution = decimal(&signed.max_fee_wei)?;
    let estimated = execution
        .checked_add(extra)
        .ok_or_else(|| invalid("Estimated fee overflow"))?;
    let reserve = execution
        .checked_add(
            extra
                .checked_mul(U256::from(2))
                .ok_or_else(|| invalid("Fee reserve overflow"))?,
        )
        .ok_or_else(|| invalid("Fee reserve overflow"))?;
    if (!crypto::has_separate_rollup_fees(signed.chain_id) && !extra.is_zero())
        || estimated != decimal(&signed.estimated_total_fee_wei)?
        || reserve != decimal(&signed.fee_reserve_wei)?
        || reserve > U256::from(crypto::maximum_fee_reserve(signed.chain_id))
    {
        return Err(conflict("Signed fee metadata is inconsistent"));
    }
    Ok(actual)
}

pub(crate) fn check_signed(
    prepared: &PreparedPayment,
    signed: &SignedPayment,
    actual: &InspectedPayment,
) -> Result<(), WalletError> {
    let request = &prepared.request;
    if signed.intent_id != request.intent_id
        || signed.approval_digest != prepared.approval_digest
        || signed.chain_id != request.chain_id
        || signed.from != request.from
        || signed.to != request.to
        || signed.asset != request.asset
        || signed.amount != request.amount
        || signed.amount_units != prepared.amount_units
        || signed.nonce != request.nonce
        || signed.max_fee_wei != prepared.max_fee_wei
        || signed.l1_fee_estimate_wei != request.l1_fee_estimate_wei
        || signed.operator_fee_estimate_wei != request.operator_fee_estimate_wei
        || signed.estimated_total_fee_wei != prepared.estimated_total_fee_wei
        || signed.fee_reserve_wei != prepared.fee_reserve_wei
        || actual.chain_id != signed.chain_id
        || actual.from != signed.from
        || actual.to != signed.to
        || actual.asset != signed.asset
        || actual.amount != signed.amount
        || actual.amount_units != signed.amount_units
        || actual.nonce != signed.nonce
        || actual.max_fee_wei != signed.max_fee_wei
        || actual.hash != signed.hash
        || actual.raw_transaction != signed.raw_transaction
        || actual.unsigned_transaction != prepared.unsigned_transaction
        || actual.gas_limit != request.gas_limit
        || actual.max_fee_per_gas != request.max_fee_per_gas
        || actual.max_priority_fee_per_gas != request.max_priority_fee_per_gas
    {
        return Err(conflict(
            "Signed bytes do not match the immutable approved payment",
        ));
    }
    Ok(())
}

pub(crate) fn verify_receipt(
    receipt: RpcReceipt,
    canonical: &str,
    transaction: &InspectedPayment,
) -> Result<VerifiedReceipt, WalletError> {
    let transaction_hash = fixed_hex(&receipt.transaction_hash, 32)?;
    let block_hash = fixed_hex(&receipt.block_hash, 32)?;
    let block_number = quantity_u64(&receipt.block_number)?;
    let from = address(&receipt.from)?;
    let to = address(&receipt.to)?;
    if transaction_hash != transaction.hash
        || block_hash != canonical
        || from != address(&transaction.from)?
        || to != address(&transaction.transaction_to)?
    {
        return Err(conflict(
            "Receipt hash, participants or canonical block do not match the recorded transaction",
        ));
    }
    if receipt.contract_address.is_some()
        || receipt
            .transaction_type
            .as_ref()
            .map(|t| quantity(t).map(|v| v != U256::from(2)))
            .transpose()?
            .unwrap_or(false)
    {
        return Err(invalid("Receipt describes an unsupported transaction type"));
    }
    let status = match quantity_u64(&receipt.status)? {
        0 => 0,
        1 => 1,
        _ => return Err(invalid("Unknown execution status")),
    };
    let gas_used = quantity(&receipt.gas_used)?;
    let gas_price = quantity(&receipt.effective_gas_price)?;
    // Gas LIMIT must reserve the EIP-7623 calldata floor (checked by the
    // signer), but the specification permits gasUsed below that floor. Do not
    // reject receipt evidence by confusing reserved gas with gas charged.
    if gas_used < U256::from(21_000u64)
        || gas_used > U256::from(transaction.gas_limit)
        || gas_price > decimal(&transaction.max_fee_per_gas)?
    {
        return Err(conflict(
            "Receipt gas charges violate the transaction minimum or approved cap",
        ));
    }
    let execution_fee = gas_used
        .checked_mul(gas_price)
        .ok_or_else(|| invalid("Receipt fee exceeds 256 bits"))?;
    if execution_fee > decimal(&transaction.max_fee_wei)? {
        return Err(conflict(
            "Receipt execution fee exceeds the approved execution maximum",
        ));
    }
    let (l1_fee, operator_fee) =
        if crypto::has_separate_rollup_fees(transaction.chain_id) {
            (
                quantity(
                    receipt
                        .l1_fee
                        .as_deref()
                        .ok_or_else(|| invalid("OP Stack receipt is missing its actual L1 fee"))?,
                )?,
                quantity(receipt.operator_fee.as_deref().ok_or_else(|| {
                    invalid("OP Stack receipt is missing its actual operator fee")
                })?)?,
            )
        } else {
            let l1 = receipt
                .l1_fee
                .as_deref()
                .map(quantity)
                .transpose()?
                .unwrap_or_default();
            let operator = receipt
                .operator_fee
                .as_deref()
                .map(quantity)
                .transpose()?
                .unwrap_or_default();
            if !l1.is_zero() || !operator.is_zero() {
                return Err(conflict(
                    "Receipt contains separate fee charges unsupported on this network",
                ));
            }
            (l1, operator)
        };
    let actual_fee = execution_fee
        .checked_add(l1_fee)
        .and_then(|fee| fee.checked_add(operator_fee))
        .ok_or_else(|| invalid("Receipt total fee exceeds 256 bits"))?;
    if receipt.logs.len() > MAX_RECEIPT_LOGS || (status == 0 && !receipt.logs.is_empty()) {
        return Err(invalid("Receipt contains an invalid or excessive log set"));
    }
    let transfer_topic = format!(
        "0x{}",
        hex::encode(Keccak256::digest(b"Transfer(address,address,uint256)"))
    );
    let from_topic = format!("0x{:0>64}", &from[2..]);
    let beneficiary = address(&transaction.to)?;
    let to_topic = format!("0x{:0>64}", &beneficiary[2..]);
    let expected_amount = decimal(&transaction.amount_units)?;
    let mut matching_transfers = 0;
    let mut contract_transfers = 0;
    let mut seen_log_indexes = HashSet::new();
    let mut logs = Vec::with_capacity(receipt.logs.len());
    for log in receipt.logs {
        if log.removed
            || fixed_hex(&log.transaction_hash, 32)? != transaction_hash
            || fixed_hex(&log.block_hash, 32)? != block_hash
            || quantity_u64(&log.block_number)? != block_number
            || log.topics.len() > 4
        {
            return Err(conflict(
                "Receipt log is removed or refers to different transaction evidence",
            ));
        }
        let log_index = quantity_u64(&log.log_index)?;
        if !seen_log_indexes.insert(log_index) {
            return Err(invalid("Receipt has duplicate log indexes"));
        }
        let log_address = address(&log.address)?;
        let topics = log
            .topics
            .iter()
            .map(|t| fixed_hex(t, 32))
            .collect::<Result<Vec<_>, _>>()?;
        if !log.data.starts_with("0x")
            || log.data.len() > 16_386
            || !log.data.len().is_multiple_of(2)
            || !log.data[2..].bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err(invalid("Receipt log data is malformed or too large"));
        }
        let data = log.data.to_ascii_lowercase();
        if !transaction.asset.is_native_on(transaction.chain_id)
            && log_address == to
            && topics.first() == Some(&transfer_topic)
        {
            contract_transfers += 1;
            if topics.len() == 3
                && topics[1] == from_topic
                && topics[2] == to_topic
                && data.len() == 66
            {
                let amount = U256::from_str_radix(&data[2..], 16)
                    .map_err(|_| invalid("Invalid token transfer amount"))?;
                if amount == expected_amount {
                    matching_transfers += 1;
                }
            }
        }
        logs.push(NormalizedLog {
            address: log_address,
            topics,
            data,
            log_index,
        });
    }
    logs.sort_by_key(|log| log.log_index);
    Ok(VerifiedReceipt {
        transaction_hash,
        block_hash,
        block_number,
        from,
        to,
        status,
        gas_used: gas_used.to_string(),
        effective_gas_price: gas_price.to_string(),
        actual_fee_wei: actual_fee.to_string(),
        actual_execution_fee_wei: execution_fee.to_string(),
        actual_l1_fee_wei: l1_fee.to_string(),
        actual_operator_fee_wei: operator_fee.to_string(),
        logs,
        expected_transfer_observed: transaction.asset.is_native_on(transaction.chain_id)
            || (matching_transfers == 1 && contract_transfers == 1),
    })
}

pub fn validate_prepared_json(prepared_json: String) -> Result<String, WalletError> {
    serde_json::to_string(&crypto::validate_prepared(&prepared_json)?)
        .map_err(|_| WalletError::Crypto)
}

pub fn validate_signed_for_prepared(
    prepared_json: String,
    signed_json: String,
) -> Result<String, WalletError> {
    let prepared = crypto::validate_prepared(&prepared_json)?;
    let signed: SignedPayment = decode(&signed_json, MAX_PAYMENT_JSON)?;
    let actual = inspected(&signed)?;
    check_signed(&prepared, &signed, &actual)?;
    serde_json::to_string(&signed).map_err(|_| WalletError::Crypto)
}

/// Stateless receipt validation. Call validate_signed_for_prepared when loading
/// browser journal records, before using this function or broadcasting bytes.
/// The fingerprint excludes observed finality so promotion is idempotent.
pub fn validate_receipt(
    signed_json: String,
    receipt_json: String,
    canonical_block_hash: String,
    finalized_height: u64,
) -> Result<String, WalletError> {
    let signed: SignedPayment = decode(&signed_json, MAX_PAYMENT_JSON)?;
    let actual = inspected(&signed)?;
    let canonical = fixed_hex(&canonical_block_hash, 32)?;
    let evidence = verify_receipt(
        decode(&receipt_json, MAX_RECEIPT_JSON)?,
        &canonical,
        &actual,
    )?;
    let fingerprint = digest(&serde_json::to_string(&evidence).map_err(|_| WalletError::Crypto)?);
    let execution = if evidence.status == 0 {
        "failed"
    } else if evidence.expected_transfer_observed {
        "success"
    } else {
        "unknown"
    };
    let inclusion = if evidence.block_number <= finalized_height {
        "finalized"
    } else {
        "included"
    };
    let phase = if execution == "unknown" {
        "unresolved"
    } else {
        inclusion
    };
    let exceeded = additional_fee_exceeded(&evidence, &signed)?;
    Ok(json!({"phase":phase,"execution":execution,"inclusion":inclusion,
        "block_number":evidence.block_number,"block_hash":evidence.block_hash,"receipt_status":evidence.status,
        "receipt_fingerprint":fingerprint,"actual_fee_wei":evidence.actual_fee_wei,
        "actual_execution_fee_wei":evidence.actual_execution_fee_wei,"actual_l1_fee_wei":evidence.actual_l1_fee_wei,
        "actual_operator_fee_wei":evidence.actual_operator_fee_wei,"additional_fee_exceeded_estimate":exceeded,
        "resolution_note":if execution == "unknown" {Some("The transaction succeeded, but the receipt did not contain exactly the expected token transfer. Payment outcome is unresolved.")} else {None},
        "evidence":evidence}).to_string())
}

pub(crate) fn additional_fee_exceeded(
    evidence: &VerifiedReceipt,
    signed: &SignedPayment,
) -> Result<bool, WalletError> {
    Ok(
        decimal(&evidence.actual_l1_fee_wei)? > decimal(&signed.l1_fee_estimate_wei)?
            || decimal(&evidence.actual_operator_fee_wei)?
                > decimal(&signed.operator_fee_estimate_wei)?,
    )
}
