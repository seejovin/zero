//! A deliberately restricted Ethereum software signer.
//!
//! The only signing operation is a reviewed transfer from a fixed mainnet and
//! asset registry. Supporting EVM transaction encoding never enables a chain.
//! BIP39, BIP32, ECDSA, AES-GCM, OS randomness and Keccak come from libraries;
//! this module implements wallet policy and canonical Ethereum serialization.
//! Recovery phrases returned across FFI are necessarily ordinary native strings.
//! Callers must promptly clear their copies; zeroization is best effort, not a
//! defense against a compromised running process or operating system.

use crate::WalletError;
use aes_gcm::{
    aead::{AeadInPlace, KeyInit},
    Aes256Gcm, Nonce,
};
use bip32::{DerivationPath, XPrv};
use bip39::{Language, Mnemonic};
use k256::ecdsa::{
    signature::hazmat::PrehashVerifier, RecoveryId, Signature, SigningKey, VerifyingKey,
};
use primitive_types::U256;
use rlp::{Rlp, RlpStream};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

pub const CHAIN_ID: u64 = 1;
pub const BASE_CHAIN_ID: u64 = 8453;
pub const OP_CHAIN_ID: u64 = 10;
pub const ARBITRUM_CHAIN_ID: u64 = 42161;
pub const AVALANCHE_CHAIN_ID: u64 = 43114;
pub const POLYGON_CHAIN_ID: u64 = 137;
pub const BNB_CHAIN_ID: u64 = 56;
pub const DERIVATION_PATH: &str = "m/44'/60'/0'/0/0";
pub const USDC_CONTRACT: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
pub const EURC_CONTRACT: &str = "0x1aBaEA1f7C830bD89Acc67eC4af516284b1bC33c";
pub const BASE_USDC_CONTRACT: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
pub const BASE_EURC_CONTRACT: &str = "0x60a3E35Cc302bFA44Cb288Bc5a4F316Fdb1adb42";
// Native Circle USDC, not a bridged token with the same ticker. Reviewed against
// https://developers.circle.com/stablecoins/usdc-contract-addresses
pub const OP_USDC_CONTRACT: &str = "0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85";
pub const ARBITRUM_USDC_CONTRACT: &str = "0xaf88d065e77c8cC2239327C5EDb3A432268e5831";
pub const AVALANCHE_USDC_CONTRACT: &str = "0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E";
pub const AVALANCHE_EURC_CONTRACT: &str = "0xC891EB4cbdEFf6e073e859e987815Ed1505c2ACD";
pub const POLYGON_USDC_CONTRACT: &str = "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359";
// Issuer-native USDt only: https://tether.to/en/supported-protocols/
pub const USDT_CONTRACT: &str = "0xdAC17F958D2ee523a2206206994597C13D831ec7";
pub const AVALANCHE_USDT_CONTRACT: &str = "0x9702230a8ea53601f5cd2dc00fdbc13d4df4a8c7";
pub const APPROVAL_LIFETIME_SECONDS: u64 = 300;
pub const MAX_GAS_LIMIT: u64 = 300_000;
pub const MAX_FEE_PER_GAS_WEI: u64 = 500_000_000_000;
pub const MAX_TOTAL_FEE_WEI: u64 = 50_000_000_000_000_000; // 0.05 ETH.

const MAX_JSON_BYTES: usize = 16_384;
const MAX_RAW_BYTES: usize = 1_024;
const VAULT_MAGIC: &[u8; 4] = b"LWVT";
const VAULT_VERSION: u8 = 1;
const VAULT_CONTEXT: &[u8] = b"LocalWallet/entropy-vault/v1\0";
const APPROVAL_CONTEXT: &[u8] = b"LocalWallet/payment-approval/v2\0";
const TRANSFER_SELECTOR: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Asset {
    #[serde(rename = "ETH")]
    Eth,
    #[serde(rename = "USDC")]
    Usdc,
    #[serde(rename = "EURC")]
    Eurc,
    #[serde(rename = "USDT")]
    Usdt,
    #[serde(rename = "AVAX")]
    Avax,
    #[serde(rename = "POL")]
    Pol,
    #[serde(rename = "BNB")]
    Bnb,
}

impl Asset {
    pub fn decimals(self) -> u8 {
        match self {
            Self::Eth | Self::Avax | Self::Pol | Self::Bnb => 18,
            Self::Usdc | Self::Eurc | Self::Usdt => 6,
        }
    }

    pub fn contract(self, chain_id: u64) -> Option<&'static str> {
        match (self, chain_id) {
            (Self::Usdc, CHAIN_ID) => Some(USDC_CONTRACT),
            (Self::Eurc, CHAIN_ID) => Some(EURC_CONTRACT),
            (Self::Usdc, BASE_CHAIN_ID) => Some(BASE_USDC_CONTRACT),
            (Self::Eurc, BASE_CHAIN_ID) => Some(BASE_EURC_CONTRACT),
            (Self::Usdc, OP_CHAIN_ID) => Some(OP_USDC_CONTRACT),
            (Self::Usdc, ARBITRUM_CHAIN_ID) => Some(ARBITRUM_USDC_CONTRACT),
            (Self::Usdc, AVALANCHE_CHAIN_ID) => Some(AVALANCHE_USDC_CONTRACT),
            (Self::Eurc, AVALANCHE_CHAIN_ID) => Some(AVALANCHE_EURC_CONTRACT),
            (Self::Usdt, AVALANCHE_CHAIN_ID) => Some(AVALANCHE_USDT_CONTRACT),
            (Self::Usdc, POLYGON_CHAIN_ID) => Some(POLYGON_USDC_CONTRACT),
            (Self::Usdt, CHAIN_ID) => Some(USDT_CONTRACT),
            _ => None,
        }
    }

    pub fn is_native_on(self, chain_id: u64) -> bool {
        native_asset(chain_id) == Some(self)
    }

    /// A missing token contract must never be interpreted as a native payment.
    pub fn supported_on(self, chain_id: u64) -> bool {
        supported_chain(chain_id)
            && (self.is_native_on(chain_id) || self.contract(chain_id).is_some())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentRequest {
    pub intent_id: String,
    pub chain_id: u64,
    pub from: String,
    pub to: String,
    pub asset: Asset,
    pub amount: String,
    pub nonce: u64,
    pub gas_limit: u64,
    pub max_fee_per_gas: String,
    pub max_priority_fee_per_gas: String,
    pub l1_fee_estimate_wei: String,
    pub operator_fee_estimate_wei: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedPayment {
    pub request: PaymentRequest,
    pub amount_units: String,
    pub max_fee_wei: String,
    pub estimated_total_fee_wei: String,
    pub fee_reserve_wei: String,
    pub unsigned_transaction: String,
    pub approval_digest: String,
    pub created_at: u64,
    pub expires_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedPayment {
    pub intent_id: String,
    pub chain_id: u64,
    pub from: String,
    pub to: String,
    pub asset: Asset,
    pub amount: String,
    pub amount_units: String,
    pub nonce: u64,
    pub max_fee_wei: String,
    pub l1_fee_estimate_wei: String,
    pub operator_fee_estimate_wei: String,
    pub estimated_total_fee_wei: String,
    pub fee_reserve_wei: String,
    pub approval_digest: String,
    pub raw_transaction: String,
    pub hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectedPayment {
    pub chain_id: u64,
    pub from: String,
    /// Beneficiary, including for ERC20 payments.
    pub to: String,
    pub asset: Asset,
    pub amount: String,
    pub amount_units: String,
    pub nonce: u64,
    pub max_fee_wei: String,
    pub raw_transaction: String,
    pub hash: String,
    pub gas_limit: u64,
    pub max_fee_per_gas: String,
    pub max_priority_fee_per_gas: String,
    pub unsigned_transaction: String,
    /// Actual transaction destination: token contract for ERC20 payments.
    pub transaction_to: String,
    pub data: String,
}

#[derive(Clone, PartialEq, Eq)]
struct Transaction {
    chain_id: u64,
    nonce: u64,
    max_priority_fee_per_gas: U256,
    max_fee_per_gas: U256,
    gas_limit: u64,
    to: [u8; 20],
    value: U256,
    data: Vec<u8>,
}

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

pub fn supported_chain(chain_id: u64) -> bool {
    matches!(
        chain_id,
        CHAIN_ID
            | BASE_CHAIN_ID
            | OP_CHAIN_ID
            | ARBITRUM_CHAIN_ID
            | AVALANCHE_CHAIN_ID
            | POLYGON_CHAIN_ID
            | BNB_CHAIN_ID
    )
}

/// Native identity is selected by the reviewed network, never by missing calldata.
pub(crate) fn native_asset(chain_id: u64) -> Option<Asset> {
    match chain_id {
        CHAIN_ID | BASE_CHAIN_ID | OP_CHAIN_ID | ARBITRUM_CHAIN_ID => Some(Asset::Eth),
        AVALANCHE_CHAIN_ID => Some(Asset::Avax),
        POLYGON_CHAIN_ID => Some(Asset::Pol),
        BNB_CHAIN_ID => Some(Asset::Bnb),
        _ => None,
    }
}

/// Local wallet spending limits, not advertised protocol limits or fee estimates.
pub(crate) fn maximum_fee_per_gas(chain_id: u64) -> u64 {
    match chain_id {
        POLYGON_CHAIN_ID => 5_000_000_000_000,
        BNB_CHAIN_ID => 10_000_000_000,
        _ => MAX_FEE_PER_GAS_WEI,
    }
}

pub(crate) fn maximum_fee_reserve(chain_id: u64) -> u64 {
    match chain_id {
        AVALANCHE_CHAIN_ID => 500_000_000_000_000_000, // 0.5 AVAX
        POLYGON_CHAIN_ID => 1_000_000_000_000_000_000, // 1 POL
        BNB_CHAIN_ID => 10_000_000_000_000_000,        // 0.01 BNB
        _ => MAX_TOTAL_FEE_WEI,                        // 0.05 ETH on the four existing networks
    }
}

/// These two reviewed OP Stack networks charge L1/operator fees separately.
/// Arbitrum's gas estimate and gas charge already include L1 posting costs.
pub(crate) fn has_separate_rollup_fees(chain_id: u64) -> bool {
    matches!(chain_id, BASE_CHAIN_ID | OP_CHAIN_ID)
}

#[cfg(not(target_arch = "wasm32"))]
fn current_time() -> Result<u64, WalletError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| policy("The device clock is invalid"))
}

#[cfg(target_arch = "wasm32")]
fn current_time() -> Result<u64, WalletError> {
    let milliseconds = js_sys::Date::now();
    if !milliseconds.is_finite() || milliseconds < 1000.0 || milliseconds > 9_007_199_254_740_991.0
    {
        return Err(policy("The device clock is invalid"));
    }
    Ok((milliseconds / 1000.0).floor() as u64)
}

/// Browser password derivation. Native Android uses a random vault key wrapped
/// by Android Keystore instead. Parameters and encoding are versioned by the UI.
pub fn derive_vault_key(password: String, salt: Vec<u8>) -> Result<Vec<u8>, WalletError> {
    use argon2::{Algorithm, Argon2, Block, Params, Version};
    let password = Zeroizing::new(password);
    if password.chars().count() < 12 || password.len() > 1024 || salt.len() != 16 {
        return Err(invalid("Password must contain at least 12 characters and at most 1024 UTF-8 bytes; salt must contain 16 bytes"));
    }
    let parameters = Params::new(65_536, 3, 1, Some(32)).map_err(|_| WalletError::Crypto)?;
    // Own and clear the full work area as well as the password/output. The
    // library's allocating convenience API frees its work area without wiping
    // every block, even with its zeroize feature enabled.
    let block_count = parameters.block_count();
    let mut memory = Zeroizing::new(Vec::<Block>::new());
    memory
        .try_reserve_exact(block_count)
        .map_err(|_| WalletError::Crypto)?;
    memory.resize(block_count, Block::default());
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, parameters);
    let mut output = Zeroizing::new(vec![0u8; 32]);
    argon
        .hash_password_into_with_memory(
            password.as_bytes(),
            &salt,
            output.as_mut_slice(),
            memory.as_mut_slice(),
        )
        .map_err(|_| WalletError::Crypto)?;
    Ok(output.to_vec())
}

fn json_input<T: for<'de> Deserialize<'de>>(input: &str) -> Result<T, WalletError> {
    if input.len() > MAX_JSON_BYTES {
        return Err(invalid("Input exceeds the size limit"));
    }
    // Do not expose serde error text: a malformed field could contain a secret.
    serde_json::from_str(input)
        .map_err(|_| invalid("JSON is malformed or contains unsupported fields"))
}

fn json_output<T: Serialize>(value: &T) -> Result<String, WalletError> {
    serde_json::to_string(value).map_err(|_| WalletError::Crypto)
}

fn prefixed_hex(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

fn parse_hex(input: &str, max_bytes: usize) -> Result<Vec<u8>, WalletError> {
    if input.len() > max_bytes.saturating_mul(2).saturating_add(2)
        || !input.starts_with("0x")
        || !input.len().is_multiple_of(2)
    {
        return Err(invalid("Invalid or oversized hexadecimal data"));
    }
    hex::decode(&input[2..]).map_err(|_| invalid("Invalid hexadecimal data"))
}

fn checksum_address(address: &[u8; 20]) -> String {
    let lower = hex::encode(address);
    let hash = Keccak256::digest(lower.as_bytes());
    let mut result = String::with_capacity(42);
    result.push_str("0x");
    for (i, c) in lower.bytes().enumerate() {
        let nibble = if i % 2 == 0 {
            hash[i / 2] >> 4
        } else {
            hash[i / 2] & 15
        };
        result.push(if c.is_ascii_lowercase() && nibble >= 8 {
            (c as char).to_ascii_uppercase()
        } else {
            c as char
        });
    }
    result
}

fn address_bytes(input: &str) -> Result<[u8; 20], WalletError> {
    if input.len() != 42 || !input.starts_with("0x") || !input[2..].is_ascii() {
        return Err(invalid(
            "An Ethereum address must contain 0x and 40 hexadecimal digits",
        ));
    }
    let decoded = hex::decode(&input[2..]).map_err(|_| invalid("Invalid Ethereum address"))?;
    let address: [u8; 20] = decoded
        .try_into()
        .map_err(|_| invalid("Invalid Ethereum address length"))?;
    let mixed_case = input[2..].bytes().any(|c| c.is_ascii_lowercase())
        && input[2..].bytes().any(|c| c.is_ascii_uppercase());
    if mixed_case && checksum_address(&address) != input {
        return Err(invalid("Ethereum address checksum is invalid"));
    }
    Ok(address)
}

pub fn normalize_address(input: &str) -> Result<String, WalletError> {
    Ok(checksum_address(&address_bytes(input)?))
}

fn address_for_key(key: &VerifyingKey) -> Result<String, WalletError> {
    let public = key.to_sec1_point(false);
    let bytes = public.as_bytes();
    if bytes.len() != 65 || bytes[0] != 4 {
        return Err(WalletError::Crypto);
    }
    let hash = Keccak256::digest(&bytes[1..]);
    let mut address = [0u8; 20];
    address.copy_from_slice(&hash[12..]);
    Ok(checksum_address(&address))
}

fn derive_key(mnemonic: &Mnemonic) -> Result<SigningKey, WalletError> {
    let seed = Zeroizing::new(mnemonic.to_seed_normalized(""));
    let path: DerivationPath = DERIVATION_PATH.parse().map_err(|_| WalletError::Crypto)?;
    let extended = XPrv::derive_from_path(&*seed, &path).map_err(|_| WalletError::Crypto)?;
    // SigningKey and its secret scalar zeroize on drop. The extended key drops
    // here; the seed is in a Zeroizing scope. No private key is serialized.
    Ok(extended.private_key().clone())
}

fn check_vault_key(key: &[u8]) -> Result<(), WalletError> {
    if key.len() != 32 {
        return Err(invalid("A vault encryption key must be exactly 32 bytes"));
    }
    Ok(())
}

fn entropy_length_allowed(length: usize) -> bool {
    matches!(length, 16 | 20 | 24 | 28 | 32)
}

fn seal_entropy(entropy: &[u8], key: &[u8]) -> Result<String, WalletError> {
    check_vault_key(key)?;
    if !entropy_length_allowed(entropy.len()) {
        return Err(WalletError::Crypto);
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| WalletError::Crypto)?;
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|_| WalletError::Crypto)?;
    let mut header = Vec::from(VAULT_MAGIC.as_slice());
    header.extend_from_slice(&[VAULT_VERSION, entropy.len() as u8]);
    let mut aad = Vec::from(VAULT_CONTEXT);
    aad.extend_from_slice(&header);
    // Encrypt in place to avoid a second plaintext allocation.
    let mut ciphertext = Zeroizing::new(entropy.to_vec());
    cipher
        .encrypt_in_place(Nonce::from_slice(&nonce), &aad, &mut *ciphertext)
        .map_err(|_| WalletError::Crypto)?;
    let mut vault = header;
    vault.extend_from_slice(&nonce);
    vault.extend_from_slice(&ciphertext);
    Ok(prefixed_hex(&vault))
}

fn open_entropy(vault_hex: &str, key: &[u8]) -> Result<Zeroizing<Vec<u8>>, WalletError> {
    check_vault_key(key)?;
    let vault = parse_hex(vault_hex, 66)?;
    if vault.len() < 50
        || &vault[..4] != VAULT_MAGIC
        || vault[4] != VAULT_VERSION
        || !entropy_length_allowed(vault[5] as usize)
        || vault.len() != 6 + 12 + vault[5] as usize + 16
    {
        return Err(invalid("Unsupported or damaged wallet vault"));
    }
    let mut aad = Vec::from(VAULT_CONTEXT);
    aad.extend_from_slice(&vault[..6]);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| WalletError::Crypto)?;
    let mut plaintext = Zeroizing::new(vault[18..].to_vec());
    cipher
        .decrypt_in_place(Nonce::from_slice(&vault[6..18]), &aad, &mut *plaintext)
        .map_err(|_| WalletError::Crypto)?;
    if plaintext.len() != vault[5] as usize {
        return Err(WalletError::Crypto);
    }
    Ok(plaintext)
}

pub fn generate_wallet(vault_key: Vec<u8>) -> Result<String, WalletError> {
    let key = Zeroizing::new(vault_key);
    check_vault_key(&key)?;
    let mut entropy = Zeroizing::new([0u8; 32]);
    getrandom::fill(&mut *entropy).map_err(|_| WalletError::Crypto)?;
    let mnemonic = Zeroizing::new(
        Mnemonic::from_entropy_in(Language::English, &*entropy).map_err(|_| WalletError::Crypto)?,
    );
    let signing_key = derive_key(&mnemonic)?;
    let address = address_for_key(signing_key.verifying_key())?;
    let vault_hex = seal_entropy(&*entropy, &key)?;
    let phrase = Zeroizing::new(mnemonic.to_string());
    // The single intentional secret-bearing return is required for backup UI.
    #[derive(Serialize)]
    struct Created<'a> {
        vault_hex: String,
        address: String,
        mnemonic: &'a str,
    }
    json_output(&Created {
        vault_hex,
        address,
        mnemonic: &phrase,
    })
}

pub fn restore_wallet(mnemonic: String, vault_key: Vec<u8>) -> Result<String, WalletError> {
    let phrase = Zeroizing::new(mnemonic);
    let key = Zeroizing::new(vault_key);
    check_vault_key(&key)?;
    if phrase.len() > 512 || !phrase.is_ascii() {
        return Err(invalid("Enter a standard English BIP39 recovery phrase"));
    }
    let words = Zeroizing::new(
        Mnemonic::parse_in_normalized(Language::English, phrase.as_str())
            .map_err(|_| invalid("The recovery phrase or its checksum is invalid"))?,
    );
    let entropy = Zeroizing::new(words.to_entropy());
    let signing_key = derive_key(&words)?;
    let address = address_for_key(signing_key.verifying_key())?;
    let vault_hex = seal_entropy(&entropy, &key)?;
    #[derive(Serialize)]
    struct Restored {
        vault_hex: String,
        address: String,
    }
    json_output(&Restored { vault_hex, address })
}

pub fn reveal_mnemonic(vault_hex: String, vault_key: Vec<u8>) -> Result<String, WalletError> {
    let key = Zeroizing::new(vault_key);
    let entropy = open_entropy(&vault_hex, &key)?;
    let mnemonic = Zeroizing::new(
        Mnemonic::from_entropy_in(Language::English, &entropy).map_err(|_| WalletError::Crypto)?,
    );
    Ok(mnemonic.to_string())
}

/// Authenticate the vault and derive its address without revealing recovery
/// words. The native app must compare this with saved public metadata before
/// exposing a receive address after unlocking an existing wallet.
pub fn wallet_address(vault_hex: String, vault_key: Vec<u8>) -> Result<String, WalletError> {
    let key = Zeroizing::new(vault_key);
    let entropy = open_entropy(&vault_hex, &key)?;
    let mnemonic = Zeroizing::new(
        Mnemonic::from_entropy_in(Language::English, &entropy).map_err(|_| WalletError::Crypto)?,
    );
    let signing_key = derive_key(&mnemonic)?;
    address_for_key(signing_key.verifying_key())
}

/// Authenticated seed access for the separately reviewed native chain signers.
/// This never crosses FFI: neither a seed nor a derived private key is returned
/// to application code. Preserve the existing BIP39 empty-passphrase wallet.
pub(crate) fn open_seed(vault_hex: &str, key: &[u8]) -> Result<Zeroizing<[u8; 64]>, WalletError> {
    let entropy = open_entropy(vault_hex, key)?;
    let mnemonic = Zeroizing::new(
        Mnemonic::from_entropy_in(Language::English, &entropy).map_err(|_| WalletError::Crypto)?,
    );
    Ok(Zeroizing::new(mnemonic.to_seed_normalized("")))
}

fn parse_uint(input: &str) -> Result<U256, WalletError> {
    if input.is_empty()
        || input.len() > 78
        || !input.bytes().all(|b| b.is_ascii_digit())
        || (input.len() > 1 && input.starts_with('0'))
    {
        return Err(invalid("Use a canonical unsigned decimal integer"));
    }
    U256::from_dec_str(input).map_err(|_| invalid("Integer exceeds the supported range"))
}

fn parse_amount(input: &str, decimals: u8) -> Result<U256, WalletError> {
    if input.is_empty() || input.len() > 100 || !input.is_ascii() {
        return Err(invalid("Invalid payment amount"));
    }
    let mut parts = input.split('.');
    let whole = parts
        .next()
        .ok_or_else(|| invalid("Invalid payment amount"))?;
    let fraction = parts.next();
    if parts.next().is_some()
        || whole.is_empty()
        || (whole.len() > 1 && whole.starts_with('0'))
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.is_some_and(|part| {
            part.is_empty()
                || part.len() > usize::from(decimals)
                || !part.bytes().all(|b| b.is_ascii_digit())
        })
    {
        return Err(invalid(
            "Use a positive decimal amount within the asset's precision",
        ));
    }
    let factor = U256::from(10u8).pow(U256::from(decimals));
    let mut units = parse_uint(whole)?
        .checked_mul(factor)
        .ok_or_else(|| invalid("Payment amount exceeds the supported range"))?;
    if let Some(part) = fraction {
        let mut fractional = U256::zero();
        for digit in part.bytes() {
            fractional = fractional
                .checked_mul(U256::from(10u8))
                .and_then(|value| value.checked_add(U256::from(digit - b'0')))
                .ok_or_else(|| invalid("Payment amount exceeds the supported range"))?;
        }
        let padding = U256::from(10u8).pow(U256::from(decimals as usize - part.len()));
        units = units
            .checked_add(fractional * padding)
            .ok_or_else(|| invalid("Payment amount exceeds the supported range"))?;
    }
    if units.is_zero() {
        return Err(invalid("Payment amount must be greater than zero"));
    }
    Ok(units)
}

fn format_units(units: U256, decimals: u8) -> String {
    let digits = units.to_string();
    if decimals == 0 {
        return digits;
    }
    let places = decimals as usize;
    let padded = if digits.len() <= places {
        format!("{}{}", "0".repeat(places + 1 - digits.len()), digits)
    } else {
        digits
    };
    let (whole, fraction) = padded.split_at(padded.len() - places);
    let trimmed = fraction.trim_end_matches('0');
    if trimmed.is_empty() {
        whole.to_owned()
    } else {
        format!("{whole}.{trimmed}")
    }
}

pub fn format_amount(units: String, decimals: u8) -> Result<String, WalletError> {
    if decimals > 18 {
        return Err(invalid("Only 0 to 18 decimal places are supported"));
    }
    Ok(format_units(parse_uint(&units)?, decimals))
}

fn normalized_request(
    mut request: PaymentRequest,
) -> Result<(PaymentRequest, Transaction, U256), WalletError> {
    if request.intent_id.is_empty()
        || request.intent_id.len() > 128
        || !request
            .intent_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(invalid(
            "Intent ID must contain 1 to 128 letters, digits, hyphens or underscores",
        ));
    }
    if !supported_chain(request.chain_id) {
        return Err(policy(
            "The chain is not in the reviewed EVM mainnet registry",
        ));
    }
    if !request.asset.supported_on(request.chain_id) {
        return Err(policy("The asset is not supported on the selected network"));
    }
    if request.nonce == u64::MAX {
        return Err(invalid("Account nonce exceeds Ethereum's supported range"));
    }
    let from = address_bytes(&request.from)?;
    let recipient = address_bytes(&request.to)?;
    if from == [0u8; 20] || recipient == [0u8; 20] {
        return Err(invalid("The zero address cannot send or receive a payment"));
    }
    // Preserve the original beneficiary policy for existing Ethereum/Base
    // records. Add the selected chain's native USDC contract without making
    // previously valid historical payments on other chains fail on reopen.
    if [
        USDC_CONTRACT,
        EURC_CONTRACT,
        BASE_USDC_CONTRACT,
        BASE_EURC_CONTRACT,
    ]
    .iter()
    .any(|contract| address_bytes(contract).is_ok_and(|address| address == recipient))
        || Asset::Usdc
            .contract(request.chain_id)
            .is_some_and(|contract| {
                address_bytes(contract).is_ok_and(|address| address == recipient)
            })
        || ([Asset::Eurc, Asset::Usdt].into_iter().any(|asset| {
            // Never retroactively invalidate legacy ETH payments on the original
            // four chains; new assets/chains receive the stricter selected-chain rule.
            (matches!(
                request.chain_id,
                AVALANCHE_CHAIN_ID | POLYGON_CHAIN_ID | BNB_CHAIN_ID
            ) || request.asset == Asset::Usdt)
                && asset.contract(request.chain_id).is_some_and(|contract| {
                    address_bytes(contract).is_ok_and(|address| address == recipient)
                })
        }))
    {
        return Err(policy("A token contract cannot be the payment beneficiary"));
    }
    request.from = checksum_address(&from);
    request.to = checksum_address(&recipient);
    let amount = parse_amount(&request.amount, request.asset.decimals())?;
    request.amount = format_units(amount, request.asset.decimals());
    let max_fee = parse_uint(&request.max_fee_per_gas)?;
    let priority_fee = parse_uint(&request.max_priority_fee_per_gas)?;
    if max_fee.is_zero() || max_fee > U256::from(maximum_fee_per_gas(request.chain_id)) {
        return Err(policy(
            "Maximum fee per gas must be positive and within this network's spending ceiling",
        ));
    }
    if priority_fee > max_fee {
        return Err(invalid("Priority fee cannot exceed maximum fee per gas"));
    }
    // BSC accepts EIP-1559 envelopes after London, with a zero base fee.
    // Lock both caps to the explicitly quoted gas price; do not invent an ETH base-fee model.
    if request.chain_id == BNB_CHAIN_ID && priority_fee != max_fee {
        return Err(policy(
            "BNB fee and priority caps must equal the reviewed gas price",
        ));
    }
    if !(21_000..=MAX_GAS_LIMIT).contains(&request.gas_limit) {
        return Err(policy("Gas limit must be between 21000 and 300000"));
    }
    let total_fee = max_fee
        .checked_mul(U256::from(request.gas_limit))
        .ok_or_else(|| invalid("Maximum network fee overflow"))?;
    if total_fee > U256::from(maximum_fee_reserve(request.chain_id)) {
        return Err(policy(
            "Maximum total network fee exceeds this network's spending ceiling",
        ));
    }
    request.max_fee_per_gas = max_fee.to_string();
    request.max_priority_fee_per_gas = priority_fee.to_string();
    let l1_fee = parse_uint(&request.l1_fee_estimate_wei)?;
    let operator_fee = parse_uint(&request.operator_fee_estimate_wei)?;
    if !has_separate_rollup_fees(request.chain_id) && (!l1_fee.is_zero() || !operator_fee.is_zero())
    {
        return Err(invalid(
            "This network does not charge separate L1 or operator fees",
        ));
    }
    request.l1_fee_estimate_wei = l1_fee.to_string();
    request.operator_fee_estimate_wei = operator_fee.to_string();
    let extra = l1_fee
        .checked_add(operator_fee)
        .ok_or_else(|| invalid("Additional fee estimate overflow"))?;
    let reserve = total_fee
        .checked_add(
            extra
                .checked_mul(U256::from(2))
                .ok_or_else(|| invalid("Fee reserve overflow"))?,
        )
        .ok_or_else(|| invalid("Fee reserve overflow"))?;
    if reserve > U256::from(maximum_fee_reserve(request.chain_id)) {
        return Err(policy(
            "Reviewed execution and additional fee reserve exceeds this network's spending ceiling",
        ));
    }
    let (destination, value, data) = match request.asset {
        asset if asset.is_native_on(request.chain_id) => {
            amount
                .checked_add(reserve)
                .ok_or_else(|| invalid("Amount plus maximum fee exceeds the supported range"))?;
            (recipient, amount, Vec::new())
        }
        asset => {
            let contract = asset
                .contract(request.chain_id)
                .ok_or_else(|| policy("The token is not in the reviewed asset registry"))?;
            let mut data = Vec::with_capacity(68);
            data.extend_from_slice(&TRANSFER_SELECTOR);
            data.extend_from_slice(&[0u8; 12]);
            data.extend_from_slice(&recipient);
            data.extend_from_slice(&amount.to_big_endian());
            (address_bytes(contract)?, U256::zero(), data)
        }
    };
    // EIP-7623 requires the gas limit to cover the calldata floor even when
    // execution would use less gas. For our non-creation, empty-access-list
    // type-2 transfers the floor (10/40 per zero/nonzero byte) dominates the
    // older intrinsic data price (4/16). Actual execution is still preflighted
    // with eth_call and eth_estimateGas by the native app.
    let minimum_gas = 21_000u64
        + data
            .iter()
            .map(|byte| if *byte == 0 { 10u64 } else { 40u64 })
            .sum::<u64>();
    if request.gas_limit < minimum_gas {
        return Err(invalid(
            "Gas limit is below the transaction's intrinsic gas or calldata floor",
        ));
    }
    let transaction = Transaction {
        chain_id: request.chain_id,
        nonce: request.nonce,
        max_priority_fee_per_gas: priority_fee,
        max_fee_per_gas: max_fee,
        gas_limit: request.gas_limit,
        to: destination,
        value,
        data,
    };
    Ok((request, transaction, amount))
}

fn append_integer(stream: &mut RlpStream, number: U256) {
    let bytes = number.to_big_endian();
    let first = bytes.iter().position(|b| *b != 0).unwrap_or(32);
    stream.append(&&bytes[first..]);
}

fn append_transaction_fields(stream: &mut RlpStream, transaction: &Transaction) {
    append_integer(stream, U256::from(transaction.chain_id));
    append_integer(stream, U256::from(transaction.nonce));
    append_integer(stream, transaction.max_priority_fee_per_gas);
    append_integer(stream, transaction.max_fee_per_gas);
    append_integer(stream, U256::from(transaction.gas_limit));
    stream.append(&transaction.to.as_slice());
    append_integer(stream, transaction.value);
    stream.append(&transaction.data.as_slice());
    stream.begin_list(0); // No access list is supported by this transfer-only signer.
}

fn encode_unsigned(transaction: &Transaction) -> Vec<u8> {
    let mut stream = RlpStream::new_list(9);
    append_transaction_fields(&mut stream, transaction);
    let mut bytes = vec![2u8];
    bytes.extend_from_slice(&stream.out());
    bytes
}

fn encode_signed(
    transaction: &Transaction,
    signature: &Signature,
    recovery: RecoveryId,
) -> Vec<u8> {
    let mut stream = RlpStream::new_list(12);
    append_transaction_fields(&mut stream, transaction);
    append_integer(&mut stream, U256::from(recovery.to_byte()));
    let bytes = signature.to_bytes();
    append_integer(&mut stream, U256::from_big_endian(&bytes[..32]));
    append_integer(&mut stream, U256::from_big_endian(&bytes[32..]));
    let mut raw = vec![2u8];
    raw.extend_from_slice(&stream.out());
    raw
}

fn approval_digest(
    request: &PaymentRequest,
    unsigned: &[u8],
    created_at: u64,
    expires_at: u64,
) -> Result<String, WalletError> {
    let mut hasher = Keccak256::new();
    hasher.update(APPROVAL_CONTEXT);
    hasher.update((request.intent_id.len() as u16).to_be_bytes());
    hasher.update(request.intent_id.as_bytes());
    hasher.update(address_bytes(&request.from)?);
    hasher.update(created_at.to_be_bytes());
    hasher.update(expires_at.to_be_bytes());
    hasher.update((unsigned.len() as u32).to_be_bytes());
    hasher.update(unsigned);
    // These quotes are review metadata, not EIP-1559 transaction fields and not
    // protocol-enforced caps. Binding them prevents a changed fee review from
    // reusing approval for the same unsigned bytes.
    hasher.update(parse_uint(&request.l1_fee_estimate_wei)?.to_big_endian());
    hasher.update(parse_uint(&request.operator_fee_estimate_wei)?.to_big_endian());
    Ok(prefixed_hex(&hasher.finalize()))
}

fn prepare_at(request: PaymentRequest, created_at: u64) -> Result<PreparedPayment, WalletError> {
    if created_at == 0 {
        return Err(policy("The device clock is invalid"));
    }
    let (request, transaction, amount) = normalized_request(request)?;
    let expires_at = created_at
        .checked_add(APPROVAL_LIFETIME_SECONDS)
        .ok_or_else(|| policy("The device clock is invalid"))?;
    let unsigned = encode_unsigned(&transaction);
    let digest = approval_digest(&request, &unsigned, created_at, expires_at)?;
    let execution = transaction.max_fee_per_gas * U256::from(transaction.gas_limit);
    let additional =
        parse_uint(&request.l1_fee_estimate_wei)? + parse_uint(&request.operator_fee_estimate_wei)?;
    Ok(PreparedPayment {
        request,
        amount_units: amount.to_string(),
        max_fee_wei: execution.to_string(),
        estimated_total_fee_wei: (execution + additional).to_string(),
        fee_reserve_wei: (execution + additional * U256::from(2)).to_string(),
        unsigned_transaction: prefixed_hex(&unsigned),
        approval_digest: digest,
        created_at,
        expires_at,
    })
}

pub fn prepare_payment(request_json: String) -> Result<String, WalletError> {
    let request: PaymentRequest = json_input(&request_json)?;
    json_output(&prepare_at(request, current_time()?)?)
}

/// Verify immutable preparation data without applying current-time expiration.
/// Journal reopening and historical reconciliation must remain possible after
/// the five-minute signing window. `sign_payment` separately enforces freshness.
pub fn validate_prepared(prepared_json: &str) -> Result<PreparedPayment, WalletError> {
    let prepared: PreparedPayment = json_input(prepared_json)?;
    let rebuilt = prepare_at(prepared.request.clone(), prepared.created_at)?;
    if prepared != rebuilt {
        return Err(policy("The reviewed transaction has changed or is invalid"));
    }
    Ok(prepared)
}

fn ensure_fresh(prepared: &PreparedPayment, now: u64) -> Result<(), WalletError> {
    if now < prepared.created_at || now >= prepared.expires_at {
        return Err(policy(
            "Transaction review has expired; prepare and review it again",
        ));
    }
    Ok(())
}

pub fn sign_payment(
    vault_hex: String,
    vault_key: Vec<u8>,
    prepared_json: String,
    approved_digest: String,
) -> Result<String, WalletError> {
    let key = Zeroizing::new(vault_key);
    check_vault_key(&key)?;
    let prepared = validate_prepared(&prepared_json)?;
    if approved_digest != prepared.approval_digest {
        return Err(policy("Approval does not match the reviewed transaction"));
    }
    ensure_fresh(&prepared, current_time()?)?;
    let entropy = open_entropy(&vault_hex, &key)?;
    let mnemonic = Zeroizing::new(
        Mnemonic::from_entropy_in(Language::English, &entropy).map_err(|_| WalletError::Crypto)?,
    );
    let signing_key = derive_key(&mnemonic)?;
    let from = address_for_key(signing_key.verifying_key())?;
    if from != prepared.request.from {
        return Err(policy(
            "The signing wallet does not match the reviewed sender",
        ));
    }
    let (_, transaction, _) = normalized_request(prepared.request.clone())?;
    let unsigned = encode_unsigned(&transaction);
    let (signature, recovery) = signing_key.sign_prehash_recoverable(&Keccak256::digest(&unsigned));
    if recovery.to_byte() > 1 || signature.normalize_s() != signature {
        return Err(WalletError::Crypto);
    }
    signing_key
        .verifying_key()
        .verify_prehash(&Keccak256::digest(&unsigned), &signature)
        .map_err(|_| WalletError::Crypto)?;
    let raw = encode_signed(&transaction, &signature, recovery);
    let inspected = inspect_bytes(&raw)?;
    if inspected.from != from
        || inspected.unsigned_transaction != prepared.unsigned_transaction
        || inspected.to != prepared.request.to
        || inspected.asset != prepared.request.asset
        || inspected.amount_units != prepared.amount_units
    {
        return Err(WalletError::Crypto);
    }
    // Check once more after expensive derivation. No signature crosses FFI if
    // the review expired while deriving the key or performing cryptography.
    ensure_fresh(&prepared, current_time()?)?;
    json_output(&SignedPayment {
        intent_id: prepared.request.intent_id,
        chain_id: inspected.chain_id,
        from: inspected.from,
        to: inspected.to,
        asset: inspected.asset,
        amount: inspected.amount,
        amount_units: inspected.amount_units,
        nonce: inspected.nonce,
        max_fee_wei: inspected.max_fee_wei,
        l1_fee_estimate_wei: prepared.request.l1_fee_estimate_wei,
        operator_fee_estimate_wei: prepared.request.operator_fee_estimate_wei,
        estimated_total_fee_wei: prepared.estimated_total_fee_wei,
        fee_reserve_wei: prepared.fee_reserve_wei,
        approval_digest: prepared.approval_digest,
        raw_transaction: inspected.raw_transaction,
        hash: inspected.hash,
    })
}

fn rlp_data<'a>(rlp: &'a Rlp<'a>, index: usize) -> Result<&'a [u8], WalletError> {
    let item = rlp
        .at(index)
        .map_err(|_| invalid("Malformed Ethereum transaction field"))?;
    if !item.is_data() {
        return Err(invalid(
            "Ethereum transaction field must be bytes, not a list",
        ));
    }
    item.data()
        .map_err(|_| invalid("Malformed Ethereum transaction field"))
}

fn rlp_uint(rlp: &Rlp<'_>, index: usize) -> Result<U256, WalletError> {
    let bytes = rlp_data(rlp, index)?;
    if bytes.len() > 32 || bytes.first() == Some(&0) {
        return Err(invalid("Noncanonical Ethereum integer"));
    }
    Ok(U256::from_big_endian(bytes))
}

fn rlp_u64(rlp: &Rlp<'_>, index: usize) -> Result<u64, WalletError> {
    let number = rlp_uint(rlp, index)?;
    if number > U256::from(u64::MAX) {
        return Err(invalid("Ethereum integer exceeds the supported range"));
    }
    Ok(number.low_u64())
}

fn inspect_bytes(raw: &[u8]) -> Result<InspectedPayment, WalletError> {
    if raw.len() < 2 || raw.len() > MAX_RAW_BYTES || raw[0] != 2 {
        return Err(policy("Only canonical EIP1559 transactions are supported"));
    }
    let rlp = Rlp::new(&raw[1..]);
    let info = rlp
        .payload_info()
        .map_err(|_| invalid("Malformed Ethereum transaction"))?;
    if info.header_len.checked_add(info.value_len) != Some(raw.len() - 1)
        || !rlp.is_list()
        || rlp
            .item_count()
            .map_err(|_| invalid("Malformed Ethereum transaction"))?
            != 12
    {
        return Err(invalid(
            "Ethereum transaction has an invalid field count or trailing data",
        ));
    }
    let chain_id = rlp_u64(&rlp, 0)?;
    if !supported_chain(chain_id) {
        return Err(policy(
            "The chain is not in the reviewed EVM mainnet registry",
        ));
    }
    let to: [u8; 20] = rlp_data(&rlp, 5)?
        .try_into()
        .map_err(|_| policy("Contract creation is not supported"))?;
    let access_list = rlp.at(8).map_err(|_| invalid("Malformed access list"))?;
    if !access_list.is_list()
        || access_list
            .item_count()
            .map_err(|_| invalid("Malformed access list"))?
            != 0
    {
        return Err(policy(
            "Access lists are not supported in this transfer-only wallet",
        ));
    }
    let transaction = Transaction {
        chain_id,
        nonce: rlp_u64(&rlp, 1)?,
        max_priority_fee_per_gas: rlp_uint(&rlp, 2)?,
        max_fee_per_gas: rlp_uint(&rlp, 3)?,
        gas_limit: rlp_u64(&rlp, 4)?,
        to,
        value: rlp_uint(&rlp, 6)?,
        data: rlp_data(&rlp, 7)?.to_vec(),
    };
    let parity = rlp_u64(&rlp, 9)?;
    if parity > 1 {
        return Err(invalid("Invalid Ethereum signature recovery parity"));
    }
    let r = rlp_uint(&rlp, 10)?.to_big_endian();
    let s = rlp_uint(&rlp, 11)?.to_big_endian();
    let signature =
        Signature::from_scalars(r, s).map_err(|_| invalid("Invalid Ethereum signature"))?;
    if signature.normalize_s() != signature {
        return Err(invalid(
            "Ethereum signatures must use canonical low-s values",
        ));
    }
    let recovery =
        RecoveryId::try_from(parity as u8).map_err(|_| invalid("Invalid signature parity"))?;
    if encode_signed(&transaction, &signature, recovery) != raw {
        return Err(invalid("Ethereum transaction encoding is not canonical"));
    }
    let (asset, recipient, units) = if transaction.data.is_empty() {
        (
            native_asset(chain_id).ok_or_else(|| policy("Unsupported native asset"))?,
            transaction.to,
            transaction.value,
        )
    } else {
        if transaction.data.len() != 68
            || transaction.data[..4] != TRANSFER_SELECTOR
            || transaction.data[4..16] != [0u8; 12]
            || !transaction.value.is_zero()
        {
            return Err(policy(
                "Only a direct ERC20 transfer with zero native value is supported",
            ));
        }
        let asset = [Asset::Usdc, Asset::Eurc, Asset::Usdt]
            .into_iter()
            .find(|asset| {
                asset.contract(chain_id).is_some_and(|contract| {
                    address_bytes(contract).is_ok_and(|address| address == transaction.to)
                })
            })
            .ok_or_else(|| policy("The token contract is not in the reviewed asset registry"))?;
        let mut recipient = [0u8; 20];
        recipient.copy_from_slice(&transaction.data[16..36]);
        let units = U256::from_big_endian(&transaction.data[36..68]);
        (asset, recipient, units)
    };
    let unsigned = encode_unsigned(&transaction);
    let verifying_key =
        VerifyingKey::recover_from_prehash(&Keccak256::digest(&unsigned), &signature, recovery)
            .map_err(|_| invalid("Ethereum signature recovery failed"))?;
    verifying_key
        .verify_prehash(&Keccak256::digest(&unsigned), &signature)
        .map_err(|_| invalid("Ethereum signature verification failed"))?;
    let from = address_for_key(&verifying_key)?;
    let (request, rebuilt, _) = normalized_request(PaymentRequest {
        intent_id: "inspection".into(),
        chain_id,
        from,
        to: checksum_address(&recipient),
        asset,
        amount: format_units(units, asset.decimals()),
        nonce: transaction.nonce,
        gas_limit: transaction.gas_limit,
        max_fee_per_gas: transaction.max_fee_per_gas.to_string(),
        max_priority_fee_per_gas: transaction.max_priority_fee_per_gas.to_string(),
        l1_fee_estimate_wei: "0".into(),
        operator_fee_estimate_wei: "0".into(),
    })?;
    if rebuilt != transaction {
        return Err(policy("Transaction does not match an allowed payment"));
    }
    Ok(InspectedPayment {
        chain_id,
        from: request.from,
        to: request.to,
        asset,
        amount: request.amount,
        amount_units: units.to_string(),
        nonce: transaction.nonce,
        max_fee_wei: (transaction.max_fee_per_gas * U256::from(transaction.gas_limit)).to_string(),
        raw_transaction: prefixed_hex(raw),
        hash: prefixed_hex(&Keccak256::digest(raw)),
        gas_limit: transaction.gas_limit,
        max_fee_per_gas: transaction.max_fee_per_gas.to_string(),
        max_priority_fee_per_gas: transaction.max_priority_fee_per_gas.to_string(),
        unsigned_transaction: prefixed_hex(&unsigned),
        transaction_to: checksum_address(&transaction.to),
        data: prefixed_hex(&transaction.data),
    })
}

pub fn inspect_signed(raw_transaction: String) -> Result<String, WalletError> {
    let bytes = parse_hex(&raw_transaction, MAX_RAW_BYTES)?;
    json_output(&inspect_bytes(&bytes)?)
}

pub fn asset_registry() -> String {
    // Fixed reviewed registry. No metadata endpoint can add a signable token.
    serde_json::json!({"networks": [
        {"chain_id": CHAIN_ID, "network": "Ethereum", "rpc_url": "https://ethereum-rpc.publicnode.com", "broadcast_url": "https://ethereum-rpc.publicnode.com", "explorer_url": "https://etherscan.io",
         "assets": [{"symbol":"ETH","decimals":18,"contract":null},{"symbol":"USDC","decimals":6,"contract":USDC_CONTRACT},{"symbol":"EURC","decimals":6,"contract":EURC_CONTRACT},{"symbol":"USDT","decimals":6,"contract":USDT_CONTRACT}]},
        {"chain_id": BASE_CHAIN_ID, "network": "Base", "rpc_url": "https://mainnet.base.org", "broadcast_url": "https://mainnet-sequencer.base.org", "explorer_url": "https://basescan.org",
         "assets": [{"symbol":"ETH","decimals":18,"contract":null},{"symbol":"USDC","decimals":6,"contract":BASE_USDC_CONTRACT},{"symbol":"EURC","decimals":6,"contract":BASE_EURC_CONTRACT}]},
        {"chain_id": OP_CHAIN_ID, "network": "OP Mainnet", "rpc_url": "https://optimism-rpc.publicnode.com", "broadcast_url": "https://mainnet-sequencer.optimism.io", "explorer_url": "https://optimistic.etherscan.io",
         "assets": [{"symbol":"ETH","decimals":18,"contract":null},{"symbol":"USDC","decimals":6,"contract":OP_USDC_CONTRACT}]},
        {"chain_id": ARBITRUM_CHAIN_ID, "network": "Arbitrum One", "rpc_url": "https://arbitrum-one-rpc.publicnode.com", "broadcast_url": "https://arb1-sequencer.arbitrum.io/rpc", "explorer_url": "https://arbiscan.io",
         "assets": [{"symbol":"ETH","decimals":18,"contract":null},{"symbol":"USDC","decimals":6,"contract":ARBITRUM_USDC_CONTRACT}]},
        {"chain_id": AVALANCHE_CHAIN_ID, "network": "Avalanche C-Chain", "rpc_url": "https://api.avax.network/ext/bc/C/rpc", "broadcast_url": "https://api.avax.network/ext/bc/C/rpc", "explorer_url": "https://snowtrace.io",
         "assets": [{"symbol":"AVAX","decimals":18,"contract":null},{"symbol":"USDC","decimals":6,"contract":AVALANCHE_USDC_CONTRACT},{"symbol":"EURC","decimals":6,"contract":AVALANCHE_EURC_CONTRACT},{"symbol":"USDT","decimals":6,"contract":AVALANCHE_USDT_CONTRACT}]},
        {"chain_id": POLYGON_CHAIN_ID, "network": "Polygon PoS", "rpc_url": "https://polygon-bor-rpc.publicnode.com", "broadcast_url": "https://polygon-bor-rpc.publicnode.com", "explorer_url": "https://polygonscan.com",
         "assets": [{"symbol":"POL","decimals":18,"contract":null},{"symbol":"USDC","decimals":6,"contract":POLYGON_USDC_CONTRACT}]},
        {"chain_id": BNB_CHAIN_ID, "network": "BNB Smart Chain", "rpc_url": "https://bsc-dataseed.bnbchain.org", "broadcast_url": "https://bsc-dataseed.bnbchain.org", "explorer_url": "https://bscscan.com",
         "assets": [{"symbol":"BNB","decimals":18,"contract":null}]}
    ]})
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> PaymentRequest {
        PaymentRequest {
            intent_id: "public-fixture".into(),
            chain_id: CHAIN_ID,
            from: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".into(),
            to: "0x70997970C51812dc3A010C7d01b50e0d17dc79C8".into(),
            asset: Asset::Eth,
            amount: "0.01".into(),
            nonce: 0,
            gas_limit: 21_000,
            max_fee_per_gas: "2000000000".into(),
            max_priority_fee_per_gas: "1000000000".into(),
            l1_fee_estimate_wei: "0".into(),
            operator_fee_estimate_wei: "0".into(),
        }
    }

    #[test]
    fn review_expiration_boundaries_and_history_validation() {
        let prepared = prepare_at(request(), 1_000).unwrap();
        assert!(ensure_fresh(&prepared, 999).is_err());
        assert!(ensure_fresh(&prepared, 1_000).is_ok());
        assert!(ensure_fresh(&prepared, 1_299).is_ok());
        assert!(ensure_fresh(&prepared, 1_300).is_err());
        assert!(validate_prepared(&json_output(&prepared).unwrap()).is_ok());
        assert!(prepare_at(request(), u64::MAX - 100).is_err());
        assert!(prepare_at(request(), 0).is_err());
    }

    #[test]
    fn signing_checks_review_expiration_before_opening_the_vault() {
        let now = current_time().unwrap();
        for created_at in [now - APPROVAL_LIFETIME_SECONDS - 1, now + 60] {
            let prepared = prepare_at(request(), created_at).unwrap();
            let result = sign_payment(
                "deliberately invalid vault".into(),
                vec![0x42; 32],
                json_output(&prepared).unwrap(),
                prepared.approval_digest,
            );
            assert!(matches!(result, Err(WalletError::Policy { .. })));
        }
    }

    #[test]
    fn public_fixture_signature_rejects_unsupported_semantics() {
        let phrase = Mnemonic::parse_in_normalized(
            Language::English,
            "test test test test test test test test test test test junk",
        )
        .unwrap();
        let key = derive_key(&phrase).unwrap();
        let (_, original, _) = normalized_request(request()).unwrap();
        let sign = |transaction: &Transaction| {
            let unsigned = encode_unsigned(transaction);
            let (signature, recovery) = key.sign_prehash_recoverable(&Keccak256::digest(unsigned));
            encode_signed(transaction, &signature, recovery)
        };
        let mut transaction = original.clone();
        transaction.chain_id = 11_155_111;
        assert!(inspect_bytes(&sign(&transaction)).is_err());
        transaction = original.clone();
        transaction.data = vec![0x09, 0x5e, 0xa7, 0xb3]; // approve(), not transfer().
        assert!(inspect_bytes(&sign(&transaction)).is_err());
        transaction = original.clone();
        transaction.value = U256::zero();
        assert!(inspect_bytes(&sign(&transaction)).is_err());
        transaction = original.clone();
        transaction.max_fee_per_gas = U256::from(MAX_FEE_PER_GAS_WEI + 1);
        assert!(inspect_bytes(&sign(&transaction)).is_err());
        let mut raw = sign(&original);
        raw.push(0);
        assert!(inspect_bytes(&raw).is_err());
    }

    #[test]
    fn quote_changes_require_new_approval_even_when_unsigned_bytes_are_identical() {
        for chain in [BASE_CHAIN_ID, OP_CHAIN_ID] {
            let mut op_stack = request();
            op_stack.chain_id = chain;
            op_stack.l1_fee_estimate_wei = "1000".into();
            op_stack.operator_fee_estimate_wei = "100".into();
            let first = prepare_at(op_stack.clone(), 1_000).unwrap();
            op_stack.l1_fee_estimate_wei = "1001".into();
            let second = prepare_at(op_stack.clone(), 1_000).unwrap();
            op_stack.l1_fee_estimate_wei = "1000".into();
            op_stack.operator_fee_estimate_wei = "101".into();
            let third = prepare_at(op_stack, 1_000).unwrap();
            assert_eq!(first.unsigned_transaction, second.unsigned_transaction);
            assert_eq!(first.unsigned_transaction, third.unsigned_transaction);
            assert_ne!(first.approval_digest, second.approval_digest);
            assert_ne!(second.approval_digest, third.approval_digest);
        }
    }

    #[test]
    fn token_contract_cannot_be_reused_under_the_wrong_network_identity() {
        let phrase = Mnemonic::parse_in_normalized(
            Language::English,
            "test test test test test test test test test test test junk",
        )
        .unwrap();
        let key = derive_key(&phrase).unwrap();
        for asset in [Asset::Usdc, Asset::Eurc] {
            for chain in [CHAIN_ID, BASE_CHAIN_ID, OP_CHAIN_ID, ARBITRUM_CHAIN_ID] {
                if !asset.supported_on(chain) {
                    continue;
                }
                let mut request = request();
                request.asset = asset;
                request.chain_id = chain;
                request.gas_limit = 80_000;
                let (_, transaction, _) = normalized_request(request).unwrap();
                for other in [CHAIN_ID, BASE_CHAIN_ID, OP_CHAIN_ID, ARBITRUM_CHAIN_ID] {
                    if chain == other {
                        continue;
                    }
                    let mut changed = transaction.clone();
                    changed.chain_id = other;
                    let unsigned = encode_unsigned(&changed);
                    let (signature, recovery) =
                        key.sign_prehash_recoverable(&Keccak256::digest(unsigned));
                    assert!(inspect_bytes(&encode_signed(&changed, &signature, recovery)).is_err());
                }
            }
        }
    }

    #[test]
    fn arbitrary_or_bridged_usdc_contracts_never_become_signable_by_ticker() {
        let phrase = Mnemonic::parse_in_normalized(
            Language::English,
            "test test test test test test test test test test test junk",
        )
        .unwrap();
        let key = derive_key(&phrase).unwrap();
        // Known bridged representations are deliberate negative fixtures.
        for (chain, bridged) in [
            (OP_CHAIN_ID, "0x7f5c764cbc14f9669b88837ca1490cca17c31607"),
            (
                ARBITRUM_CHAIN_ID,
                "0xff970a61a04b1ca14834a43f5de4533ebddb5cc8",
            ),
        ] {
            let mut request = request();
            request.asset = Asset::Usdc;
            request.chain_id = chain;
            request.gas_limit = 80_000;
            let (_, mut transaction, _) = normalized_request(request).unwrap();
            for contract in [bridged, "0x1111111111111111111111111111111111111111"] {
                transaction.to = address_bytes(contract).unwrap();
                let unsigned = encode_unsigned(&transaction);
                let (signature, recovery) =
                    key.sign_prehash_recoverable(&Keccak256::digest(unsigned));
                assert!(inspect_bytes(&encode_signed(&transaction, &signature, recovery)).is_err());
            }
        }
    }

    #[test]
    fn inspection_rejects_high_s_noncanonical_integers_and_access_lists() {
        let (_, transaction, _) = normalized_request(request()).unwrap();
        let phrase = Mnemonic::parse_in_normalized(
            Language::English,
            "test test test test test test test test test test test junk",
        )
        .unwrap();
        let key = derive_key(&phrase).unwrap();
        let unsigned = encode_unsigned(&transaction);
        let (signature, recovery) = key.sign_prehash_recoverable(&Keccak256::digest(unsigned));
        let raw = encode_signed(&transaction, &signature, recovery);
        let encoded = Rlp::new(&raw[1..]);
        let mutate = |field: usize, replacement: &[u8]| {
            let mut stream = RlpStream::new_list(12);
            for i in 0..12 {
                if i == field {
                    stream.append_raw(replacement, 1);
                } else {
                    stream.append_raw(encoded.at(i).unwrap().as_raw(), 1);
                }
            }
            let mut bytes = vec![2];
            bytes.extend_from_slice(&stream.out());
            bytes
        };
        // A zero integer must be encoded as empty bytes, never the byte 0x00.
        assert!(inspect_bytes(&mutate(1, &[0x00])).is_err());
        assert!(inspect_bytes(&mutate(1, &[0xc0])).is_err());
        assert!(inspect_bytes(&mutate(8, &[0xc1, 0xc0])).is_err());
        assert!(inspect_bytes(&mutate(9, &[0x02])).is_err());
        assert!(inspect_bytes(&mutate(10, &[0x80])).is_err());
        let curve_order = U256::from_big_endian(
            &hex::decode("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141")
                .unwrap(),
        );
        let low_s = U256::from_big_endian(&signature.to_bytes()[32..]);
        let mut encoded_high_s = RlpStream::new();
        append_integer(&mut encoded_high_s, curve_order - low_s);
        assert!(inspect_bytes(&mutate(11, &encoded_high_s.out())).is_err());
    }
}
