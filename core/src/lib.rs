//! Local Wallet's restricted signing boundary and durable transaction journal.
//!
//! No API signs arbitrary messages, hashes, contracts, or caller-supplied calldata.
//! Accepts reviewed EVM networks, Solana and TRON from fixed registries.
//! This crate is not an audited product.

#![forbid(unsafe_code)]

pub mod activity;
pub mod crypto;
#[cfg(feature = "native")]
pub mod family_journal;
#[cfg(feature = "native")]
pub mod journal;
#[cfg(feature = "native")]
pub mod private_bridge;
pub mod private_engine_keys;
pub mod private_fee_schedule;
#[cfg(feature = "native")]
pub mod private_journal;
pub mod private_payment;
pub mod private_pool;
#[cfg(feature = "native")]
pub mod private_recovery;
#[cfg(feature = "native")]
pub mod private_recovery_bridge;
pub mod solana;
pub mod tron;
pub mod validation;
#[cfg(feature = "wasm")]
mod wasm;

#[cfg(feature = "native")]
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "native", derive(uniffi::Error))]
pub enum WalletError {
    #[error("Invalid input: {reason}")]
    InvalidInput { reason: String },
    #[error("Operation is outside wallet policy: {reason}")]
    Policy { reason: String },
    #[error("Cryptographic operation failed")]
    Crypto,
    #[error("Storage operation failed: {reason}")]
    Storage { reason: String },
    #[error("Transaction conflict: {reason}")]
    Conflict { reason: String },
}

#[cfg_attr(feature = "native", uniffi::export)]
pub fn generate_wallet(vault_key: Vec<u8>) -> Result<String, WalletError> {
    crypto::generate_wallet(vault_key)
}

#[cfg_attr(feature = "native", uniffi::export)]
pub fn restore_wallet(mnemonic: String, vault_key: Vec<u8>) -> Result<String, WalletError> {
    crypto::restore_wallet(mnemonic, vault_key)
}

#[cfg_attr(feature = "native", uniffi::export)]
pub fn reveal_mnemonic(vault_hex: String, vault_key: Vec<u8>) -> Result<String, WalletError> {
    crypto::reveal_mnemonic(vault_hex, vault_key)
}

/// Authenticate the vault and derive its public address without exporting words.
#[cfg_attr(feature = "native", uniffi::export)]
pub fn wallet_address(vault_hex: String, vault_key: Vec<u8>) -> Result<String, WalletError> {
    crypto::wallet_address(vault_hex, vault_key)
}

#[cfg_attr(feature = "native", uniffi::export)]
pub fn prepare_payment(request_json: String) -> Result<String, WalletError> {
    crypto::prepare_payment(request_json)
}

#[cfg_attr(feature = "native", uniffi::export)]
pub fn sign_payment(
    vault_hex: String,
    vault_key: Vec<u8>,
    prepared_json: String,
    approval_digest: String,
) -> Result<String, WalletError> {
    crypto::sign_payment(vault_hex, vault_key, prepared_json, approval_digest)
}

#[cfg_attr(feature = "native", uniffi::export)]
pub fn inspect_signed(raw_transaction: String) -> Result<String, WalletError> {
    crypto::inspect_signed(raw_transaction)
}

#[cfg_attr(feature = "native", uniffi::export)]
pub fn asset_registry() -> String {
    crypto::asset_registry()
}

#[cfg_attr(feature = "native", uniffi::export)]
pub fn format_amount(units: String, decimals: u8) -> Result<String, WalletError> {
    crypto::format_amount(units, decimals)
}

/// The database contains public transaction information and signed transactions,
/// never a recovery phrase or an unencrypted private key. Signed bytes are still
/// sensitive: anyone who obtains them can broadcast that exact payment.
#[cfg(feature = "native")]
#[derive(uniffi::Object)]
pub struct WalletJournal {
    inner: journal::Journal,
}

#[cfg(feature = "native")]
#[uniffi::export]
impl WalletJournal {
    #[uniffi::constructor]
    pub fn new(path: String) -> Result<Arc<Self>, WalletError> {
        Ok(Arc::new(Self {
            inner: journal::Journal::open(path)?,
        }))
    }

    pub fn record_prepared(&self, prepared_json: String) -> Result<String, WalletError> {
        self.inner.record_prepared(prepared_json)
    }

    pub fn record_signed(&self, signed_json: String) -> Result<String, WalletError> {
        self.inner.record_signed(signed_json)
    }

    pub fn mark_submission(
        &self,
        intent_id: String,
        outcome: String,
    ) -> Result<String, WalletError> {
        self.inner.mark_submission(intent_id, outcome)
    }

    pub fn observe_receipt(
        &self,
        intent_id: String,
        receipt_json: String,
        canonical_block_hash: String,
        finalized_height: u64,
    ) -> Result<String, WalletError> {
        self.inner.observe_receipt(
            intent_id,
            receipt_json,
            canonical_block_hash,
            finalized_height,
        )
    }

    pub fn mark_reorg(
        &self,
        intent_id: String,
        previous_block_hash: String,
    ) -> Result<String, WalletError> {
        self.inner.mark_reorg(intent_id, previous_block_hash)
    }

    pub fn list_payments(&self) -> Result<String, WalletError> {
        self.inner.list_payments()
    }

    pub fn get_payment(&self, intent_id: String) -> Result<String, WalletError> {
        self.inner.get_payment(intent_id)
    }

    pub fn list_events(&self, intent_id: String) -> Result<String, WalletError> {
        self.inner.list_events(intent_id)
    }

    pub fn record_balance(&self, snapshot_json: String) -> Result<String, WalletError> {
        self.inner.record_balance(snapshot_json)
    }

    pub fn latest_balance(&self) -> Result<String, WalletError> {
        self.inner.latest_balance()
    }

    pub fn cancel_prepared(&self, intent_id: String) -> Result<String, WalletError> {
        self.inner.cancel_prepared(intent_id)
    }
}

#[cfg(feature = "native")]
uniffi::setup_scaffolding!();

#[cfg_attr(feature = "native", uniffi::export)]
pub fn derive_vault_key(password: String, salt: Vec<u8>) -> Result<Vec<u8>, WalletError> {
    crypto::derive_vault_key(password, salt)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn validate_prepared_json(prepared_json: String) -> Result<String, WalletError> {
    validation::validate_prepared_json(prepared_json)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn validate_signed_for_prepared(
    prepared_json: String,
    signed_json: String,
) -> Result<String, WalletError> {
    validation::validate_signed_for_prepared(prepared_json, signed_json)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn validate_receipt(
    signed_json: String,
    receipt_json: String,
    canonical_block_hash: String,
    finalized_height: u64,
) -> Result<String, WalletError> {
    validation::validate_receipt(
        signed_json,
        receipt_json,
        canonical_block_hash,
        finalized_height,
    )
}

#[cfg_attr(feature = "native", uniffi::export)]
pub fn validate_activity(
    chain_id: u64,
    address: String,
    transaction_json: String,
    receipt_json: String,
    canonical_block_json: String,
    finalized_height: u64,
) -> Result<String, WalletError> {
    activity::validate_activity(
        chain_id,
        address,
        transaction_json,
        receipt_json,
        canonical_block_json,
        finalized_height,
    )
}

fn public_json<T: serde::Serialize>(value: T) -> Result<String, WalletError> {
    serde_json::to_string(&value).map_err(|_| WalletError::Crypto)
}

#[cfg_attr(feature = "native", uniffi::export)]
pub fn solana_address(vault_hex: String, vault_key: Vec<u8>) -> Result<String, WalletError> {
    let key = zeroize::Zeroizing::new(vault_key);
    solana::address(&vault_hex, &key)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn solana_validate_address(address: String) -> Result<String, WalletError> {
    solana::validate_address(&address)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn solana_token_account(owner: String, symbol: String) -> Result<String, WalletError> {
    solana::token_account(&owner, &symbol)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn solana_prepare(request_json: String) -> Result<String, WalletError> {
    public_json(solana::prepare(&request_json)?)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn solana_validate_prepared(prepared_json: String) -> Result<String, WalletError> {
    public_json(solana::validate_prepared(&prepared_json)?)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn solana_sign(
    vault_hex: String,
    vault_key: Vec<u8>,
    prepared_json: String,
    approval_digest: String,
) -> Result<String, WalletError> {
    let key = zeroize::Zeroizing::new(vault_key);
    public_json(solana::sign(
        &vault_hex,
        &key,
        &prepared_json,
        &approval_digest,
    )?)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn solana_validate_signed(
    prepared_json: String,
    signed_json: String,
) -> Result<String, WalletError> {
    public_json(solana::validate_signed(&prepared_json, &signed_json)?)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn solana_validate_receipt(
    prepared_json: String,
    signed_json: String,
    evidence_json: String,
) -> Result<String, WalletError> {
    public_json(solana::validate_receipt(
        &prepared_json,
        &signed_json,
        &evidence_json,
    )?)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn solana_validate_activity(
    address: String,
    evidence_json: String,
) -> Result<String, WalletError> {
    public_json(solana::validate_activity(&address, &evidence_json)?)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn solana_check_expiry(evidence_json: String) -> Result<String, WalletError> {
    public_json(solana::check_expiry(&evidence_json)?)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn tron_address(vault_hex: String, vault_key: Vec<u8>) -> Result<String, WalletError> {
    tron::wallet_address(vault_hex, vault_key)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn tron_normalize_address(address: String) -> Result<String, WalletError> {
    tron::normalize_address(&address)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn tron_address_hex(address: String) -> Result<String, WalletError> {
    tron::address_hex(&address)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn tron_prepare(request_json: String) -> Result<String, WalletError> {
    public_json(tron::prepare(&request_json)?)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn tron_validate_prepared(prepared_json: String) -> Result<String, WalletError> {
    public_json(tron::validate_prepared(&prepared_json)?)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn tron_sign(
    vault_hex: String,
    vault_key: Vec<u8>,
    prepared_json: String,
    approval_digest: String,
) -> Result<String, WalletError> {
    public_json(tron::sign(
        vault_hex,
        vault_key,
        prepared_json,
        approval_digest,
    )?)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn tron_validate_signed(signed_json: String) -> Result<String, WalletError> {
    public_json(tron::validate_signed(&signed_json)?)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn tron_validate_receipt(
    prepared_json: String,
    signed_json: String,
    evidence_json: String,
) -> Result<String, WalletError> {
    public_json(tron::validate_receipt(
        &prepared_json,
        &signed_json,
        &evidence_json,
    )?)
}
#[cfg_attr(feature = "native", uniffi::export)]
pub fn tron_validate_activity(
    address: String,
    evidence_json: String,
) -> Result<String, WalletError> {
    public_json(tron::validate_activity(&address, &evidence_json)?)
}

#[cfg(feature = "native")]
#[derive(uniffi::Object)]
pub struct FamilyJournal {
    inner: family_journal::Journal,
}

#[cfg(feature = "native")]
#[uniffi::export]
impl FamilyJournal {
    #[uniffi::constructor]
    pub fn new(path: String, network: String, address: String) -> Result<Arc<Self>, WalletError> {
        Ok(Arc::new(Self {
            inner: family_journal::Journal::open(path, network, address)?,
        }))
    }
    pub fn record_prepared(&self, prepared_json: String) -> Result<String, WalletError> {
        self.inner.record_prepared(prepared_json)
    }
    pub fn record_signed(
        &self,
        intent_id: String,
        signed_json: String,
    ) -> Result<String, WalletError> {
        self.inner.record_signed(intent_id, signed_json)
    }
    pub fn mark_submission(
        &self,
        intent_id: String,
        outcome: String,
    ) -> Result<String, WalletError> {
        self.inner.mark_submission(intent_id, outcome)
    }
    pub fn observe_receipt(
        &self,
        intent_id: String,
        evidence_json: String,
    ) -> Result<String, WalletError> {
        self.inner.observe_receipt(intent_id, evidence_json)
    }
    pub fn cancel_prepared(&self, intent_id: String) -> Result<String, WalletError> {
        self.inner.cancel_prepared(intent_id)
    }
    pub fn list_payments(&self) -> Result<String, WalletError> {
        self.inner.list_payments()
    }
    pub fn get_payment(&self, intent_id: String) -> Result<String, WalletError> {
        self.inner.get_payment(intent_id)
    }
    pub fn list_events(&self, intent_id: String) -> Result<String, WalletError> {
        self.inner.list_events(intent_id)
    }
    pub fn record_balance(&self, snapshot_json: String) -> Result<String, WalletError> {
        self.inner.record_balance(snapshot_json)
    }
    pub fn latest_balance(&self) -> Result<String, WalletError> {
        self.inner.latest_balance()
    }
    pub fn track_transaction(&self, hash: String) -> Result<(), WalletError> {
        self.inner.track_transaction(hash)
    }
    pub fn record_activity(
        &self,
        hash: String,
        evidence_json: String,
    ) -> Result<String, WalletError> {
        self.inner.record_activity(hash, evidence_json)
    }
    pub fn list_activity(&self) -> Result<String, WalletError> {
        self.inner.list_activity()
    }
    pub fn list_tracked(&self) -> Result<String, WalletError> {
        self.inner.list_tracked()
    }
}
