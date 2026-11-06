//! WebAssembly bridge. No RPC, DOM, network, or persistence is inside this
//! module. The browser worker owns the transient key and clears JS buffers.
use wasm_bindgen::prelude::*;

fn error(error: crate::WalletError) -> JsValue {
    JsValue::from_str(&error.to_string())
}

#[wasm_bindgen]
pub fn generate_wallet(vault_key: Vec<u8>) -> Result<String, JsValue> {
    crate::generate_wallet(vault_key).map_err(error)
}

#[wasm_bindgen]
pub fn restore_wallet(mnemonic: String, vault_key: Vec<u8>) -> Result<String, JsValue> {
    crate::restore_wallet(mnemonic, vault_key).map_err(error)
}

#[wasm_bindgen]
pub fn reveal_mnemonic(vault_hex: String, vault_key: Vec<u8>) -> Result<String, JsValue> {
    crate::reveal_mnemonic(vault_hex, vault_key).map_err(error)
}

#[wasm_bindgen]
pub fn wallet_address(vault_hex: String, vault_key: Vec<u8>) -> Result<String, JsValue> {
    crate::wallet_address(vault_hex, vault_key).map_err(error)
}

#[wasm_bindgen]
pub fn derive_vault_key(password: String, salt: Vec<u8>) -> Result<Vec<u8>, JsValue> {
    crate::derive_vault_key(password, salt).map_err(error)
}

#[wasm_bindgen]
pub fn prepare_payment(request_json: String) -> Result<String, JsValue> {
    crate::prepare_payment(request_json).map_err(error)
}

#[wasm_bindgen]
pub fn sign_payment(
    vault_hex: String,
    vault_key: Vec<u8>,
    prepared_json: String,
    approval_digest: String,
) -> Result<String, JsValue> {
    crate::sign_payment(vault_hex, vault_key, prepared_json, approval_digest).map_err(error)
}

#[wasm_bindgen]
pub fn inspect_signed(raw_transaction: String) -> Result<String, JsValue> {
    crate::inspect_signed(raw_transaction).map_err(error)
}

#[wasm_bindgen]
pub fn asset_registry() -> String {
    crate::asset_registry()
}

#[wasm_bindgen]
pub fn format_amount(units: String, decimals: u8) -> Result<String, JsValue> {
    crate::format_amount(units, decimals).map_err(error)
}

#[wasm_bindgen]
pub fn validate_prepared_json(prepared_json: String) -> Result<String, JsValue> {
    crate::validate_prepared_json(prepared_json).map_err(error)
}

#[wasm_bindgen]
pub fn validate_signed_for_prepared(
    prepared_json: String,
    signed_json: String,
) -> Result<String, JsValue> {
    crate::validate_signed_for_prepared(prepared_json, signed_json).map_err(error)
}

#[wasm_bindgen]
pub fn validate_receipt(
    signed_json: String,
    receipt_json: String,
    canonical_block_hash: String,
    finalized_height: u64,
) -> Result<String, JsValue> {
    crate::validate_receipt(
        signed_json,
        receipt_json,
        canonical_block_hash,
        finalized_height,
    )
    .map_err(error)
}

#[wasm_bindgen]
pub fn validate_activity(
    chain_id: u64,
    address: String,
    transaction_json: String,
    receipt_json: String,
    canonical_block_json: String,
    finalized_height: u64,
) -> Result<String, JsValue> {
    crate::validate_activity(
        chain_id,
        address,
        transaction_json,
        receipt_json,
        canonical_block_json,
        finalized_height,
    )
    .map_err(error)
}
