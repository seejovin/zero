//! Local private-payment recovery. Backups contain authenticated encrypted
//! history, never a mnemonic or plaintext recipient/principal. Encryption keys
//! are domain-separated from the authenticated BIP39 seed; a replacement device
//! can restore the same vault under a different Android Keystore key.
//!
//! Authentication is not freshness. An older valid backup cannot reveal events
//! omitted after it was made. Import therefore enters persistent quarantine.
//! A trusted local coordinator may present an independently retained latest
//! checkpoint, or explicitly authorize a recovery self-sweep. An unspent-note
//! scan alone never revokes a previously released proof.

use crate::private_journal::{Journal, RecoverySnapshot};
use crate::private_payment::{self as payment, PrivateDomain, PrivateInputNote, PrivateIntent};
use crate::{crypto, validation, WalletError};
use aes_gcm::{
    aead::{AeadInPlace, KeyInit},
    Aes256Gcm, Nonce,
};
use hkdf::Hkdf;
use primitive_types::U256;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"LWPRB001";
const HEADER_BYTES: usize = 8 + 32 + 12 + 8;
const BACKUP_CONTEXT: &[u8] = b"LocalWallet/private-history-backup/v1\0";
const BINDING_CONTEXT: &[u8] = b"LocalWallet/private-recovery-wallet-binding/v1\0";
pub const MAX_BACKUP_BYTES: usize = 70 * 1024 * 1024;
pub const MAX_RECOVERY_NOTES: usize = 1024;

fn invalid(reason: &str) -> WalletError {
    WalletError::InvalidInput {
        reason: reason.into(),
    }
}
fn blocked(reason: &str) -> WalletError {
    WalletError::Policy {
        reason: reason.into(),
    }
}
fn current_time() -> Result<u64, WalletError> {
    let value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| WalletError::Crypto)?
        .as_secs();
    if value == 0 || value > i64::MAX as u64 {
        return Err(invalid("Invalid recovery device time"));
    }
    Ok(value)
}
fn digest_bytes(domain: &[u8], bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    format!("0x{}", hex::encode(hash.finalize()))
}
fn typed_digest<T: Serialize>(domain: &[u8], value: &T) -> Result<String, WalletError> {
    let bytes = Zeroizing::new(serde_json::to_vec(value).map_err(|_| WalletError::Crypto)?);
    if bytes.len() > payment::MAX_JSON_BYTES {
        return Err(invalid("Recovery plan is too large"));
    }
    Ok(digest_bytes(domain, &bytes))
}
fn seed_binding(seed: &[u8]) -> Result<String, WalletError> {
    let mut binding = Zeroizing::new([0u8; 32]);
    Hkdf::<sha2_modern::Sha256>::new(Some(BINDING_CONTEXT), seed)
        .expand(b"wallet identity", &mut *binding)
        .map_err(|_| WalletError::Crypto)?;
    Ok(format!("0x{}", hex::encode(*binding)))
}
fn backup_key(seed: &[u8], salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, WalletError> {
    let mut key = Zeroizing::new([0u8; 32]);
    Hkdf::<sha2_modern::Sha256>::new(Some(salt), seed)
        .expand(BACKUP_CONTEXT, &mut *key)
        .map_err(|_| WalletError::Crypto)?;
    Ok(key)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryCheckpoint {
    pub schema_version: u8,
    pub journal_identity: String,
    pub wallet_binding: String,
    pub source_sequence: u64,
    pub source_tip: String,
    pub backup_digest: String,
}
impl RecoveryCheckpoint {
    pub(crate) fn validate(&self) -> Result<(), WalletError> {
        if self.schema_version != 1 || self.source_sequence > 50_000 {
            return Err(invalid("Unsupported recovery checkpoint"));
        }
        for value in [
            &self.journal_identity,
            &self.wallet_binding,
            &self.backup_digest,
        ] {
            payment::canonical_hash(value)?;
        }
        if self.source_sequence == 0 && self.source_tip == format!("0x{}", "00".repeat(32)) {
            return Ok(());
        }
        payment::canonical_hash(&self.source_tip)
    }
}

pub struct RecoveryBackup {
    pub encrypted_bytes: Vec<u8>,
    pub checkpoint_json: String,
}
impl std::fmt::Debug for RecoveryBackup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecoveryBackup([ENCRYPTED])")
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupDocument {
    schema_version: u8,
    wallet_binding: String,
    snapshot: RecoverySnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryReason {
    RestoredBackup,
    MissingHistory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverySession {
    pub schema_version: u8,
    pub recovery_id: String,
    pub wallet_binding: String,
    pub journal_identity: String,
    pub reason: RecoveryReason,
    pub checkpoint: Option<RecoveryCheckpoint>,
    pub started_at: u64,
}

/// Latest observed canonical head captured durably by the trusted local chain
/// adapter before recovery scanning begins. It may initially be unfinalized.
/// A later plan must prove complete scan coverage and finality through it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryCutoff {
    pub schema_version: u8,
    pub recovery_id: String,
    pub domain: PrivateDomain,
    pub block_number: u64,
    pub block_hash: String,
    pub observed_at: u64,
}
impl RecoveryCutoff {
    pub(crate) fn validate(&self) -> Result<(), WalletError> {
        payment::canonical_hash(&self.recovery_id)?;
        payment::canonical_hash(&self.block_hash)?;
        payment::validate_domain(&self.domain)?;
        if self.schema_version != 1
            || self.block_number == 0
            || self.block_number > i64::MAX as u64
            || self.observed_at == 0
            || self.observed_at > i64::MAX as u64
        {
            return Err(invalid("Invalid recovery start cutoff"));
        }
        Ok(())
    }
}
impl RecoverySession {
    pub(crate) fn validate(&self, identity: &str) -> Result<(), WalletError> {
        if self.schema_version != 1
            || self.journal_identity != identity
            || self.started_at == 0
            || self.started_at > i64::MAX as u64
        {
            return Err(invalid(
                "Recovery identity or version does not match the local journal",
            ));
        }
        for hash in [
            &self.recovery_id,
            &self.wallet_binding,
            &self.journal_identity,
        ] {
            payment::canonical_hash(hash)?;
        }
        match (&self.reason, &self.checkpoint) {
            (RecoveryReason::MissingHistory, None) => Ok(()),
            (RecoveryReason::RestoredBackup, Some(c))
                if c.wallet_binding == self.wallet_binding
                    && c.journal_identity == self.journal_identity =>
            {
                c.validate()
            }
            _ => Err(invalid("Recovery origin and checkpoint are inconsistent")),
        }
    }
}

pub(crate) fn session(
    identity: String,
    wallet_binding: String,
    checkpoint: Option<RecoveryCheckpoint>,
) -> Result<RecoverySession, WalletError> {
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|_| WalletError::Crypto)?;
    let session = RecoverySession {
        schema_version: 1,
        recovery_id: format!("0x{}", hex::encode(random)),
        wallet_binding,
        journal_identity: identity.clone(),
        reason: if checkpoint.is_some() {
            RecoveryReason::RestoredBackup
        } else {
            RecoveryReason::MissingHistory
        },
        checkpoint,
        started_at: current_time()?,
    };
    session.validate(&identity)?;
    Ok(session)
}

/// Only the local authenticated vault is used. The returned opaque bytes are
/// suitable for a user-controlled backup destination, never company telemetry.
/// Retain the checkpoint independently and update it whenever a newer export is
/// retained. A checkpoint copied from the same backup is not proof of freshness.
pub fn export_backup(
    journal: &Journal,
    vault_hex: String,
    vault_key: Vec<u8>,
) -> Result<RecoveryBackup, WalletError> {
    let vault_key = Zeroizing::new(vault_key);
    let seed = crypto::open_seed(&vault_hex, &vault_key)?;
    let wallet_binding = seed_binding(&*seed)?;
    let snapshot = journal.recovery_snapshot(&wallet_binding)?;
    let mut checkpoint = RecoveryCheckpoint {
        schema_version: 1,
        journal_identity: snapshot.identity.clone(),
        wallet_binding: wallet_binding.clone(),
        source_sequence: snapshot.source_sequence,
        source_tip: snapshot.source_tip.clone(),
        backup_digest: String::new(),
    };
    let document = BackupDocument {
        schema_version: 1,
        wallet_binding,
        snapshot,
    };
    let mut ciphertext =
        Zeroizing::new(serde_json::to_vec(&document).map_err(|_| WalletError::Crypto)?);
    if ciphertext.len() > MAX_BACKUP_BYTES - HEADER_BYTES - 16 {
        return Err(invalid("Recovery history exceeds the backup size limit"));
    }
    let mut salt = [0u8; 32];
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut salt).map_err(|_| WalletError::Crypto)?;
    getrandom::fill(&mut nonce).map_err(|_| WalletError::Crypto)?;
    let key = backup_key(&*seed, &salt)?;
    let mut header = Vec::from(MAGIC.as_slice());
    header.extend_from_slice(&salt);
    header.extend_from_slice(&nonce);
    header.extend_from_slice(&((ciphertext.len() + 16) as u64).to_be_bytes());
    let mut aad = Vec::from(BACKUP_CONTEXT);
    aad.extend_from_slice(&header);
    Aes256Gcm::new_from_slice(&*key)
        .map_err(|_| WalletError::Crypto)?
        .encrypt_in_place(Nonce::from_slice(&nonce), &aad, &mut *ciphertext)
        .map_err(|_| WalletError::Crypto)?;
    header.extend_from_slice(&ciphertext);
    checkpoint.backup_digest = digest_bytes(b"LocalWallet/private-backup-file/v1\0", &header);
    Ok(RecoveryBackup {
        encrypted_bytes: header,
        checkpoint_json: payment::encode(&checkpoint)?,
    })
}

fn open_backup(
    backup: &[u8],
    vault_hex: &str,
    vault_key: &[u8],
) -> Result<(BackupDocument, RecoveryCheckpoint), WalletError> {
    if backup.len() < HEADER_BYTES + 16 || backup.len() > MAX_BACKUP_BYTES || &backup[..8] != MAGIC
    {
        return Err(invalid("Unsupported or excessive private recovery backup"));
    }
    let stated = u64::from_be_bytes(backup[52..60].try_into().map_err(|_| WalletError::Crypto)?);
    if stated != (backup.len() - HEADER_BYTES) as u64 {
        return Err(invalid("Recovery backup length is inconsistent"));
    }
    let seed = crypto::open_seed(vault_hex, vault_key)?;
    let binding = seed_binding(&*seed)?;
    let key = backup_key(&*seed, &backup[8..40])?;
    let mut plaintext = Zeroizing::new(backup[HEADER_BYTES..].to_vec());
    let mut aad = Vec::from(BACKUP_CONTEXT);
    aad.extend_from_slice(&backup[..HEADER_BYTES]);
    Aes256Gcm::new_from_slice(&*key)
        .map_err(|_| WalletError::Crypto)?
        .decrypt_in_place(Nonce::from_slice(&backup[40..52]), &aad, &mut *plaintext)
        .map_err(|_| WalletError::Crypto)?;
    let document: BackupDocument = serde_json::from_slice(&plaintext)
        .map_err(|_| invalid("Recovery backup document is malformed or ambiguous"))?;
    if document.schema_version != 1 || document.wallet_binding != binding {
        return Err(invalid(
            "Recovery backup belongs to another wallet or version",
        ));
    }
    document.snapshot.validate()?;
    document.snapshot.ensure_wallet_binding(&binding)?;
    let checkpoint = RecoveryCheckpoint {
        schema_version: 1,
        journal_identity: document.snapshot.identity.clone(),
        wallet_binding: binding,
        source_sequence: document.snapshot.source_sequence,
        source_tip: document.snapshot.source_tip.clone(),
        backup_digest: digest_bytes(b"LocalWallet/private-backup-file/v1\0", backup),
    };
    checkpoint.validate()?;
    Ok((document, checkpoint))
}

/// Decrypts only in native memory and returns local manifest identifiers. No
/// plaintext notes, recipients, principals, witnesses or seed are returned.
pub fn inspect_backup(
    backup: Vec<u8>,
    vault_hex: String,
    vault_key: Vec<u8>,
) -> Result<String, WalletError> {
    let key = Zeroizing::new(vault_key);
    let (_, checkpoint) = open_backup(&backup, &vault_hex, &key)?;
    payment::encode(&checkpoint)
}

/// Import always requires a new isolated path and preserves the original
/// journal identity. Activate the returned journal only after this succeeds;
/// failed imports never overwrite or truncate an existing journal.
pub fn import_backup(
    backup: Vec<u8>,
    vault_hex: String,
    vault_key: Vec<u8>,
    new_path: String,
    expected_identity: String,
    new_device_key: Vec<u8>,
) -> Result<Journal, WalletError> {
    let key = Zeroizing::new(vault_key);
    let device_key = Zeroizing::new(new_device_key);
    let (document, checkpoint) = open_backup(&backup, &vault_hex, &key)?;
    if checkpoint.journal_identity != expected_identity {
        return Err(invalid(
            "Recovery backup journal identity does not match the selected wallet",
        ));
    }
    let session = session(
        expected_identity.clone(),
        checkpoint.wallet_binding.clone(),
        Some(checkpoint),
    )?;
    Journal::from_recovery_snapshot(
        new_path,
        expected_identity,
        device_key.to_vec(),
        document.snapshot,
        session,
    )
}

/// Seed-only restoration provides key access but no proof of pending-history
/// completeness. The new journal remains quarantined until explicit recovery.
pub fn create_quarantined_journal(
    new_path: String,
    identity: String,
    new_device_key: Vec<u8>,
    vault_hex: String,
    vault_key: Vec<u8>,
) -> Result<Journal, WalletError> {
    let key = Zeroizing::new(vault_key);
    let seed = crypto::open_seed(&vault_hex, &key)?;
    let binding = seed_binding(&*seed)?;
    let session = session(identity.clone(), binding, None)?;
    Journal::from_recovery_snapshot(
        new_path,
        identity.clone(),
        new_device_key,
        RecoverySnapshot::empty(identity),
        session,
    )
}

/// Confirm that the current authenticated vault owns the journal's previously
/// established seed binding. This read does not create a binding, export a
/// proof, or imply that a recovery checkpoint is complete or current.
pub fn authenticate_journal(
    journal: &Journal,
    vault_hex: String,
    vault_key: Vec<u8>,
) -> Result<String, WalletError> {
    let key = Zeroizing::new(vault_key);
    let seed = crypto::open_seed(&vault_hex, &key)?;
    let binding = seed_binding(&*seed)?;
    journal.authenticate_wallet_binding(&binding)
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryNote {
    pub tree_number: u16,
    pub note: PrivateInputNote,
    pub created_block_number: u64,
}

/// A normalized result from a reviewed local SDK and canonical-chain adapter.
/// `complete_from_genesis` is an adapter obligation, not a proof verified here.
/// The cutoff is fixed after stopping old-device spending and waiting for the
/// selected canonical head to finalize; new sweep outputs must be later.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryScan {
    pub schema_version: u8,
    pub domain: PrivateDomain,
    pub wallet_binding: String,
    pub cutoff_block_number: u64,
    pub cutoff_block_hash: String,
    pub scanned_through_block: u64,
    pub scanned_block_hash: String,
    pub canonical_scanned_block_hash: String,
    pub finalized_height: u64,
    pub state_root: String,
    pub complete_from_genesis: bool,
    pub spendable_notes: Vec<RecoveryNote>,
    pub scan_digest: String,
}
impl std::fmt::Debug for RecoveryScan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecoveryScan([PRIVATE LOCAL STATE])")
    }
}
fn validate_scan_body(scan: &RecoveryScan) -> Result<(), WalletError> {
    payment::validate_domain(&scan.domain)?;
    payment::canonical_hash(&scan.wallet_binding)?;
    for hash in [
        &scan.cutoff_block_hash,
        &scan.scanned_block_hash,
        &scan.canonical_scanned_block_hash,
        &scan.state_root,
    ] {
        payment::canonical_hash(hash)?;
    }
    if scan.schema_version != 1
        || !scan.complete_from_genesis
        || scan.cutoff_block_number == 0
        || scan.scanned_through_block < scan.cutoff_block_number
        || scan.scanned_through_block > i64::MAX as u64
        || scan.finalized_height < scan.scanned_through_block
        || scan.finalized_height > i64::MAX as u64
        || scan.scanned_block_hash != scan.canonical_scanned_block_hash
        || scan.spendable_notes.len() > MAX_RECOVERY_NOTES
    {
        return Err(blocked(
            "Recovery needs a complete canonical finalized note scan",
        ));
    }
    if scan.scanned_through_block == scan.cutoff_block_number
        && scan.scanned_block_hash != scan.cutoff_block_hash
    {
        return Err(invalid("Recovery cutoff hash is inconsistent"));
    }
    let mut nullifiers = HashSet::new();
    for entry in &scan.spendable_notes {
        payment::canonical_hash(&entry.note.commitment)?;
        payment::canonical_hash(&entry.note.nullifier)?;
        if !nullifiers.insert((entry.tree_number, &entry.note.nullifier))
            || validation::decimal(&entry.note.units)?.is_zero()
            || entry.created_block_number == 0
            || entry.created_block_number > scan.scanned_through_block
        {
            return Err(invalid("Recovery scan contains invalid or duplicate notes"));
        }
    }
    Ok(())
}
pub fn recovery_scan_digest(scan: &RecoveryScan) -> Result<String, WalletError> {
    validate_scan_body(scan)?;
    let mut unsigned = scan.clone();
    unsigned.scan_digest.clear();
    typed_digest(b"LocalWallet/private-recovery-scan/v1\0", &unsigned)
}
pub fn validate_recovery_scan(scan: &RecoveryScan) -> Result<(), WalletError> {
    payment::canonical_hash(&scan.scan_digest)?;
    if scan.scan_digest != recovery_scan_digest(scan)? {
        return Err(invalid("Recovery scan does not match its digest"));
    }
    Ok(())
}
pub fn seal_recovery_scan_json(text: &str) -> Result<String, WalletError> {
    let mut scan: RecoveryScan = payment::decode(text)?;
    if scan.scan_digest.is_empty() {
        scan.scan_digest = recovery_scan_digest(&scan)?;
    }
    validate_recovery_scan(&scan)?;
    payment::encode(&scan)
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverySweepPlan {
    pub schema_version: u8,
    pub recovery_id: String,
    pub own_private_recipient: String,
    pub initial_scan: RecoveryScan,
    pub max_total_fee_units: String,
    pub approved_at: u64,
    pub plan_digest: String,
}
impl std::fmt::Debug for RecoverySweepPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecoverySweepPlan([PRIVATE LOCAL STATE])")
    }
}
fn validate_plan_body(plan: &RecoverySweepPlan) -> Result<(), WalletError> {
    payment::canonical_hash(&plan.recovery_id)?;
    validate_recovery_scan(&plan.initial_scan)?;
    payment::validate_private_recipient(&plan.own_private_recipient)?;
    validation::decimal(&plan.max_total_fee_units)?;
    if plan.schema_version != 1
        || plan.approved_at == 0
        || plan.approved_at > i64::MAX as u64
        || plan.initial_scan.scanned_through_block != plan.initial_scan.cutoff_block_number
        || plan
            .initial_scan
            .spendable_notes
            .iter()
            .any(|n| n.created_block_number > plan.initial_scan.cutoff_block_number)
    {
        return Err(invalid("Recovery sweep plan has an invalid initial cutoff"));
    }
    Ok(())
}
pub fn recovery_plan_digest(plan: &RecoverySweepPlan) -> Result<String, WalletError> {
    validate_plan_body(plan)?;
    let mut unsigned = plan.clone();
    unsigned.plan_digest.clear();
    typed_digest(b"LocalWallet/private-self-sweep-plan/v1\0", &unsigned)
}
pub fn validate_recovery_plan(plan: &RecoverySweepPlan) -> Result<(), WalletError> {
    payment::canonical_hash(&plan.plan_digest)?;
    if plan.plan_digest != recovery_plan_digest(plan)? {
        return Err(invalid(
            "Recovery sweep plan does not match its approval digest",
        ));
    }
    Ok(())
}
pub fn seal_recovery_plan_json(text: &str) -> Result<String, WalletError> {
    let mut plan: RecoverySweepPlan = payment::decode(text)?;
    if plan.plan_digest.is_empty() {
        plan.plan_digest = recovery_plan_digest(&plan)?;
    }
    validate_recovery_plan(&plan)?;
    payment::encode(&plan)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryFinalReceipt {
    pub intent_id: String,
    pub receipt: crate::private_journal::PrivateReceiptObservation,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryResolution {
    pub schema_version: u8,
    pub recovery_id: String,
    pub plan_digest: String,
    pub final_scan: RecoveryScan,
    pub finalized_receipts: Vec<RecoveryFinalReceipt>,
}

pub(crate) fn validate_sweep_intent(
    plan: &RecoverySweepPlan,
    intent: &PrivateIntent,
) -> Result<(), WalletError> {
    payment::validate_intent(intent)?;
    if intent.domain != plan.initial_scan.domain
        || intent.recipient != plan.own_private_recipient
        || intent.change_units != "0"
        || intent.change_output_commitment.is_some()
    {
        return Err(blocked(
            "Recovery permits only an approved self-transfer of the selected notes",
        ));
    }
    for note in &intent.input_notes {
        if !plan
            .initial_scan
            .spendable_notes
            .iter()
            .any(|entry| entry.tree_number == intent.tree_number && entry.note == *note)
        {
            return Err(blocked(
                "Recovery batch contains an input outside the approved complete scan",
            ));
        }
    }
    if validation::decimal(&intent.fee.fee_units)? > validation::decimal(&plan.max_total_fee_units)?
    {
        return Err(blocked("Recovery service fee exceeds the approved total"));
    }
    Ok(())
}
pub(crate) fn fee_units(value: &str) -> Result<U256, WalletError> {
    validation::decimal(value)
}
