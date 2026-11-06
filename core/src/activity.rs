//! Read-only activity validation. Discovery services supply candidate hashes,
//! never amounts or receipt authority. This module cannot sign or broadcast.
use crate::crypto::{self, Asset};
use crate::validation::{address, fixed_hex, quantity, quantity_u64, MAX_RECEIPT_JSON};
use crate::WalletError;
use primitive_types::U256;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

const TRANSFER: &str = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
fn invalid(reason: &str) -> WalletError {
    WalletError::InvalidInput {
        reason: reason.into(),
    }
}
fn decode<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, WalletError> {
    if text.len() > MAX_RECEIPT_JSON {
        return Err(invalid("Activity evidence exceeds its size limit"));
    }
    serde_json::from_str(text).map_err(|_| invalid("Activity evidence is malformed or ambiguous"))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Transaction {
    hash: String,
    block_hash: String,
    block_number: String,
    from: String,
    to: Option<String>,
    value: String,
    #[serde(default)]
    chain_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Receipt {
    transaction_hash: String,
    block_hash: String,
    block_number: String,
    from: String,
    to: Option<String>,
    status: String,
    logs: Vec<Log>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Log {
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
#[derive(Deserialize)]
struct Block {
    hash: String,
    number: String,
    timestamp: String,
    #[serde(default)]
    transactions: Option<Vec<Value>>,
}
#[derive(Serialize)]
struct Activity {
    id: String,
    chain_id: u64,
    address: String,
    direction: &'static str,
    asset: Asset,
    asset_contract: Option<&'static str>,
    amount_units: String,
    amount: String,
    from: String,
    to: String,
    transaction_hash: String,
    block_number: u64,
    block_hash: String,
    log_index: Option<u64>,
    timestamp: u64,
    inclusion: &'static str,
}
fn participant(value: &str) -> Result<String, WalletError> {
    // Token mint/burn events legitimately contain the zero participant.
    crypto::normalize_address(value)
}
fn indexed_participant(topic: &str) -> Result<String, WalletError> {
    let topic = fixed_hex(topic, 32)?;
    if topic[2..26] != "0".repeat(24) {
        return Err(invalid("Transfer topic does not encode an address"));
    }
    participant(&format!("0x{}", &topic[26..]))
}

/// Returns supported successful native ETH/direct-value and ERC20 Transfer
/// observations involving this wallet. It does not infer internal ETH transfers
/// or financial effects from balance deltas. Provider claims are not proofs of
/// consensus; callers must verify network and recheck canonical block afterwards.
pub fn validate_activity(
    chain_id: u64,
    wallet_address: String,
    transaction_json: String,
    receipt_json: String,
    canonical_block_json: String,
    finalized_height: u64,
) -> Result<String, WalletError> {
    if !crypto::supported_chain(chain_id) {
        return Err(invalid("Unsupported activity network"));
    }
    let account = address(&wallet_address)?;
    let tx: Transaction = decode(&transaction_json)?;
    let receipt: Receipt = decode(&receipt_json)?;
    let block: Block = decode(&canonical_block_json)?;
    let tx_hash = fixed_hex(&tx.hash, 32)?;
    let block_hash = fixed_hex(&block.hash, 32)?;
    let height = quantity_u64(&block.number)?;
    let timestamp = quantity_u64(&block.timestamp)?;
    if height > 9_007_199_254_740_991 || timestamp > 9_007_199_254_740_991 {
        return Err(invalid("Activity block exceeds browser integer range"));
    }
    let sender = participant(&tx.from)?;
    let destination = tx.to.as_deref().map(participant).transpose()?;
    if fixed_hex(&receipt.transaction_hash, 32)? != tx_hash
        || fixed_hex(&tx.block_hash, 32)? != block_hash
        || fixed_hex(&receipt.block_hash, 32)? != block_hash
        || quantity_u64(&tx.block_number)? != height
        || quantity_u64(&receipt.block_number)? != height
        || participant(&receipt.from)? != sender
        || receipt.to.as_deref().map(participant).transpose()? != destination
        || tx
            .chain_id
            .as_deref()
            .map(quantity_u64)
            .transpose()?
            .is_some_and(|id| id != chain_id)
    {
        return Err(invalid(
            "Activity transaction, receipt, network or canonical block do not match",
        ));
    }
    if let Some(transactions) = &block.transactions {
        if transactions.len() > 30_000 {
            return Err(invalid("Block transaction set is too large"));
        }
        let mut matches = 0;
        for item in transactions {
            let hash = match item {
                Value::String(hash) => hash.as_str(),
                Value::Object(value) => value
                    .get("hash")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("Malformed block transaction entry"))?,
                _ => return Err(invalid("Malformed block transaction entry")),
            };
            if fixed_hex(hash, 32)? == tx_hash {
                matches += 1;
            }
        }
        if matches != 1 {
            return Err(invalid(
                "Transaction is not uniquely present in the canonical block",
            ));
        }
    }
    let status = quantity_u64(&receipt.status)?;
    if status > 1 || receipt.logs.len() > 1024 || (status == 0 && !receipt.logs.is_empty()) {
        return Err(invalid("Invalid activity execution status or log set"));
    }
    let value = quantity(&tx.value)?;
    if status == 0 {
        return Ok(json!({"activities":[]}).to_string());
    }
    let inclusion = if height <= finalized_height {
        "finalized"
    } else {
        "included"
    };
    let mut activities = Vec::new();
    let direction = |from: &str, to: &str| -> Option<&'static str> {
        let sent = from.eq_ignore_ascii_case(&account);
        let received = to.eq_ignore_ascii_case(&account);
        match (sent, received) {
            (true, true) => Some("self"),
            (true, false) => Some("sent"),
            (false, true) => Some("received"),
            _ => None,
        }
    };
    let make = |asset: Asset,
                amount: U256,
                from: String,
                to: String,
                log_index: Option<u64>,
                suffix: String,
                dir: &'static str|
     -> Result<Activity, WalletError> {
        Ok(Activity {
            id: format!("{chain_id}:{account}:{tx_hash}:{suffix}"),
            chain_id,
            address: crypto::normalize_address(&wallet_address)?,
            direction: dir,
            asset,
            asset_contract: asset.contract(chain_id),
            amount_units: amount.to_string(),
            amount: crypto::format_amount(amount.to_string(), asset.decimals())?,
            from,
            to,
            transaction_hash: tx_hash.clone(),
            block_number: height,
            block_hash: block_hash.clone(),
            log_index,
            timestamp,
            inclusion,
        })
    };
    if !value.is_zero() {
        if let Some(to) = destination {
            if let Some(dir) = direction(&sender, &to) {
                activities.push(make(
                    crypto::native_asset(chain_id)
                        .ok_or_else(|| invalid("Unsupported native asset"))?,
                    value,
                    sender,
                    to,
                    None,
                    "native".into(),
                    dir,
                )?);
            }
        }
    }
    let mut indexes = HashSet::new();
    for (ordinal, log) in receipt.logs.into_iter().enumerate() {
        if log.removed
            || fixed_hex(&log.transaction_hash, 32)? != tx_hash
            || fixed_hex(&log.block_hash, 32)? != block_hash
            || quantity_u64(&log.block_number)? != height
            || log.topics.len() > 4
        {
            return Err(invalid(
                "Activity log is removed or refers to different transaction evidence",
            ));
        }
        let index = quantity_u64(&log.log_index)?;
        if index > 9_007_199_254_740_991 || !indexes.insert(index) {
            return Err(invalid("Activity has invalid or duplicate log indexes"));
        }
        let contract = participant(&log.address)?;
        let topics: Vec<String> = log
            .topics
            .iter()
            .map(|topic| fixed_hex(topic, 32))
            .collect::<Result<_, _>>()?;
        if !log.data.starts_with("0x")
            || !log.data.len().is_multiple_of(2)
            || log.data.len() > 16_386
            || !log.data[2..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid("Malformed activity log data"));
        }
        let asset = [Asset::Usdc, Asset::Eurc, Asset::Usdt]
            .into_iter()
            .find(|asset| {
                asset
                    .contract(chain_id)
                    .is_some_and(|expected| expected.eq_ignore_ascii_case(&contract))
            });
        if let Some(asset) = asset {
            if topics.first().map(String::as_str) == Some(TRANSFER) {
                if topics.len() != 3 || log.data.len() != 66 {
                    return Err(invalid("Malformed supported-token Transfer event"));
                }
                let from = indexed_participant(&topics[1])?;
                let to = indexed_participant(&topics[2])?;
                let amount = U256::from_str_radix(&log.data[2..], 16)
                    .map_err(|_| invalid("Invalid Transfer amount"))?;
                if !amount.is_zero() {
                    if let Some(dir) = direction(&from, &to) {
                        activities.push(make(
                            asset,
                            amount,
                            from,
                            to,
                            Some(index),
                            format!("log-{ordinal}"),
                            dir,
                        )?);
                    }
                }
            }
        }
    }
    serde_json::to_string(&json!({"activities":activities})).map_err(|_| WalletError::Crypto)
}
