//! Encrypted, append-only private-payment journal.
//!
//! Recipient, amount, note ownership, envelope and observations are encrypted
//! before SQLite sees them. A per-file random salt, sequence numbers, a hash
//! chain and authenticated tip detect row substitution, modification and
//! truncation. This does not protect against rollback of an entire valid file
//! snapshot, a compromised unlocked device, or loss of the device/backup keys.
//!
//! Observations are normalized inputs from a future reviewed protocol adapter;
//! they are NOT independent consensus proofs. No proof verification or network
//! submission is implemented by this journal. The production release gate is
//! separate and remains closed in this candidate.

use crate::private_payment::{self as payment, PrivateEnvelope, PrivateIntent};
use crate::private_recovery::{
    self as recovery, RecoveryCheckpoint, RecoveryCutoff, RecoveryResolution, RecoverySession,
    RecoverySweepPlan,
};
use crate::WalletError;
use aes_gcm::{
    aead::{AeadInPlace, KeyInit},
    Aes256Gcm, Nonce,
};
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

const APPLICATION_ID: i64 = 0x4c57505a;
const SCHEMA_VERSION: i64 = 2;
const MAX_EVENTS: u64 = 50_000;
const MAX_EVENT_BYTES: usize = payment::MAX_JSON_BYTES * 2;
/// Hard resource limit for encrypted event records, including their nonces,
/// hash-chain links and sequence numbers. Reaching it preserves every record
/// and requires a reviewed archival/migration strategy; it never clears notes.
pub const MAX_ENCRYPTED_JOURNAL_BYTES: usize = 64 * 1024 * 1024;
const EVENT_STORAGE_OVERHEAD: usize = 12 + 32 + 32 + 8;
const CONTEXT: &[u8] = b"LocalWallet/private-journal/v2\0";
const ZERO_HASH: [u8; 32] = [0; 32];
type MetadataRow = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);
const SCHEMA: &str = "
CREATE TABLE metadata (
 id INTEGER PRIMARY KEY CHECK(id=1),
 salt BLOB NOT NULL CHECK(length(salt)=32),
 header_nonce BLOB NOT NULL CHECK(length(header_nonce)=12),
 header_ciphertext BLOB NOT NULL CHECK(length(header_ciphertext) BETWEEN 16 AND 16384),
 tip_nonce BLOB NOT NULL CHECK(length(tip_nonce)=12),
 tip_ciphertext BLOB NOT NULL CHECK(length(tip_ciphertext) BETWEEN 16 AND 4096)
) STRICT;
CREATE TABLE events (
 seq INTEGER PRIMARY KEY CHECK(seq>0),
 nonce BLOB NOT NULL CHECK(length(nonce)=12),
 ciphertext BLOB NOT NULL CHECK(length(ciphertext) BETWEEN 16 AND 1048576),
 previous_hash BLOB NOT NULL CHECK(length(previous_hash)=32),
 event_hash BLOB NOT NULL CHECK(length(event_hash)=32)
) STRICT;
CREATE TRIGGER metadata_identity_immutable BEFORE UPDATE ON metadata
 WHEN NEW.id != OLD.id OR NEW.salt != OLD.salt OR NEW.header_nonce != OLD.header_nonce OR NEW.header_ciphertext != OLD.header_ciphertext
 BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER metadata_no_delete BEFORE DELETE ON metadata BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER events_no_update BEFORE UPDATE ON events BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER events_no_delete BEFORE DELETE ON events BEGIN SELECT RAISE(ABORT,'immutable'); END;
";

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
fn corrupt() -> WalletError {
    WalletError::Storage {
        reason: "Private journal authentication or evidence validation failed; existing data was preserved".into(),
    }
}
fn storage(_: impl std::fmt::Debug) -> WalletError {
    WalletError::Storage {
        reason: "The encrypted private-payment journal could not be read or saved".into(),
    }
}
fn capacity() -> WalletError {
    WalletError::Storage {
        reason: "Private journal reached its encrypted history capacity; existing records and input reservations were preserved".into(),
    }
}
fn checked_history_bytes(previous: usize, ciphertext_len: usize) -> Result<usize, WalletError> {
    previous
        .checked_add(EVENT_STORAGE_OVERHEAD)
        .and_then(|bytes| bytes.checked_add(ciphertext_len))
        .filter(|bytes| *bytes <= MAX_ENCRYPTED_JOURNAL_BYTES)
        .ok_or_else(capacity)
}
fn now() -> Result<u64, WalletError> {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(storage)?
        .as_secs();
    if n == 0 || n > i64::MAX as u64 {
        return Err(invalid("Invalid device time"));
    }
    Ok(n)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Approved,
    Ready,
    Released,
    Unknown,
    Included,
    Finalized,
    Failed,
    Reorg,
    Conflict,
    Cancelled,
    /// A finalized recovery spend consumed at least one input of this older
    /// authorization. It is not a successful receipt for the older payment.
    Superseded,
}

/// A normalized observation, not a raw RPC result or a cryptographic receipt.
/// The protocol adapter must match actual chain calldata and protocol events
/// before creating this value. Matching nullifiers alone is never sufficient.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateReceiptObservation {
    pub operation_digest: String,
    pub chain_id: u64,
    pub pool_contract: String,
    pub transaction_hash: String,
    pub block_hash: String,
    pub canonical_block_hash: String,
    pub block_number: u64,
    pub finalized_height: u64,
    pub execution_success: bool,
    pub nullifiers: Vec<String>,
    pub output_commitments: Vec<String>,
    pub fee_units_charged: String,
    pub native_fee_wei: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateReorgObservation {
    pub block_number: u64,
    pub previous_block_hash: String,
    pub canonical_block_hash: String,
    pub finalized_height: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    schema_version: u8,
    identity: String,
    salt: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tip {
    seq: u64,
    hash: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case", deny_unknown_fields)]
enum EventPayload {
    WalletBound {
        wallet_binding: String,
    },
    RecoveryStarted {
        session: RecoverySession,
    },
    RecoveryCheckpointAccepted {
        checkpoint: RecoveryCheckpoint,
    },
    RecoveryCutoffRecorded {
        cutoff: RecoveryCutoff,
    },
    RecoverySweepApproved {
        plan: RecoverySweepPlan,
    },
    RecoveryBatchApproved {
        recovery_id: String,
        intent: PrivateIntent,
    },
    RecoveryCompleted {
        resolution: RecoveryResolution,
    },
    Approved {
        intent: PrivateIntent,
    },
    Ready {
        envelope: PrivateEnvelope,
    },
    BackupExposed,
    Released,
    Unknown,
    Receipt {
        observation: PrivateReceiptObservation,
    },
    Reorg {
        observation: PrivateReorgObservation,
    },
    Cancelled,
}

impl EventPayload {
    fn kind(&self) -> &'static str {
        match self {
            Self::WalletBound { .. } => "wallet_bound",
            Self::RecoveryStarted { .. } => "recovery_started",
            Self::RecoveryCheckpointAccepted { .. } => "recovery_checkpoint_accepted",
            Self::RecoveryCutoffRecorded { .. } => "recovery_cutoff_recorded",
            Self::RecoverySweepApproved { .. } => "recovery_sweep_approved",
            Self::RecoveryBatchApproved { .. } => "recovery_batch_approved",
            Self::RecoveryCompleted { .. } => "recovery_completed",
            Self::Approved { .. } => "approved",
            Self::Ready { .. } => "ready",
            Self::BackupExposed => "backup_exposed",
            Self::Released => "released",
            Self::Unknown => "unknown",
            Self::Receipt { .. } => "receipt",
            Self::Reorg { .. } => "reorg",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    schema_version: u8,
    intent_id: String,
    at: u64,
    payload: EventPayload,
}

#[derive(Clone)]
struct Payment {
    intent: PrivateIntent,
    envelope: Option<PrivateEnvelope>,
    phase: Phase,
    release_count: u64,
    receipt: Option<PrivateReceiptObservation>,
    // Reconstructed from authenticated events. A reverted outer attempt does
    // not invalidate its private proof, and delayed old receipts must not
    // replace a later successful attempt of the same exact operation.
    finalized_reverts: BTreeMap<String, PrivateReceiptObservation>,
    envelope_backed_up: bool,
    sequence: u64,
    recovery_id: Option<String>,
    release_history_incomplete: bool,
}

impl Payment {
    fn record(&self) -> serde_json::Value {
        json!({
            "intent_id":self.intent.intent_id,
            "phase":self.phase,
            "chain_id":self.intent.domain.chain_id,
            "token_contract":self.intent.domain.token_contract,
            "operation_digest":self.envelope.as_ref().map(|e|e.operation_digest.as_str()),
            "release_count":self.release_count,
            "sequence":self.sequence,
            "receipt":self.receipt,
            "proof_verification":"not_performed_by_journal"
            ,"recovery_id":self.recovery_id,"release_history_incomplete":self.release_history_incomplete,
            "envelope_backed_up":self.envelope_backed_up
        })
    }

    fn full(&self) -> Result<String, WalletError> {
        payment::encode(&json!({
            "record":self.record(),
            "intent_json":payment::encode(&self.intent)?,
            "envelope_json":self.envelope.as_ref().map(payment::canonical_envelope).transpose()?
        }))
    }
}

#[derive(Default)]
struct Loaded {
    payments: BTreeMap<String, Payment>,
    events: Vec<(u64, Event)>,
    seq: u64,
    hash: [u8; 32],
    encrypted_bytes: usize,
    identity: String,
    wallet_binding: Option<String>,
    recovery: Option<RecoveryState>,
}

#[derive(Clone)]
struct RecoveryState {
    session: RecoverySession,
    quarantined: bool,
    incident: bool,
    plan: Option<RecoverySweepPlan>,
    cutoff: Option<RecoveryCutoff>,
    resolution_scan_digest: Option<String>,
    resolution_intent_ids: HashSet<String>,
}
impl RecoveryState {
    fn view(&self) -> serde_json::Value {
        json!({"recovery_id":self.session.recovery_id,"wallet_binding":self.session.wallet_binding,
            "reason":self.session.reason,"quarantined":self.quarantined,"incident":self.incident,
            "source_checkpoint":self.session.checkpoint,"plan_digest":self.plan.as_ref().map(|p|p.plan_digest.as_str()),
            "selected_input_count":self.plan.as_ref().map(|p|p.initial_scan.spendable_notes.len()),
            "resolution_scan_digest":self.resolution_scan_digest,"cutoff":self.cutoff,
            "scan_verification":"trusted_local_adapter_required","freshness":"not_proven_by_backup_authentication"})
    }
}

/// Only serialized inside an authenticated encrypted recovery document. Event
/// payloads remain private; callers never receive a plaintext snapshot API.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoverySnapshot {
    schema_version: u8,
    pub(crate) identity: String,
    pub(crate) source_sequence: u64,
    pub(crate) source_tip: String,
    events: Vec<(u64, Event)>,
}
impl RecoverySnapshot {
    pub(crate) fn empty(identity: String) -> Self {
        Self {
            schema_version: 2,
            identity,
            source_sequence: 0,
            source_tip: format!("0x{}", hex::encode(ZERO_HASH)),
            events: vec![],
        }
    }
    pub(crate) fn validate(&self) -> Result<(), WalletError> {
        payment::canonical_hash(&self.identity)?;
        if self.schema_version != 2
            || self.source_sequence != self.events.len() as u64
            || self.source_sequence > MAX_EVENTS
        {
            return Err(corrupt());
        }
        if self.source_sequence == 0 {
            if self.source_tip != format!("0x{}", hex::encode(ZERO_HASH)) {
                return Err(corrupt());
            }
        } else {
            payment::canonical_hash(&self.source_tip)?;
        }
        let mut loaded = Loaded {
            identity: self.identity.clone(),
            ..Loaded::default()
        };
        for (seq, event) in &self.events {
            if *seq != loaded.seq + 1 {
                return Err(corrupt());
            }
            let bytes = Zeroizing::new(serde_json::to_vec(event).map_err(|_| WalletError::Crypto)?);
            if bytes.len() > MAX_EVENT_BYTES - 16 {
                return Err(capacity());
            }
            loaded.encrypted_bytes =
                checked_history_bytes(loaded.encrypted_bytes, bytes.len() + 16)?;
            apply_event(&mut loaded, *seq, event)?;
            loaded.seq = *seq;
        }
        Ok(())
    }
    pub(crate) fn ensure_wallet_binding(&self, binding: &str) -> Result<(), WalletError> {
        payment::canonical_hash(binding)?;
        for (_, event) in &self.events {
            match &event.payload {
                EventPayload::WalletBound { wallet_binding } if wallet_binding != binding => {
                    return Err(corrupt())
                }
                EventPayload::RecoveryStarted { session } if session.wallet_binding != binding => {
                    return Err(corrupt())
                }
                _ => {}
            }
        }
        Ok(())
    }
}

pub struct Journal {
    connection: Mutex<Connection>,
    key: Zeroizing<Vec<u8>>,
    salt: [u8; 32],
    identity: String,
}

fn schema(connection: &Connection) -> Result<BTreeMap<String, (String, String)>, WalletError> {
    let mut q = connection
        .prepare(
            "SELECT name,type,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map_err(storage)?;
    let rows = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (r.get::<_, String>(1)?, r.get::<_, String>(2)?),
            ))
        })
        .map_err(storage)?;
    rows.collect::<Result<_, _>>().map_err(storage)
}

fn expected_schema() -> Result<BTreeMap<String, (String, String)>, WalletError> {
    let c = Connection::open_in_memory().map_err(storage)?;
    c.execute_batch(SCHEMA).map_err(storage)?;
    schema(&c)
}

fn aad(salt: &[u8; 32], kind: &[u8], seq: u64, previous: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(CONTEXT.len() + 32 + kind.len() + 8 + 32 + 1);
    out.extend_from_slice(CONTEXT);
    out.extend_from_slice(salt);
    out.extend_from_slice(kind);
    out.push(0);
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(previous);
    out
}

fn encrypt<T: Serialize>(
    key: &[u8],
    context: &[u8],
    value: &T,
) -> Result<([u8; 12], Vec<u8>), WalletError> {
    let mut plain = Zeroizing::new(serde_json::to_vec(value).map_err(|_| WalletError::Crypto)?);
    if plain.len() > MAX_EVENT_BYTES - 16 {
        return Err(invalid("Private journal event is too large"));
    }
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|_| WalletError::Crypto)?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| WalletError::Crypto)?;
    // aead's `Buffer` implementation operates on Vec, while Zeroizing controls
    // its lifetime. On failure any remaining plaintext is wiped on return.
    cipher
        .encrypt_in_place(Nonce::from_slice(&nonce), context, &mut *plain)
        .map_err(|_| WalletError::Crypto)?;
    Ok((nonce, plain.to_vec()))
}

fn decrypt<T: for<'a> Deserialize<'a>>(
    key: &[u8],
    context: &[u8],
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<T, WalletError> {
    if nonce.len() != 12 || ciphertext.len() < 16 || ciphertext.len() > MAX_EVENT_BYTES {
        return Err(corrupt());
    }
    let mut plain = Zeroizing::new(ciphertext.to_vec());
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| corrupt())?;
    cipher
        .decrypt_in_place(Nonce::from_slice(nonce), context, &mut *plain)
        .map_err(|_| corrupt())?;
    serde_json::from_slice(&plain).map_err(|_| corrupt())
}

fn event_hash(
    salt: &[u8; 32],
    seq: u64,
    nonce: &[u8],
    ciphertext: &[u8],
    previous: &[u8; 32],
) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(aad(salt, b"event-hash", seq, previous));
    hash.update(nonce);
    hash.update((ciphertext.len() as u64).to_be_bytes());
    hash.update(ciphertext);
    hash.finalize().into()
}

fn create_regular_file(path: &Path) -> Result<(), WalletError> {
    if !path.is_absolute() {
        return Err(invalid(
            "Private journal requires an absolute application-private path",
        ));
    }
    let parent = path.parent().ok_or_else(corrupt)?;
    let parent_metadata = fs::symlink_metadata(parent).map_err(storage)?;
    if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
        return Err(corrupt());
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(corrupt());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if metadata.nlink() != 1 {
                    return Err(corrupt());
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(path) {
                Ok(file) => {
                    file.sync_all().map_err(storage)?;
                    #[cfg(unix)]
                    fs::File::open(parent)
                        .and_then(|f| f.sync_all())
                        .map_err(storage)?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // A concurrent opener created it; inspect again rather
                    // than following an attacker-created link.
                    return create_regular_file(path);
                }
                Err(e) => return Err(storage(e)),
            }
        }
        Err(e) => return Err(storage(e)),
    }
    Ok(())
}

impl Journal {
    pub fn open(path: String, identity: String, key: Vec<u8>) -> Result<Self, WalletError> {
        let key = Zeroizing::new(key);
        if key.len() != 32 {
            return Err(invalid(
                "Private journal encryption requires a 32-byte local key",
            ));
        }
        payment::canonical_hash(&identity)?;
        create_regular_file(Path::new(&path))?;
        let mut connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_FULL_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(storage)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(storage)?;
        connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON; PRAGMA mmap_size=0;").map_err(storage)?;

        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let app_id: i64 = tx
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .map_err(storage)?;
        let version: i64 = tx
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(storage)?;
        let objects = schema(&tx)?;
        let expected = expected_schema()?;
        if app_id == 0 && version == 0 && objects.is_empty() {
            tx.execute_batch(SCHEMA).map_err(storage)?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)
                .map_err(storage)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(storage)?;
            let mut salt = [0u8; 32];
            getrandom::fill(&mut salt).map_err(|_| WalletError::Crypto)?;
            let header = Header {
                schema_version: 2,
                identity: identity.clone(),
                salt: hex::encode(salt),
            };
            let tip = Tip {
                seq: 0,
                hash: hex::encode(ZERO_HASH),
            };
            let (header_nonce, header_ciphertext) =
                encrypt(&key, &aad(&salt, b"header", 0, &ZERO_HASH), &header)?;
            let (tip_nonce, tip_ciphertext) =
                encrypt(&key, &aad(&salt, b"tip", 0, &ZERO_HASH), &tip)?;
            tx.execute(
                "INSERT INTO metadata(id,salt,header_nonce,header_ciphertext,tip_nonce,tip_ciphertext) VALUES(1,?1,?2,?3,?4,?5)",
                params![salt.as_slice(), header_nonce.as_slice(), header_ciphertext, tip_nonce.as_slice(), tip_ciphertext],
            ).map_err(storage)?;
        } else if app_id == APPLICATION_ID && version == 1 {
            return Err(WalletError::Storage { reason: "Private journal version 1 lacks input-tree identities and requires an explicit reviewed migration; existing data was preserved".into() });
        } else if app_id != APPLICATION_ID || version != SCHEMA_VERSION || objects != expected {
            return Err(corrupt());
        }
        let salt: Vec<u8> = tx
            .query_row(
                "SELECT substr(salt,1,33) FROM metadata WHERE id=1",
                [],
                |r| r.get(0),
            )
            .map_err(storage)?;
        let salt: [u8; 32] = salt.try_into().map_err(|_| corrupt())?;
        tx.commit().map_err(storage)?;
        // WAL and FULL are required; do not silently use an unsupported mode.
        let wal: String = connection
            .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
            .map_err(storage)?;
        if !wal.eq_ignore_ascii_case("wal") {
            return Err(corrupt());
        }
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(storage)?;
        let journal = Self {
            connection: Mutex::new(connection),
            key,
            salt,
            identity,
        };
        {
            let mut connection = journal.lock()?;
            let tx = connection
                .transaction_with_behavior(TransactionBehavior::Deferred)
                .map_err(storage)?;
            journal.load(&tx)?;
        }
        Ok(journal)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, WalletError> {
        self.connection.lock().map_err(|_| corrupt())
    }

    fn load(&self, connection: &Connection) -> Result<Loaded, WalletError> {
        if schema(connection)? != expected_schema()? {
            return Err(corrupt());
        }
        // Check lengths before materializing attacker-modifiable SQLite blobs.
        let metadata_bounded: bool = connection.query_row(
            "SELECT COUNT(*)=1 AND COALESCE(MIN(id=1 AND length(salt)=32 AND length(header_nonce)=12 AND length(header_ciphertext) BETWEEN 16 AND 16384 AND length(tip_nonce)=12 AND length(tip_ciphertext) BETWEEN 16 AND 4096),0)=1 FROM metadata",
            [], |r| r.get(0),
        ).map_err(storage)?;
        if !metadata_bounded {
            return Err(corrupt());
        }
        let (salt, header_nonce, header_ciphertext, tip_nonce, tip_ciphertext): MetadataRow = connection.query_row(
            "SELECT salt,header_nonce,header_ciphertext,tip_nonce,tip_ciphertext FROM metadata WHERE id=1", [],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        ).map_err(storage)?;
        if salt != self.salt {
            return Err(corrupt());
        }
        let header: Header = decrypt(
            &self.key,
            &aad(&self.salt, b"header", 0, &ZERO_HASH),
            &header_nonce,
            &header_ciphertext,
        )?;
        if header.schema_version != 2
            || header.identity != self.identity
            || header.salt != hex::encode(self.salt)
        {
            return Err(corrupt());
        }
        let tip: Tip = decrypt(
            &self.key,
            &aad(&self.salt, b"tip", 0, &ZERO_HASH),
            &tip_nonce,
            &tip_ciphertext,
        )?;
        if tip.seq > MAX_EVENTS || tip.hash.len() != 64 {
            return Err(corrupt());
        }
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .map_err(storage)?;
        if count < 0 || count as u64 != tip.seq {
            return Err(corrupt());
        }
        // COUNT is bounded by the authenticated tip before SUM is evaluated.
        // SQLite reads BLOB lengths without copying their contents into Rust.
        let encrypted_bytes: i64 = connection.query_row(
            "SELECT COALESCE(SUM(length(nonce)+length(ciphertext)+length(previous_hash)+length(event_hash)+8),0) FROM events",
            [], |r| r.get(0),
        ).map_err(storage)?;
        if encrypted_bytes < 0 || encrypted_bytes as u64 > MAX_ENCRYPTED_JOURNAL_BYTES as u64 {
            return Err(capacity());
        }

        let mut loaded = Loaded {
            identity: self.identity.clone(),
            ..Loaded::default()
        };
        let mut query = connection
            .prepare(
                "SELECT seq,nonce,ciphertext,previous_hash,event_hash FROM events ORDER BY seq",
            )
            .map_err(storage)?;
        let mut rows = query.query([]).map_err(storage)?;
        while let Some(row) = rows.next().map_err(storage)? {
            let seq: i64 = row.get(0).map_err(storage)?;
            let nonce = row
                .get_ref(1)
                .map_err(storage)?
                .as_blob()
                .map_err(storage)?;
            let ciphertext = row
                .get_ref(2)
                .map_err(storage)?
                .as_blob()
                .map_err(storage)?;
            let previous = row
                .get_ref(3)
                .map_err(storage)?
                .as_blob()
                .map_err(storage)?;
            let hash = row
                .get_ref(4)
                .map_err(storage)?
                .as_blob()
                .map_err(storage)?;
            if nonce.len() != 12
                || !(16..=MAX_EVENT_BYTES).contains(&ciphertext.len())
                || previous.len() != 32
                || hash.len() != 32
            {
                return Err(corrupt());
            }
            let next_bytes = checked_history_bytes(loaded.encrypted_bytes, ciphertext.len())?;
            if seq <= 0 || seq as u64 != loaded.seq + 1 || previous != loaded.hash {
                return Err(corrupt());
            }
            let expected_hash = event_hash(&self.salt, seq as u64, nonce, ciphertext, &loaded.hash);
            if hash != expected_hash {
                return Err(corrupt());
            }
            let event: Event = decrypt(
                &self.key,
                &aad(&self.salt, b"event", seq as u64, &loaded.hash),
                nonce,
                ciphertext,
            )?;
            apply_event(&mut loaded, seq as u64, &event).map_err(|_| corrupt())?;
            loaded.seq = seq as u64;
            loaded.hash = expected_hash;
            loaded.encrypted_bytes = next_bytes;
            loaded.events.push((loaded.seq, event));
        }
        if loaded.seq != tip.seq
            || hex::encode(loaded.hash) != tip.hash
            || loaded.encrypted_bytes != encrypted_bytes as usize
        {
            return Err(corrupt());
        }
        Ok(loaded)
    }

    fn append(
        &self,
        connection: &Connection,
        loaded: &mut Loaded,
        event: Event,
    ) -> Result<(), WalletError> {
        if loaded.seq >= MAX_EVENTS {
            return Err(invalid(
                "Private journal reached its reviewed event capacity",
            ));
        }
        let seq = loaded.seq + 1;
        apply_event(loaded, seq, &event)?;
        let (nonce, ciphertext) = encrypt(
            &self.key,
            &aad(&self.salt, b"event", seq, &loaded.hash),
            &event,
        )?;
        let next_bytes = checked_history_bytes(loaded.encrypted_bytes, ciphertext.len())?;
        let hash = event_hash(&self.salt, seq, &nonce, &ciphertext, &loaded.hash);
        connection.execute("INSERT INTO events(seq,nonce,ciphertext,previous_hash,event_hash) VALUES(?1,?2,?3,?4,?5)",
            params![seq as i64,nonce.as_slice(),ciphertext,loaded.hash.as_slice(),hash.as_slice()]).map_err(storage)?;
        let tip = Tip {
            seq,
            hash: hex::encode(hash),
        };
        let (tip_nonce, tip_ciphertext) =
            encrypt(&self.key, &aad(&self.salt, b"tip", 0, &ZERO_HASH), &tip)?;
        if connection
            .execute(
                "UPDATE metadata SET tip_nonce=?1,tip_ciphertext=?2 WHERE id=1",
                params![tip_nonce.as_slice(), tip_ciphertext],
            )
            .map_err(storage)?
            != 1
        {
            return Err(corrupt());
        }
        loaded.seq = seq;
        loaded.hash = hash;
        loaded.encrypted_bytes = next_bytes;
        loaded.events.push((seq, event));
        Ok(())
    }

    pub fn record_approved(&self, intent_json: String) -> Result<String, WalletError> {
        let intent_json = Zeroizing::new(intent_json);
        let intent = payment::validate_intent_json(&intent_json)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut loaded = self.load(&tx)?;
        if let Some(existing) = loaded.payments.get(&intent.intent_id) {
            if existing.intent != intent {
                return Err(conflict(
                    "A private intent identifier already has different approved contents",
                ));
            }
            return payment::encode(&existing.record());
        }
        if loaded
            .recovery
            .as_ref()
            .is_some_and(|r| r.quarantined || r.incident)
        {
            return Err(conflict("Private recovery is quarantined; a new ordinary authorization cannot reserve notes"));
        }
        let at = now()?;
        if intent.created_at > at.saturating_add(30) || intent.fee.quote_valid_until < at {
            return Err(invalid(
                "The private fee quote expired or the device clock is inconsistent",
            ));
        }
        let id = intent.intent_id.clone();
        self.append(
            &tx,
            &mut loaded,
            Event {
                schema_version: 2,
                intent_id: id.clone(),
                at,
                payload: EventPayload::Approved { intent },
            },
        )?;
        let result = payment::encode(&loaded.payments[&id].record())?;
        tx.commit().map_err(storage)?;
        Ok(result)
    }

    pub fn record_ready(
        &self,
        intent_id: String,
        envelope_json: String,
    ) -> Result<String, WalletError> {
        let envelope_json = Zeroizing::new(envelope_json);
        let envelope = payment::validate_envelope_json(&envelope_json)?;
        self.mutate(intent_id, EventPayload::Ready { envelope })
    }

    /// This is a durable egress marker, NOT authorization to bypass the closed
    /// production release gate. Call it before any external use, even remote
    /// simulation, of an externally usable proof/operation.
    pub fn mark_released(&self, intent_id: String) -> Result<String, WalletError> {
        self.mutate(intent_id, EventPayload::Released)
    }

    pub fn mark_unknown(&self, intent_id: String) -> Result<String, WalletError> {
        self.mutate(intent_id, EventPayload::Unknown)
    }

    pub fn observe_receipt(
        &self,
        intent_id: String,
        observation_json: String,
    ) -> Result<String, WalletError> {
        let observation = payment::decode(&observation_json)?;
        self.mutate(intent_id, EventPayload::Receipt { observation })
    }

    pub fn mark_reorg(
        &self,
        intent_id: String,
        observation_json: String,
    ) -> Result<String, WalletError> {
        let observation = payment::decode(&observation_json)?;
        self.mutate(intent_id, EventPayload::Reorg { observation })
    }

    /// Local abandonment only. It never invalidates an on-chain authorization.
    /// READY may be abandoned only before any externally usable copy has left
    /// the local engine. All external paths must first mark_released durably.
    pub fn cancel_approved(&self, intent_id: String) -> Result<String, WalletError> {
        self.mutate(intent_id, EventPayload::Cancelled)
    }

    fn mutate(&self, intent_id: String, payload: EventPayload) -> Result<String, WalletError> {
        payment::canonical_hash(&intent_id)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut loaded = self.load(&tx)?;
        if loaded.recovery.as_ref().is_some_and(|r| r.incident)
            && matches!(
                payload,
                EventPayload::Ready { .. } | EventPayload::Released | EventPayload::Cancelled
            )
        {
            return Err(conflict("Recovery finality is contradictory; further private releases require investigation"));
        }
        let existing = loaded
            .payments
            .get(&intent_id)
            .ok_or_else(|| invalid("Private intent does not exist"))?;
        if matches!(payload, EventPayload::Cancelled)
            && loaded.recovery.as_ref().is_some_and(|r| {
                r.quarantined
                    && (existing.recovery_id.as_deref() != Some(r.session.recovery_id.as_str())
                        || r.incident)
            })
        {
            return Err(conflict("Recovery cannot discard an older authorization whose release history may be incomplete"));
        }
        if no_change(existing, &payload)? {
            return payment::encode(&existing.record());
        }
        let at = now()?;
        self.append(
            &tx,
            &mut loaded,
            Event {
                schema_version: 2,
                intent_id: intent_id.clone(),
                at,
                payload,
            },
        )?;
        let result = payment::encode(&loaded.payments[&intent_id].record())?;
        tx.commit().map_err(storage)?;
        Ok(result)
    }

    pub(crate) fn recovery_snapshot(&self, binding: &str) -> Result<RecoverySnapshot, WalletError> {
        payment::canonical_hash(binding)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut loaded = self.load(&tx)?;
        if loaded
            .wallet_binding
            .as_deref()
            .is_some_and(|old| old != binding)
        {
            return Err(conflict(
                "The authenticated recovery vault differs from this journal's wallet binding",
            ));
        }
        if loaded.wallet_binding.is_none() {
            self.append(
                &tx,
                &mut loaded,
                Event {
                    schema_version: 2,
                    intent_id: binding.into(),
                    at: now()?,
                    payload: EventPayload::WalletBound {
                        wallet_binding: binding.into(),
                    },
                },
            )?;
        }
        // The user-controlled encrypted export carries an externally usable
        // proof once restored with the seed. A source READY operation therefore
        // cannot later be cancelled and its inputs reassigned, even though no
        // network submission has occurred. Commit before releasing any bytes.
        let exposed: Vec<_> = loaded
            .payments
            .values()
            .filter(|p| p.phase == Phase::Ready && !p.envelope_backed_up)
            .map(|p| p.intent.intent_id.clone())
            .collect();
        for intent_id in exposed {
            self.append(
                &tx,
                &mut loaded,
                Event {
                    schema_version: 2,
                    intent_id,
                    at: now()?,
                    payload: EventPayload::BackupExposed,
                },
            )?;
        }
        let snapshot = RecoverySnapshot {
            schema_version: 2,
            identity: self.identity.clone(),
            source_sequence: loaded.seq,
            source_tip: format!("0x{}", hex::encode(loaded.hash)),
            events: loaded.events,
        };
        tx.commit().map_err(storage)?;
        Ok(snapshot)
    }

    pub(crate) fn from_recovery_snapshot(
        path: String,
        identity: String,
        key: Vec<u8>,
        snapshot: RecoverySnapshot,
        session: RecoverySession,
    ) -> Result<Self, WalletError> {
        let key = Zeroizing::new(key);
        if key.len() != 32 {
            return Err(invalid(
                "Private recovery requires a 32-byte new device key",
            ));
        }
        snapshot.validate()?;
        session.validate(&identity)?;
        snapshot.ensure_wallet_binding(&session.wallet_binding)?;
        if snapshot.identity != identity {
            return Err(corrupt());
        }
        if let Some(checkpoint) = &session.checkpoint {
            if checkpoint.source_sequence != snapshot.source_sequence
                || checkpoint.source_tip != snapshot.source_tip
            {
                return Err(corrupt());
            }
        } else if !snapshot.events.is_empty() {
            return Err(corrupt());
        }
        let target = Path::new(&path);
        if !target.is_absolute() {
            return Err(invalid(
                "Recovery requires a new absolute application-private path",
            ));
        }
        let parent = target.parent().ok_or_else(corrupt)?;
        let metadata = fs::symlink_metadata(parent).map_err(storage)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(corrupt());
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file=options.open(target).map_err(|_|conflict("Recovery target already exists or cannot be created; existing files were preserved"))?;
        file.sync_all().map_err(storage)?;
        drop(file);
        // The caller must not register this isolated path until this factory
        // returns. All imported events plus quarantine commit together.
        let journal = Self::open(path, identity, key.to_vec())?;
        {
            let mut connection = journal.lock()?;
            let tx = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            let mut loaded = journal.load(&tx)?;
            if loaded.seq != 0 {
                return Err(corrupt());
            }
            for (_, event) in snapshot.events {
                journal.append(&tx, &mut loaded, event)?;
            }
            let id = session.recovery_id.clone();
            journal.append(
                &tx,
                &mut loaded,
                Event {
                    schema_version: 2,
                    intent_id: id,
                    at: now()?,
                    payload: EventPayload::RecoveryStarted { session },
                },
            )?;
            tx.commit().map_err(storage)?;
        }
        Ok(journal)
    }

    pub fn recovery_status(&self) -> Result<String, WalletError> {
        let mut c = self.lock()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(storage)?;
        let loaded = self.load(&tx)?;
        payment::encode(&loaded.recovery.as_ref().map(RecoveryState::view).unwrap_or_else(||json!({"quarantined":false,"incident":false,"recovery_id":null,"wallet_binding":loaded.wallet_binding})))
    }

    /// Authenticate a previously established seed binding without binding a
    /// new journal, exposing an envelope, or changing recovery state. A device
    /// encryption key and a non-null binding alone do not identify the current
    /// unlocked wallet. The caller derives this value inside the native vault
    /// boundary, rather than accepting it from an operator or UI preference.
    pub(crate) fn authenticate_wallet_binding(&self, binding: &str) -> Result<String, WalletError> {
        payment::canonical_hash(binding)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(storage)?;
        let loaded = self.load(&tx)?;
        if loaded.wallet_binding.as_deref() != Some(binding) {
            return Err(conflict(
                "The authenticated vault does not match this journal's established wallet binding",
            ));
        }
        payment::encode(&loaded.recovery.as_ref().map(RecoveryState::view).unwrap_or_else(||json!({"quarantined":false,"incident":false,"recovery_id":null,"wallet_binding":loaded.wallet_binding})))
    }

    /// Local-only recovery resume data, read from one authenticated snapshot.
    /// This is never an operator request or telemetry object. Full per-payment
    /// intent/envelope bytes remain available through get_payment; callers must
    /// not keep a competing authoritative copy of the plan in UI preferences.
    pub fn recovery_workspace(&self) -> Result<String, WalletError> {
        let mut c = self.lock()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(storage)?;
        let loaded = self.load(&tx)?;
        let Some(state) = &loaded.recovery else {
            return payment::encode(&json!({
                "status":{"quarantined":false,"incident":false,"recovery_id":null,
                    "wallet_binding":loaded.wallet_binding},
                "cutoff_json":null,"plan_json":null,"batches":[],
                "reconciled_records":[],"remaining_note_commitments":[]
            }));
        };
        let mut batches = Vec::new();
        let mut reconciled = Vec::new();
        let mut unavailable = HashSet::new();
        for payment in loaded.payments.values() {
            let current_batch =
                payment.recovery_id.as_deref() == Some(state.session.recovery_id.as_str());
            if current_batch {
                batches.push(payment.record());
            }
            let reconciled_prior = !current_batch
                && payment.phase == Phase::Finalized
                && payment
                    .receipt
                    .as_ref()
                    .is_some_and(|r| r.execution_success)
                && state.plan.as_ref().is_some_and(|p| {
                    payment.intent.domain == p.initial_scan.domain
                        && p.initial_scan.spendable_notes.iter().any(|old| {
                            old.tree_number == payment.intent.tree_number
                                && payment
                                    .intent
                                    .input_notes
                                    .iter()
                                    .any(|n| n.nullifier == old.note.nullifier)
                        })
                });
            if reconciled_prior {
                reconciled.push(payment.record());
            }
            if (current_batch && payment.phase != Phase::Cancelled) || reconciled_prior {
                for note in &payment.intent.input_notes {
                    unavailable.insert((payment.intent.tree_number, note.nullifier.as_str()));
                }
            }
        }
        let remaining: Vec<_> = state
            .plan
            .as_ref()
            .map(|p| {
                p.initial_scan
                    .spendable_notes
                    .iter()
                    .filter(|n| !unavailable.contains(&(n.tree_number, n.note.nullifier.as_str())))
                    .map(|n| {
                        json!({"tree_number":n.tree_number,
                    "commitment":n.note.commitment,"nullifier":n.note.nullifier})
                    })
                    .collect()
            })
            .unwrap_or_default();
        payment::encode(&json!({
            "status":state.view(),
            "cutoff_json":state.cutoff.as_ref().map(payment::encode).transpose()?,
            "plan_json":state.plan.as_ref().map(payment::encode).transpose()?,
            "batches":batches,"reconciled_records":reconciled,
            "remaining_note_commitments":remaining
        }))
    }

    /// Only the trusted local recovery coordinator may call this with a latest
    /// checkpoint retained independently of the selected backup. No remote
    /// service, scan response, or checkpoint extracted from that backup proves
    /// freshness. Existing Unknown reservations remain in place after release.
    pub fn accept_recovery_checkpoint(
        &self,
        recovery_id: String,
        checkpoint_json: String,
    ) -> Result<String, WalletError> {
        let checkpoint: RecoveryCheckpoint = payment::decode(&checkpoint_json)?;
        self.recovery_transition(
            recovery_id,
            EventPayload::RecoveryCheckpointAccepted { checkpoint },
        )
    }
    pub fn approve_recovery_sweep(&self, plan_json: String) -> Result<String, WalletError> {
        let text = Zeroizing::new(plan_json);
        let plan: RecoverySweepPlan = payment::decode(&text)?;
        self.recovery_transition(
            plan.recovery_id.clone(),
            EventPayload::RecoverySweepApproved { plan },
        )
    }
    pub fn record_recovery_cutoff(
        &self,
        recovery_id: String,
        cutoff_json: String,
    ) -> Result<String, WalletError> {
        let cutoff: RecoveryCutoff = payment::decode(&cutoff_json)?;
        self.recovery_transition(recovery_id, EventPayload::RecoveryCutoffRecorded { cutoff })
    }
    pub fn record_recovery_sweep_batch(
        &self,
        recovery_id: String,
        intent_json: String,
    ) -> Result<String, WalletError> {
        let text = Zeroizing::new(intent_json);
        let intent = payment::validate_intent_json(&text)?;
        self.recovery_transition(
            intent.intent_id.clone(),
            EventPayload::RecoveryBatchApproved {
                recovery_id,
                intent,
            },
        )
    }
    pub fn complete_recovery_sweep(&self, resolution_json: String) -> Result<String, WalletError> {
        let text = Zeroizing::new(resolution_json);
        let resolution: RecoveryResolution = payment::decode(&text)?;
        self.recovery_transition(
            resolution.recovery_id.clone(),
            EventPayload::RecoveryCompleted { resolution },
        )
    }
    fn recovery_transition(
        &self,
        id: String,
        payload: EventPayload,
    ) -> Result<String, WalletError> {
        payment::canonical_hash(&id)?;
        let mut c = self.lock()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut loaded = self.load(&tx)?;
        if let EventPayload::RecoveryBatchApproved {
            recovery_id,
            intent,
        } = &payload
        {
            if let Some(previous) = loaded.payments.get(&id) {
                if previous.intent == *intent
                    && previous.recovery_id.as_deref() == Some(recovery_id)
                {
                    return payment::encode(&previous.record());
                }
                return Err(conflict(
                    "Recovery batch identifier already has different contents",
                ));
            }
        }
        self.append(
            &tx,
            &mut loaded,
            Event {
                schema_version: 2,
                intent_id: id.clone(),
                at: now()?,
                payload,
            },
        )?;
        let result = if let Some(p) = loaded.payments.get(&id) {
            payment::encode(&p.record())?
        } else {
            payment::encode(&loaded.recovery.as_ref().ok_or_else(corrupt)?.view())?
        };
        tx.commit().map_err(storage)?;
        Ok(result)
    }

    /// Local-only return value: it contains private intent JSON and usable
    /// proof bytes. Do not expose it in telemetry, logs or an operator API.
    pub fn get_payment(&self, intent_id: String) -> Result<String, WalletError> {
        payment::canonical_hash(&intent_id)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(storage)?;
        let loaded = self.load(&tx)?;
        loaded
            .payments
            .get(&intent_id)
            .ok_or_else(|| invalid("Private intent does not exist"))?
            .full()
    }

    pub fn list_payments(&self) -> Result<String, WalletError> {
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(storage)?;
        let loaded = self.load(&tx)?;
        let mut records: Vec<_> = loaded.payments.values().collect();
        records.sort_by_key(|p| p.sequence);
        payment::encode(&records.into_iter().map(Payment::record).collect::<Vec<_>>())
    }

    pub fn list_events(&self, intent_id: String) -> Result<String, WalletError> {
        payment::canonical_hash(&intent_id)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(storage)?;
        let loaded = self.load(&tx)?;
        if !loaded.payments.contains_key(&intent_id) {
            return Err(invalid("Private intent does not exist"));
        }
        payment::encode(
            &loaded
                .events
                .iter()
                .filter(|(_, e)| e.intent_id == intent_id)
                .map(|(seq, e)| {
                    json!({
                        "sequence":seq,"kind":e.payload.kind(),"at":e.at
                    })
                })
                .collect::<Vec<_>>(),
        )
    }
}

fn no_change(existing: &Payment, payload: &EventPayload) -> Result<bool, WalletError> {
    match payload {
        EventPayload::Ready { envelope } => {
            if existing.release_history_incomplete && existing.envelope.is_none() {
                return Err(conflict("The restored authorization may already have an unknown proof; use explicit recovery instead of silently creating another"));
            }
            payment::validate_envelope_for_intent(&existing.intent, envelope)?;
            match &existing.envelope {
                Some(old) if old != envelope => Err(conflict(
                    "A different private envelope is already durably bound to this intent",
                )),
                Some(_) => Ok(true),
                None => Ok(false),
            }
        }
        EventPayload::Released => Ok(existing.phase == Phase::Released),
        EventPayload::Unknown => Ok(matches!(
            existing.phase,
            Phase::Unknown
                | Phase::Included
                | Phase::Finalized
                | Phase::Conflict
                | Phase::Cancelled
                | Phase::Superseded
        )),
        EventPayload::Cancelled => Ok(existing.phase == Phase::Cancelled),
        EventPayload::Receipt { observation } => {
            let envelope = existing
                .envelope
                .as_ref()
                .ok_or_else(|| conflict("Private proof was not durably recorded"))?;
            validate_observation(envelope, observation)?;
            if matches!(
                existing.phase,
                Phase::Cancelled | Phase::Conflict | Phase::Superseded
            ) {
                return Err(conflict(
                    "Private intent is cancelled or requires conflict review",
                ));
            }
            if existing
                .receipt
                .as_ref()
                .is_none_or(|current| current.transaction_hash != observation.transaction_hash)
                && existing
                    .finalized_reverts
                    .get(&observation.transaction_hash)
                    .is_some_and(|prior| {
                        same_execution(prior, observation)
                            && observation.finalized_height >= observation.block_number
                    })
            {
                return Ok(true);
            }
            Ok(existing.receipt.as_ref().is_some_and(|old| {
                same_execution(old, observation)
                    && old.finalized_height >= observation.finalized_height
            }))
        }
        _ => Ok(false),
    }
}

fn apply_event(loaded: &mut Loaded, seq: u64, event: &Event) -> Result<(), WalletError> {
    if event.schema_version != 2 || event.at == 0 || event.at > i64::MAX as u64 {
        return Err(corrupt());
    }
    payment::canonical_hash(&event.intent_id)?;
    if apply_recovery_event(loaded, seq, event)? {
        return Ok(());
    }
    if let EventPayload::Approved { intent } = &event.payload {
        if loaded
            .recovery
            .as_ref()
            .is_some_and(|r| r.quarantined || r.incident)
        {
            return Err(conflict(
                "Ordinary approvals are blocked during recovery quarantine",
            ));
        }
        payment::validate_intent(intent)?;
        if intent.intent_id != event.intent_id || loaded.payments.contains_key(&event.intent_id) {
            return Err(conflict(
                "Private intent already exists or mismatches its journal identity",
            ));
        }
        if intent.created_at > event.at.saturating_add(30)
            || intent.fee.quote_valid_until < event.at
        {
            return Err(invalid(
                "Private intent was approved outside its fee quote window",
            ));
        }
        let nullifiers: HashSet<_> = intent
            .input_notes
            .iter()
            .map(|n| n.nullifier.as_str())
            .collect();
        for existing in loaded.payments.values() {
            if existing.phase == Phase::Cancelled
                || existing.intent.domain.chain_id != intent.domain.chain_id
                || existing.intent.domain.pool_contract != intent.domain.pool_contract
                || existing.intent.tree_number != intent.tree_number
            {
                continue;
            }
            if existing
                .intent
                .input_notes
                .iter()
                .any(|n| nullifiers.contains(n.nullifier.as_str()))
            {
                return Err(conflict(
                    "Private input notes are reserved or already spent by another intent",
                ));
            }
        }
        loaded.payments.insert(
            event.intent_id.clone(),
            Payment {
                intent: intent.clone(),
                envelope: None,
                phase: Phase::Approved,
                release_count: 0,
                receipt: None,
                finalized_reverts: BTreeMap::new(),
                envelope_backed_up: false,
                sequence: seq,
                recovery_id: None,
                release_history_incomplete: false,
            },
        );
        return Ok(());
    }
    let p = loaded
        .payments
        .get_mut(&event.intent_id)
        .ok_or_else(|| invalid("Private intent does not exist"))?;
    match &event.payload {
        EventPayload::Ready { envelope } => {
            if p.phase != Phase::Approved || p.envelope.is_some() || p.release_history_incomplete {
                return Err(conflict(
                    "Private intent cannot accept another ready operation",
                ));
            }
            payment::validate_envelope_for_intent(&p.intent, envelope)?;
            p.envelope = Some(envelope.clone());
            p.phase = Phase::Ready;
        }
        EventPayload::BackupExposed => {
            if p.phase != Phase::Ready || p.envelope.is_none() || p.envelope_backed_up {
                return Err(conflict(
                    "Only an unexported ready envelope can gain a backup exposure marker",
                ));
            }
            p.envelope_backed_up = true;
        }
        EventPayload::Released => {
            if p.envelope.is_none()
                || !matches!(
                    p.phase,
                    Phase::Ready | Phase::Unknown | Phase::Reorg | Phase::Failed
                )
            {
                return Err(conflict(
                    "Private intent cannot be released in its current state",
                ));
            }
            // A reverted outer transaction did not spend the private notes.
            // A later attempt of the same authorized operation gets its own
            // receipt; the prior failure remains in the encrypted event log.
            if p.phase == Phase::Failed {
                if p.receipt
                    .as_ref()
                    .is_none_or(|r| r.execution_success || r.finalized_height < r.block_number)
                {
                    return Err(conflict(
                        "A reverted outer attempt must finalize before retry",
                    ));
                }
                p.receipt = None;
            }
            p.phase = Phase::Released;
            p.release_count = p.release_count.checked_add(1).ok_or_else(corrupt)?;
        }
        EventPayload::Unknown => {
            if p.phase != Phase::Released {
                return Err(conflict(
                    "Only a released private operation can become uncertain",
                ));
            }
            p.phase = Phase::Unknown;
        }
        EventPayload::Receipt { observation } => {
            let envelope = p
                .envelope
                .as_ref()
                .ok_or_else(|| conflict("Private proof was not durably recorded"))?;
            if matches!(
                p.phase,
                Phase::Approved | Phase::Cancelled | Phase::Conflict | Phase::Superseded
            ) {
                return Err(conflict("Private intent cannot accept this observation"));
            }
            validate_observation(envelope, observation)?;
            let prior_finalized_conflict = p
                .finalized_reverts
                .get(&observation.transaction_hash)
                .is_some_and(|prior| !same_execution(prior, observation));
            if let Some(old) = &p.receipt {
                let next_outer_attempt = p.phase == Phase::Failed
                    && !old.execution_success
                    && old.finalized_height >= old.block_number
                    && old.transaction_hash != observation.transaction_hash
                    && observation.finalized_height >= old.finalized_height;
                if prior_finalized_conflict
                    || (!same_execution(old, observation) && !next_outer_attempt)
                {
                    // Preserve the earlier result and record the contradictory
                    // observation as encrypted evidence requiring review.
                    p.phase = Phase::Conflict;
                    if let Some(recovery) = &mut loaded.recovery {
                        if p.recovery_id.is_some()
                            || recovery.resolution_intent_ids.contains(&p.intent.intent_id)
                        {
                            recovery.quarantined = true;
                            recovery.incident = true;
                        }
                    }
                    p.sequence = seq;
                    return Ok(());
                }
                if observation.finalized_height < old.finalized_height {
                    return Err(conflict("Finality observation moved backwards"));
                }
            } else if prior_finalized_conflict {
                p.phase = Phase::Conflict;
                if let Some(recovery) = &mut loaded.recovery {
                    recovery.quarantined = true;
                    recovery.incident = true;
                }
                p.sequence = seq;
                return Ok(());
            }
            p.phase = if !observation.execution_success {
                Phase::Failed
            } else if observation.finalized_height >= observation.block_number {
                Phase::Finalized
            } else {
                Phase::Included
            };
            if !observation.execution_success
                && observation.finalized_height >= observation.block_number
            {
                p.finalized_reverts
                    .insert(observation.transaction_hash.clone(), observation.clone());
            }
            p.receipt = Some(observation.clone());
        }
        EventPayload::Reorg { observation } => {
            let old_finalized_revert = p.finalized_reverts.values().find(|old| {
                old.block_hash == observation.previous_block_hash
                    && old.block_number == observation.block_number
            });
            if !matches!(p.phase, Phase::Included | Phase::Finalized | Phase::Failed)
                && old_finalized_revert.is_none()
            {
                return Err(conflict(
                    "Private intent has no canonical inclusion to reorganize",
                ));
            }
            let old = old_finalized_revert
                .or(p.receipt.as_ref())
                .ok_or_else(corrupt)?;
            payment::canonical_hash(&observation.previous_block_hash)?;
            payment::canonical_hash(&observation.canonical_block_hash)?;
            if observation.previous_block_hash != old.block_hash
                || observation.block_number != old.block_number
                || observation.previous_block_hash == observation.canonical_block_hash
                || observation.finalized_height > i64::MAX as u64
            {
                return Err(conflict(
                    "Reorganization evidence does not address the observed block",
                ));
            }
            if old.finalized_height >= old.block_number {
                p.phase = Phase::Conflict;
                if let Some(recovery) = &mut loaded.recovery {
                    if p.recovery_id.is_some()
                        || recovery.resolution_intent_ids.contains(&p.intent.intent_id)
                    {
                        recovery.quarantined = true;
                        recovery.incident = true;
                    }
                }
            } else {
                p.phase = Phase::Reorg;
                p.receipt = None;
            }
        }
        EventPayload::Cancelled => {
            if !matches!(p.phase, Phase::Approved | Phase::Ready)
                || p.release_count != 0
                || p.receipt.is_some()
                || p.release_history_incomplete
                || p.envelope_backed_up
            {
                return Err(conflict(
                    "An externally released private operation cannot be cancelled locally",
                ));
            }
            p.phase = Phase::Cancelled;
        }
        EventPayload::Approved { .. }
        | EventPayload::WalletBound { .. }
        | EventPayload::RecoveryStarted { .. }
        | EventPayload::RecoveryCheckpointAccepted { .. }
        | EventPayload::RecoveryCutoffRecorded { .. }
        | EventPayload::RecoverySweepApproved { .. }
        | EventPayload::RecoveryBatchApproved { .. }
        | EventPayload::RecoveryCompleted { .. } => unreachable!(),
    }
    p.sequence = seq;
    Ok(())
}

fn apply_recovery_event(loaded: &mut Loaded, seq: u64, event: &Event) -> Result<bool, WalletError> {
    match &event.payload {
        EventPayload::WalletBound { wallet_binding } => {
            payment::canonical_hash(wallet_binding)?;
            if event.intent_id != *wallet_binding || loaded.wallet_binding.is_some() {
                return Err(conflict(
                    "Private journal wallet binding cannot be replaced",
                ));
            }
            loaded.wallet_binding = Some(wallet_binding.clone());
        }
        EventPayload::RecoveryStarted { session } => {
            session.validate(&loaded.identity)?;
            if event.intent_id != session.recovery_id
                || loaded
                    .wallet_binding
                    .as_ref()
                    .is_some_and(|binding| binding != &session.wallet_binding)
            {
                return Err(conflict(
                    "Recovery wallet binding does not match this history",
                ));
            }
            loaded.wallet_binding = Some(session.wallet_binding.clone());
            let earlier_resolution_ids = loaded
                .recovery
                .as_ref()
                .map(|r| r.resolution_intent_ids.clone())
                .unwrap_or_default();
            for payment in loaded.payments.values_mut() {
                if matches!(payment.phase, Phase::Approved | Phase::Ready) {
                    payment.release_history_incomplete = true;
                }
            }
            loaded.recovery = Some(RecoveryState {
                session: session.clone(),
                quarantined: true,
                incident: false,
                plan: None,
                cutoff: None,
                resolution_scan_digest: None,
                resolution_intent_ids: earlier_resolution_ids,
            });
        }
        EventPayload::RecoveryCheckpointAccepted { checkpoint } => {
            checkpoint.validate()?;
            let state = loaded
                .recovery
                .as_mut()
                .ok_or_else(|| conflict("No recovery is in progress"))?;
            if event.intent_id != state.session.recovery_id
                || !state.quarantined
                || state.incident
                || state.plan.is_some()
                || state.session.checkpoint.as_ref() != Some(checkpoint)
            {
                return Err(conflict(
                    "The independently retained checkpoint does not match this quarantined backup",
                ));
            }
            // This does not release or cancel any existing note reservation.
            state.quarantined = false;
        }
        EventPayload::RecoveryCutoffRecorded { cutoff } => {
            cutoff.validate()?;
            let state = loaded
                .recovery
                .as_mut()
                .ok_or_else(|| conflict("No recovery is in progress"))?;
            if event.intent_id != state.session.recovery_id
                || cutoff.recovery_id != state.session.recovery_id
                || !state.quarantined
                || state.incident
                || state.plan.is_some()
                || cutoff.observed_at < state.session.started_at
                || cutoff.observed_at > event.at.saturating_add(30)
                || state.cutoff.as_ref().is_some_and(|old| {
                    cutoff.block_number < old.block_number
                        || cutoff.observed_at < old.observed_at
                        || cutoff.domain != old.domain
                })
            {
                return Err(conflict("The durable recovery cutoff cannot move backwards or change after plan approval"));
            }
            state.cutoff = Some(cutoff.clone());
        }
        EventPayload::RecoverySweepApproved { plan } => {
            recovery::validate_recovery_plan(plan)?;
            let state = loaded
                .recovery
                .as_mut()
                .ok_or_else(|| conflict("No recovery is in progress"))?;
            let cutoff = state.cutoff.as_ref().ok_or_else(|| {
                conflict("Record the latest observed recovery cutoff before scanning")
            })?;
            if event.intent_id != state.session.recovery_id
                || plan.recovery_id != state.session.recovery_id
                || plan.initial_scan.wallet_binding != state.session.wallet_binding
                || !state.quarantined
                || state.incident
                || state.plan.is_some()
                || plan.approved_at > event.at.saturating_add(30)
                || plan.approved_at < cutoff.observed_at
                || plan.initial_scan.domain != cutoff.domain
                || plan.initial_scan.cutoff_block_number != cutoff.block_number
                || plan.initial_scan.cutoff_block_hash != cutoff.block_hash
            {
                return Err(conflict(
                    "Recovery plan is not authorized for this quarantined wallet",
                ));
            }
            state.plan = Some(plan.clone());
        }
        EventPayload::RecoveryBatchApproved {
            recovery_id,
            intent,
        } => {
            let state = loaded
                .recovery
                .as_ref()
                .ok_or_else(|| conflict("No recovery is in progress"))?;
            let plan = state
                .plan
                .as_ref()
                .ok_or_else(|| conflict("An explicit recovery self-sweep has not been approved"))?;
            if !state.quarantined
                || state.incident
                || *recovery_id != state.session.recovery_id
                || event.intent_id != intent.intent_id
                || loaded.payments.contains_key(&intent.intent_id)
                || intent.created_at > event.at.saturating_add(30)
                || intent.fee.quote_valid_until < event.at
            {
                return Err(conflict(
                    "Recovery batch is not authorized or its fee quote expired",
                ));
            }
            recovery::validate_sweep_intent(plan, intent)?;
            let mut committed_fee = recovery::fee_units(&intent.fee.fee_units)?;
            let proposed_inputs: HashSet<_> = intent
                .input_notes
                .iter()
                .map(|n| n.nullifier.as_str())
                .collect();
            for previous in loaded.payments.values() {
                if previous.recovery_id.as_deref() == Some(recovery_id)
                    && previous.phase != Phase::Cancelled
                {
                    committed_fee = committed_fee
                        .checked_add(recovery::fee_units(&previous.intent.fee.fee_units)?)
                        .ok_or_else(corrupt)?;
                    if previous.intent.tree_number == intent.tree_number
                        && previous
                            .intent
                            .input_notes
                            .iter()
                            .any(|n| proposed_inputs.contains(n.nullifier.as_str()))
                    {
                        return Err(conflict(
                            "Another current recovery batch already reserves these inputs",
                        ));
                    }
                }
                // A recovery may intentionally invalidate older released
                // proofs, but every new output must be freshly constructed.
                if intent.output_commitments.iter().any(|output| {
                    previous.intent.output_commitments.contains(output)
                        || previous
                            .intent
                            .input_notes
                            .iter()
                            .any(|n| &n.commitment == output)
                }) {
                    return Err(conflict(
                        "Recovery outputs must be fresh, not reused from earlier authorizations",
                    ));
                }
            }
            if committed_fee > recovery::fee_units(&plan.max_total_fee_units)? {
                return Err(conflict(
                    "Recovery batches exceed the user-approved total fee",
                ));
            }
            if intent.output_commitments.iter().any(|output| {
                plan.initial_scan
                    .spendable_notes
                    .iter()
                    .any(|n| &n.note.commitment == output)
            }) {
                return Err(conflict("Recovery cannot reuse an old note commitment"));
            }
            loaded.payments.insert(
                intent.intent_id.clone(),
                Payment {
                    intent: intent.clone(),
                    envelope: None,
                    phase: Phase::Approved,
                    release_count: 0,
                    receipt: None,
                    finalized_reverts: BTreeMap::new(),
                    envelope_backed_up: false,
                    sequence: seq,
                    recovery_id: Some(recovery_id.clone()),
                    release_history_incomplete: false,
                },
            );
        }
        EventPayload::RecoveryCompleted { resolution } => {
            let state = loaded
                .recovery
                .as_ref()
                .ok_or_else(|| conflict("No recovery is in progress"))?;
            let plan = state
                .plan
                .as_ref()
                .ok_or_else(|| conflict("No recovery self-sweep was approved"))?;
            recovery::validate_recovery_scan(&resolution.final_scan)?;
            let scan = &resolution.final_scan;
            let initial = &plan.initial_scan;
            if resolution.schema_version != 1
                || event.intent_id != state.session.recovery_id
                || resolution.recovery_id != state.session.recovery_id
                || resolution.plan_digest != plan.plan_digest
                || !state.quarantined
                || state.incident
                || scan.wallet_binding != state.session.wallet_binding
                || scan.domain != initial.domain
                || scan.cutoff_block_number != initial.cutoff_block_number
                || scan.cutoff_block_hash != initial.cutoff_block_hash
                || resolution.finalized_receipts.len() > recovery::MAX_RECOVERY_NOTES
            {
                return Err(conflict(
                    "Recovery resolution does not match the approved plan and canonical cutoff",
                ));
            }
            if scan.spendable_notes.iter().any(|entry| {
                entry.created_block_number <= initial.cutoff_block_number
                    || initial.spendable_notes.iter().any(|old| {
                        old.tree_number == entry.tree_number
                            && old.note.nullifier == entry.note.nullifier
                    })
            }) {
                return Err(conflict(
                    "The canonical rescan still contains uncovered pre-recovery inputs",
                ));
            }
            let mut covered = HashSet::new();
            let mut ids = HashSet::new();
            for final_receipt in &resolution.finalized_receipts {
                payment::canonical_hash(&final_receipt.intent_id)?;
                if !ids.insert(final_receipt.intent_id.clone()) {
                    return Err(conflict("Recovery repeats a receipt"));
                }
                let payment = loaded
                    .payments
                    .get(&final_receipt.intent_id)
                    .ok_or_else(|| conflict("Recovery receipt has no saved authorization"))?;
                let envelope = payment.envelope.as_ref().ok_or_else(corrupt)?;
                let receipt = &final_receipt.receipt;
                validate_observation(envelope, receipt)?;
                if payment.phase != Phase::Finalized
                    || !receipt.execution_success
                    || receipt.finalized_height < receipt.block_number
                    || receipt.block_number > scan.scanned_through_block
                    || !payment.receipt.as_ref().is_some_and(|old| {
                        same_execution(old, receipt)
                            && old.finalized_height <= receipt.finalized_height
                    })
                {
                    return Err(conflict("Recovery requires current canonical finalized receipts for exact saved operations"));
                }
                for n in &receipt.nullifiers {
                    covered.insert((payment.intent.tree_number, n.clone()));
                }
                if payment.recovery_id.as_deref() == Some(state.session.recovery_id.as_str())
                    && !scan.spendable_notes.iter().any(|entry| {
                        entry.note.commitment == payment.intent.recipient_output_commitment
                            && entry.note.units == payment.intent.amount_units
                            && entry.created_block_number == receipt.block_number
                            && entry.created_block_number > initial.cutoff_block_number
                    })
                {
                    return Err(conflict(
                        "The rescan does not recover the exact fresh self-sweep output",
                    ));
                }
            }
            if initial
                .spendable_notes
                .iter()
                .any(|entry| !covered.contains(&(entry.tree_number, entry.note.nullifier.clone())))
            {
                return Err(conflict("Every selected old input must be consumed by a finalized self-sweep or reconciled prior authorization"));
            }
            // Old competing authorizations can no longer spend these inputs.
            // Preserve their envelope/receipt evidence and mark them honestly;
            // never label them successful merely because a nullifier is spent.
            for p in loaded.payments.values_mut() {
                if !matches!(
                    p.phase,
                    Phase::Finalized | Phase::Cancelled | Phase::Superseded
                ) && p
                    .intent
                    .input_notes
                    .iter()
                    .any(|n| covered.contains(&(p.intent.tree_number, n.nullifier.clone())))
                {
                    p.phase = Phase::Superseded;
                    p.sequence = seq;
                }
            }
            let state = loaded.recovery.as_mut().ok_or_else(corrupt)?;
            state.quarantined = false;
            state.resolution_scan_digest = Some(scan.scan_digest.clone());
            state.resolution_intent_ids.extend(ids);
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn validate_observation(
    envelope: &PrivateEnvelope,
    r: &PrivateReceiptObservation,
) -> Result<(), WalletError> {
    for hash in [
        &r.operation_digest,
        &r.transaction_hash,
        &r.block_hash,
        &r.canonical_block_hash,
    ] {
        payment::canonical_hash(hash)?;
    }
    if r.operation_digest != envelope.operation_digest
        || r.chain_id != envelope.domain.chain_id
        || r.pool_contract != envelope.domain.pool_contract
        || r.block_hash != r.canonical_block_hash
        || r.block_number == 0
        || r.block_number > i64::MAX as u64
        || r.finalized_height > i64::MAX as u64
    {
        return Err(invalid(
            "Private receipt is not for this operation on its observed canonical chain",
        ));
    }
    let fee = crate::validation::decimal(&r.fee_units_charged)?;
    let native = crate::validation::decimal(&r.native_fee_wei)?;
    if native > crate::validation::decimal(&envelope.fee.max_native_fee_wei)? {
        return Err(conflict(
            "Observed native charge exceeds the approved exposure",
        ));
    }
    if r.execution_success {
        if r.nullifiers != envelope.nullifiers
            || r.output_commitments != envelope.output_commitments
            || r.fee_units_charged != envelope.fee.fee_units
        {
            return Err(invalid(
                "Private receipt does not contain the exact expected outputs and fee",
            ));
        }
    } else if !r.nullifiers.is_empty() || !r.output_commitments.is_empty() || !fee.is_zero() {
        return Err(invalid(
            "A reverted atomic private transfer cannot create outputs or earn the provider fee",
        ));
    }
    Ok(())
}

fn same_execution(a: &PrivateReceiptObservation, b: &PrivateReceiptObservation) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    a.finalized_height = 0;
    b.finalized_height = 0;
    a == b
}

#[cfg(test)]
mod resource_limits {
    use super::*;

    #[test]
    fn encrypted_history_budget_is_checked_without_overflow() {
        let prior = MAX_ENCRYPTED_JOURNAL_BYTES - EVENT_STORAGE_OVERHEAD - 16;
        assert_eq!(
            checked_history_bytes(prior, 16).unwrap(),
            MAX_ENCRYPTED_JOURNAL_BYTES
        );
        assert!(checked_history_bytes(prior, 17).is_err());
        assert!(checked_history_bytes(usize::MAX, 16).is_err());
        assert!(checked_history_bytes(0, usize::MAX).is_err());
    }
}
