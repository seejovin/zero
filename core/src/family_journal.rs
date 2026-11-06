//! Append-only, network-bound journals for Solana and TRON.
//!
//! Exact signed bytes must commit before submission. Unknown/expired transactions
//! retain their reservation: absence from an RPC is not proof of non-execution.
//! All projections are rebuilt from validated evidence; no balance adjustment API.
use crate::{solana, tron, WalletError};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde::{
    de::{MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const APP_ID: i64 = 0x4c57464d;
const MAX_JSON: usize = 1_048_576;
// TRON evidence includes a canonical block, which is larger than a prepared
// payment. Protocol validators retain their own narrower bounds where needed.
const MAX_EVIDENCE_JSON: usize = 4 * 1024 * 1024;
const SCHEMA: &str = "
CREATE TABLE identity (id INTEGER PRIMARY KEY CHECK(id=1), network TEXT NOT NULL, address TEXT NOT NULL) STRICT;
CREATE TABLE events (seq INTEGER PRIMARY KEY AUTOINCREMENT, intent TEXT NOT NULL, kind TEXT NOT NULL, body TEXT NOT NULL, at INTEGER NOT NULL) STRICT;
CREATE TABLE balances (seq INTEGER PRIMARY KEY AUTOINCREMENT, block_hash TEXT NOT NULL UNIQUE, body TEXT NOT NULL) STRICT;
CREATE TABLE activity (seq INTEGER PRIMARY KEY AUTOINCREMENT, hash TEXT NOT NULL, evidence TEXT NOT NULL, UNIQUE(hash,evidence)) STRICT;
CREATE TABLE tracked (hash TEXT PRIMARY KEY, at INTEGER NOT NULL) STRICT;
CREATE TRIGGER identity_no_update BEFORE UPDATE ON identity BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER identity_no_delete BEFORE DELETE ON identity BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER events_no_update BEFORE UPDATE ON events BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER events_no_delete BEFORE DELETE ON events BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER balances_no_update BEFORE UPDATE ON balances BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER balances_no_delete BEFORE DELETE ON balances BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER activity_no_update BEFORE UPDATE ON activity BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER activity_no_delete BEFORE DELETE ON activity BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER tracked_no_update BEFORE UPDATE ON tracked BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER tracked_no_delete BEFORE DELETE ON tracked BEGIN SELECT RAISE(ABORT,'immutable'); END;
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
        reason: "Family journal evidence failed validation; existing records were preserved".into(),
    }
}
fn storage(_: rusqlite::Error) -> WalletError {
    WalletError::Storage {
        reason: "The durable transaction journal could not be read or saved".into(),
    }
}
fn encode<T: Serialize>(value: &T) -> Result<String, WalletError> {
    serde_json::to_string(value).map_err(|_| corrupt())
}
fn decode<T: for<'a> Deserialize<'a>>(text: &str) -> Result<T, WalletError> {
    if text.len() > MAX_JSON {
        return Err(invalid("Journal evidence is too large"));
    }
    serde_json::from_value(strict_value(text)?).map_err(|_| invalid("Malformed journal evidence"))
}
/// Do not let duplicate critical RPC or balance keys disappear into a Value.
struct UniqueValue(Value);
impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("unambiguous JSON")
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(json!(v)))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(json!(v)))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(json!(v)))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| UniqueValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("Nonfinite JSON number"))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(v.into())))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(v)))
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(v)) = a.next_element()? {
                    if values.len() >= 50_000 {
                        return Err(serde::de::Error::custom("Excessive JSON array"));
                    }
                    values.push(v);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if values.contains_key(&k) || values.len() >= 50_000 {
                        return Err(serde::de::Error::custom("Duplicate or excessive JSON keys"));
                    }
                    let UniqueValue(v) = a.next_value()?;
                    values.insert(k, v);
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}
fn strict_value(text: &str) -> Result<Value, WalletError> {
    strict_value_limit(text, MAX_JSON)
}
fn strict_evidence(text: &str) -> Result<Value, WalletError> {
    strict_value_limit(text, MAX_EVIDENCE_JSON)
}
fn strict_value_limit(text: &str, limit: usize) -> Result<Value, WalletError> {
    if text.len() > limit {
        return Err(invalid("Journal evidence is too large"));
    }
    let UniqueValue(value) = serde_json::from_str(text)
        .map_err(|_| invalid("Malformed or ambiguous journal evidence"))?;
    fn bounded(value: &Value, depth: usize, nodes: &mut usize) -> bool {
        *nodes += 1;
        if depth > 32 || *nodes > 50_000 {
            return false;
        }
        match value {
            Value::Array(a) => a.iter().all(|v| bounded(v, depth + 1, nodes)),
            Value::Object(m) => m.values().all(|v| bounded(v, depth + 1, nodes)),
            _ => true,
        }
    }
    if !bounded(&value, 0, &mut 0) {
        return Err(invalid("Journal evidence exceeds structural limits"));
    }
    Ok(value)
}
fn now() -> Result<u64, WalletError> {
    let value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| invalid("Invalid device clock"))?;
    if value == 0 || value > i64::MAX as u64 {
        return Err(invalid("Invalid device clock"));
    }
    Ok(value)
}
fn db_integer(value: u64) -> Result<i64, WalletError> {
    i64::try_from(value).map_err(|_| corrupt())
}
fn positive_integer(value: i64) -> Result<u64, WalletError> {
    if value <= 0 {
        return Err(corrupt());
    }
    u64::try_from(value).map_err(|_| corrupt())
}
fn field<'a>(v: &'a Value, name: &str) -> Result<&'a str, WalletError> {
    v.get(name).and_then(Value::as_str).ok_or_else(corrupt)
}
fn id(text: &str) -> Result<(), WalletError> {
    if text.is_empty()
        || text.len() > 128
        || !text
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(invalid("Invalid payment identifier"));
    }
    Ok(())
}

#[derive(Clone)]
struct Payment {
    prepared: String,
    signed: Option<String>,
    record: Value,
}
impl Payment {
    fn phase(&self) -> &str {
        self.record["phase"].as_str().unwrap_or("unresolved")
    }
    fn active(&self) -> bool {
        !matches!(self.phase(), "cancelled" | "finalized")
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Balance {
    network: String,
    address: String,
    block_number: u64,
    block_hash: String,
    balances: BTreeMap<String, String>,
    observed_at: u64,
}

#[derive(Clone, PartialEq, Eq)]
struct ActivityContext {
    finalized: bool,
    success: bool,
    height: u64,
    block: String,
    fee: u64,
}
struct ActivityTransaction {
    rows: BTreeMap<String, Value>,
    context: ActivityContext,
    conflict: bool,
}
struct ActivityLedger {
    rows: Vec<Value>,
    statuses: BTreeMap<String, String>,
}

fn schema(connection: &Connection) -> Result<BTreeMap<String, (String, String)>, WalletError> {
    let mut query = connection
        .prepare(
            "SELECT name,type,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map_err(storage)?;
    let rows = query
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (r.get::<_, String>(1)?, r.get::<_, String>(2)?),
            ))
        })
        .map_err(storage)?;
    rows.collect::<Result<_, _>>().map_err(storage)
}

pub struct Journal {
    connection: Mutex<Connection>,
    network: String,
    address: String,
}
impl Journal {
    pub fn open(path: String, network: String, address: String) -> Result<Self, WalletError> {
        let normalized = match network.as_str() {
            "solana-mainnet" => solana::validate_address(&address)?,
            "tron-mainnet" => tron::normalize_address(&address)?,
            _ => return Err(invalid("Unsupported journal network")),
        };
        if address != normalized {
            return Err(invalid("Noncanonical wallet address"));
        }
        let path = Path::new(&path);
        if !path.is_absolute() || path.as_os_str().len() > 4096 {
            return Err(invalid("Journal requires an absolute local path"));
        }
        if let Ok(meta) = fs::symlink_metadata(path) {
            if !meta.is_file() || meta.file_type().is_symlink() {
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
                    #[cfg(unix)]
                    fs::File::open(path.parent().ok_or_else(corrupt)?)
                        .and_then(|p| p.sync_all())
                        .map_err(|_| corrupt())?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(corrupt()),
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::symlink_metadata(path)
                .map_err(|_| corrupt())?
                .permissions()
                .mode();
            if mode & 0o077 != 0 {
                return Err(invalid("Journal must be private to the app"));
            }
        }
        let mut connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(storage)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(storage)?;
        connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON;").map_err(storage)?;
        let journal_mode: String = connection
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .map_err(storage)?;
        let sync: i64 = connection
            .query_row("PRAGMA synchronous", [], |r| r.get(0))
            .map_err(storage)?;
        if journal_mode != "wal" || sync != 2 {
            return Err(corrupt());
        }
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let version: i64 = tx
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(storage)?;
        let app: i64 = tx
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .map_err(storage)?;
        if version == 0 && app == 0 {
            let objects: i64 = tx
                .query_row(
                    "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                    [],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if objects != 0 {
                return Err(corrupt());
            }
            tx.execute_batch(SCHEMA).map_err(storage)?;
            tx.execute(
                "INSERT INTO identity VALUES(1,?1,?2)",
                params![network, address],
            )
            .map_err(storage)?;
            tx.pragma_update(None, "user_version", 1).map_err(storage)?;
            tx.pragma_update(None, "application_id", APP_ID)
                .map_err(storage)?;
        } else if version != 1 || app != APP_ID {
            return Err(corrupt());
        }
        let bound: (String, String) = tx
            .query_row("SELECT network,address FROM identity WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(storage)?;
        if bound != (network.clone(), address.clone()) {
            return Err(conflict("Journal belongs to another network or wallet"));
        }
        let expected = Connection::open_in_memory().map_err(storage)?;
        expected.execute_batch(SCHEMA).map_err(storage)?;
        if schema(&tx)? != schema(&expected)? {
            return Err(corrupt());
        }
        tx.commit().map_err(storage)?;
        let integrity: String = connection
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(storage)?;
        if integrity != "ok" {
            return Err(corrupt());
        }
        let result = Self {
            connection: Mutex::new(connection),
            network,
            address,
        };
        result.list_payments()?;
        result.latest_balance()?;
        result.list_tracked()?;
        Ok(result)
    }
    fn lock(&self) -> Result<MutexGuard<'_, Connection>, WalletError> {
        self.connection.lock().map_err(|_| corrupt())
    }
    fn chain(&self) -> i64 {
        if self.network == "solana-mainnet" {
            -101
        } else {
            -195
        }
    }
    fn prepare(&self, text: &str) -> Result<Value, WalletError> {
        strict_value(text)?;
        let canonical = if self.network == "solana-mainnet" {
            encode(&solana::validate_prepared(text)?)?
        } else {
            encode(&tron::validate_prepared(text)?)?
        };
        let p: Value = decode(&canonical)?;
        if field(&p["request"], "network")? != self.network
            || field(&p["request"], "from")? != self.address
        {
            return Err(conflict("Payment belongs to a different wallet or network"));
        }
        id(field(&p["request"], "intent_id")?)?;
        Ok(p)
    }
    fn signed(&self, prepared: &str, text: &str) -> Result<Value, WalletError> {
        strict_value(text)?;
        if self.network == "solana-mainnet" {
            return serde_json::to_value(solana::validate_signed(prepared, text)?)
                .map_err(|_| corrupt());
        }
        let signed = tron::validate_signed(text)?;
        if tron::validate_prepared(&signed.prepared_json)? != tron::validate_prepared(prepared)? {
            return Err(conflict("Signature belongs to another approved payment"));
        }
        serde_json::to_value(signed).map_err(|_| corrupt())
    }
    fn new_payment(&self, p: Value, at: u64) -> Result<Payment, WalletError> {
        let r = &p["request"];
        let units = field(&p, "amount_units")?;
        let decimals = if r["asset"] == "SOL" { 9 } else { 6 };
        let amount = crate::crypto::format_amount(units.to_owned(), decimals)?;
        let record = json!({"intent_id":r["intent_id"], "network":self.network, "chain_id":self.chain(), "from":self.address,
            "to":r["to"],"asset":r["asset"],"amount":amount,"amount_units":units,"phase":"prepared","execution":"unknown",
            "inclusion":"unseen","hash":null,"block_number":null,"block_hash":null,"actual_fee_wei":null,
            "fee_reserve_wei":p["fee_reserve_units"],"fee_exceeds_reserve":false,"cost_exceeds_review":false,
            "actual_account_rent_units":null,"actual_total_cost_units":null,"created_at":at,"updated_at":at,"resolution_note":null});
        Ok(Payment {
            prepared: encode(&p)?,
            signed: None,
            record,
        })
    }
    fn apply(
        &self,
        payments: &mut BTreeMap<String, Payment>,
        intent: &str,
        kind: &str,
        body: &str,
        at: u64,
    ) -> Result<(), WalletError> {
        id(intent)?;
        if at == 0 || at > i64::MAX as u64 {
            return Err(corrupt());
        }
        if kind == "prepared" {
            let prepared = self.prepare(body)?;
            if field(&prepared["request"], "intent_id")? != intent
                || payments.contains_key(intent)
                || payments.values().any(Payment::active)
            {
                return Err(conflict("Another transfer is still unresolved. Reconcile it before creating a new payment"));
            }
            payments.insert(intent.into(), self.new_payment(prepared, at)?);
            return Ok(());
        }
        if kind == "signed" {
            let old = payments.get(intent).ok_or_else(corrupt)?;
            if old.phase() != "prepared" || old.signed.is_some() {
                return Err(conflict("Only an unsigned review can receive a signature"));
            }
            if payments
                .iter()
                .any(|(other, payment)| other != intent && payment.active())
            {
                return Err(conflict(
                    "Another transfer has unresolved evidence; signing is blocked",
                ));
            }
            let signed = self.signed(&old.prepared, body)?;
            let hash = field(&signed, "hash")?;
            if payments
                .values()
                .any(|p| p.record["hash"].as_str() == Some(hash))
            {
                return Err(conflict("This transaction is already recorded"));
            }
            let payment = payments.get_mut(intent).ok_or_else(corrupt)?;
            payment.signed = Some(encode(&signed)?);
            payment.record["hash"] = json!(hash);
            payment.record["phase"] = json!("signed");
        } else {
            let payment = payments.get_mut(intent).ok_or_else(corrupt)?;
            match kind {
                "cancelled" => {
                    if payment.phase() != "prepared" || payment.signed.is_some() || body != "null" {
                        return Err(conflict("A signed transfer cannot be cancelled locally"));
                    }
                    payment.record["phase"] = json!("cancelled");
                }
                "submission" => {
                    let state: String = decode(body)?;
                    if !matches!(state.as_str(), "pending" | "unknown") || payment.signed.is_none()
                    {
                        return Err(conflict("Submission requires a durable signed transfer"));
                    }
                    if matches!(payment.phase(), "signed" | "unknown")
                        || (payment.phase() == "pending" && state == "pending")
                    {
                        payment.record["phase"] = json!(state);
                    }
                }
                "receipt" => {
                    strict_evidence(body)?;
                    let signed = payment.signed.as_ref().ok_or_else(corrupt)?;
                    let (
                        finalized,
                        success,
                        hash,
                        height,
                        block,
                        fee,
                        rent,
                        total_cost,
                        cost_exceeds_review,
                    ) = if self.network == "solana-mainnet" {
                        let o = solana::validate_receipt(&payment.prepared, signed, body)?;
                        (
                            o.finalized,
                            o.success,
                            o.hash,
                            o.block_number,
                            o.block_hash,
                            o.fee_units,
                            o.account_rent_units,
                            o.total_cost_units,
                            o.cost_exceeds_review,
                        )
                    } else {
                        let o = tron::validate_receipt(&payment.prepared, signed, body)?;
                        (
                            true,
                            o.state == "finalized",
                            o.hash,
                            o.block_number,
                            o.block_hash,
                            o.actual_fee_units.clone(),
                            "0".to_owned(),
                            o.actual_fee_units,
                            o.fee_exceeds_reserve,
                        )
                    };
                    let execution = if success { "success" } else { "failed" };
                    let actual =
                        primitive_types::U256::from_dec_str(&fee).map_err(|_| corrupt())?;
                    let reserved = primitive_types::U256::from_dec_str(field(
                        &payment.record,
                        "fee_reserve_wei",
                    )?)
                    .map_err(|_| corrupt())?;
                    if payment.phase() == "unresolved"
                        || (payment.phase() == "finalized"
                            && (payment.record["block_hash"] != block
                                || payment.record["block_number"] != height
                                || payment.record["execution"] != execution
                                || payment.record["actual_fee_wei"] != fee
                                || payment.record["actual_account_rent_units"] != rent
                                || payment.record["actual_total_cost_units"] != total_cost))
                    {
                        payment.record["phase"] = json!("unresolved");
                        payment.record["resolution_note"] = json!(
                            "Conflicting finality evidence. No replacement transfer is permitted."
                        );
                    } else if payment.phase() != "finalized" || finalized {
                        payment.record["phase"] =
                            json!(if finalized { "finalized" } else { "included" });
                        payment.record["inclusion"] =
                            json!(if finalized { "finalized" } else { "included" });
                        payment.record["execution"] = json!(execution);
                        payment.record["hash"] = json!(hash);
                        payment.record["block_number"] = json!(height);
                        payment.record["block_hash"] = json!(block);
                        payment.record["actual_fee_wei"] = json!(fee);
                        payment.record["actual_account_rent_units"] = json!(rent);
                        payment.record["actual_total_cost_units"] = json!(total_cost);
                        payment.record["fee_exceeds_reserve"] = json!(actual > reserved);
                        payment.record["cost_exceeds_review"] = json!(cost_exceeds_review);
                        payment.record["resolution_note"] = if cost_exceeds_review
                            || actual > reserved
                        {
                            json!("The observed network cost exceeded a reviewed fee or account-rent allowance; its actual cost is recorded.")
                        } else {
                            Value::Null
                        };
                    }
                }
                _ => return Err(corrupt()),
            }
        }
        let p = payments.get_mut(intent).ok_or_else(corrupt)?;
        if at < p.record["updated_at"].as_u64().ok_or_else(corrupt)? {
            return Err(corrupt());
        }
        p.record["updated_at"] = json!(at);
        Ok(())
    }
    fn load(&self, connection: &Connection) -> Result<BTreeMap<String, Payment>, WalletError> {
        let mut payments = BTreeMap::new();
        let mut statement = connection
            .prepare("SELECT seq,intent,kind,body,at FROM events ORDER BY seq")
            .map_err(storage)?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })
            .map_err(storage)?;
        for row in rows {
            let (seq, intent, kind, body, at) = row.map_err(storage)?;
            positive_integer(seq)?;
            self.apply(&mut payments, &intent, &kind, &body, positive_integer(at)?)?;
        }
        Ok(payments)
    }
    fn append(&self, intent: &str, kind: &str, body: &str) -> Result<String, WalletError> {
        id(intent)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut payments = self.load(&tx)?;
        // Idempotency and state checks live inside the same write transaction as
        // the append, including when separate processes race with identical work.
        if let Some(old) = payments.get(intent) {
            if kind == "prepared" {
                if old.prepared == body {
                    return encode(&old.record);
                }
                return Err(conflict(
                    "The payment intent already contains different details",
                ));
            }
            if kind == "signed" {
                let canonical = encode(&self.signed(&old.prepared, body)?)?;
                if let Some(saved) = &old.signed {
                    if saved == &canonical {
                        return encode(&old.record);
                    }
                    return Err(conflict("A different signature is already saved"));
                }
            }
            if kind == "cancelled" && old.phase() == "cancelled" && body == "null" {
                return encode(&old.record);
            }
        }
        let before = payments.get(intent).map(|p| p.record.clone());
        let at = now()?.max(
            payments
                .get(intent)
                .and_then(|p| p.record["updated_at"].as_u64())
                .unwrap_or(0),
        );
        self.apply(&mut payments, intent, kind, body, at)?;
        let record = &payments.get(intent).ok_or_else(corrupt)?.record;
        if let Some(mut previous) = before {
            // Repeated RPC polling cannot renew a payment's evidence timestamp
            // or grow the audit log when the validated projection is unchanged.
            let durable = previous.clone();
            previous["updated_at"] = json!(at);
            if previous == *record {
                return encode(&durable);
            }
        }
        tx.execute(
            "INSERT INTO events(intent,kind,body,at) VALUES(?1,?2,?3,?4)",
            params![intent, kind, body, db_integer(at)?],
        )
        .map_err(storage)?;
        let record = encode(record)?;
        tx.commit().map_err(storage)?;
        Ok(record)
    }
    pub fn record_prepared(&self, text: String) -> Result<String, WalletError> {
        let p = self.prepare(&text)?;
        self.append(field(&p["request"], "intent_id")?, "prepared", &encode(&p)?)
    }
    pub fn record_signed(&self, intent: String, text: String) -> Result<String, WalletError> {
        // apply() validates it against the exact durable preparation while locked.
        let canonical = encode(&strict_value(&text)?)?;
        self.append(&intent, "signed", &canonical)
    }
    pub fn mark_submission(&self, intent: String, outcome: String) -> Result<String, WalletError> {
        self.append(&intent, "submission", &encode(&outcome)?)
    }
    pub fn observe_receipt(&self, intent: String, evidence: String) -> Result<String, WalletError> {
        self.append(&intent, "receipt", &encode(&strict_evidence(&evidence)?)?)
    }
    pub fn cancel_prepared(&self, intent: String) -> Result<String, WalletError> {
        self.append(&intent, "cancelled", "null")
    }
    pub fn list_payments(&self) -> Result<String, WalletError> {
        let connection = self.lock()?;
        let mut values: Vec<Value> = self
            .load(&connection)?
            .into_values()
            .map(|p| p.record)
            .collect();
        values.sort_by_key(|v| std::cmp::Reverse(v["created_at"].as_u64().unwrap_or(0)));
        encode(&values)
    }
    /// Private submission API. Never include this result in audit exports or logs.
    pub fn get_payment(&self, intent: String) -> Result<String, WalletError> {
        let connection = self.lock()?;
        let all = self.load(&connection)?;
        let p = all.get(&intent).ok_or_else(corrupt)?;
        encode(&json!({"record":p.record,"prepared_json":p.prepared,"signed_json":p.signed}))
    }
    pub fn list_events(&self, intent: String) -> Result<String, WalletError> {
        id(&intent)?;
        let connection = self.lock()?;
        self.load(&connection)?;
        let mut query = connection
            .prepare("SELECT seq,kind,at FROM events WHERE intent=?1 ORDER BY seq")
            .map_err(storage)?;
        let rows = query
            .query_map([intent], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(storage)?;
        let mut events = Vec::new();
        for row in rows {
            let (seq, kind, at) = row.map_err(storage)?;
            events.push(json!({"sequence":positive_integer(seq)?,"kind":kind,"timestamp":positive_integer(at)?}));
        }
        encode(&events)
    }
    fn balance(&self, text: &str) -> Result<Balance, WalletError> {
        let b: Balance = decode(text)?;
        let symbols: &[&str] = if self.network == "solana-mainnet" {
            &["SOL", "USDC", "EURC", "USDT"]
        } else {
            &["TRX", "USDT"]
        };
        if b.network != self.network
            || b.address != self.address
            || b.observed_at == 0
            || b.observed_at > i64::MAX as u64
            || b.block_number == 0
            || b.block_number > i64::MAX as u64
            || b.balances.len() != symbols.len()
            || b.balances.keys().any(|k| !symbols.contains(&k.as_str()))
        {
            return Err(invalid("Balance does not match this network and wallet"));
        }
        for (symbol, amount) in &b.balances {
            if amount.is_empty()
                || amount.len() > 78
                || !amount.bytes().all(|b| b.is_ascii_digit())
                || (amount.len() > 1 && amount.starts_with('0'))
                || primitive_types::U256::from_dec_str(amount).is_err()
            {
                return Err(invalid("Malformed balance units"));
            }
            let units = primitive_types::U256::from_dec_str(amount)
                .map_err(|_| invalid("Malformed balance units"))?;
            if (self.network == "solana-mainnet" && units > primitive_types::U256::from(u64::MAX))
                || (symbol == "TRX" && units > primitive_types::U256::from(i64::MAX as u64))
            {
                return Err(invalid("Balance exceeds its protocol integer range"));
            }
        }
        self.validate_hash(&b.block_hash, true)?;
        Ok(b)
    }
    fn load_balances(&self, connection: &Connection) -> Result<Vec<Balance>, WalletError> {
        let mut query = connection
            .prepare("SELECT block_hash,body FROM balances ORDER BY seq")
            .map_err(storage)?;
        let rows = query
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(storage)?;
        let mut all: Vec<Balance> = Vec::new();
        let mut hashes = HashSet::new();
        for row in rows {
            let (hash, text) = row.map_err(storage)?;
            let b = self.balance(&text)?;
            if b.block_hash != hash
                || !hashes.insert(hash)
                || all.last().is_some_and(|p| {
                    b.block_number < p.block_number || b.observed_at < p.observed_at
                })
            {
                return Err(corrupt());
            }
            all.push(b);
        }
        Ok(all)
    }
    pub fn record_balance(&self, text: String) -> Result<String, WalletError> {
        let b = self.balance(&text)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let previous = self.load_balances(&tx)?;
        if let Some(old) = previous.iter().find(|p| p.block_hash == b.block_hash) {
            if old.block_number != b.block_number || old.balances != b.balances {
                return Err(conflict(
                    "Balances disagree at the same block; existing evidence was preserved",
                ));
            }
            // Re-reading an old block cannot roll the wallet back to old balances.
            return encode(previous.last().ok_or_else(corrupt)?);
        }
        if previous
            .last()
            .is_some_and(|p| b.block_number < p.block_number || b.observed_at < p.observed_at)
        {
            return Err(conflict(
                "Older balance evidence cannot replace the latest observation",
            ));
        }
        let encoded = encode(&b)?;
        tx.execute(
            "INSERT INTO balances(block_hash,body) VALUES(?1,?2)",
            params![b.block_hash, encoded],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(encoded)
    }
    pub fn latest_balance(&self) -> Result<String, WalletError> {
        let connection = self.lock()?;
        let all = self.load_balances(&connection)?;
        match all.last() {
            Some(b) => encode(b),
            None => Ok("null".into()),
        }
    }
    fn validate_hash(&self, hash: &str, block: bool) -> Result<(), WalletError> {
        if self.network == "solana-mainnet" {
            if hash.len() > 88 || hash.is_empty() {
                return Err(invalid("Invalid Solana identifier"));
            }
            let raw = bs58::decode(hash)
                .into_vec()
                .map_err(|_| invalid("Invalid Solana identifier"))?;
            if raw.len() != if block { 32 } else { 64 }
                || raw.iter().all(|b| *b == 0)
                || bs58::encode(&raw).into_string() != hash
            {
                return Err(invalid("Invalid Solana identifier"));
            }
        } else if hash.len() != 64
            || hash.bytes().all(|b| b == b'0')
            || !hash
                .bytes()
                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
        {
            return Err(invalid("Invalid TRON identifier"));
        }
        Ok(())
    }
    pub fn track_transaction(&self, hash: String) -> Result<(), WalletError> {
        self.validate_hash(&hash, false)?;
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT OR IGNORE INTO tracked(hash,at) VALUES(?1,?2)",
                params![hash, db_integer(now()?)?],
            )
            .map_err(storage)?;
        Ok(())
    }
    fn activity(&self, evidence: &str) -> Result<Vec<Value>, WalletError> {
        strict_evidence(evidence)?;
        if self.network == "tron-mainnet" {
            return tron::validate_activity(&self.address, evidence)?
                .into_iter()
                .map(|a| {
                    let mut row = serde_json::to_value(a).map_err(|_| corrupt())?;
                    row["verification_state"] = json!("observed");
                    Ok(row)
                })
                .collect();
        }
        solana::validate_activity(&self.address, evidence)?.into_iter().map(|a| {
            let decimals = if a.asset=="SOL" {9} else {6};
            let amount = crate::crypto::format_amount(a.amount_units.clone(),decimals)?;
            Ok(json!({"id":format!("solana:{}:{}",a.hash,a.event_index),"network":self.network,"chain_id":self.chain(),"address":self.address,
                "asset":a.asset,"amount_units":a.amount_units,"amount":amount,"direction":a.direction,"from":a.from,"to":a.to,
                "transaction_hash":a.hash,"block_number":a.block_number,"block_hash":a.block_hash,"log_index":a.event_index,
                "inclusion":if a.finalized {"finalized"} else {"included"},"verification_state":"observed","timestamp":0}))
        }).collect()
    }
    fn activity_hash(&self, evidence: &str) -> Result<String, WalletError> {
        let value = strict_evidence(evidence)?;
        let hash = if self.network == "tron-mainnet" {
            field(&value["transaction"], "txID")?.to_ascii_lowercase()
        } else {
            let text = value["transaction"]["transaction"][0]
                .as_str()
                .ok_or_else(corrupt)?;
            let bytes = BASE64.decode(text).map_err(|_| corrupt())?;
            // The validated packet is <=1232 bytes, so its signature count fits
            // in one compact-length byte. The first signature is the tx ID.
            if bytes.first().is_none_or(|b| *b == 0 || *b >= 128) {
                return Err(corrupt());
            }
            bs58::encode(bytes.get(1..65).ok_or_else(corrupt)?).into_string()
        };
        self.validate_hash(&hash, false)?;
        Ok(hash)
    }
    pub fn record_activity(&self, hash: String, evidence: String) -> Result<String, WalletError> {
        self.validate_hash(&hash, false)?;
        let canonical = encode(&strict_evidence(&evidence)?)?;
        let rows = self.activity(&canonical)?;
        if self.activity_hash(&canonical)? != hash
            || rows.iter().any(|row| row["transaction_hash"] != hash)
        {
            return Err(conflict("Deposit evidence belongs to another transaction"));
        }
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        tx.execute(
            "INSERT OR IGNORE INTO activity(hash,evidence) VALUES(?1,?2)",
            params![hash, canonical],
        )
        .map_err(storage)?;
        let result = self.load_activity(&tx)?;
        tx.commit().map_err(storage)?;
        encode(
            &result
                .rows
                .into_iter()
                .filter(|row| row["transaction_hash"] == hash)
                .collect::<Vec<_>>(),
        )
    }
    // Read after protocol validation. Keeping transaction context even for an
    // empty result prevents a failed/unrelated finalized transaction from later
    // silently turning into a received payment under the same transaction hash.
    fn activity_context(&self, evidence: &str) -> Result<ActivityContext, WalletError> {
        let value = strict_evidence(evidence)?;
        if self.network == "tron-mainnet" {
            Ok(ActivityContext {
                finalized: true,
                success: field(&value["transaction"]["ret"][0], "contractRet")? == "SUCCESS",
                height: value["info"]["blockNumber"].as_u64().ok_or_else(corrupt)?,
                block: field(&value["block"], "blockID")?.to_ascii_lowercase(),
                fee: value["info"]
                    .get("fee")
                    .map_or(Some(0), Value::as_u64)
                    .ok_or_else(corrupt)?,
            })
        } else {
            Ok(ActivityContext {
                finalized: field(&value["signature_status"], "confirmationStatus")? == "finalized",
                success: value["transaction"]["meta"]
                    .get("err")
                    .ok_or_else(corrupt)?
                    .is_null(),
                height: value["transaction"]["slot"].as_u64().ok_or_else(corrupt)?,
                block: field(&value["canonical_block"], "blockhash")?.into(),
                fee: value["transaction"]["meta"]["fee"]
                    .as_u64()
                    .ok_or_else(corrupt)?,
            })
        }
    }
    fn load_activity(&self, connection: &Connection) -> Result<ActivityLedger, WalletError> {
        let mut query = connection
            .prepare("SELECT hash,evidence FROM activity ORDER BY seq")
            .map_err(storage)?;
        let rows = query
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(storage)?;
        let mut transactions: BTreeMap<String, ActivityTransaction> = BTreeMap::new();
        for row in rows {
            let (hash, evidence) = row.map_err(storage)?;
            let next = self.activity(&evidence)?;
            if self.activity_hash(&evidence)? != hash {
                return Err(corrupt());
            }
            let mut context = self.activity_context(&evidence)?;
            let mut contradiction = false;
            let mut observed = BTreeMap::new();
            for activity in next {
                if activity["transaction_hash"] != hash
                    || observed
                        .insert(field(&activity, "id")?.to_owned(), activity)
                        .is_some()
                {
                    return Err(corrupt());
                }
            }
            if let Some(previous) = transactions.get(&hash) {
                contradiction = previous.conflict
                    || (previous.context.finalized
                        && (previous.context.height != context.height
                            || previous.context.block != context.block
                            || previous.context.success != context.success
                            || previous.context.fee != context.fee));
                for (id, old) in &previous.rows {
                    let replacement = observed.get(id);
                    let economic_fields = [
                        "network",
                        "address",
                        "asset",
                        "amount_units",
                        "direction",
                        "from",
                        "to",
                        "transaction_hash",
                        "log_index",
                    ];
                    let economic_change = replacement
                        .is_none_or(|v| economic_fields.iter().any(|k| old[*k] != v[*k]));
                    let finality_change = old["inclusion"] == "finalized"
                        && replacement.is_some_and(|v| {
                            old["block_hash"] != v["block_hash"]
                                || old["block_number"] != v["block_number"]
                        });
                    if old["verification_state"] == "conflict" || economic_change || finality_change
                    {
                        contradiction = true;
                        let mut retained = old.clone();
                        retained["verification_state"] = json!("conflict");
                        observed.insert(id.clone(), retained);
                    } else if old["inclusion"] == "finalized"
                        && replacement.is_some_and(|v| v["inclusion"] != "finalized")
                    {
                        observed.insert(id.clone(), old.clone());
                    }
                }
                // A finalized transaction cannot acquire additional economic events later.
                if previous.context.finalized {
                    if observed.keys().any(|id| !previous.rows.contains_key(id)) {
                        contradiction = true;
                    }
                    // Preserve the strongest already-validated observation when a
                    // slower provider reports the same transaction as confirmed.
                    if !context.finalized {
                        context = previous.context.clone();
                    }
                }
            }
            if contradiction {
                for row in observed.values_mut() {
                    row["verification_state"] = json!("conflict");
                }
            }
            transactions.insert(
                hash,
                ActivityTransaction {
                    rows: observed,
                    context,
                    conflict: contradiction,
                },
            );
        }
        let mut ledger = ActivityLedger {
            rows: Vec::new(),
            statuses: BTreeMap::new(),
        };
        for (hash, transaction) in transactions {
            let status = if transaction.conflict {
                "conflict"
            } else if !transaction.rows.is_empty() {
                "verified"
            } else if !transaction.context.finalized {
                "pending"
            } else if !transaction.context.success {
                "failed"
            } else {
                "not_recipient"
            };
            ledger.statuses.insert(hash, status.into());
            ledger.rows.extend(transaction.rows.into_values());
        }
        Ok(ledger)
    }
    pub fn list_activity(&self) -> Result<String, WalletError> {
        let connection = self.lock()?;
        encode(&self.load_activity(&connection)?.rows)
    }
    pub fn list_tracked(&self) -> Result<String, WalletError> {
        let connection = self.lock()?;
        let ledger = self.load_activity(&connection)?;
        let mut query = connection
            .prepare("SELECT hash,at FROM tracked ORDER BY at DESC,hash")
            .map_err(storage)?;
        let rows = query
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(storage)?;
        let mut result = Vec::new();
        for row in rows {
            let (hash, at) = row.map_err(storage)?;
            self.validate_hash(&hash, false)?;
            result.push(json!({"hash":hash,"status":ledger.statuses.get(&hash).map_or("unavailable",String::as_str),"tracked_at":positive_integer(at)?}));
        }
        encode(&result)
    }
}
