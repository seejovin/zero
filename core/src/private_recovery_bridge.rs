//! Native recovery primitives for the trusted local Android coordinator.
//! Neither server-supplied checkpoints nor raw remote scan JSON establish
//! freshness or canonical chain evidence. No recovery method broadcasts funds.
use crate::{
    private_bridge::PrivatePaymentJournal, private_payment, private_recovery, WalletError,
};
use std::sync::Arc;
use zeroize::Zeroizing;

#[derive(uniffi::Record)]
pub struct PrivateRecoveryBackup {
    pub encrypted_bytes: Vec<u8>,
    pub checkpoint_json: String,
}

#[uniffi::export]
pub fn private_export_recovery_backup(
    journal: Arc<PrivatePaymentJournal>,
    vault_hex: String,
    vault_key: Vec<u8>,
) -> Result<PrivateRecoveryBackup, WalletError> {
    let backup = private_recovery::export_backup(&journal.inner, vault_hex, vault_key)?;
    Ok(PrivateRecoveryBackup {
        encrypted_bytes: backup.encrypted_bytes,
        checkpoint_json: backup.checkpoint_json,
    })
}

#[uniffi::export]
pub fn private_inspect_recovery_backup(
    encrypted_bytes: Vec<u8>,
    vault_hex: String,
    vault_key: Vec<u8>,
) -> Result<String, WalletError> {
    private_recovery::inspect_backup(encrypted_bytes, vault_hex, vault_key)
}

#[uniffi::export]
pub fn private_import_recovery_backup(
    encrypted_bytes: Vec<u8>,
    vault_hex: String,
    vault_key: Vec<u8>,
    new_path: String,
    expected_identity: String,
    new_device_key: Vec<u8>,
) -> Result<Arc<PrivatePaymentJournal>, WalletError> {
    Ok(Arc::new(PrivatePaymentJournal {
        inner: private_recovery::import_backup(
            encrypted_bytes,
            vault_hex,
            vault_key,
            new_path,
            expected_identity,
            new_device_key,
        )?,
    }))
}

#[uniffi::export]
pub fn private_create_recovery_journal(
    new_path: String,
    identity: String,
    new_device_key: Vec<u8>,
    vault_hex: String,
    vault_key: Vec<u8>,
) -> Result<Arc<PrivatePaymentJournal>, WalletError> {
    Ok(Arc::new(PrivatePaymentJournal {
        inner: private_recovery::create_quarantined_journal(
            new_path,
            identity,
            new_device_key,
            vault_hex,
            vault_key,
        )?,
    }))
}

#[uniffi::export]
pub fn private_recovery_status(journal: Arc<PrivatePaymentJournal>) -> Result<String, WalletError> {
    journal.inner.recovery_status()
}

/// Required after opening a journal for an unlocked wallet. Returns recovery
/// status only after authenticating the vault and matching its seed-derived
/// binding. It has no backup-export or journal mutation side effects.
#[uniffi::export]
pub fn private_authenticate_journal(
    journal: Arc<PrivatePaymentJournal>,
    vault_hex: String,
    vault_key: Vec<u8>,
) -> Result<String, WalletError> {
    private_recovery::authenticate_journal(&journal.inner, vault_hex, vault_key)
}

#[uniffi::export]
pub fn private_recovery_workspace(
    journal: Arc<PrivatePaymentJournal>,
) -> Result<String, WalletError> {
    journal.inner.recovery_workspace()
}

/// The coordinator must obtain this checkpoint from independently retained
/// trusted local storage, never from the backup being tested or a remote API.
#[uniffi::export]
pub fn private_accept_recovery_checkpoint(
    journal: Arc<PrivatePaymentJournal>,
    recovery_id: String,
    checkpoint_json: String,
) -> Result<String, WalletError> {
    journal
        .inner
        .accept_recovery_checkpoint(recovery_id, checkpoint_json)
}

#[uniffi::export]
pub fn private_prepare_recovery_scan(scan_json: String) -> Result<String, WalletError> {
    let text = Zeroizing::new(scan_json);
    private_recovery::seal_recovery_scan_json(&text)
}

#[uniffi::export]
pub fn private_record_recovery_cutoff(
    journal: Arc<PrivatePaymentJournal>,
    recovery_id: String,
    cutoff_json: String,
) -> Result<String, WalletError> {
    journal
        .inner
        .record_recovery_cutoff(recovery_id, cutoff_json)
}

#[uniffi::export]
pub fn private_prepare_recovery_sweep(plan_json: String) -> Result<String, WalletError> {
    let text = Zeroizing::new(plan_json);
    private_recovery::seal_recovery_plan_json(&text)
}

#[uniffi::export]
pub fn private_approve_recovery_sweep(
    journal: Arc<PrivatePaymentJournal>,
    plan_json: String,
) -> Result<String, WalletError> {
    journal.inner.approve_recovery_sweep(plan_json)
}

#[uniffi::export]
pub fn private_record_recovery_sweep_batch(
    journal: Arc<PrivatePaymentJournal>,
    recovery_id: String,
    intent_json: String,
) -> Result<String, WalletError> {
    journal
        .inner
        .record_recovery_sweep_batch(recovery_id, intent_json)
}

#[uniffi::export]
pub fn private_complete_recovery_sweep(
    journal: Arc<PrivatePaymentJournal>,
    resolution_json: String,
) -> Result<String, WalletError> {
    journal.inner.complete_recovery_sweep(resolution_json)
}

#[uniffi::export]
pub fn private_prepare_transfer_approval(approval_json: String) -> Result<String, WalletError> {
    let text = Zeroizing::new(approval_json);
    private_payment::seal_transfer_approval_json(&text)
}

#[uniffi::export]
pub fn private_validate_transfer_approval_binding(
    approval_json: String,
    intent_json: String,
) -> Result<String, WalletError> {
    let approval_json = Zeroizing::new(approval_json);
    let intent_json = Zeroizing::new(intent_json);
    let approval: private_payment::PrivateTransferApproval =
        private_payment::decode(&approval_json)?;
    let intent = private_payment::validate_intent_json(&intent_json)?;
    private_payment::validate_intent_for_approval(&approval, &intent)?;
    private_payment::encode(&intent)
}
