//! Durable, append-only payment evidence with a validated current projection.
//!
//! This is a local transaction journal, not a custodial financial ledger. The
//! chain is the balance authority. No method invents balances, signs, broadcasts,
//! retries a payment or changes a signed payment's nonce. Persistence must finish
//! before the caller broadcasts the exact bytes returned by `record_signed`.
//!
//! Append-only is enforced by this API and SQLite triggers. It is not a claim of
//! tamper-proof storage against an attacker who controls the app or filesystem.

use crate::crypto::{self, Asset, PreparedPayment, SignedPayment};
use crate::validation::{
    additional_fee_exceeded, address, check_signed, decimal, digest, fixed_hex, inspected,
    verify_receipt, RpcReceipt, MAX_RECEIPT_JSON,
};
use crate::WalletError;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: i64 = 4;
const LEGACY_ROLLUP_SCHEMA_VERSION: i64 = 3;
const LEGACY_MAINNET_SCHEMA_VERSION: i64 = 2;
const APPLICATION_ID: i64 = 0x4c57414c; // LWAL.
const MAX_PAYMENT_JSON: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Prepared,
    Signed,
    Unknown,
    Pending,
    Included,
    Finalized,
    Unresolved,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Execution {
    Unknown,
    Success,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Inclusion {
    Unseen,
    Included,
    Finalized,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PaymentRecord {
    intent_id: String,
    chain_id: u64,
    from: String,
    to: String,
    asset: Asset,
    amount: String,
    amount_units: String,
    nonce: u64,
    phase: Phase,
    execution: Execution,
    inclusion: Inclusion,
    hash: Option<String>,
    raw_transaction: Option<String>,
    block_number: Option<u64>,
    block_hash: Option<String>,
    actual_fee_wei: Option<String>,
    actual_execution_fee_wei: Option<String>,
    actual_l1_fee_wei: Option<String>,
    actual_operator_fee_wei: Option<String>,
    additional_fee_exceeded_estimate: Option<bool>,
    max_fee_wei: String,
    l1_fee_estimate_wei: String,
    operator_fee_estimate_wei: String,
    estimated_total_fee_wei: String,
    fee_reserve_wei: String,
    approval_digest: String,
    created_at: u64,
    updated_at: u64,
    receipt_status: Option<u8>,
    receipt_fingerprint: Option<String>,
    resolution_note: Option<String>,
}

struct StoredPayment {
    prepared: PreparedPayment,
    signed: Option<SignedPayment>,
    record: PaymentRecord,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BalanceValues {
    #[serde(
        rename = "ETH",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_balance"
    )]
    eth: Option<String>,
    #[serde(
        rename = "USDC",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_balance"
    )]
    usdc: Option<String>,
    #[serde(
        rename = "EURC",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_balance"
    )]
    eurc: Option<String>,
    #[serde(
        rename = "USDT",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_balance"
    )]
    usdt: Option<String>,
    #[serde(
        rename = "AVAX",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_balance"
    )]
    avax: Option<String>,
    #[serde(
        rename = "POL",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_balance"
    )]
    pol: Option<String>,
    #[serde(
        rename = "BNB",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_balance"
    )]
    bnb: Option<String>,
}

fn present_balance<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    // Absence is allowed only by the selected chain's asset policy. An explicit
    // null must not masquerade as an observed amount or erase a required field.
    String::deserialize(deserializer).map(Some)
}

impl BalanceValues {
    fn validate_for_chain(&self, chain_id: u64) -> Result<(), WalletError> {
        if !crypto::supported_chain(chain_id) {
            return Err(invalid(
                "Balance observation is not for a supported mainnet",
            ));
        }
        for (asset, observed) in [
            (Asset::Eth, &self.eth),
            (Asset::Usdc, &self.usdc),
            (Asset::Eurc, &self.eurc),
            (Asset::Usdt, &self.usdt),
            (Asset::Avax, &self.avax),
            (Asset::Pol, &self.pol),
            (Asset::Bnb, &self.bnb),
        ] {
            match (asset.supported_on(chain_id), observed) {
                (true, Some(amount)) => {
                    decimal(amount)?;
                }
                (false, None) => {}
                // Existing Ethereum observations precede native USDT support.
                // Absence remains unknown; never synthesize a zero or rewrite evidence.
                (true, None) if chain_id == crypto::CHAIN_ID && asset == Asset::Usdt => {}
                _ => return Err(invalid("Balance assets do not match the selected network")),
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BalanceSnapshot {
    chain_id: u64,
    address: String,
    block_number: u64,
    block_hash: String,
    balances: BalanceValues,
}

/// One protected SQLite database per network and wallet account. The first
/// payment or balance observation binds it to that chain ID and address.
pub struct Journal {
    connection: Mutex<Connection>,
}

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

fn storage(_: rusqlite::Error) -> WalletError {
    WalletError::Storage {
        reason: "A durable journal operation failed; no unrecorded transaction may be broadcast"
            .into(),
    }
}

fn corrupt() -> WalletError {
    WalletError::Storage {
        reason: "Journal data failed validation; stop broadcasting and investigate".into(),
    }
}

fn now() -> Result<u64, WalletError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| invalid("The device clock is invalid"))
}

fn encode<T: Serialize>(value: &T) -> Result<String, WalletError> {
    serde_json::to_string(value).map_err(|_| corrupt())
}

fn decode<T: for<'a> Deserialize<'a>>(input: &str, maximum: usize) -> Result<T, WalletError> {
    if input.len() > maximum {
        return Err(invalid("Journal input exceeds its size limit"));
    }
    serde_json::from_str(input)
        .map_err(|_| invalid("Journal input is malformed or has unsupported fields"))
}

fn validate_intent(intent: &str) -> Result<(), WalletError> {
    if intent.is_empty()
        || intent.len() > 128
        || !intent
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err(invalid("Invalid payment intent identifier"));
    }
    Ok(())
}

fn new_record(prepared: &PreparedPayment) -> PaymentRecord {
    let request = &prepared.request;
    PaymentRecord {
        intent_id: request.intent_id.clone(),
        chain_id: request.chain_id,
        from: request.from.clone(),
        to: request.to.clone(),
        asset: request.asset,
        amount: request.amount.clone(),
        amount_units: prepared.amount_units.clone(),
        nonce: request.nonce,
        phase: Phase::Prepared,
        execution: Execution::Unknown,
        inclusion: Inclusion::Unseen,
        hash: None,
        raw_transaction: None,
        block_number: None,
        block_hash: None,
        actual_fee_wei: None,
        actual_execution_fee_wei: None,
        actual_l1_fee_wei: None,
        actual_operator_fee_wei: None,
        additional_fee_exceeded_estimate: None,
        max_fee_wei: prepared.max_fee_wei.clone(),
        l1_fee_estimate_wei: request.l1_fee_estimate_wei.clone(),
        operator_fee_estimate_wei: request.operator_fee_estimate_wei.clone(),
        estimated_total_fee_wei: prepared.estimated_total_fee_wei.clone(),
        fee_reserve_wei: prepared.fee_reserve_wei.clone(),
        approval_digest: prepared.approval_digest.clone(),
        created_at: prepared.created_at,
        updated_at: prepared.created_at,
        receipt_status: None,
        receipt_fingerprint: None,
        resolution_note: None,
    }
}

fn validate_record(stored: &StoredPayment, active: i64) -> Result<(), WalletError> {
    let record = &stored.record;
    let base = new_record(&stored.prepared);
    if record.intent_id != base.intent_id
        || record.chain_id != base.chain_id
        || record.from != base.from
        || record.to != base.to
        || record.asset != base.asset
        || record.amount != base.amount
        || record.amount_units != base.amount_units
        || record.nonce != base.nonce
        || record.max_fee_wei != base.max_fee_wei
        || record.l1_fee_estimate_wei != base.l1_fee_estimate_wei
        || record.operator_fee_estimate_wei != base.operator_fee_estimate_wei
        || record.estimated_total_fee_wei != base.estimated_total_fee_wei
        || record.fee_reserve_wei != base.fee_reserve_wei
        || record.approval_digest != base.approval_digest
        || record.created_at != base.created_at
        || record.updated_at < record.created_at
        || active != i64::from(record.phase != Phase::Cancelled)
    {
        return Err(corrupt());
    }
    match &stored.signed {
        Some(signed) => {
            let actual = inspected(signed).map_err(|_| corrupt())?;
            check_signed(&stored.prepared, signed, &actual).map_err(|_| corrupt())?;
            if record.hash.as_ref() != Some(&signed.hash)
                || record.raw_transaction.as_ref() != Some(&signed.raw_transaction)
                || matches!(record.phase, Phase::Prepared | Phase::Cancelled)
            {
                return Err(corrupt());
            }
        }
        None => {
            if record.hash.is_some()
                || record.raw_transaction.is_some()
                || !matches!(record.phase, Phase::Prepared | Phase::Cancelled)
            {
                return Err(corrupt());
            }
        }
    }
    match record.inclusion {
        Inclusion::Unseen => {
            if record.block_number.is_some()
                || record.block_hash.is_some()
                || record.actual_fee_wei.is_some()
                || record.actual_execution_fee_wei.is_some()
                || record.actual_l1_fee_wei.is_some()
                || record.actual_operator_fee_wei.is_some()
                || record.additional_fee_exceeded_estimate.is_some()
                || record.receipt_status.is_some()
                || record.receipt_fingerprint.is_some()
                || record.execution != Execution::Unknown
                || matches!(record.phase, Phase::Included | Phase::Finalized)
            {
                return Err(corrupt());
            }
        }
        Inclusion::Included | Inclusion::Finalized => {
            if record.block_number.is_none()
                || record.block_hash.is_none()
                || record.actual_fee_wei.is_none()
                || record.actual_execution_fee_wei.is_none()
                || record.actual_l1_fee_wei.is_none()
                || record.actual_operator_fee_wei.is_none()
                || record.additional_fee_exceeded_estimate.is_none()
                || record.receipt_status.is_none()
                || record.receipt_fingerprint.is_none()
                || !matches!(
                    record.phase,
                    Phase::Included | Phase::Finalized | Phase::Unresolved
                )
            {
                return Err(corrupt());
            }
            if record.phase == Phase::Included && record.inclusion != Inclusion::Included
                || record.phase == Phase::Finalized && record.inclusion != Inclusion::Finalized
            {
                return Err(corrupt());
            }
            fixed_hex(record.block_hash.as_deref().ok_or_else(corrupt)?, 32)
                .map_err(|_| corrupt())?;
            let actual = decimal(record.actual_fee_wei.as_deref().ok_or_else(corrupt)?)
                .map_err(|_| corrupt())?;
            let execution_fee = decimal(
                record
                    .actual_execution_fee_wei
                    .as_deref()
                    .ok_or_else(corrupt)?,
            )
            .map_err(|_| corrupt())?;
            let l1_fee = decimal(record.actual_l1_fee_wei.as_deref().ok_or_else(corrupt)?)
                .map_err(|_| corrupt())?;
            let operator_fee = decimal(
                record
                    .actual_operator_fee_wei
                    .as_deref()
                    .ok_or_else(corrupt)?,
            )
            .map_err(|_| corrupt())?;
            let exceeded = l1_fee > decimal(&record.l1_fee_estimate_wei).map_err(|_| corrupt())?
                || operator_fee
                    > decimal(&record.operator_fee_estimate_wei).map_err(|_| corrupt())?;
            if execution_fee > decimal(&record.max_fee_wei).map_err(|_| corrupt())?
                || execution_fee
                    .checked_add(l1_fee)
                    .and_then(|fee| fee.checked_add(operator_fee))
                    != Some(actual)
                || record.additional_fee_exceeded_estimate != Some(exceeded)
                || (!crypto::has_separate_rollup_fees(record.chain_id)
                    && (!l1_fee.is_zero() || !operator_fee.is_zero()))
            {
                return Err(corrupt());
            }
            match (record.receipt_status, record.execution) {
                (Some(0), Execution::Failed)
                | (Some(1), Execution::Success)
                | (Some(1), Execution::Unknown) => {}
                _ => return Err(corrupt()),
            }
        }
    }
    Ok(())
}

fn load_payment(
    connection: &Connection,
    intent: &str,
) -> Result<Option<StoredPayment>, WalletError> {
    let row: Option<(String, Option<String>, String, i64)> = connection.query_row(
        "SELECT prepared_json, signed_json, record_json, active FROM payments WHERE intent_id = ?1",
        [intent], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).optional().map_err(storage)?;
    row.map(|(prepared_json, signed_json, record_json, active)| {
        let prepared = crypto::validate_prepared(&prepared_json).map_err(|_| corrupt())?;
        let signed = signed_json
            .map(|value| decode(&value, MAX_PAYMENT_JSON).map_err(|_| corrupt()))
            .transpose()?;
        let record = decode(&record_json, MAX_PAYMENT_JSON).map_err(|_| corrupt())?;
        let stored = StoredPayment {
            prepared,
            signed,
            record,
        };
        validate_record(&stored, active)?;
        validate_identity(connection, stored.record.chain_id, &stored.record.from)?;
        Ok(stored)
    })
    .transpose()
}

fn required_payment(connection: &Connection, intent: &str) -> Result<StoredPayment, WalletError> {
    load_payment(connection, intent)?.ok_or_else(|| conflict("Payment intent is not recorded"))
}

fn save_record(connection: &Connection, record: &PaymentRecord) -> Result<(), WalletError> {
    let changed = connection
        .execute(
            "UPDATE payments SET record_json = ?2, active = ?3 WHERE intent_id = ?1",
            params![
                record.intent_id,
                encode(record)?,
                i64::from(record.phase != Phase::Cancelled)
            ],
        )
        .map_err(storage)?;
    if changed != 1 {
        return Err(corrupt());
    }
    Ok(())
}

fn append_event(
    connection: &Connection,
    intent: &str,
    kind: &str,
    payload: &Value,
) -> Result<(), WalletError> {
    connection.execute(
        "INSERT INTO payment_events(intent_id, kind, observed_at, payload_json) VALUES (?1, ?2, ?3, ?4)",
        params![intent, kind, i64::try_from(now()?).map_err(|_| corrupt())?, encode(payload)?],
    ).map_err(storage)?;
    Ok(())
}

fn validate_journal_network(connection: &Connection, chain_id: u64) -> Result<(), WalletError> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(storage)?;
    let supported = match version {
        LEGACY_MAINNET_SCHEMA_VERSION => {
            matches!(chain_id, crypto::CHAIN_ID | crypto::BASE_CHAIN_ID)
        }
        LEGACY_ROLLUP_SCHEMA_VERSION => matches!(
            chain_id,
            crypto::CHAIN_ID
                | crypto::BASE_CHAIN_ID
                | crypto::OP_CHAIN_ID
                | crypto::ARBITRUM_CHAIN_ID
        ),
        SCHEMA_VERSION => crypto::supported_chain(chain_id),
        _ => false,
    };
    if !supported {
        return Err(invalid("Unsupported journal network"));
    }
    Ok(())
}

fn bind_account(connection: &Connection, chain_id: u64, account: &str) -> Result<(), WalletError> {
    validate_journal_network(connection, chain_id)?;
    let account = address(account)?;
    let bound: Option<String> = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = 'wallet_address'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    if let Some(bound) = bound {
        if bound != account {
            return Err(conflict(
                "This journal belongs to a different wallet account",
            ));
        }
    } else {
        connection
            .execute(
                "INSERT INTO metadata(key, value) VALUES ('wallet_address', ?1)",
                [&account],
            )
            .map_err(storage)?;
    }
    let bound_chain: Option<String> = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = 'chain_id'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    match bound_chain {
        Some(bound) if bound != chain_id.to_string() => {
            return Err(conflict("This journal belongs to a different network"))
        }
        Some(_) => {}
        None => {
            connection
                .execute(
                    "INSERT INTO metadata(key, value) VALUES ('chain_id', ?1)",
                    [chain_id.to_string()],
                )
                .map_err(storage)?;
        }
    }
    Ok(())
}

fn validate_identity(
    connection: &Connection,
    chain_id: u64,
    account: &str,
) -> Result<(), WalletError> {
    validate_journal_network(connection, chain_id).map_err(|_| corrupt())?;
    let bound: String = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = 'wallet_address'",
            [],
            |row| row.get(0),
        )
        .map_err(|_| corrupt())?;
    let network: String = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = 'chain_id'",
            [],
            |row| row.get(0),
        )
        .map_err(|_| corrupt())?;
    if bound != address(account).map_err(|_| corrupt())? || network != chain_id.to_string() {
        return Err(corrupt());
    }
    Ok(())
}

impl Journal {
    pub fn open(path: String) -> Result<Self, WalletError> {
        let path = Path::new(&path);
        if !path.is_absolute() || path.as_os_str().len() > 4096 {
            return Err(invalid("Journal requires an absolute local file path"));
        }
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(invalid("Journal path must be a regular file"));
            }
        } else {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(path) {
                Ok(file) => {
                    file.sync_all().map_err(|_| corrupt())?;
                    // The filename itself must also survive a power loss. The
                    // protected parent directory is created by the native app.
                    #[cfg(unix)]
                    fs::File::open(path.parent().ok_or_else(corrupt)?)
                        .and_then(|directory| directory.sync_all())
                        .map_err(|_| corrupt())?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => {
                    return Err(WalletError::Storage {
                        reason: "Cannot create the durable journal file".into(),
                    })
                }
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|_| corrupt())?;
        }
        let mut connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_FULL_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(storage)?;
        connection
            .busy_timeout(Duration::from_secs(3))
            .map_err(storage)?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
             PRAGMA trusted_schema = OFF;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA fullfsync = ON;
             PRAGMA checkpoint_fullfsync = ON;
             PRAGMA wal_autocheckpoint = 1000;",
            )
            .map_err(storage)?;
        let mode: String = connection
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .map_err(storage)?;
        let sync: i64 = connection
            .query_row("PRAGMA synchronous", [], |r| r.get(0))
            .map_err(storage)?;
        if mode != "wal" || sync != 2 {
            return Err(corrupt());
        }
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let version: i64 = tx
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(storage)?;
        let app_id: i64 = tx
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .map_err(storage)?;
        if version == 0 && app_id == 0 {
            let objects: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                    [],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if objects != 0 {
                return Err(invalid("File is not an empty wallet journal"));
            }
            tx.execute_batch(SCHEMA).map_err(storage)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(storage)?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)
                .map_err(storage)?;
        // Existing per-network schema-2/3 journals retain their original schema,
        // chain binding, prepared/signed bytes and event history. Only newly
        // created journals need the wider (still fixed) chain constraint.
        } else if ![
            LEGACY_MAINNET_SCHEMA_VERSION,
            LEGACY_ROLLUP_SCHEMA_VERSION,
            SCHEMA_VERSION,
        ]
        .contains(&version)
            || app_id != APPLICATION_ID
        {
            return Err(WalletError::Storage {
                reason: "Unsupported journal format; existing records were preserved".into(),
            });
        }
        tx.commit().map_err(storage)?;
        let integrity: String = connection
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(storage)?;
        if integrity != "ok" {
            return Err(corrupt());
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, WalletError> {
        self.connection.lock().map_err(|_| WalletError::Storage {
            reason: "Journal lock failed; restart before proceeding".into(),
        })
    }

    pub fn record_prepared(&self, prepared_json: String) -> Result<String, WalletError> {
        let prepared = crypto::validate_prepared(&prepared_json)?;
        validate_intent(&prepared.request.intent_id)?;
        let canonical = encode(&prepared)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        if let Some(existing) = load_payment(&tx, &prepared.request.intent_id)? {
            if existing.prepared != prepared {
                return Err(conflict(
                    "Payment intent already exists with different approved details",
                ));
            }
            return encode(&existing.record);
        }
        bind_account(&tx, prepared.request.chain_id, &prepared.request.from)?;
        let sender = address(&prepared.request.from)?;
        let reserved: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM payments WHERE chain_id = ?1 AND sender = ?2 AND nonce = ?3 AND active = 1)",
            params![prepared.request.chain_id as i64, sender, prepared.request.nonce.to_string()], |r| r.get(0),
        ).map_err(storage)?;
        if reserved {
            return Err(conflict(
                "This nonce is already reserved by another local payment",
            ));
        }
        let mut record = new_record(&prepared);
        record.updated_at = now()?.max(record.created_at);
        tx.execute(
            "INSERT INTO payments(intent_id, chain_id, sender, nonce, active, prepared_json, signed_json, record_json) VALUES (?1, ?2, ?3, ?4, 1, ?5, NULL, ?6)",
            params![record.intent_id, record.chain_id as i64, sender, record.nonce.to_string(), canonical, encode(&record)?],
        ).map_err(storage)?;
        append_event(
            &tx,
            &record.intent_id,
            "prepared",
            &serde_json::to_value(&prepared).map_err(|_| corrupt())?,
        )?;
        tx.commit().map_err(storage)?;
        encode(&record)
    }

    pub fn record_signed(&self, signed_json: String) -> Result<String, WalletError> {
        let signed: SignedPayment = decode(&signed_json, MAX_PAYMENT_JSON)?;
        validate_intent(&signed.intent_id)?;
        let actual = inspected(&signed)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut stored = required_payment(&tx, &signed.intent_id)?;
        check_signed(&stored.prepared, &signed, &actual)?;
        if let Some(existing) = stored.signed {
            if existing != signed {
                return Err(conflict(
                    "A different signed transaction is already recorded for this intent",
                ));
            }
            return encode(&stored.record);
        }
        if stored.record.phase != Phase::Prepared {
            return Err(conflict(
                "Only a live prepared payment may receive a signature",
            ));
        }
        stored.record.phase = Phase::Signed;
        stored.record.hash = Some(signed.hash.clone());
        stored.record.raw_transaction = Some(signed.raw_transaction.clone());
        stored.record.updated_at = now()?.max(stored.record.updated_at);
        tx.execute(
            "UPDATE payments SET signed_json = ?2 WHERE intent_id = ?1",
            params![signed.intent_id, encode(&signed)?],
        )
        .map_err(storage)?;
        save_record(&tx, &stored.record)?;
        append_event(
            &tx,
            &signed.intent_id,
            "signed",
            &serde_json::to_value(&signed).map_err(|_| corrupt())?,
        )?;
        // This commit is the essential broadcast gate. The caller must use only
        // the returned saved raw_transaction, and must not broadcast on error.
        tx.commit().map_err(storage)?;
        encode(&stored.record)
    }

    pub fn mark_submission(
        &self,
        intent_id: String,
        outcome: String,
    ) -> Result<String, WalletError> {
        validate_intent(&intent_id)?;
        let desired = match outcome.as_str() {
            "pending" => Phase::Pending,
            "unknown" => Phase::Unknown,
            _ => return Err(invalid("Submission outcome must be pending or unknown")),
        };
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut stored = required_payment(&tx, &intent_id)?;
        if stored.signed.is_none() {
            return Err(conflict("Submission cannot precede durable signing"));
        }
        // Later RPC failures cannot erase earlier acceptance, receipt evidence,
        // finality or an unresolved discrepancy.
        if stored.record.phase == desired
            || !matches!(
                stored.record.phase,
                Phase::Signed | Phase::Unknown | Phase::Pending
            )
            || stored.record.phase == Phase::Pending && desired == Phase::Unknown
        {
            return encode(&stored.record);
        }
        let before = stored.record.phase;
        stored.record.phase = desired;
        stored.record.updated_at = now()?.max(stored.record.updated_at);
        save_record(&tx, &stored.record)?;
        append_event(
            &tx,
            &intent_id,
            "submission",
            &json!({"before":before,"outcome":outcome,"hash":stored.record.hash}),
        )?;
        tx.commit().map_err(storage)?;
        encode(&stored.record)
    }

    pub fn observe_receipt(
        &self,
        intent_id: String,
        receipt_json: String,
        canonical_block_hash: String,
        finalized_height: u64,
    ) -> Result<String, WalletError> {
        validate_intent(&intent_id)?;
        let receipt: RpcReceipt = decode(&receipt_json, MAX_RECEIPT_JSON)?;
        let canonical_block_hash = fixed_hex(&canonical_block_hash, 32)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut stored = required_payment(&tx, &intent_id)?;
        let signed = stored
            .signed
            .as_ref()
            .ok_or_else(|| conflict("A receipt cannot precede a durable signature"))?;
        let actual = inspected(signed)?;
        let evidence = verify_receipt(receipt, &canonical_block_hash, &actual)?;
        let fingerprint = digest(&encode(&evidence)?);
        if let Some(previous_hash) = &stored.record.block_hash {
            if previous_hash != &evidence.block_hash {
                return Err(conflict(
                    "Correct the prior inclusion with mark_reorg before recording another block",
                ));
            }
            if stored.record.receipt_fingerprint.as_ref() != Some(&fingerprint) {
                return Err(conflict("Providers reported different receipt contents for the same block; existing evidence was preserved"));
            }
        }
        let inclusion = if evidence.block_number <= finalized_height
            || stored.record.inclusion == Inclusion::Finalized
        {
            Inclusion::Finalized
        } else {
            Inclusion::Included
        };
        if stored.record.receipt_fingerprint.as_ref() == Some(&fingerprint)
            && stored.record.inclusion == inclusion
        {
            return encode(&stored.record);
        }
        let execution = if evidence.status == 0 {
            Execution::Failed
        } else if evidence.expected_transfer_observed {
            Execution::Success
        } else {
            Execution::Unknown
        };
        stored.record.execution = execution;
        stored.record.inclusion = inclusion;
        stored.record.phase = if execution == Execution::Unknown {
            Phase::Unresolved
        } else if inclusion == Inclusion::Finalized {
            Phase::Finalized
        } else {
            Phase::Included
        };
        stored.record.block_number = Some(evidence.block_number);
        stored.record.block_hash = Some(evidence.block_hash.clone());
        stored.record.actual_fee_wei = Some(evidence.actual_fee_wei.clone());
        stored.record.actual_execution_fee_wei = Some(evidence.actual_execution_fee_wei.clone());
        stored.record.actual_l1_fee_wei = Some(evidence.actual_l1_fee_wei.clone());
        stored.record.actual_operator_fee_wei = Some(evidence.actual_operator_fee_wei.clone());
        stored.record.additional_fee_exceeded_estimate =
            Some(additional_fee_exceeded(&evidence, signed)?);
        stored.record.receipt_status = Some(evidence.status);
        stored.record.receipt_fingerprint = Some(fingerprint);
        stored.record.resolution_note = if execution == Execution::Unknown {
            Some("The transaction succeeded, but the receipt did not contain exactly the expected token transfer. Payment outcome is unresolved.".into())
        } else {
            None
        };
        stored.record.updated_at = now()?.max(stored.record.updated_at);
        save_record(&tx, &stored.record)?;
        append_event(
            &tx,
            &intent_id,
            "receipt",
            &json!({
                "canonical_block_hash":canonical_block_hash,"finalized_height":finalized_height,
                "evidence":evidence,"phase":stored.record.phase,"execution":stored.record.execution,"inclusion":inclusion
            }),
        )?;
        tx.commit().map_err(storage)?;
        encode(&stored.record)
    }

    pub fn mark_reorg(
        &self,
        intent_id: String,
        previous_block_hash: String,
    ) -> Result<String, WalletError> {
        validate_intent(&intent_id)?;
        let previous_block_hash = fixed_hex(&previous_block_hash, 32)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut stored = required_payment(&tx, &intent_id)?;
        if stored.signed.is_none() {
            return Err(conflict("Unsigned payment has no inclusion to correct"));
        }
        if stored.record.block_hash.is_none() {
            let repeated: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM payment_events WHERE intent_id = ?1 AND kind = 'reorg' AND json_extract(payload_json, '$.previous_block_hash') = ?2)",
                params![intent_id, previous_block_hash], |r| r.get(0),
            ).map_err(storage)?;
            if repeated {
                return encode(&stored.record);
            }
        }
        if stored.record.block_hash.as_ref() != Some(&previous_block_hash) {
            return Err(conflict(
                "Reorg correction does not match the currently recorded inclusion",
            ));
        }
        let previous = stored.record.clone();
        stored.record.phase = if previous.inclusion == Inclusion::Finalized {
            Phase::Unresolved
        } else {
            Phase::Unknown
        };
        stored.record.execution = Execution::Unknown;
        stored.record.inclusion = Inclusion::Unseen;
        stored.record.block_number = None;
        stored.record.block_hash = None;
        stored.record.actual_fee_wei = None;
        stored.record.actual_execution_fee_wei = None;
        stored.record.actual_l1_fee_wei = None;
        stored.record.actual_operator_fee_wei = None;
        stored.record.additional_fee_exceeded_estimate = None;
        stored.record.receipt_status = None;
        stored.record.receipt_fingerprint = None;
        stored.record.resolution_note = Some(if previous.inclusion == Inclusion::Finalized {
            "Previously finalized inclusion disagrees with the chain observation. Stop automatic activity and investigate provider or consensus failure."
        } else { "The previously observed block is no longer canonical. The original signed payment remains unresolved." }.into());
        stored.record.updated_at = now()?.max(stored.record.updated_at);
        save_record(&tx, &stored.record)?;
        append_event(
            &tx,
            &intent_id,
            "reorg",
            &json!({"previous_block_hash":previous_block_hash,"previous_record":previous}),
        )?;
        tx.commit().map_err(storage)?;
        encode(&stored.record)
    }

    pub fn cancel_prepared(&self, intent_id: String) -> Result<String, WalletError> {
        validate_intent(&intent_id)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut stored = required_payment(&tx, &intent_id)?;
        if stored.record.phase == Phase::Cancelled {
            return encode(&stored.record);
        }
        if stored.signed.is_some() || stored.record.phase != Phase::Prepared {
            return Err(conflict("A signed payment cannot be cancelled locally"));
        }
        stored.record.phase = Phase::Cancelled;
        stored.record.updated_at = now()?.max(stored.record.updated_at);
        save_record(&tx, &stored.record)?;
        append_event(
            &tx,
            &intent_id,
            "cancelled_unsigned",
            &json!({"nonce":stored.record.nonce}),
        )?;
        tx.commit().map_err(storage)?;
        encode(&stored.record)
    }

    pub fn get_payment(&self, intent_id: String) -> Result<String, WalletError> {
        validate_intent(&intent_id)?;
        let connection = self.lock()?;
        encode(&required_payment(&connection, &intent_id)?.record)
    }

    pub fn list_payments(&self) -> Result<String, WalletError> {
        let connection = self.lock()?;
        let intents = {
            let mut statement = connection
                .prepare("SELECT intent_id FROM payments ORDER BY rowid DESC")
                .map_err(storage)?;
            let rows = statement
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(storage)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(storage)?
        };
        let records = intents
            .iter()
            .map(|id| required_payment(&connection, id).map(|p| p.record))
            .collect::<Result<Vec<_>, _>>()?;
        encode(&records)
    }

    pub fn list_events(&self, intent_id: String) -> Result<String, WalletError> {
        validate_intent(&intent_id)?;
        let connection = self.lock()?;
        required_payment(&connection, &intent_id)?;
        let mut statement = connection.prepare("SELECT sequence, kind, observed_at, payload_json FROM payment_events WHERE intent_id = ?1 ORDER BY sequence").map_err(storage)?;
        let rows = statement
            .query_map([&intent_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(storage)?;
        let mut events = Vec::new();
        for row in rows {
            let (sequence, kind, observed_at, payload) = row.map_err(storage)?;
            let payload: Value = serde_json::from_str(&payload).map_err(|_| corrupt())?;
            events.push(json!({"sequence":sequence,"intent_id":intent_id,"kind":kind,"observed_at":observed_at,"payload":payload}));
        }
        encode(&events)
    }

    pub fn record_balance(&self, snapshot_json: String) -> Result<String, WalletError> {
        let mut snapshot: BalanceSnapshot = decode(&snapshot_json, MAX_PAYMENT_JSON)?;
        snapshot.balances.validate_for_chain(snapshot.chain_id)?;
        snapshot.address = address(&snapshot.address)?;
        snapshot.block_hash = fixed_hex(&snapshot.block_hash, 32)?;
        let canonical = encode(&snapshot)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        bind_account(&tx, snapshot.chain_id, &snapshot.address)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT snapshot_json FROM balance_snapshots WHERE block_hash = ?1",
                [&snapshot.block_hash],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        if let Some(existing) = existing {
            if existing != canonical {
                return Err(conflict(
                    "Balances disagree at the same block; existing evidence was preserved",
                ));
            }
            return Ok(existing);
        }
        let previous: Option<String> = tx
            .query_row(
                "SELECT snapshot_json FROM balance_snapshots ORDER BY sequence DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        if let Some(previous) = previous {
            let previous: BalanceSnapshot =
                decode(&previous, MAX_PAYMENT_JSON).map_err(|_| corrupt())?;
            if snapshot.block_number < previous.block_number {
                return Err(conflict("Older balance observation cannot replace a newer one; reconcile the chain context"));
            }
        }
        tx.execute(
            "INSERT INTO balance_snapshots(block_hash, observed_at, snapshot_json) VALUES (?1, ?2, ?3)",
            params![snapshot.block_hash, i64::try_from(now()?).map_err(|_| corrupt())?, canonical],
        ).map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(canonical)
    }

    pub fn latest_balance(&self) -> Result<String, WalletError> {
        let connection = self.lock()?;
        let snapshot: Option<String> = connection
            .query_row(
                "SELECT snapshot_json FROM balance_snapshots ORDER BY sequence DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        if let Some(snapshot) = snapshot {
            let parsed: BalanceSnapshot =
                decode(&snapshot, MAX_PAYMENT_JSON).map_err(|_| corrupt())?;
            parsed
                .balances
                .validate_for_chain(parsed.chain_id)
                .map_err(|_| corrupt())?;
            address(&parsed.address).map_err(|_| corrupt())?;
            fixed_hex(&parsed.block_hash, 32).map_err(|_| corrupt())?;
            validate_identity(&connection, parsed.chain_id, &parsed.address)?;
            Ok(snapshot)
        } else {
            Ok("null".into())
        }
    }
}

const SCHEMA: &str = r#"
CREATE TABLE metadata (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;
CREATE TABLE payments (
    intent_id TEXT PRIMARY KEY NOT NULL,
    chain_id INTEGER NOT NULL CHECK (chain_id IN (1, 8453, 10, 42161, 43114, 137, 56)),
    sender TEXT NOT NULL,
    nonce TEXT NOT NULL,
    active INTEGER NOT NULL CHECK (active IN (0, 1)),
    prepared_json TEXT NOT NULL,
    signed_json TEXT,
    record_json TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX payment_nonce_reservation ON payments(chain_id, sender, nonce) WHERE active = 1;
CREATE TABLE payment_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    intent_id TEXT NOT NULL REFERENCES payments(intent_id),
    kind TEXT NOT NULL,
    observed_at INTEGER NOT NULL,
    payload_json TEXT NOT NULL
) STRICT;
CREATE INDEX payment_event_intent ON payment_events(intent_id, sequence);
CREATE TABLE balance_snapshots (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    block_hash TEXT NOT NULL UNIQUE,
    observed_at INTEGER NOT NULL,
    snapshot_json TEXT NOT NULL
) STRICT;
CREATE TRIGGER immutable_payment_fields BEFORE UPDATE ON payments WHEN
    NEW.intent_id <> OLD.intent_id OR NEW.chain_id <> OLD.chain_id OR NEW.sender <> OLD.sender
    OR NEW.nonce <> OLD.nonce OR NEW.prepared_json <> OLD.prepared_json
    OR (OLD.signed_json IS NOT NULL AND NEW.signed_json IS NOT OLD.signed_json)
    OR (OLD.active = 0 AND NEW.active <> 0)
BEGIN SELECT RAISE(ABORT, 'immutable payment evidence'); END;
CREATE TRIGGER retain_payments BEFORE DELETE ON payments
BEGIN SELECT RAISE(ABORT, 'payment deletion is prohibited'); END;
CREATE TRIGGER retain_event_updates BEFORE UPDATE ON payment_events
BEGIN SELECT RAISE(ABORT, 'event updates are prohibited'); END;
CREATE TRIGGER retain_event_deletes BEFORE DELETE ON payment_events
BEGIN SELECT RAISE(ABORT, 'event deletion is prohibited'); END;
CREATE TRIGGER retain_balance_updates BEFORE UPDATE ON balance_snapshots
BEGIN SELECT RAISE(ABORT, 'balance observation updates are prohibited'); END;
CREATE TRIGGER retain_balance_deletes BEFORE DELETE ON balance_snapshots
BEGIN SELECT RAISE(ABORT, 'balance observation deletion is prohibited'); END;
CREATE TRIGGER immutable_metadata_updates BEFORE UPDATE ON metadata
BEGIN SELECT RAISE(ABORT, 'journal identity is immutable'); END;
CREATE TRIGGER immutable_metadata_deletes BEFORE DELETE ON metadata
BEGIN SELECT RAISE(ABORT, 'journal identity is immutable'); END;
"#;
