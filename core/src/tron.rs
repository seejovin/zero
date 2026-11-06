//! Restricted TRON mainnet wallet policy. Providers supply public observations,
//! never transaction bytes to sign. Only TRX and Tether's canonical TRC20 USDT
//! transfer are supported. Protobuf encoding is performed by prost.
use crate::WalletError;
use bip32::{DerivationPath, XPrv};
use k256::ecdsa::{RecoveryId, Signature, SigningKey, VerifyingKey};
use primitive_types::U256;
use prost::Message;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sha3::Keccak256;
use zeroize::Zeroizing;

pub const NETWORK: &str = "tron-mainnet";
pub const MAINNET_CHAIN_ID: &str = "0x2b6653dc";
pub const USDT: &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
pub const DERIVATION_PATH: &str = "m/44'/195'/0'/0/0";
const TRANSFER_EVENT: &str = "ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
const MAX_JSON: usize = 4 * 1024 * 1024;
const MAX_SIGNED_BYTES: usize = 512;
const MAX_ENERGY_FEE_SUN: u64 = 200_000_000;
const MAX_REVIEW_AGE_MS: u64 = 120_000;
const TX_LIFETIME_MS: u64 = 180_000;

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
fn decode<T: for<'de> Deserialize<'de>>(input: &str) -> Result<T, WalletError> {
    if input.len() > MAX_JSON {
        return Err(invalid("TRON evidence exceeds its size limit"));
    }
    serde_json::from_str(input).map_err(|_| invalid("TRON data is malformed or ambiguous"))
}
fn now_ms() -> Result<u64, WalletError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|v| v.as_millis() as u64)
            .map_err(|_| policy("The device clock is invalid"))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let n = js_sys::Date::now();
        if !n.is_finite() || !(1.0..9_007_199_254_740_991.0).contains(&n) {
            return Err(policy("The device clock is invalid"));
        }
        Ok(n as u64)
    }
}
fn uint(input: &str) -> Result<U256, WalletError> {
    if input.is_empty()
        || input.len() > 78
        || !input.bytes().all(|c| c.is_ascii_digit())
        || (input.len() > 1 && input.starts_with('0'))
    {
        return Err(invalid("Expected a canonical unsigned integer"));
    }
    U256::from_dec_str(input).map_err(|_| invalid("Integer exceeds the supported range"))
}
fn u64_units(input: &str) -> Result<u64, WalletError> {
    let n = uint(input)?;
    if n > U256::from(i64::MAX as u64) {
        return Err(invalid("TRON integer exceeds its signed 64-bit range"));
    }
    Ok(n.low_u64())
}
fn sum(a: u64, b: u64) -> Result<u64, WalletError> {
    a.checked_add(b)
        .ok_or_else(|| invalid("TRON fee arithmetic overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, WalletError> {
    a.checked_mul(b)
        .ok_or_else(|| invalid("TRON fee arithmetic overflow"))
}
fn hex_bytes(input: &str, len: usize) -> Result<Vec<u8>, WalletError> {
    if input.len() != len * 2 || !input.bytes().all(|v| v.is_ascii_hexdigit()) {
        return Err(invalid(
            "TRON hexadecimal value has an invalid length or encoding",
        ));
    }
    hex::decode(input).map_err(|_| invalid("Malformed hexadecimal value"))
}
fn digest(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn normalize_address(input: &str) -> Result<String, WalletError> {
    let bytes = address_bytes(input)?;
    Ok(bs58::encode(bytes).with_check().into_string())
}
pub fn address_hex(input: &str) -> Result<String, WalletError> {
    Ok(hex::encode(address_bytes(input)?))
}
fn address_bytes(input: &str) -> Result<Vec<u8>, WalletError> {
    if input.len() != 34 || !input.starts_with('T') {
        return Err(invalid("Enter a complete TRON mainnet address"));
    }
    let bytes = bs58::decode(input)
        .with_check(None)
        .into_vec()
        .map_err(|_| invalid("The TRON address checksum is invalid"))?;
    if bytes.len() != 21
        || bytes[0] != 0x41
        || bytes[1..].iter().all(|b| *b == 0)
        || bs58::encode(&bytes).with_check().into_string() != input
    {
        return Err(invalid("The TRON address is invalid or unsupported"));
    }
    Ok(bytes)
}
fn participant(bytes: &[u8]) -> Result<String, WalletError> {
    if bytes.len() != 21 || bytes[0] != 0x41 {
        return Err(invalid("Invalid TRON participant address"));
    }
    Ok(bs58::encode(bytes).with_check().into_string())
}
fn key_address(key: &VerifyingKey) -> String {
    let point = key.to_sec1_point(false);
    let hash = Keccak256::digest(&point.as_bytes()[1..]);
    let mut bytes = vec![0x41];
    bytes.extend_from_slice(&hash[12..]);
    bs58::encode(bytes).with_check().into_string()
}
fn derive(vault_hex: &str, key: &[u8]) -> Result<SigningKey, WalletError> {
    let seed = crate::crypto::open_seed(vault_hex, key)?;
    let path: DerivationPath = DERIVATION_PATH.parse().map_err(|_| WalletError::Crypto)?;
    Ok(XPrv::derive_from_path(&*seed, &path)
        .map_err(|_| WalletError::Crypto)?
        .private_key()
        .clone())
}
pub fn wallet_address(vault_hex: String, vault_key: Vec<u8>) -> Result<String, WalletError> {
    let key = Zeroizing::new(vault_key);
    Ok(key_address(derive(&vault_hex, &key)?.verifying_key()))
}
fn parse_amount(input: &str) -> Result<U256, WalletError> {
    if input.len() > 90 {
        return Err(invalid("Amount is too long"));
    }
    let mut pieces = input.split('.');
    let whole = pieces.next().unwrap_or("");
    let fraction = pieces.next().unwrap_or("");
    if pieces.next().is_some()
        || (input.contains('.') && fraction.is_empty())
        || fraction.len() > 6
        || !fraction.bytes().all(|v| v.is_ascii_digit())
    {
        return Err(invalid("TRX and USDT support up to six decimal places"));
    }
    let whole = uint(whole)?;
    let mut f = fraction.to_owned();
    while f.len() < 6 {
        f.push('0');
    }
    let units = whole
        .checked_mul(U256::from(1_000_000))
        .and_then(|v| v.checked_add(U256::from_dec_str(&f).ok()?))
        .ok_or_else(|| invalid("Amount exceeds the integer range"))?;
    if units.is_zero() {
        return Err(invalid("Transfer amount must be positive"));
    }
    Ok(units)
}

// Exact fields from tronprotocol/protocol core/Tron.proto. Unsupported fields
// cannot be constructed; canonical re-encoding rejects their presence on input.
#[derive(Clone, PartialEq, Message)]
struct AnyMessage {
    #[prost(string, tag = "1")]
    type_url: String,
    #[prost(bytes = "vec", tag = "2")]
    value: Vec<u8>,
}
#[derive(Clone, PartialEq, Message)]
struct Contract {
    #[prost(int32, tag = "1")]
    kind: i32,
    #[prost(message, optional, tag = "2")]
    parameter: Option<AnyMessage>,
}
#[derive(Clone, PartialEq, Message)]
struct RawData {
    #[prost(bytes = "vec", tag = "1")]
    ref_block_bytes: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    ref_block_hash: Vec<u8>,
    #[prost(int64, tag = "8")]
    expiration: i64,
    #[prost(message, repeated, tag = "11")]
    contract: Vec<Contract>,
    #[prost(int64, tag = "14")]
    timestamp: i64,
    #[prost(int64, tag = "18")]
    fee_limit: i64,
}
#[derive(Clone, PartialEq, Message)]
struct WireTransaction {
    #[prost(message, optional, tag = "1")]
    raw_data: Option<RawData>,
    #[prost(bytes = "vec", repeated, tag = "2")]
    signature: Vec<Vec<u8>>,
}
#[derive(Clone, PartialEq, Message)]
struct Transfer {
    #[prost(bytes = "vec", tag = "1")]
    owner_address: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    to_address: Vec<u8>,
    #[prost(int64, tag = "3")]
    amount: i64,
}
#[derive(Clone, PartialEq, Message)]
struct Trigger {
    #[prost(bytes = "vec", tag = "1")]
    owner_address: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    contract_address: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    data: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub intent_id: String,
    pub network: String,
    pub from: String,
    pub to: String,
    pub asset: String,
    pub amount: String,
    pub maximum: bool,
    pub balance_sun: String,
    pub token_balance_units: String,
    pub ref_block_id: String,
    pub ref_block_number: u64,
    pub ref_block_timestamp_ms: u64,
    pub observed_at_ms: u64,
    pub recipient_active: bool,
    pub staked_bandwidth: u64,
    pub free_bandwidth: u64,
    pub available_energy: u64,
    pub bandwidth_price_sun: String,
    pub energy_price_sun: String,
    pub creation_fee_sun: String,
    pub creation_bandwidth_fee_sun: String,
    pub creation_bandwidth_rate: u64,
    pub max_fee_limit_sun: String,
    pub max_energy_factor: u64,
    pub energy_used: u64,
    pub energy_penalty: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Prepared {
    pub request: Request,
    pub amount_units: String,
    pub fee_reserve_units: String,
    pub estimated_fee_units: String,
    pub approval_digest: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub raw_data_hex: String,
    pub hash: String,
    pub bandwidth_bytes: u64,
    pub fee_limit_sun: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Signed {
    pub intent_id: String,
    pub network: String,
    pub from: String,
    pub raw_transaction: String,
    pub hash: String,
    pub approval_digest: String,
    pub amount_units: String,
    pub prepared_json: String,
    pub broadcast: Value,
}

fn raw(request: &Request, amount: U256, energy_limit: u64) -> Result<RawData, WalletError> {
    let owner = address_bytes(&request.from)?;
    let to = address_bytes(&request.to)?;
    let contract = if request.asset == "TRX" {
        if amount > U256::from(i64::MAX as u64) {
            return Err(invalid("TRX amount exceeds its protocol range"));
        }
        Contract {
            kind: 1,
            parameter: Some(AnyMessage {
                type_url: "type.googleapis.com/protocol.TransferContract".into(),
                value: Transfer {
                    owner_address: owner,
                    to_address: to,
                    amount: amount.low_u64() as i64,
                }
                .encode_to_vec(),
            }),
        }
    } else if request.asset == "USDT" {
        let mut data = hex::decode("a9059cbb").expect("fixed selector");
        data.extend([0u8; 12]);
        data.extend(&to[1..]);
        data.extend(amount.to_big_endian());
        Contract {
            kind: 31,
            parameter: Some(AnyMessage {
                type_url: "type.googleapis.com/protocol.TriggerSmartContract".into(),
                value: Trigger {
                    owner_address: owner,
                    contract_address: address_bytes(USDT)?,
                    data,
                }
                .encode_to_vec(),
            }),
        }
    } else {
        return Err(policy("TRON supports native TRX and official USDT only"));
    };
    let block = hex_bytes(&request.ref_block_id, 32)?;
    Ok(RawData {
        ref_block_bytes: request.ref_block_number.to_be_bytes()[6..].to_vec(),
        ref_block_hash: block[8..16].to_vec(),
        expiration: sum(request.observed_at_ms, TX_LIFETIME_MS)? as i64,
        contract: vec![contract],
        timestamp: request.observed_at_ms as i64,
        fee_limit: energy_limit as i64,
    })
}
fn size_for(request: &Request, amount: U256, fee_limit: u64) -> Result<u64, WalletError> {
    Ok(WireTransaction {
        raw_data: Some(raw(request, amount, fee_limit)?),
        signature: vec![vec![0; 65]],
    }
    .encoded_len() as u64
        + 64)
}
fn prepare_inner(request: Request) -> Result<Prepared, WalletError> {
    if request.network != NETWORK
        || request.intent_id.len() < 8
        || request.intent_id.len() > 80
        || !request
            .intent_id
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || b"-_".contains(&v))
    {
        return Err(policy("Invalid TRON network or payment identity"));
    }
    normalize_address(&request.from)?;
    normalize_address(&request.to)?;
    if request.to == USDT || request.from == request.to {
        return Err(policy("Choose a different personal receiving address"));
    }
    let block = hex_bytes(&request.ref_block_id, 32)?;
    if block[..8] != request.ref_block_number.to_be_bytes()
        || request.ref_block_number == 0
        || request.observed_at_ms > i64::MAX as u64 - TX_LIFETIME_MS
        || request.ref_block_timestamp_ms > sum(request.observed_at_ms, 5_000)?
        || request
            .observed_at_ms
            .saturating_sub(request.ref_block_timestamp_ms)
            > 60_000
    {
        return Err(policy("The TRON reference block is invalid or stale"));
    }
    let balance = u64_units(&request.balance_sun)?;
    let token_balance = uint(&request.token_balance_units)?;
    let byte_price = u64_units(&request.bandwidth_price_sun)?;
    let energy_price = u64_units(&request.energy_price_sun)?;
    let creation = u64_units(&request.creation_fee_sun)?;
    let creation_bw = u64_units(&request.creation_bandwidth_fee_sun)?;
    if byte_price == 0
        || byte_price > 100_000
        || energy_price == 0
        || energy_price > 100_000
        || creation > 10_000_000
        || creation_bw > 10_000_000
        || !(1..=10_000).contains(&request.creation_bandwidth_rate)
        || request.max_energy_factor > 100_000
    {
        return Err(policy(
            "TRON fee parameters exceed the reviewed wallet policy",
        ));
    }
    let fee_limit = if request.asset == "USDT" {
        if request.energy_used == 0
            || request.energy_used > 10_000_000
            || request.energy_penalty > request.energy_used
        {
            return Err(policy("A successful, exact USDT simulation is required"));
        }
        // Reserve against dynamic-energy factor changes rather than extrapolating
        // a fixed fee from a different recipient's transfer.
        let base = request.energy_used - request.energy_penalty;
        let worst = sum(mul(base, sum(10_000, request.max_energy_factor)?)?, 9_999)? / 10_000;
        let energy = worst.max(request.energy_used);
        let limit = sum(mul(mul(energy, energy_price)?, 120)?, 99)? / 100;
        if limit == 0
            || limit > MAX_ENERGY_FEE_SUN
            || limit > u64_units(&request.max_fee_limit_sun)?
        {
            return Err(policy(
                "The simulated TRON energy budget exceeds the approved policy limit",
            ));
        }
        limit
    } else if request.asset == "TRX" {
        if request.energy_used != 0 || request.energy_penalty != 0 {
            return Err(invalid("Native TRX does not execute a token contract"));
        }
        0
    } else {
        return Err(policy("TRON supports native TRX and official USDT only"));
    };
    let worst_size = size_for(
        &request,
        if request.asset == "TRX" {
            U256::from(i64::MAX as u64)
        } else {
            U256::MAX
        },
        fee_limit,
    )?;
    let creates_account = request.asset == "TRX" && !request.recipient_active;
    let activation = if creates_account { creation } else { 0 };
    let bandwidth_reserve = if creates_account {
        creation_bw
    } else {
        mul(worst_size, byte_price)?
    };
    let reserve = sum(sum(fee_limit, bandwidth_reserve)?, activation)?;
    let amount =
        if request.maximum {
            if request.asset == "TRX" {
                U256::from(balance.checked_sub(reserve).ok_or_else(|| {
                    policy("The TRX balance cannot cover the network fee reserve")
                })?)
            } else {
                token_balance
            }
        } else {
            parse_amount(&request.amount)?
        };
    if amount.is_zero() {
        return Err(policy(
            "There is no transferable balance after network fees",
        ));
    }
    if request.asset == "TRX" {
        if amount > U256::from(balance)
            || amount
                .checked_add(U256::from(reserve))
                .ok_or_else(|| invalid("Transfer total overflow"))?
                > U256::from(balance)
        {
            return Err(policy("TRX amount plus fee reserve exceeds the balance"));
        }
    } else if amount > token_balance || balance < reserve {
        return Err(policy(
            "The account needs enough USDT and TRX to cover the transfer and network fee reserve",
        ));
    }
    let data = raw(&request, amount, fee_limit)?;
    let bytes = data.encode_to_vec();
    let size = size_for(&request, amount, fee_limit)?;
    let bandwidth_estimate = if creates_account {
        if request.staked_bandwidth >= mul(size, request.creation_bandwidth_rate)? {
            0
        } else {
            creation_bw
        }
    } else if request.staked_bandwidth >= size || request.free_bandwidth >= size {
        0
    } else {
        mul(size, byte_price)?
    };
    let energy_estimate = mul(
        request.energy_used.saturating_sub(request.available_energy),
        energy_price,
    )?;
    let estimate = sum(sum(activation, bandwidth_estimate)?, energy_estimate)?;
    let created = request.observed_at_ms;
    let mut prepared = Prepared {
        request,
        amount_units: amount.to_string(),
        fee_reserve_units: reserve.to_string(),
        estimated_fee_units: estimate.to_string(),
        approval_digest: String::new(),
        created_at: created,
        expires_at: sum(created, TX_LIFETIME_MS)?,
        raw_data_hex: hex::encode(&bytes),
        hash: digest(&bytes),
        bandwidth_bytes: size,
        fee_limit_sun: fee_limit.to_string(),
    };
    let mut binding = b"local-wallet/tron/mainnet/review/v1\0".to_vec();
    binding.extend(serde_json::to_vec(&prepared).map_err(|_| WalletError::Crypto)?);
    prepared.approval_digest = digest(&binding);
    Ok(prepared)
}
pub fn prepare(request_json: &str) -> Result<Prepared, WalletError> {
    let prepared = prepare_inner(decode(request_json)?)?;
    require_fresh(&prepared)?;
    Ok(prepared)
}
pub fn validate_prepared(prepared_json: &str) -> Result<Prepared, WalletError> {
    let value: Prepared = decode(prepared_json)?;
    let expected = prepare_inner(value.request.clone())?;
    if expected != value {
        return Err(policy("The reviewed TRON payment was changed"));
    }
    Ok(value)
}
fn require_fresh(prepared: &Prepared) -> Result<(), WalletError> {
    let now = now_ms()?;
    if prepared.created_at > sum(now, 5_000)?
        || now.saturating_sub(prepared.created_at) > MAX_REVIEW_AGE_MS
        || prepared.expires_at <= sum(now, 15_000)?
    {
        return Err(policy(
            "This TRON review expired. Review a new transfer before signing",
        ));
    }
    Ok(())
}
fn canonical<T: Message + Default>(bytes: &[u8]) -> Result<T, WalletError> {
    let decoded = T::decode(bytes).map_err(|_| invalid("Malformed TRON protobuf"))?;
    if decoded.encode_to_vec() != bytes {
        return Err(policy("Noncanonical or unsupported TRON protobuf fields"));
    }
    Ok(decoded)
}
fn broadcast_value(raw: &RawData, signature: &[u8], txid: &str) -> Result<Value, WalletError> {
    let contract = raw
        .contract
        .first()
        .ok_or_else(|| invalid("Missing TRON contract"))?;
    let parameter = contract
        .parameter
        .as_ref()
        .ok_or_else(|| invalid("Missing TRON contract parameter"))?;
    let (kind, value) = if contract.kind == 1 {
        let transfer: Transfer = canonical(&parameter.value)?;
        (
            "TransferContract",
            json!({"owner_address":hex::encode(transfer.owner_address),"to_address":hex::encode(transfer.to_address),"amount":transfer.amount}),
        )
    } else if contract.kind == 31 {
        let trigger: Trigger = canonical(&parameter.value)?;
        (
            "TriggerSmartContract",
            json!({"owner_address":hex::encode(trigger.owner_address),"contract_address":hex::encode(trigger.contract_address),"data":hex::encode(trigger.data)}),
        )
    } else {
        return Err(policy("Unsupported TRON contract"));
    };
    let mut data = json!({"ref_block_bytes":hex::encode(&raw.ref_block_bytes),"ref_block_hash":hex::encode(&raw.ref_block_hash),"expiration":raw.expiration,"contract":[{"type":kind,"parameter":{"type_url":parameter.type_url,"value":value}}],"timestamp":raw.timestamp});
    if raw.fee_limit > 0 {
        data["fee_limit"] = json!(raw.fee_limit);
    }
    Ok(
        json!({"visible":false,"txID":txid,"raw_data":data,"raw_data_hex":hex::encode(raw.encode_to_vec()),"signature":[hex::encode(signature)]}),
    )
}
pub fn sign(
    vault_hex: String,
    vault_key: Vec<u8>,
    prepared_json: String,
    approval_digest: String,
) -> Result<Signed, WalletError> {
    let prepared = validate_prepared(&prepared_json)?;
    require_fresh(&prepared)?;
    if prepared.approval_digest != approval_digest {
        return Err(policy("TRON approval does not match the reviewed payment"));
    }
    let secret = Zeroizing::new(vault_key);
    let key = derive(&vault_hex, &secret)?;
    if key_address(key.verifying_key()) != prepared.request.from {
        return Err(policy("The vault does not own this TRON address"));
    }
    let bytes = hex::decode(&prepared.raw_data_hex).map_err(|_| WalletError::Crypto)?;
    let raw: RawData = canonical(&bytes)?;
    let hash = Sha256::digest(&bytes);
    let (signature, recovery) = key.sign_prehash_recoverable(&hash);
    if recovery.to_byte() > 1 || signature.normalize_s() != signature {
        return Err(WalletError::Crypto);
    }
    let mut sig = signature.to_bytes().to_vec();
    sig.push(recovery.to_byte());
    let wire = WireTransaction {
        raw_data: Some(raw.clone()),
        signature: vec![sig.clone()],
    }
    .encode_to_vec();
    let signed = Signed {
        intent_id: prepared.request.intent_id.clone(),
        network: NETWORK.into(),
        from: prepared.request.from.clone(),
        raw_transaction: hex::encode(wire),
        hash: prepared.hash.clone(),
        approval_digest: prepared.approval_digest.clone(),
        amount_units: prepared.amount_units.clone(),
        prepared_json,
        broadcast: broadcast_value(&raw, &sig, &prepared.hash)?,
    };
    validate_signed(&serde_json::to_string(&signed).map_err(|_| WalletError::Crypto)?)
}
pub fn validate_signed(signed_json: &str) -> Result<Signed, WalletError> {
    let signed: Signed = decode(signed_json)?;
    let prepared = validate_prepared(&signed.prepared_json)?;
    if signed.network != NETWORK
        || signed.intent_id != prepared.request.intent_id
        || signed.from != prepared.request.from
        || signed.approval_digest != prepared.approval_digest
        || signed.amount_units != prepared.amount_units
        || signed.hash != prepared.hash
        || signed.raw_transaction.len() > MAX_SIGNED_BYTES * 2
    {
        return Err(policy(
            "Signed TRON metadata does not match the approved payment",
        ));
    }
    let bytes =
        hex::decode(&signed.raw_transaction).map_err(|_| invalid("Malformed signed TRON bytes"))?;
    let wire: WireTransaction = canonical(&bytes)?;
    let raw = wire
        .raw_data
        .ok_or_else(|| invalid("Missing TRON transaction payload"))?;
    if wire.signature.len() != 1
        || wire.signature[0].len() != 65
        || hex::encode(raw.encode_to_vec()) != prepared.raw_data_hex
    {
        return Err(policy(
            "TRON signed payload differs from the approved transfer",
        ));
    }
    let (asset, from, to, amount) = transfer_parts(&raw)?;
    if asset != prepared.request.asset
        || from != prepared.request.from
        || to != prepared.request.to
        || amount.to_string() != prepared.amount_units
    {
        return Err(policy(
            "TRON wire transfer differs from its displayed intent",
        ));
    }
    let signature = &wire.signature[0];
    let sig = Signature::from_slice(&signature[..64]).map_err(|_| WalletError::Crypto)?;
    if sig.normalize_s() != sig || signature[64] > 1 {
        return Err(policy("Unsupported TRON signature encoding"));
    }
    let key = VerifyingKey::recover_from_prehash(
        &hex_bytes(&signed.hash, 32)?,
        &sig,
        RecoveryId::from_byte(signature[64]).ok_or(WalletError::Crypto)?,
    )
    .map_err(|_| WalletError::Crypto)?;
    if key_address(&key) != signed.from
        || broadcast_value(&raw, signature, &signed.hash)? != signed.broadcast
    {
        return Err(policy("TRON signer or broadcast payload was changed"));
    }
    Ok(signed)
}

#[derive(Deserialize)]
struct ReturnStatus {
    #[serde(rename = "contractRet")]
    contract_ret: String,
}
#[derive(Deserialize)]
struct TransactionBody {
    #[serde(rename = "txID")]
    id: String,
    raw_data_hex: String,
    #[serde(default)]
    signature: Vec<String>,
    ret: Vec<ReturnStatus>,
}
#[derive(Deserialize)]
struct Receipt {
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    energy_fee: u64,
    #[serde(default)]
    net_fee: u64,
}
#[derive(Deserialize)]
struct Log {
    address: String,
    topics: Vec<String>,
    data: String,
}
#[derive(Deserialize)]
struct TransactionInfo {
    id: String,
    #[serde(default)]
    fee: u64,
    #[serde(rename = "blockNumber")]
    block_number: u64,
    #[serde(rename = "blockTimeStamp")]
    block_time_stamp: u64,
    #[serde(default)]
    result: Option<String>,
    #[serde(default, rename = "contractResult")]
    contract_result: Vec<String>,
    receipt: Receipt,
    #[serde(default)]
    log: Vec<Log>,
}
#[derive(Deserialize)]
struct HeaderData {
    number: u64,
    timestamp: u64,
}
#[derive(Deserialize)]
struct Header {
    raw_data: HeaderData,
}
#[derive(Deserialize)]
struct Block {
    #[serde(rename = "blockID")]
    block_id: String,
    block_header: Header,
    transactions: Vec<TransactionBody>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    network: String,
    chain_id: String,
    finality_source: String,
    transaction: TransactionBody,
    info: TransactionInfo,
    block: Block,
    solid_height: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub state: String,
    pub hash: String,
    pub block_number: u64,
    pub block_hash: String,
    pub actual_fee_units: String,
    pub amount_units: String,
    pub fee_exceeds_reserve: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Activity {
    pub id: String,
    pub chain_id: i64,
    pub network: String,
    pub address: String,
    pub direction: String,
    pub asset: String,
    pub asset_contract: Option<String>,
    pub amount_units: String,
    pub amount: String,
    pub from: String,
    pub to: String,
    pub transaction_hash: String,
    pub block_number: u64,
    pub block_hash: String,
    pub log_index: Option<u64>,
    pub timestamp: u64,
    pub inclusion: String,
}
fn evidence(input: &str) -> Result<Evidence, WalletError> {
    let e: Evidence = decode(input)?;
    if e.network != NETWORK
        || e.chain_id != MAINNET_CHAIN_ID
        || e.finality_source != "walletsolidity"
        || e.solid_height < e.info.block_number
    {
        return Err(policy(
            "TRON receipt is not a mainnet solidified observation",
        ));
    }
    let id = hex::encode(hex_bytes(&e.transaction.id, 32)?);
    let raw = hex::decode(&e.transaction.raw_data_hex)
        .map_err(|_| invalid("Malformed transaction raw bytes"))?;
    if raw.len() > 65_536
        || digest(&raw) != id
        || e.info.id.to_lowercase() != id
        || e.transaction.ret.len() != 1
    {
        return Err(policy("TRON transaction and receipt identity do not match"));
    }
    let block_id = hex_bytes(&e.block.block_id, 32)?;
    let height = e.block.block_header.raw_data.number;
    if height != e.info.block_number
        || block_id[..8] != height.to_be_bytes()
        || e.info.block_time_stamp != e.block.block_header.raw_data.timestamp
    {
        return Err(policy("TRON receipt does not match its canonical block"));
    }
    let matches: Vec<_> = e
        .block
        .transactions
        .iter()
        .filter(|tx| tx.id.to_lowercase() == id)
        .collect();
    if matches.len() != 1
        || matches[0].raw_data_hex.to_lowercase() != e.transaction.raw_data_hex.to_lowercase()
        || matches[0].ret.len() != 1
        || matches[0].ret[0].contract_ret != e.transaction.ret[0].contract_ret
        || matches[0].signature != e.transaction.signature
    {
        return Err(policy(
            "TRON transaction was not found unchanged in the solidified block",
        ));
    }
    if sum(e.info.receipt.energy_fee, e.info.receipt.net_fee)? > e.info.fee {
        return Err(invalid(
            "TRON fee components exceed the reported transaction fee",
        ));
    }
    Ok(e)
}
fn execution_success(e: &Evidence, token: bool) -> Result<bool, WalletError> {
    let result = &e.transaction.ret[0].contract_ret;
    let known = [
        "SUCCESS",
        "REVERT",
        "OUT_OF_ENERGY",
        "OUT_OF_TIME",
        "ILLEGAL_OPERATION",
        "BAD_JUMP_DESTINATION",
        "OUT_OF_MEMORY",
        "PRECOMPILED_CONTRACT",
        "STACK_TOO_SMALL",
        "STACK_TOO_LARGE",
        "JVM_STACK_OVER_FLOW",
        "STACK_OVERFLOW",
        "TRANSFER_FAILED",
        "INVALID_CODE",
        "UNKNOWN",
    ];
    if !known.contains(&result.as_str()) {
        return Err(invalid("Unknown TRON execution status"));
    }
    if result != "SUCCESS" {
        if token && e.info.receipt.result.as_deref() != Some(result.as_str()) {
            return Err(policy(
                "TRON transaction and receipt failure statuses conflict",
            ));
        }
        return Ok(false);
    }
    if e.info.result.as_deref() == Some("FAILED") {
        return Err(policy(
            "TRON transaction and receipt execution statuses conflict",
        ));
    }
    if token && e.info.receipt.result.as_deref() != Some("SUCCESS") {
        return Err(policy(
            "The USDT execution receipt is unavailable or inconsistent",
        ));
    }
    if !token
        && e.info
            .receipt
            .result
            .as_deref()
            .is_some_and(|r| r != "SUCCESS")
    {
        return Err(policy("TRX execution statuses conflict"));
    }
    Ok(true)
}
fn transfer_parts(raw: &RawData) -> Result<(String, String, String, U256), WalletError> {
    if raw.contract.len() != 1
        || raw.ref_block_bytes.len() != 2
        || raw.ref_block_hash.len() != 8
        || raw.timestamp <= 0
        || raw.expiration <= raw.timestamp
        || raw.fee_limit < 0
    {
        return Err(policy("Unsupported TRON transaction shape"));
    }
    let c = &raw.contract[0];
    let p = c
        .parameter
        .as_ref()
        .ok_or_else(|| invalid("Missing contract"))?;
    if c.kind == 1 && p.type_url == "type.googleapis.com/protocol.TransferContract" {
        let t: Transfer = canonical(&p.value)?;
        if t.amount <= 0 || raw.fee_limit != 0 {
            return Err(policy("Invalid TRX transfer"));
        }
        Ok((
            "TRX".into(),
            participant(&t.owner_address)?,
            participant(&t.to_address)?,
            U256::from(t.amount as u64),
        ))
    } else if c.kind == 31 && p.type_url == "type.googleapis.com/protocol.TriggerSmartContract" {
        let t: Trigger = canonical(&p.value)?;
        if t.contract_address != address_bytes(USDT)?
            || t.data.len() != 68
            || t.data[..4] != [0xa9, 0x05, 0x9c, 0xbb]
            || t.data[4..16] != [0; 12]
            || raw.fee_limit == 0
        {
            return Err(policy(
                "Only the exact official USDT transfer method is supported",
            ));
        }
        let mut to = vec![0x41];
        to.extend(&t.data[16..36]);
        let amount = U256::from_big_endian(&t.data[36..]);
        if amount.is_zero() {
            return Err(policy("Zero-value transfers are not deposit evidence"));
        }
        Ok((
            "USDT".into(),
            participant(&t.owner_address)?,
            participant(&to)?,
            amount,
        ))
    } else {
        Err(policy("Unsupported TRON contract or token"))
    }
}
fn matching_token_logs(
    e: &Evidence,
    from: &str,
    to: &str,
    amount: U256,
) -> Result<Vec<u64>, WalletError> {
    let contract = hex::encode(&address_bytes(USDT)?[1..]);
    let from = hex::encode(&address_bytes(from)?[1..]);
    let to = hex::encode(&address_bytes(to)?[1..]);
    let mut found = Vec::new();
    for (index, log) in e.info.log.iter().enumerate() {
        if log.address.to_lowercase() != contract
            || log
                .topics
                .first()
                .is_none_or(|s| s.to_lowercase() != TRANSFER_EVENT)
        {
            continue;
        }
        if log.topics.len() != 3 {
            return Err(invalid("Malformed USDT Transfer event"));
        }
        let f = hex_bytes(&log.topics[1], 32)?;
        let t = hex_bytes(&log.topics[2], 32)?;
        let a = hex_bytes(&log.data, 32)?;
        if f[..12] != [0; 12] || t[..12] != [0; 12] {
            return Err(invalid("Malformed USDT participant topic"));
        }
        if hex::encode(&f[12..]) == from
            && hex::encode(&t[12..]) == to
            && U256::from_big_endian(&a) == amount
        {
            found.push(index as u64);
        }
    }
    Ok(found)
}
pub fn validate_receipt(
    prepared_json: &str,
    signed_json: &str,
    evidence_json: &str,
) -> Result<Observation, WalletError> {
    let p = validate_prepared(prepared_json)?;
    let s = validate_signed(signed_json)?;
    if validate_prepared(&s.prepared_json)? != p {
        return Err(policy(
            "The signed TRON transfer belongs to a different review",
        ));
    }
    let e = evidence(evidence_json)?;
    let signature = s.broadcast["signature"][0]
        .as_str()
        .ok_or_else(|| invalid("Missing signature"))?;
    if e.transaction.id.to_lowercase() != s.hash
        || e.transaction.raw_data_hex.to_lowercase() != p.raw_data_hex
        || e.transaction.signature != vec![signature.to_owned()]
    {
        return Err(policy("The receipt is for a different signed transaction"));
    }
    let successful = execution_success(&e, p.request.asset == "USDT")?;
    if successful
        && p.request.asset == "USDT"
        && (e.info.contract_result.len() != 1
            || hex_bytes(&e.info.contract_result[0], 32)? != U256::one().to_big_endian()
            || matching_token_logs(&e, &p.request.from, &p.request.to, uint(&p.amount_units)?)?
                .len()
                != 1)
    {
        return Err(policy(
            "USDT receipt does not prove the approved token delivery",
        ));
    }
    Ok(Observation {
        state: if successful { "finalized" } else { "reverted" }.into(),
        hash: s.hash,
        block_number: e.info.block_number,
        block_hash: e.block.block_id.to_lowercase(),
        actual_fee_units: e.info.fee.to_string(),
        amount_units: p.amount_units,
        fee_exceeds_reserve: U256::from(e.info.fee) > uint(&p.fee_reserve_units)?,
    })
}
pub fn validate_activity(
    wallet_address: &str,
    evidence_json: &str,
) -> Result<Vec<Activity>, WalletError> {
    let wallet = normalize_address(wallet_address)?;
    let e = evidence(evidence_json)?;
    let raw_bytes =
        hex::decode(&e.transaction.raw_data_hex).map_err(|_| invalid("Malformed TRON raw data"))?;
    // Activity is read-only. An exchange may deliver USDT through a batch call,
    // so this parser accepts canonical token Transfer logs from other successful
    // contract methods. The signing boundary above remains transfer-only.
    let raw =
        RawData::decode(&*raw_bytes).map_err(|_| invalid("Malformed TRON transaction protobuf"))?;
    if raw.contract.len() != 1 {
        return Err(policy("Unsupported TRON activity contract count"));
    }
    let contract = &raw.contract[0];
    if !execution_success(&e, contract.kind == 31)? {
        return Ok(Vec::new());
    }
    let mut entries: Vec<(String, String, String, U256, Option<u64>)> = Vec::new();
    if contract.kind == 1 {
        let parameter = contract
            .parameter
            .as_ref()
            .ok_or_else(|| invalid("Missing TRON transfer parameter"))?;
        if parameter.type_url != "type.googleapis.com/protocol.TransferContract" {
            return Err(invalid("TRON transfer type mismatch"));
        }
        let transfer: Transfer = canonical(&parameter.value)?;
        if transfer.amount > 0 {
            entries.push((
                "TRX".into(),
                participant(&transfer.owner_address)?,
                participant(&transfer.to_address)?,
                U256::from(transfer.amount as u64),
                None,
            ));
        }
    }
    if contract.kind == 31 {
        let token_hex = hex::encode(&address_bytes(USDT)?[1..]);
        for (index, log) in e.info.log.iter().enumerate() {
            if log.address.to_lowercase() != token_hex
                || log
                    .topics
                    .first()
                    .is_none_or(|v| v.to_lowercase() != TRANSFER_EVENT)
            {
                continue;
            }
            if log.topics.len() != 3 {
                return Err(invalid("Malformed USDT Transfer log"));
            }
            let f = hex_bytes(&log.topics[1], 32)?;
            let t = hex_bytes(&log.topics[2], 32)?;
            if f[..12] != [0; 12] || t[..12] != [0; 12] {
                return Err(invalid("Malformed USDT participant topics"));
            }
            let mut from = vec![0x41];
            from.extend(&f[12..]);
            let mut to = vec![0x41];
            to.extend(&t[12..]);
            let amount = U256::from_big_endian(&hex_bytes(&log.data, 32)?);
            if !amount.is_zero() {
                entries.push((
                    "USDT".into(),
                    participant(&from)?,
                    participant(&to)?,
                    amount,
                    Some(index as u64),
                ));
            }
        }
    }
    let hash = e.transaction.id.to_lowercase();
    entries
        .into_iter()
        .filter(|(_, from, to, _, _)| from == &wallet || to == &wallet)
        .map(|(asset, from, to, amount, log_index)| {
            Ok(Activity {
                id: format!(
                    "tron:{}:{}",
                    hash,
                    log_index.map_or_else(|| "native".into(), |n| n.to_string())
                ),
                chain_id: -195,
                network: NETWORK.into(),
                address: wallet.clone(),
                direction: if from == wallet && to == wallet {
                    "self"
                } else if to == wallet {
                    "received"
                } else {
                    "sent"
                }
                .into(),
                asset_contract: if asset == "USDT" {
                    Some(USDT.into())
                } else {
                    None
                },
                asset,
                amount_units: amount.to_string(),
                amount: crate::crypto::format_amount(amount.to_string(), 6)?,
                from,
                to,
                transaction_hash: hash.clone(),
                block_number: e.info.block_number,
                block_hash: e.block.block_id.to_lowercase(),
                log_index,
                timestamp: e.info.block_time_stamp / 1_000,
                inclusion: "finalized".into(),
            })
        })
        .collect()
}
