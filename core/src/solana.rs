//! Restricted Solana mainnet transfers. No arbitrary program signing, approvals,
//! address lookup tables, durable nonces, staking, swaps or token-account closing.
//!
//! The SDK constructs legacy messages; Ed25519 and SLIP-0010 come from maintained
//! libraries. Review commits to every message byte and fee/rent quote. A retry is
//! the identical signed transaction, never a new blockhash or second signature.
//! RPC observations are independently checked for consistency, not represented as
//! cryptographic proofs of the Solana ledger.

use crate::{crypto, WalletError};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize, Serialize,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use solana_hash::Hash;
use solana_instruction::{AccountMeta, Instruction};
use solana_message::legacy::Message;
use solana_pubkey::Pubkey;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use zeroize::Zeroizing;

pub const NETWORK: &str = "solana-mainnet";
pub const GENESIS_HASH: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
pub const DERIVATION_PATH: &str = "m/44'/501'/0'/0'";
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub const EURC_MINT: &str = "HzwqbKZw8HxMN6bF2yFZNrht3c2iXXzpKcFu7uBEDKtr";
pub const USDT_MINT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
pub const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
pub const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const ASSOCIATED_TOKEN_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
pub const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";
pub const MAX_SOURCE_ACCOUNTS: usize = 8;
pub const MAX_PACKET_BYTES: usize = 1_232;
pub const APPROVAL_LIFETIME_SECONDS: u64 = 90;
const MAX_JSON: usize = 32_768;
const MAX_EVIDENCE_JSON: usize = 2_000_000;
const MAX_FEE_LAMPORTS: u64 = 50_000_000;
const MAX_RENT_LAMPORTS: u64 = 10_000_000;
const APPROVAL_DOMAIN: &[u8] = b"LocalWallet/Solana/transfer-review/v1\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Asset {
    #[serde(rename = "SOL")]
    Sol,
    #[serde(rename = "USDC")]
    Usdc,
    #[serde(rename = "EURC")]
    Eurc,
    #[serde(rename = "USDT")]
    Usdt,
}

impl Asset {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Sol => "SOL",
            Self::Usdc => "USDC",
            Self::Eurc => "EURC",
            Self::Usdt => "USDT",
        }
    }
    pub fn decimals(self) -> u8 {
        if self == Self::Sol {
            9
        } else {
            6
        }
    }
    pub fn mint(self) -> Option<&'static str> {
        match self {
            Self::Sol => None,
            Self::Usdc => Some(USDC_MINT),
            Self::Eurc => Some(EURC_MINT),
            Self::Usdt => Some(USDT_MINT),
        }
    }
    fn from_mint(mint: &str) -> Option<Self> {
        match mint {
            USDC_MINT => Some(Self::Usdc),
            EURC_MINT => Some(Self::Eurc),
            USDT_MINT => Some(Self::Usdt),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAccount {
    pub address: String,
    /// Exact amount withdrawn from this verified token account, not its balance.
    pub amount_units: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub intent_id: String,
    pub network: String,
    pub genesis_hash: String,
    pub from: String,
    pub to: String,
    pub asset: Asset,
    pub amount: String,
    pub recent_blockhash: String,
    pub last_valid_block_height: u64,
    pub context_slot: u64,
    pub fee_lamports: String,
    pub rent_lamports: String,
    pub create_destination_ata: bool,
    pub source_accounts: Vec<SourceAccount>,
    pub compute_unit_limit: u32,
    pub compute_unit_price_micro_lamports: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prepared {
    pub request: Request,
    pub amount_units: String,
    pub fee_reserve_units: String,
    /// Base64 legacy message, WITHOUT a signature or signature-count prefix.
    pub unsigned_transaction: String,
    pub destination_token_account: Option<String>,
    pub approval_digest: String,
    pub created_at: u64,
    pub expires_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signed {
    pub intent_id: String,
    pub network: String,
    pub from: String,
    pub raw_transaction: String,
    /// Base58 first signature, Solana's transaction identifier.
    pub hash: String,
    pub approval_digest: String,
    pub amount_units: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub status: String,
    pub success: bool,
    pub finalized: bool,
    pub hash: String,
    pub block_number: u64,
    pub block_hash: String,
    /// Actual network fee, kept separate from refundable token-account rent.
    pub fee_units: String,
    pub account_rent_units: String,
    pub total_cost_units: String,
    /// Preserve a real inclusion even if a live quote underestimated a cost.
    pub cost_exceeds_review: bool,
    pub confirmations: u64,
    pub observed_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Activity {
    pub network: String,
    pub asset: String,
    pub direction: String,
    pub amount_units: String,
    pub hash: String,
    pub block_number: u64,
    pub block_hash: String,
    pub finalized: bool,
    pub from: String,
    pub to: String,
    pub event_index: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpiryEvidence {
    pub network: String,
    pub genesis_hash: String,
    pub recent_blockhash: String,
    pub last_valid_block_height: u64,
    pub context_slot: u64,
    pub finalized_context_slot: u64,
    pub current_finalized_block_height: u64,
    pub history_search_enabled: bool,
    pub signature_found: bool,
    pub blockhash_valid: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExpiryObservation {
    /// The blockhash can no longer be submitted. This is NOT proof that the
    /// transaction never landed: a provider may have pruned transaction history.
    pub expired: bool,
    pub rebroadcastable: bool,
    pub status: String,
}

fn invalid(reason: impl Into<String>) -> WalletError {
    WalletError::InvalidInput {
        reason: reason.into(),
    }
}
fn policy(reason: impl Into<String>) -> WalletError {
    WalletError::Policy {
        reason: reason.into(),
    }
}
fn parse_json<T: serde::de::DeserializeOwned>(text: &str, limit: usize) -> Result<T, WalletError> {
    if text.len() > limit {
        return Err(invalid("Solana input exceeds its size limit"));
    }
    // Value's default deserializer overwrites duplicate keys. RPC evidence and
    // durable replay must reject them even when called directly through FFI.
    let value: StrictJson =
        serde_json::from_str(text).map_err(|_| invalid("Malformed or ambiguous Solana data"))?;
    serde_json::from_value(value.0).map_err(|_| invalid("Malformed Solana data"))
}
struct StrictJson(Value);
impl<'de> Deserialize<'de> for StrictJson {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictJson;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("unambiguous JSON")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(StrictJson(Value::Bool(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(StrictJson(Value::Number(v.into())))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(StrictJson(Value::Number(v.into())))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| StrictJson(Value::Number(n)))
                    .ok_or_else(|| E::custom("invalid JSON number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StrictJson(Value::String(v.into())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(StrictJson(Value::String(v)))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictJson(Value::Null))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictJson(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<StrictJson>()? {
                    values.push(value.0);
                }
                Ok(StrictJson(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, value)) = object.next_entry::<String, StrictJson>()? {
                    if values.insert(key, value.0).is_some() {
                        return Err(de::Error::custom("duplicate JSON field"));
                    }
                }
                Ok(StrictJson(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}
fn now() -> Result<u64, WalletError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|x| x.as_secs())
            .map_err(|_| policy("Device clock is invalid"))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let milliseconds = js_sys::Date::now();
        if !milliseconds.is_finite() || milliseconds < 0.0 {
            return Err(policy("Device clock is invalid"));
        }
        Ok((milliseconds / 1000.0) as u64)
    }
}
fn identity(network: &str, genesis_hash: &str) -> Result<(), WalletError> {
    if network != NETWORK || genesis_hash != GENESIS_HASH {
        return Err(policy("Solana mainnet identity does not match"));
    }
    Ok(())
}
fn pubkey(text: &str) -> Result<Pubkey, WalletError> {
    if !(32..=44).contains(&text.len()) {
        return Err(invalid("Invalid Solana address"));
    }
    let key = Pubkey::from_str(text).map_err(|_| invalid("Invalid Solana address"))?;
    if key.to_string() != text {
        return Err(invalid("Noncanonical Solana address"));
    }
    Ok(key)
}
fn wallet_key(text: &str) -> Result<Pubkey, WalletError> {
    let key = pubkey(text)?;
    // Restricted to ordinary wallet recipients. Program-derived addresses and
    // program/mint accounts need a different receiving flow, not a blind send.
    if !key.is_on_curve() || key.to_bytes() == [0; 32] {
        return Err(policy("Use an ordinary Solana wallet address"));
    }
    let verifying = VerifyingKey::from_bytes(&key.to_bytes())
        .map_err(|_| invalid("Invalid Ed25519 wallet address"))?;
    if verifying.is_weak() {
        return Err(policy("Weak Ed25519 wallet address"));
    }
    Ok(key)
}
pub fn validate_address(text: &str) -> Result<String, WalletError> {
    Ok(wallet_key(text)?.to_string())
}
fn hash(text: &str) -> Result<Hash, WalletError> {
    if text.len() < 32 || text.len() > 44 {
        return Err(invalid("Invalid Solana blockhash"));
    }
    let value = Hash::from_str(text).map_err(|_| invalid("Invalid Solana blockhash"))?;
    if value.to_string() != text || value.to_bytes() == [0; 32] {
        return Err(invalid("Invalid Solana blockhash"));
    }
    Ok(value)
}
fn units(text: &str) -> Result<u64, WalletError> {
    if text.is_empty()
        || text.len() > 20
        || !text.bytes().all(|b| b.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return Err(invalid("Amount must be an unsigned canonical integer"));
    }
    text.parse()
        .map_err(|_| invalid("Amount exceeds Solana's integer range"))
}
pub fn parse_amount(text: &str, decimals: u8) -> Result<u64, WalletError> {
    if decimals > 9 || text.is_empty() || text.len() > 32 || text.trim() != text {
        return Err(invalid("Invalid Solana amount"));
    }
    let mut split = text.split('.');
    let whole = split
        .next()
        .ok_or_else(|| invalid("Invalid Solana amount"))?;
    let fraction = split.next().unwrap_or("");
    if split.next().is_some()
        || (text.contains('.') && fraction.is_empty())
        || fraction.len() > decimals as usize
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid("Amount has invalid or excessive decimals"));
    }
    let whole = units(whole)?;
    let scale = 10u64.pow(decimals as u32);
    let fractional = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<u64>()
            .map_err(|_| invalid("Invalid fractional amount"))?
            * 10u64.pow(decimals as u32 - fraction.len() as u32)
    };
    whole
        .checked_mul(scale)
        .and_then(|v| v.checked_add(fractional))
        .ok_or_else(|| invalid("Amount exceeds Solana's integer range"))
}
fn decode_base64(text: &str, limit: usize) -> Result<Vec<u8>, WalletError> {
    if text.len() > limit.saturating_mul(4).div_ceil(3) + 4 {
        return Err(invalid("Solana binary input is too large"));
    }
    let bytes = BASE64
        .decode(text)
        .map_err(|_| invalid("Invalid base64 Solana data"))?;
    if bytes.len() > limit || BASE64.encode(&bytes) != text {
        return Err(invalid("Noncanonical Solana binary data"));
    }
    Ok(bytes)
}
fn signature(text: &str) -> Result<[u8; 64], WalletError> {
    if !(64..=88).contains(&text.len()) {
        return Err(invalid("Invalid Solana transaction identifier"));
    }
    let bytes = bs58::decode(text)
        .into_vec()
        .map_err(|_| invalid("Invalid Solana transaction identifier"))?;
    if bs58::encode(&bytes).into_string() != text {
        return Err(invalid("Noncanonical Solana transaction identifier"));
    }
    bytes
        .try_into()
        .map_err(|_| invalid("Invalid Solana signature length"))
}

fn signing_key(vault_hex: &str, key: &[u8]) -> Result<SigningKey, WalletError> {
    let seed = crypto::open_seed(vault_hex, key)?;
    let private = Zeroizing::new(slip10_ed25519::derive_ed25519_private_key(
        &*seed,
        &[44, 501, 0, 0],
    ));
    Ok(SigningKey::from_bytes(&private))
}
pub fn address(vault_hex: &str, key: &[u8]) -> Result<String, WalletError> {
    Ok(bs58::encode(signing_key(vault_hex, key)?.verifying_key().as_bytes()).into_string())
}
fn associated_account(owner: &Pubkey, mint: &Pubkey) -> Result<Pubkey, WalletError> {
    let token = pubkey(TOKEN_PROGRAM)?;
    Pubkey::try_find_program_address(
        &[owner.as_ref(), token.as_ref(), mint.as_ref()],
        &pubkey(ASSOCIATED_TOKEN_PROGRAM)?,
    )
    .map(|(key, _)| key)
    .ok_or_else(|| policy("Unable to derive the associated token account"))
}
pub fn token_account(owner: &str, symbol: &str) -> Result<String, WalletError> {
    let asset = match symbol {
        "USDC" => Asset::Usdc,
        "EURC" => Asset::Eurc,
        "USDT" => Asset::Usdt,
        _ => return Err(policy("Unsupported Solana token")),
    };
    Ok(associated_account(
        &wallet_key(owner)?,
        &pubkey(
            asset
                .mint()
                .ok_or_else(|| policy("Native SOL has no token account"))?,
        )?,
    )?
    .to_string())
}

fn construct(request: &Request) -> Result<(Message, u64, u64, Option<String>), WalletError> {
    identity(&request.network, &request.genesis_hash)?;
    if request.intent_id.len() < 16
        || request.intent_id.len() > 80
        || !request
            .intent_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(invalid("Invalid Solana payment intent"));
    }
    let from = wallet_key(&request.from)?;
    let to = wallet_key(&request.to)?;
    if from == to {
        return Err(policy("Sender and recipient are the same wallet"));
    }
    let blockhash = hash(&request.recent_blockhash)?;
    if request.context_slot == 0 || request.last_valid_block_height == 0 {
        return Err(policy("Missing recent blockhash context"));
    }
    let amount = parse_amount(&request.amount, request.asset.decimals())?;
    if amount == 0 {
        return Err(policy("Transfer amount must be greater than zero"));
    }
    let fee = units(&request.fee_lamports)?;
    let rent = units(&request.rent_lamports)?;
    if fee == 0 || fee > MAX_FEE_LAMPORTS || rent > MAX_RENT_LAMPORTS {
        return Err(policy(
            "Solana fee or rent exceeds the configured safety limit",
        ));
    }
    let priority_price = units(&request.compute_unit_price_micro_lamports)?;
    if request.compute_unit_limit == 0 || request.compute_unit_limit > 1_400_000 {
        return Err(policy("Invalid Solana compute limit"));
    }
    let priority =
        (request.compute_unit_limit as u128 * priority_price as u128).div_ceil(1_000_000);
    if priority >= fee as u128 {
        return Err(policy("Fee quote does not cover the base and priority fee"));
    }
    let reserve = fee
        .checked_add(rent)
        .ok_or_else(|| policy("Solana cost overflow"))?;
    let mut instructions = Vec::new();
    let mut limit_data = vec![2];
    limit_data.extend_from_slice(&request.compute_unit_limit.to_le_bytes());
    instructions.push(Instruction {
        program_id: pubkey(COMPUTE_BUDGET_PROGRAM)?,
        accounts: vec![],
        data: limit_data,
    });
    let mut price_data = vec![3];
    price_data.extend_from_slice(&priority_price.to_le_bytes());
    instructions.push(Instruction {
        program_id: pubkey(COMPUTE_BUDGET_PROGRAM)?,
        accounts: vec![],
        data: price_data,
    });
    let destination = if request.asset == Asset::Sol {
        if request.create_destination_ata || rent != 0 || !request.source_accounts.is_empty() {
            return Err(policy(
                "Native SOL cannot include token-account instructions",
            ));
        }
        amount
            .checked_add(reserve)
            .ok_or_else(|| policy("Transfer plus fee exceeds Solana's integer range"))?;
        let mut data = 2u32.to_le_bytes().to_vec();
        data.extend_from_slice(&amount.to_le_bytes());
        instructions.push(Instruction {
            program_id: pubkey(SYSTEM_PROGRAM)?,
            accounts: vec![AccountMeta::new(from, true), AccountMeta::new(to, false)],
            data,
        });
        None
    } else {
        if request.source_accounts.is_empty() || request.source_accounts.len() > MAX_SOURCE_ACCOUNTS
        {
            return Err(policy(
                "Token transfer requires one to eight verified source accounts",
            ));
        }
        if request.create_destination_ata != (rent > 0) {
            return Err(policy(
                "Associated-account creation and rent quote disagree",
            ));
        }
        let mint = pubkey(
            request
                .asset
                .mint()
                .ok_or_else(|| policy("Unsupported Solana token"))?,
        )?;
        let destination = associated_account(&to, &mint)?;
        if request.create_destination_ata {
            instructions.push(Instruction {
                program_id: pubkey(ASSOCIATED_TOKEN_PROGRAM)?,
                accounts: vec![
                    AccountMeta::new(from, true),
                    AccountMeta::new(destination, false),
                    AccountMeta::new_readonly(to, false),
                    AccountMeta::new_readonly(mint, false),
                    AccountMeta::new_readonly(pubkey(SYSTEM_PROGRAM)?, false),
                    AccountMeta::new_readonly(pubkey(TOKEN_PROGRAM)?, false),
                ],
                data: vec![1],
            }); // CreateIdempotent; never close or change authority.
        }
        let mut sources = BTreeSet::new();
        let mut total = 0u64;
        for source in &request.source_accounts {
            let source_key = pubkey(&source.address)?;
            if source_key == destination
                || source_key == from
                || !sources.insert(source.address.clone())
            {
                return Err(policy("Duplicate or invalid source token account"));
            }
            let source_amount = units(&source.amount_units)?;
            if source_amount == 0 {
                return Err(policy("Source token allocation must be positive"));
            }
            total = total
                .checked_add(source_amount)
                .ok_or_else(|| policy("Token allocation overflow"))?;
            let mut data = vec![12];
            data.extend_from_slice(&source_amount.to_le_bytes());
            data.push(request.asset.decimals());
            instructions.push(Instruction {
                program_id: pubkey(TOKEN_PROGRAM)?,
                accounts: vec![
                    AccountMeta::new(source_key, false),
                    AccountMeta::new_readonly(mint, false),
                    AccountMeta::new(destination, false),
                    AccountMeta::new_readonly(from, true),
                ],
                data,
            });
        }
        if total != amount {
            return Err(policy(
                "Source allocations do not equal the reviewed amount",
            ));
        }
        Some(destination.to_string())
    };
    let message = Message::new_with_blockhash(&instructions, Some(&from), &blockhash);
    if message.header.num_required_signatures != 1
        || message.serialize().len() + 65 > MAX_PACKET_BYTES
    {
        return Err(policy(
            "Transfer exceeds Solana's packet size; use fewer source accounts",
        ));
    }
    Ok((message, amount, reserve, destination))
}

fn digest(prepared: &Prepared) -> Result<String, WalletError> {
    let mut reviewed = prepared.clone();
    reviewed.approval_digest.clear();
    let bytes =
        serde_json::to_vec(&reviewed).map_err(|_| invalid("Cannot encode Solana review"))?;
    let mut hasher = Sha256::new();
    hasher.update(APPROVAL_DOMAIN);
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}
pub fn prepare(request_json: &str) -> Result<Prepared, WalletError> {
    let request: Request = parse_json(request_json, MAX_JSON)?;
    let (message, amount, reserve, destination) = construct(&request)?;
    let created_at = now()?;
    let mut prepared = Prepared {
        request,
        amount_units: amount.to_string(),
        fee_reserve_units: reserve.to_string(),
        unsigned_transaction: BASE64.encode(message.serialize()),
        destination_token_account: destination,
        approval_digest: String::new(),
        created_at,
        expires_at: created_at
            .checked_add(APPROVAL_LIFETIME_SECONDS)
            .ok_or_else(|| policy("Device clock overflow"))?,
    };
    prepared.approval_digest = digest(&prepared)?;
    Ok(prepared)
}
/// Historical validation deliberately remains valid after approval/blockhash expiry.
pub fn validate_prepared(prepared_json: &str) -> Result<Prepared, WalletError> {
    let prepared: Prepared = parse_json(prepared_json, MAX_JSON)?;
    let (message, amount, reserve, destination) = construct(&prepared.request)?;
    if prepared.amount_units != amount.to_string()
        || prepared.fee_reserve_units != reserve.to_string()
        || prepared.destination_token_account != destination
        || prepared.unsigned_transaction != BASE64.encode(message.serialize())
        || prepared.expires_at
            != prepared
                .created_at
                .checked_add(APPROVAL_LIFETIME_SECONDS)
                .ok_or_else(|| policy("Invalid review lifetime"))?
        || prepared.approval_digest != digest(&prepared)?
    {
        return Err(policy("Solana transaction review has changed"));
    }
    Ok(prepared)
}
pub fn sign(
    vault_hex: &str,
    key: &[u8],
    prepared_json: &str,
    approval_digest: &str,
) -> Result<Signed, WalletError> {
    let prepared = validate_prepared(prepared_json)?;
    if approval_digest != prepared.approval_digest {
        return Err(policy("Solana approval does not match the transaction"));
    }
    let time = now()?;
    if time < prepared.created_at || time >= prepared.expires_at {
        return Err(policy(
            "Solana review expired; prepare and review a new transfer",
        ));
    }
    let signer = signing_key(vault_hex, key)?;
    if bs58::encode(signer.verifying_key().as_bytes()).into_string() != prepared.request.from {
        return Err(policy("Solana signer does not match this wallet"));
    }
    let message = decode_base64(&prepared.unsigned_transaction, MAX_PACKET_BYTES - 65)?;
    let signature = signer.sign(&message).to_bytes();
    let mut raw = Vec::with_capacity(message.len() + 65);
    raw.push(1);
    raw.extend_from_slice(&signature);
    raw.extend_from_slice(&message);
    Ok(Signed {
        intent_id: prepared.request.intent_id,
        network: NETWORK.into(),
        from: prepared.request.from,
        raw_transaction: BASE64.encode(raw),
        hash: bs58::encode(signature).into_string(),
        approval_digest: prepared.approval_digest,
        amount_units: prepared.amount_units,
    })
}
pub fn validate_signed(prepared_json: &str, signed_json: &str) -> Result<Signed, WalletError> {
    let prepared = validate_prepared(prepared_json)?;
    let signed: Signed = parse_json(signed_json, MAX_JSON)?;
    if signed.intent_id != prepared.request.intent_id
        || signed.network != NETWORK
        || signed.from != prepared.request.from
        || signed.approval_digest != prepared.approval_digest
        || signed.amount_units != prepared.amount_units
    {
        return Err(policy("Signed Solana transfer does not match its review"));
    }
    let raw = decode_base64(&signed.raw_transaction, MAX_PACKET_BYTES)?;
    let message = decode_base64(&prepared.unsigned_transaction, MAX_PACKET_BYTES - 65)?;
    let sig = signature(&signed.hash)?;
    if raw.len() != message.len() + 65
        || raw.first() != Some(&1)
        || raw.get(1..65) != Some(sig.as_slice())
        || raw.get(65..) != Some(message.as_slice())
    {
        return Err(policy(
            "Signed Solana bytes do not match the reviewed message",
        ));
    }
    VerifyingKey::from_bytes(&wallet_key(&signed.from)?.to_bytes())
        .map_err(|_| WalletError::Crypto)?
        .verify_strict(&message, &Signature::from_bytes(&sig))
        .map_err(|_| policy("Invalid Solana signature"))?;
    Ok(signed)
}

pub fn check_expiry(evidence_json: &str) -> Result<ExpiryObservation, WalletError> {
    let evidence: ExpiryEvidence = parse_json(evidence_json, MAX_JSON)?;
    identity(&evidence.network, &evidence.genesis_hash)?;
    hash(&evidence.recent_blockhash)?;
    if evidence.context_slot == 0
        || evidence.last_valid_block_height == 0
        || evidence.finalized_context_slot < evidence.context_slot
    {
        return Err(policy("Solana expiry observation is stale"));
    }
    let expired = evidence.current_finalized_block_height > evidence.last_valid_block_height
        && evidence.history_search_enabled
        && !evidence.signature_found
        && !evidence.blockhash_valid;
    // A found signature is reconciled, never treated as an unsubmitted payment.
    let rebroadcastable = !evidence.signature_found
        && evidence.blockhash_valid
        && evidence.current_finalized_block_height <= evidence.last_valid_block_height;
    Ok(ExpiryObservation {
        expired,
        rebroadcastable,
        status: if expired {
            "expired_unresolved"
        } else if evidence.signature_found {
            "observed"
        } else if rebroadcastable {
            "live"
        } else {
            "unresolved"
        }
        .into(),
    })
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, WalletError> {
    value
        .get(name)
        .ok_or_else(|| invalid(format!("Missing Solana evidence field: {name}")))
}
fn text<'a>(value: &'a Value, name: &str) -> Result<&'a str, WalletError> {
    field(value, name)?
        .as_str()
        .ok_or_else(|| invalid(format!("Invalid Solana evidence field: {name}")))
}
fn number(value: &Value, name: &str) -> Result<u64, WalletError> {
    field(value, name)?
        .as_u64()
        .ok_or_else(|| invalid(format!("Invalid Solana evidence number: {name}")))
}

struct LedgerEvidence {
    transaction: Value,
    raw: Vec<u8>,
    hash: String,
    slot: u64,
    block_hash: String,
    finalized: bool,
    confirmations: u64,
    fee: u64,
    success: bool,
}
/// Evidence contains the unmodified RPC `result` objects, not wallet-authored status.
fn ledger_evidence(evidence_json: &str) -> Result<LedgerEvidence, WalletError> {
    let evidence: Value = parse_json(evidence_json, MAX_EVIDENCE_JSON)?;
    identity(
        text(&evidence, "network")?,
        text(&evidence, "genesis_hash")?,
    )?;
    let transaction = field(&evidence, "transaction")?.clone();
    let status = field(&evidence, "signature_status")?;
    let block = field(&evidence, "canonical_block")?;
    let slot = number(&transaction, "slot")?;
    if slot == 0 || slot != number(status, "slot")? {
        return Err(policy("Solana transaction and status slots disagree"));
    }
    let encoding = field(&transaction, "transaction")?
        .as_array()
        .ok_or_else(|| invalid("Solana transaction must use base64 encoding"))?;
    if encoding.len() != 2 || encoding[1].as_str() != Some("base64") {
        return Err(invalid("Unsupported Solana transaction encoding"));
    }
    let raw = decode_base64(
        encoding[0]
            .as_str()
            .ok_or_else(|| invalid("Missing Solana wire transaction"))?,
        MAX_PACKET_BYTES,
    )?;
    let parsed = parse_wire(&raw, field(&transaction, "meta")?)?;
    let tx_hash = bs58::encode(
        parsed
            .signatures
            .first()
            .ok_or_else(|| invalid("Missing Solana signature"))?,
    )
    .into_string();
    if text(&evidence, "requested_signature")? != tx_hash
        || number(&evidence, "canonical_block_slot")? != slot
    {
        return Err(policy(
            "Solana evidence is not bound to the requested signature and block slot",
        ));
    }
    let block_hash = text(block, "blockhash")?.to_owned();
    hash(&block_hash)?;
    if number(block, "parentSlot")? >= slot {
        return Err(policy("Invalid canonical Solana block parent"));
    }
    let signatures = field(block, "signatures")?
        .as_array()
        .ok_or_else(|| invalid("Missing canonical block signatures"))?;
    if signatures.len() > 10_000
        || signatures
            .iter()
            .filter(|s| s.as_str() == Some(&tx_hash))
            .count()
            != 1
    {
        return Err(policy(
            "Solana transaction is not in the observed canonical block",
        ));
    }
    let meta = field(&transaction, "meta")?;
    let err = field(meta, "err")?;
    if field(status, "err")? != err {
        return Err(policy("Solana receipt and signature status disagree"));
    }
    let confirmation = text(status, "confirmationStatus")?;
    if confirmation != "confirmed" && confirmation != "finalized" {
        return Err(policy(
            "Solana transaction has not reached confirmed commitment",
        ));
    }
    let finalized_slot = number(&evidence, "finalized_slot")?;
    let finalized = confirmation == "finalized";
    if finalized && finalized_slot < slot {
        return Err(policy(
            "Finalized Solana status is not covered by finalized slot",
        ));
    }
    let confirmations = if finalized {
        1
    } else {
        status
            .get("confirmations")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let fee = number(meta, "fee")?;
    if fee == 0 {
        return Err(policy("Invalid Solana transaction fee"));
    }
    let success = err.is_null();
    Ok(LedgerEvidence {
        transaction,
        raw,
        hash: tx_hash,
        slot,
        block_hash,
        finalized,
        confirmations,
        fee,
        success,
    })
}

#[derive(Clone)]
struct TokenBalance {
    mint: String,
    owner: String,
    amount: u64,
    decimals: u8,
}
fn token_balances(
    meta: &Value,
    name: &str,
    key_count: usize,
) -> Result<BTreeMap<usize, TokenBalance>, WalletError> {
    let array = field(meta, name)?
        .as_array()
        .ok_or_else(|| invalid("Missing Solana token balance evidence"))?;
    if array.len() > key_count {
        return Err(invalid("Excessive Solana token balance evidence"));
    }
    let mut result = BTreeMap::new();
    for entry in array {
        let index = usize::try_from(number(entry, "accountIndex")?)
            .map_err(|_| invalid("Invalid token account index"))?;
        if index >= key_count || result.contains_key(&index) {
            return Err(policy("Duplicate or invalid Solana token account index"));
        }
        let mint = text(entry, "mint")?.to_owned();
        pubkey(&mint)?;
        let owner = text(entry, "owner")?.to_owned();
        pubkey(&owner)?;
        if text(entry, "programId")? != TOKEN_PROGRAM {
            // Token-2022 and unrelated program assets are not accepted as any of
            // the registry's original SPL tokens. Keep them out of this view.
            continue;
        }
        let amount_info = field(entry, "uiTokenAmount")?;
        let amount = units(text(amount_info, "amount")?)?;
        let decimals = u8::try_from(number(amount_info, "decimals")?)
            .map_err(|_| invalid("Invalid token decimals"))?;
        if Asset::from_mint(&mint).is_some() && decimals != 6 {
            return Err(policy("Canonical token decimals mismatch"));
        }
        result.insert(
            index,
            TokenBalance {
                mint,
                owner,
                amount,
                decimals,
            },
        );
    }
    Ok(result)
}
fn lamport_balances(meta: &Value, name: &str, key_count: usize) -> Result<Vec<u64>, WalletError> {
    let values = field(meta, name)?
        .as_array()
        .ok_or_else(|| invalid("Missing Solana native balance evidence"))?;
    if values.len() != key_count {
        return Err(policy("Solana account and balance counts disagree"));
    }
    values
        .iter()
        .map(|v| {
            v.as_u64()
                .ok_or_else(|| invalid("Invalid Solana native balance"))
        })
        .collect()
}
fn require_token<'a>(
    entries: &'a BTreeMap<usize, TokenBalance>,
    index: usize,
    mint: &str,
    owner: &str,
) -> Result<&'a TokenBalance, WalletError> {
    let token = entries
        .get(&index)
        .ok_or_else(|| policy("Missing canonical token-account balance"))?;
    if token.mint != mint || token.owner != owner || token.decimals != 6 {
        return Err(policy("Token account owner, mint or decimals mismatch"));
    }
    Ok(token)
}

pub fn validate_receipt(
    prepared_json: &str,
    signed_json: &str,
    evidence_json: &str,
) -> Result<Observation, WalletError> {
    let prepared = validate_prepared(prepared_json)?;
    let signed = validate_signed(prepared_json, signed_json)?;
    let evidence = ledger_evidence(evidence_json)?;
    if evidence.hash != signed.hash
        || evidence.raw != decode_base64(&signed.raw_transaction, MAX_PACKET_BYTES)?
        || evidence.slot < prepared.request.context_slot
    {
        return Err(policy(
            "Solana receipt belongs to a different transfer or stale context",
        ));
    }
    let (message, amount, _, _) = construct(&prepared.request)?;
    let meta = field(&evidence.transaction, "meta")?;
    let pre = lamport_balances(meta, "preBalances", message.account_keys.len())?;
    let post = lamport_balances(meta, "postBalances", message.account_keys.len())?;
    let expected_fee = units(&prepared.request.fee_lamports)?;
    let expected_rent = units(&prepared.request.rent_lamports)?;
    let mut actual_account_rent = 0u64;
    let index = |address: &str| -> Result<usize, WalletError> {
        message
            .account_keys
            .iter()
            .position(|k| k.to_string() == address)
            .ok_or_else(|| policy("Reviewed Solana account is missing"))
    };
    let payer_index = index(&prepared.request.from)?;
    let mut expected_native = vec![0i128; pre.len()];
    expected_native[payer_index] = -(evidence.fee as i128);
    if evidence.success {
        if prepared.request.asset == Asset::Sol {
            expected_native[payer_index] -= amount as i128;
            expected_native[index(&prepared.request.to)?] += amount as i128;
        } else {
            let pre_tokens = token_balances(meta, "preTokenBalances", pre.len())?;
            let post_tokens = token_balances(meta, "postTokenBalances", pre.len())?;
            let mint = prepared
                .request
                .asset
                .mint()
                .ok_or_else(|| policy("Unsupported Solana token"))?;
            for source in &prepared.request.source_accounts {
                let source_index = index(&source.address)?;
                let before =
                    require_token(&pre_tokens, source_index, mint, &prepared.request.from)?;
                let after =
                    require_token(&post_tokens, source_index, mint, &prepared.request.from)?;
                if before.amount.checked_sub(after.amount) != Some(units(&source.amount_units)?) {
                    return Err(policy(
                        "Solana source token debit does not match the reviewed amount",
                    ));
                }
            }
            let destination = prepared
                .destination_token_account
                .as_deref()
                .ok_or_else(|| policy("Missing destination associated token account"))?;
            let destination_index = index(destination)?;
            let after = require_token(&post_tokens, destination_index, mint, &prepared.request.to)?;
            let before = if pre_tokens.contains_key(&destination_index) {
                require_token(&pre_tokens, destination_index, mint, &prepared.request.to)?.amount
            } else if prepared.request.create_destination_ata {
                0
            } else {
                return Err(policy(
                    "Destination token account was not present before transfer",
                ));
            };
            if after.amount.checked_sub(before) != Some(amount) {
                return Err(policy(
                    "Solana destination token credit does not match the reviewed amount",
                ));
            }
            let actual_rent = post[destination_index]
                .checked_sub(pre[destination_index])
                .ok_or_else(|| {
                    policy("Destination associated-account rent decreased unexpectedly")
                })?;
            if !prepared.request.create_destination_ata && actual_rent != 0 {
                return Err(policy(
                    "Solana receipt funds an account without a reviewed creation instruction",
                ));
            }
            actual_account_rent = actual_rent;
            expected_native[payer_index] -= actual_rent as i128;
            expected_native[destination_index] += actual_rent as i128;
        }
    } else {
        let pre_tokens = token_balances(meta, "preTokenBalances", pre.len())?;
        let post_tokens = token_balances(meta, "postTokenBalances", pre.len())?;
        if pre_tokens.len() != post_tokens.len()
            || pre_tokens.iter().any(|(idx, before)| {
                post_tokens.get(idx).is_none_or(|after| {
                    before.mint != after.mint
                        || before.owner != after.owner
                        || before.amount != after.amount
                        || before.decimals != after.decimals
                })
            })
        {
            return Err(policy("Reverted Solana transaction changed token balances"));
        }
    }
    if pre
        .iter()
        .zip(&post)
        .zip(&expected_native)
        .any(|((&a, &b), &expected)| b as i128 - a as i128 != expected)
    {
        return Err(policy(
            "Solana native debits or credits disagree with the reviewed transfer",
        ));
    }
    Ok(Observation {
        status: if !evidence.success {
            "reverted"
        } else if evidence.finalized {
            "finalized"
        } else {
            "confirmed"
        }
        .into(),
        success: evidence.success,
        finalized: evidence.finalized,
        hash: evidence.hash,
        block_number: evidence.slot,
        block_hash: evidence.block_hash,
        fee_units: evidence.fee.to_string(),
        account_rent_units: actual_account_rent.to_string(),
        total_cost_units: evidence
            .fee
            .checked_add(actual_account_rent)
            .ok_or_else(|| policy("Solana actual costs overflow"))?
            .to_string(),
        cost_exceeds_review: evidence.fee > expected_fee || actual_account_rent > expected_rent,
        confirmations: evidence.confirmations,
        observed_at: now()?,
    })
}

struct WireInstruction {
    program: usize,
    accounts: Vec<usize>,
    data: Vec<u8>,
}
struct ParsedWire {
    signatures: Vec<[u8; 64]>,
    keys: Vec<String>,
    instructions: Vec<WireInstruction>,
    signer_count: usize,
}

/// Inner instructions include attempted CPIs. A caller can handle a CPI error
/// while the outer transaction succeeds. Only complete runtime invocation logs
/// with successful enclosing invocations establish that an inner transfer ran.
/// Missing/truncated/ambiguous logs deliberately produce no inner activity rows.
fn successful_inner_instructions(
    meta: &Value,
    keys: &[String],
    top_instructions: &[WireInstruction],
) -> Result<BTreeSet<(u32, u32)>, WalletError> {
    let mut expected = Vec::new();
    if let Some(groups) = meta.get("innerInstructions").and_then(Value::as_array) {
        let mut sorted = groups.iter().collect::<Vec<_>>();
        sorted.sort_by_key(|group| {
            group
                .get("index")
                .and_then(Value::as_u64)
                .unwrap_or(u64::MAX)
        });
        for group in sorted {
            let parent = u32::try_from(number(group, "index")?)
                .map_err(|_| invalid("Invalid inner parent"))?;
            let instructions = field(group, "instructions")?
                .as_array()
                .ok_or_else(|| invalid("Invalid inner instructions"))?;
            for (index, instruction) in instructions.iter().enumerate() {
                let program = usize::try_from(number(instruction, "programIdIndex")?)
                    .map_err(|_| invalid("Invalid inner program"))?;
                let key = keys
                    .get(program)
                    .ok_or_else(|| invalid("Invalid inner program index"))?;
                expected.push((
                    parent,
                    index as u32,
                    key.as_str(),
                    instruction.get("stackHeight").and_then(Value::as_u64),
                ));
            }
        }
    }
    if expected.is_empty() {
        return Ok(BTreeSet::new());
    }
    let Some(logs) = meta.get("logMessages").and_then(Value::as_array) else {
        return Ok(BTreeSet::new());
    };
    if logs.len() > 20_000 {
        return Err(invalid("Excessive Solana runtime logs"));
    }
    struct Invocation {
        program: String,
        parent: Option<usize>,
        depth: usize,
        success: Option<bool>,
    }
    let mut invocations: Vec<Invocation> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for item in logs {
        let Some(line) = item.as_str() else {
            return Err(invalid("Invalid Solana runtime log"));
        };
        if line == "Log truncated" {
            return Ok(BTreeSet::new());
        }
        let Some(body) = line.strip_prefix("Program ") else {
            continue;
        };
        if let Some((program, tail)) = body.split_once(" invoke [") {
            if pubkey(program).is_err() {
                continue;
            } // user logs have "Program log:" prefix.
            let Some(depth_text) = tail.strip_suffix(']') else {
                return Ok(BTreeSet::new());
            };
            let Ok(depth) = depth_text.parse::<usize>() else {
                return Ok(BTreeSet::new());
            };
            if depth != stack.len() + 1 || depth > 64 {
                return Ok(BTreeSet::new());
            }
            invocations.push(Invocation {
                program: program.to_owned(),
                parent: stack.last().copied(),
                depth,
                success: None,
            });
            stack.push(invocations.len() - 1);
        } else {
            let completed = body
                .strip_suffix(" success")
                .map(|program| (program, true))
                .or_else(|| {
                    body.split_once(" failed: ")
                        .map(|(program, _)| (program, false))
                });
            if let Some((program, success)) = completed {
                if pubkey(program).is_err() {
                    continue;
                }
                let Some(index) = stack.pop() else {
                    return Ok(BTreeSet::new());
                };
                if invocations[index].program != program {
                    return Ok(BTreeSet::new());
                }
                invocations[index].success = Some(success);
            }
        }
    }
    if !stack.is_empty() || invocations.iter().any(|call| call.success.is_none()) {
        return Ok(BTreeSet::new());
    }
    let inner = invocations
        .iter()
        .enumerate()
        .filter(|(_, call)| call.depth > 1)
        .collect::<Vec<_>>();
    if inner.len() != expected.len() {
        return Ok(BTreeSet::new());
    }
    let mut result = BTreeSet::new();
    for ((index, call), (parent, inner_index, program, depth)) in inner.into_iter().zip(expected) {
        if call.program != program || depth.is_some_and(|depth| depth != call.depth as u64) {
            return Ok(BTreeSet::new());
        }
        let mut ancestor = Some(index);
        let mut successful = true;
        let mut root = index;
        while let Some(index) = ancestor {
            if invocations[index].success != Some(true) {
                successful = false;
                break;
            }
            root = index;
            ancestor = invocations[index].parent;
        }
        if successful {
            let top = top_instructions
                .get(parent as usize)
                .ok_or_else(|| invalid("Invalid inner instruction parent"))?;
            if invocations[root].program != keys[top.program] {
                return Ok(BTreeSet::new());
            }
        }
        if successful {
            result.insert((parent, inner_index));
        }
    }
    Ok(result)
}
struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], WalletError> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or_else(|| invalid("Malformed Solana wire lengths"))?;
        let result = self
            .bytes
            .get(self.cursor..end)
            .ok_or_else(|| invalid("Truncated Solana wire transaction"))?;
        self.cursor = end;
        Ok(result)
    }
    fn byte(&mut self) -> Result<u8, WalletError> {
        Ok(self.take(1)?[0])
    }
    fn short(&mut self) -> Result<usize, WalletError> {
        let mut value = 0usize;
        for shift in 0..3 {
            let byte = self.byte()?;
            if shift == 2 && byte > 3 {
                return Err(invalid("Solana compact length overflows"));
            }
            value |= ((byte & 127) as usize) << (7 * shift);
            if byte & 128 == 0 {
                if shift > 0 && byte == 0 {
                    return Err(invalid("Noncanonical Solana compact length"));
                }
                return Ok(value);
            }
        }
        Err(invalid("Invalid Solana compact length"))
    }
}
fn parse_wire(raw: &[u8], meta: &Value) -> Result<ParsedWire, WalletError> {
    let mut reader = Reader {
        bytes: raw,
        cursor: 0,
    };
    let count = reader.short()?;
    if count == 0 || count > 12 {
        return Err(policy("Unsupported Solana signature count"));
    }
    let mut signatures = Vec::with_capacity(count);
    for _ in 0..count {
        signatures.push(
            reader
                .take(64)?
                .try_into()
                .map_err(|_| invalid("Invalid Solana signature"))?,
        );
    }
    let message_start = reader.cursor;
    let first = reader.byte()?;
    let versioned = first & 128 != 0;
    let required = if versioned {
        if first != 128 {
            return Err(policy("Unsupported Solana message version"));
        }
        reader.byte()?
    } else {
        first
    } as usize;
    let readonly_signed = reader.byte()? as usize;
    let readonly_unsigned = reader.byte()? as usize;
    let static_count = reader.short()?;
    if required != count
        || static_count == 0
        || static_count > 64
        || required > static_count
        || readonly_signed >= required
        || readonly_unsigned > static_count - required
    {
        return Err(policy("Invalid Solana message account header"));
    }
    let mut keys = Vec::with_capacity(static_count);
    for _ in 0..static_count {
        keys.push(bs58::encode(reader.take(32)?).into_string());
    }
    hash(&bs58::encode(reader.take(32)?).into_string())?;
    let instruction_count = reader.short()?;
    if instruction_count == 0 || instruction_count > 64 {
        return Err(policy("Unsupported Solana instruction count"));
    }
    let mut instructions = Vec::with_capacity(instruction_count);
    for _ in 0..instruction_count {
        let program = reader.byte()? as usize;
        let account_count = reader.short()?;
        if account_count > 64 {
            return Err(invalid("Too many Solana instruction accounts"));
        }
        let accounts = reader
            .take(account_count)?
            .iter()
            .map(|x| *x as usize)
            .collect();
        let length = reader.short()?;
        instructions.push(WireInstruction {
            program,
            accounts,
            data: reader.take(length)?.to_vec(),
        });
    }
    if versioned {
        let lookup_count = reader.short()?;
        if lookup_count > 16 {
            return Err(policy("Too many Solana address lookups"));
        }
        let mut writable_count = 0usize;
        let mut readonly_count = 0usize;
        for _ in 0..lookup_count {
            reader.take(32)?;
            let writable = reader.short()?;
            if writable > 64 {
                return Err(invalid("Too many loaded Solana accounts"));
            }
            reader.take(writable)?;
            writable_count += writable;
            let readonly = reader.short()?;
            if readonly > 64 {
                return Err(invalid("Too many loaded Solana accounts"));
            }
            reader.take(readonly)?;
            readonly_count += readonly;
        }
        if static_count + writable_count + readonly_count > 64 {
            return Err(policy("Too many Solana accounts"));
        }
        let loaded = field(meta, "loadedAddresses")?;
        for (name, count) in [("writable", writable_count), ("readonly", readonly_count)] {
            let values = field(loaded, name)?
                .as_array()
                .ok_or_else(|| invalid("Missing resolved Solana lookup addresses"))?;
            if values.len() != count {
                return Err(policy("Solana lookup counts disagree"));
            }
            for value in values {
                keys.push(
                    pubkey(
                        value
                            .as_str()
                            .ok_or_else(|| invalid("Invalid loaded Solana address"))?,
                    )?
                    .to_string(),
                );
            }
        }
    }
    if reader.cursor != raw.len()
        || keys.iter().collect::<BTreeSet<_>>().len() != keys.len()
        || instructions
            .iter()
            .any(|i| i.program >= keys.len() || i.accounts.iter().any(|a| *a >= keys.len()))
    {
        return Err(policy(
            "Invalid Solana wire account references or trailing data",
        ));
    }
    for (index, signature) in signatures.iter().enumerate() {
        let key = VerifyingKey::from_bytes(&pubkey(&keys[index])?.to_bytes())
            .map_err(|_| policy("Invalid Solana signing key"))?;
        key.verify_strict(&raw[message_start..], &Signature::from_bytes(signature))
            .map_err(|_| policy("Invalid Solana ledger signature"))?;
    }
    Ok(ParsedWire {
        signatures,
        keys,
        instructions,
        signer_count: required,
    })
}

/// Receipts are generated from actual system/SPL transfer instructions in a
/// successful canonical transaction, never from a wallet balance difference.
/// Other instructions remain ignored and cannot masquerade as supported assets.
pub fn validate_activity(owner: &str, evidence_json: &str) -> Result<Vec<Activity>, WalletError> {
    wallet_key(owner)?;
    let evidence = ledger_evidence(evidence_json)?;
    if !evidence.success {
        return Ok(Vec::new());
    }
    let meta = field(&evidence.transaction, "meta")?;
    let parsed = parse_wire(&evidence.raw, meta)?;
    let before = token_balances(meta, "preTokenBalances", parsed.keys.len())?;
    let after = token_balances(meta, "postTokenBalances", parsed.keys.len())?;
    let successful_inner = successful_inner_instructions(meta, &parsed.keys, &parsed.instructions)?;
    let top_instruction_count = parsed.instructions.len();
    let mut instructions: Vec<(u32, WireInstruction, bool)> = parsed
        .instructions
        .into_iter()
        .enumerate()
        .map(|(idx, i)| ((idx as u32) * 1_000, i, false))
        .collect();
    if let Some(groups) = meta.get("innerInstructions").and_then(Value::as_array) {
        if groups.len() > 64 {
            return Err(invalid("Excessive Solana inner instruction groups"));
        }
        let mut groups_seen = BTreeSet::new();
        for group in groups {
            let parent = number(group, "index")?;
            if parent >= top_instruction_count as u64 || !groups_seen.insert(parent) {
                return Err(invalid("Invalid Solana inner instruction parent"));
            }
            let inner = field(group, "instructions")?
                .as_array()
                .ok_or_else(|| invalid("Missing inner instructions"))?;
            if inner.len() >= 1_000 {
                return Err(invalid("Excessive Solana inner instructions"));
            }
            for (idx, instruction) in inner.iter().enumerate() {
                let program = usize::try_from(number(instruction, "programIdIndex")?)
                    .map_err(|_| invalid("Invalid inner program index"))?;
                let accounts = field(instruction, "accounts")?
                    .as_array()
                    .ok_or_else(|| invalid("Invalid inner account indexes"))?
                    .iter()
                    .map(|v| {
                        v.as_u64()
                            .and_then(|n| usize::try_from(n).ok())
                            .ok_or_else(|| invalid("Invalid inner account index"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let encoded = text(instruction, "data")?;
                if encoded.len() > 2_000 {
                    return Err(invalid("Excessive inner instruction data"));
                }
                let data = bs58::decode(encoded)
                    .into_vec()
                    .map_err(|_| invalid("Invalid inner instruction data"))?;
                if bs58::encode(&data).into_string() != encoded
                    || program >= parsed.keys.len()
                    || accounts.iter().any(|a| *a >= parsed.keys.len())
                {
                    return Err(invalid("Invalid Solana inner instruction"));
                }
                if !successful_inner.contains(&(parent as u32, idx as u32)) {
                    continue;
                }
                instructions.push((
                    parent as u32 * 1_000 + idx as u32 + 1,
                    WireInstruction {
                        program,
                        accounts,
                        data,
                    },
                    true,
                ));
            }
        }
    }
    let mut result = Vec::new();
    for (event_index, instruction, inner) in instructions {
        let program = &parsed.keys[instruction.program];
        let transfer = if program == SYSTEM_PROGRAM
            && instruction.data.len() == 12
            && instruction.data[..4] == 2u32.to_le_bytes()
            && instruction.accounts.len() == 2
        {
            if !inner && instruction.accounts[0] >= parsed.signer_count {
                return Err(policy("Native Solana transfer has no sender signature"));
            }
            // Associated-account rent funding is a transfer instruction inside
            // account creation, not a second user payment of native SOL.
            if inner
                && (before.contains_key(&instruction.accounts[1])
                    || after.contains_key(&instruction.accounts[1]))
            {
                None
            } else {
                Some((
                    Asset::Sol,
                    parsed.keys[instruction.accounts[0]].clone(),
                    parsed.keys[instruction.accounts[1]].clone(),
                    u64::from_le_bytes(
                        instruction.data[4..12]
                            .try_into()
                            .map_err(|_| invalid("Invalid native transfer amount"))?,
                    ),
                ))
            }
        } else if program == TOKEN_PROGRAM
            && (instruction.data.first() == Some(&3) || instruction.data.first() == Some(&12))
        {
            let checked = instruction.data[0] == 12;
            if instruction.data.len() != if checked { 10 } else { 9 }
                || instruction.accounts.len() < if checked { 4 } else { 3 }
            {
                return Err(invalid("Malformed SPL transfer instruction"));
            }
            let source_index = instruction.accounts[0];
            let destination_index = instruction.accounts[if checked { 2 } else { 1 }];
            let source = before
                .get(&source_index)
                .or_else(|| after.get(&source_index));
            let destination = after
                .get(&destination_index)
                .or_else(|| before.get(&destination_index));
            if let (Some(source), Some(destination)) = (source, destination) {
                if source.mint != destination.mint || source.decimals != destination.decimals {
                    return Err(policy(
                        "SPL transfer evidence has inconsistent mint or decimals",
                    ));
                }
                if let Some(asset) = Asset::from_mint(&source.mint) {
                    if source.decimals != asset.decimals()
                        || (checked
                            && (parsed.keys[instruction.accounts[1]] != source.mint
                                || instruction.data[9] != asset.decimals()))
                    {
                        return Err(policy("SPL checked transfer has wrong mint or decimals"));
                    }
                    Some((
                        asset,
                        source.owner.clone(),
                        destination.owner.clone(),
                        u64::from_le_bytes(
                            instruction.data[1..9]
                                .try_into()
                                .map_err(|_| invalid("Invalid SPL transfer amount"))?,
                        ),
                    ))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        if let Some((asset, from, to, amount)) = transfer {
            if amount == 0 || from == to || (from != owner && to != owner) {
                continue;
            }
            result.push(Activity {
                network: NETWORK.into(),
                asset: asset.symbol().into(),
                direction: if to == owner { "received" } else { "sent" }.into(),
                amount_units: amount.to_string(),
                hash: evidence.hash.clone(),
                block_number: evidence.slot,
                block_hash: evidence.block_hash.clone(),
                finalized: evidence.finalized,
                from,
                to,
                event_index,
            });
        }
    }
    // Reconcile reported events to the transaction's real account changes.
    // Complex mint/burn/close effects outside this restricted activity parser
    // remain unsupported; they cannot manufacture a "received" transfer row.
    let mut unreconciled = BTreeSet::new();
    for asset in [Asset::Usdc, Asset::Eurc, Asset::Usdt] {
        let symbol = asset.symbol();
        let mint = asset
            .mint()
            .ok_or_else(|| policy("Missing canonical token mint"))?;
        let observed_before: i128 = before
            .values()
            .filter(|t| t.owner == owner && t.mint == mint)
            .map(|t| t.amount as i128)
            .sum();
        let observed_after: i128 = after
            .values()
            .filter(|t| t.owner == owner && t.mint == mint)
            .map(|t| t.amount as i128)
            .sum();
        let events: i128 = result
            .iter()
            .filter(|e| e.asset == symbol)
            .map(|e| {
                units(&e.amount_units).map(|amount| {
                    if e.direction == "received" {
                        amount as i128
                    } else {
                        -(amount as i128)
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum();
        if events != observed_after - observed_before {
            unreconciled.insert(symbol);
        }
    }
    if result.iter().any(|event| event.asset == "SOL") {
        let native_before = lamport_balances(meta, "preBalances", parsed.keys.len())?;
        let native_after = lamport_balances(meta, "postBalances", parsed.keys.len())?;
        if let Some(index) = parsed.keys.iter().position(|key| key == owner) {
            let events: i128 = result
                .iter()
                .filter(|event| event.asset == "SOL")
                .map(|event| {
                    units(&event.amount_units).map(|amount| {
                        if event.direction == "received" {
                            amount as i128
                        } else {
                            -(amount as i128)
                        }
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .sum();
            let fee = if index == 0 { evidence.fee as i128 } else { 0 };
            if events - fee != native_after[index] as i128 - native_before[index] as i128 {
                unreconciled.insert("SOL");
            }
        } else {
            unreconciled.insert("SOL");
        }
    }
    result.retain(|event| !unreconciled.contains(event.asset.as_str()));
    result.sort_by_key(|entry| entry.event_index);
    Ok(result)
}
