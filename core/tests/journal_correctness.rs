//! All signatures use the publicly known Hardhat development mnemonic. These
//! fixtures are not wallets for a user and must never receive production funds.

use primitive_types::U256;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::sync::{Arc, Barrier, OnceLock};
use tempfile::TempDir;
use wallet_core::crypto::{self, CHAIN_ID};
use wallet_core::journal::Journal;

const PUBLIC_TEST_PHRASE: &str = "test test test test test test test test test test test junk";
const FROM: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const TO: &str = "0x1111111111111111111111111111111111111111";

fn value(input: String) -> Value {
    serde_json::from_str(&input).unwrap()
}

fn journal() -> (TempDir, Journal, String) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory
        .path()
        .join("wallet.sqlite")
        .to_str()
        .unwrap()
        .to_owned();
    let journal = Journal::open(path.clone()).unwrap();
    (directory, journal, path)
}

fn request(intent: &str, nonce: u64, asset: &str) -> Value {
    json!({
        "intent_id":intent,"chain_id":CHAIN_ID,"from":FROM,"to":TO,"asset":asset,
        "amount":if asset == "ETH" {"0.001"} else {"1.25"},"nonce":nonce,
        "gas_limit":if asset == "ETH" {21_000} else {60_000},
        "max_fee_per_gas":"2000000000","max_priority_fee_per_gas":"1000000000",
        "l1_fee_estimate_wei":"0","operator_fee_estimate_wei":"0"
    })
}

fn prepare(intent: &str, nonce: u64, asset: &str) -> String {
    crypto::prepare_payment(request(intent, nonce, asset).to_string()).unwrap()
}

fn sign(prepared: &str) -> String {
    static VAULT: OnceLock<String> = OnceLock::new();
    let vault = VAULT.get_or_init(|| {
        let wallet = value(crypto::restore_wallet(PUBLIC_TEST_PHRASE.into(), vec![7; 32]).unwrap());
        assert_eq!(wallet["address"], FROM);
        wallet["vault_hex"].as_str().unwrap().to_owned()
    });
    let preparation: Value = serde_json::from_str(prepared).unwrap();
    crypto::sign_payment(
        vault.clone(),
        vec![7; 32],
        prepared.into(),
        preparation["approval_digest"].as_str().unwrap().into(),
    )
    .unwrap()
}

fn signed_record(journal: &Journal, intent: &str, nonce: u64, asset: &str) -> (String, Value) {
    let preparation = prepare(intent, nonce, asset);
    journal.record_prepared(preparation.clone()).unwrap();
    let signed = sign(&preparation);
    let record = value(journal.record_signed(signed.clone()).unwrap());
    (signed, record)
}

fn hash(byte: u8) -> String {
    format!("0x{}", hex::encode([byte; 32]))
}

fn receipt(signed: &str, block: u64, block_hash: &str, succeeded: bool) -> Value {
    let signed: Value = serde_json::from_str(signed).unwrap();
    let inspected =
        value(crypto::inspect_signed(signed["raw_transaction"].as_str().unwrap().into()).unwrap());
    let token = signed["asset"] != "ETH";
    let logs = if token && succeeded {
        let units = U256::from_dec_str(signed["amount_units"].as_str().unwrap()).unwrap();
        vec![json!({
            "address":inspected["transaction_to"],
            "topics":[
                "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                format!("0x{:0>64}", &FROM[2..].to_ascii_lowercase()),
                format!("0x{:0>64}", &TO[2..])
            ],
            "data":format!("0x{units:064x}"),"transactionHash":signed["hash"],"blockHash":block_hash,
            "blockNumber":format!("0x{block:x}"),"logIndex":"0x3","transactionIndex":"0x0","removed":false
        })]
    } else {
        Vec::new()
    };
    json!({
        "transactionHash":signed["hash"],"blockHash":block_hash,"blockNumber":format!("0x{block:x}"),
        "from":signed["from"],"to":inspected["transaction_to"],"status":if succeeded {"0x1"} else {"0x0"},
        "gasUsed":if token {"0xc350"} else {"0x5208"},"effectiveGasPrice":"0x59682f00",
        "type":"0x2","contractAddress":null,"logs":logs,
        "transactionIndex":"0x0","cumulativeGasUsed":"0x100000","logsBloom":format!("0x{}","00".repeat(256))
    })
}

fn event_count(journal: &Journal, id: &str) -> usize {
    value(journal.list_events(id.into()).unwrap())
        .as_array()
        .unwrap()
        .len()
}

#[test]
fn storage_is_durable_wal_and_rejects_unsafe_paths() {
    assert!(Journal::open(":memory:".into()).is_err());
    assert!(Journal::open("relative.sqlite".into()).is_err());
    let (directory, journal, path) = journal();
    assert!(Journal::open(directory.path().to_str().unwrap().into()).is_err());
    let prepared = prepare("survives-reopen", 0, "ETH");
    journal.record_prepared(prepared).unwrap();
    drop(journal);
    let reopened = Journal::open(path.clone()).unwrap();
    assert_eq!(
        value(reopened.get_payment("survives-reopen".into()).unwrap())["phase"],
        "prepared"
    );
    let connection = Connection::open(&path).unwrap();
    let mode: String = connection
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let link = directory.path().join("link.sqlite");
        symlink(&path, &link).unwrap();
        assert!(Journal::open(link.to_str().unwrap().into()).is_err());
    }
}

#[test]
fn idempotency_and_nonce_uniqueness_are_intent_level() {
    let (_directory, journal, _path) = journal();
    let prepared = prepare("payment-1", 0, "ETH");
    let first = journal.record_prepared(prepared.clone()).unwrap();
    assert_eq!(first, journal.record_prepared(prepared).unwrap());
    assert_eq!(event_count(&journal, "payment-1"), 1);
    assert!(journal
        .record_prepared(prepare("payment-2", 0, "ETH"))
        .is_err());
    let mut changed = request("payment-1", 0, "ETH");
    changed["amount"] = json!("0.002");
    assert!(journal
        .record_prepared(crypto::prepare_payment(changed.to_string()).unwrap())
        .is_err());
    assert_eq!(first, journal.get_payment("payment-1".into()).unwrap());
    journal
        .record_prepared(prepare("payment-2", 1, "ETH"))
        .unwrap();
    assert_eq!(
        value(journal.list_payments().unwrap())
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn unsigned_cancel_releases_nonce_but_preserves_evidence() {
    let (_directory, journal, _path) = journal();
    let prepared = prepare("unsigned", 0, "ETH");
    let signed_before_cancel = sign(&prepared);
    journal.record_prepared(prepared.clone()).unwrap();
    let cancelled = journal.cancel_prepared("unsigned".into()).unwrap();
    assert_eq!(value(cancelled.clone())["phase"], "cancelled");
    assert_eq!(
        cancelled,
        journal.cancel_prepared("unsigned".into()).unwrap()
    );
    assert_eq!(cancelled, journal.record_prepared(prepared).unwrap());
    assert!(journal.record_signed(signed_before_cancel).is_err());
    journal
        .record_prepared(prepare("new-review", 0, "ETH"))
        .unwrap();
    assert_eq!(event_count(&journal, "unsigned"), 2);
}

#[test]
fn signing_requires_preparation_and_persists_exact_bytes_before_submission() {
    let (_directory, journal, path) = journal();
    let prepared = prepare("persist-first", 0, "ETH");
    let signed = sign(&prepared);
    assert!(journal.record_signed(signed.clone()).is_err());
    journal.record_prepared(prepared).unwrap();
    assert!(journal
        .mark_submission("persist-first".into(), "pending".into())
        .is_err());
    let record = journal.record_signed(signed.clone()).unwrap();
    assert_eq!(record, journal.record_signed(signed).unwrap());
    assert_eq!(event_count(&journal, "persist-first"), 2);
    assert!(journal.cancel_prepared("persist-first".into()).is_err());
    drop(journal);
    let reopened = Journal::open(path).unwrap();
    assert_eq!(
        record,
        reopened.get_payment("persist-first".into()).unwrap()
    );
    let unknown = value(
        reopened
            .mark_submission("persist-first".into(), "unknown".into())
            .unwrap(),
    );
    assert_eq!(unknown["phase"], "unknown");
    assert_eq!(unknown["execution"], "unknown");
    let pending = reopened
        .mark_submission("persist-first".into(), "pending".into())
        .unwrap();
    assert_eq!(
        pending,
        reopened
            .mark_submission("persist-first".into(), "unknown".into())
            .unwrap()
    );
    assert!(reopened
        .mark_submission("persist-first".into(), "failed".into())
        .is_err());
    assert!(reopened
        .record_prepared(prepare("duplicate-effect", 0, "ETH"))
        .is_err());
}

#[test]
fn changed_fee_bytes_cannot_hide_under_the_original_approval_digest() {
    let (_directory, journal, _path) = journal();
    let prepared = prepare("fee-binding", 0, "ETH");
    journal.record_prepared(prepared.clone()).unwrap();
    let mut another_request = request("fee-binding", 0, "ETH");
    another_request["max_priority_fee_per_gas"] = json!("500000000");
    let other_prepared = crypto::prepare_payment(another_request.to_string()).unwrap();
    let mut other_signed = value(sign(&other_prepared));
    other_signed["approval_digest"] =
        serde_json::from_str::<Value>(&prepared).unwrap()["approval_digest"].clone();
    // Every human payment summary field and total maximum fee is the same;
    // different unsigned transaction bytes must still be rejected.
    assert!(journal.record_signed(other_signed.to_string()).is_err());
    assert_eq!(
        value(journal.get_payment("fee-binding".into()).unwrap())["phase"],
        "prepared"
    );
}

#[test]
fn signed_metadata_or_signature_tampering_is_rejected() {
    let (_directory, journal, _path) = journal();
    let prepared = prepare("tampering", 0, "ETH");
    journal.record_prepared(prepared.clone()).unwrap();
    let signed = value(sign(&prepared));
    for field in [
        "from",
        "to",
        "amount",
        "amount_units",
        "hash",
        "max_fee_wei",
        "approval_digest",
    ] {
        let mut tampered = signed.clone();
        tampered[field] = json!("bad");
        assert!(
            journal.record_signed(tampered.to_string()).is_err(),
            "field {field}"
        );
    }
    let mut tampered = signed.clone();
    let raw = tampered["raw_transaction"].as_str().unwrap();
    let replacement = if raw.ends_with("00") { "ff" } else { "00" };
    tampered["raw_transaction"] = json!(format!("{}{replacement}", &raw[..raw.len() - 2]));
    assert!(journal.record_signed(tampered.to_string()).is_err());
    assert_eq!(event_count(&journal, "tampering"), 1);
}

#[test]
fn receipts_distinguish_execution_inclusion_finality_and_actual_fee() {
    let (_directory, journal, _path) = journal();
    let (signed, _) = signed_record(&journal, "native-success", 0, "ETH");
    let block_hash = hash(1);
    let receipt = receipt(&signed, 100, &block_hash, true).to_string();
    let included = value(
        journal
            .observe_receipt(
                "native-success".into(),
                receipt.clone(),
                block_hash.clone(),
                99,
            )
            .unwrap(),
    );
    assert_eq!(included["phase"], "included");
    assert_eq!(included["execution"], "success");
    assert_eq!(included["inclusion"], "included");
    assert_eq!(included["actual_fee_wei"], "31500000000000");
    let count = event_count(&journal, "native-success");
    journal
        .observe_receipt(
            "native-success".into(),
            receipt.clone(),
            block_hash.clone(),
            99,
        )
        .unwrap();
    assert_eq!(event_count(&journal, "native-success"), count);
    let finalized = journal
        .observe_receipt(
            "native-success".into(),
            receipt.clone(),
            block_hash.clone(),
            100,
        )
        .unwrap();
    assert_eq!(value(finalized.clone())["phase"], "finalized");
    assert_eq!(
        finalized,
        journal
            .observe_receipt("native-success".into(), receipt, block_hash, 99)
            .unwrap()
    );
    assert_eq!(
        finalized,
        journal
            .mark_submission("native-success".into(), "unknown".into())
            .unwrap()
    );
}

#[test]
fn reverted_transactions_keep_the_gas_charge_without_payment_success() {
    let (_directory, journal, _path) = journal();
    let (signed, _) = signed_record(&journal, "reverted", 0, "USDC");
    let block_hash = hash(2);
    let receipt = receipt(&signed, 100, &block_hash, false).to_string();
    let failed = value(
        journal
            .observe_receipt("reverted".into(), receipt, block_hash, 100)
            .unwrap(),
    );
    assert_eq!(failed["phase"], "finalized");
    assert_eq!(failed["execution"], "failed");
    assert_eq!(failed["receipt_status"], 0);
    assert_eq!(failed["actual_fee_wei"], "75000000000000");
    assert!(journal
        .record_prepared(prepare("retry-new-intent", 0, "USDC"))
        .is_err());
    assert_eq!(journal.latest_balance().unwrap(), "null");
}

#[test]
fn token_success_requires_the_exact_reviewed_contract_and_transfer_event() {
    for asset in ["USDC", "EURC"] {
        let (_directory, journal, _path) = journal();
        let (signed, _) = signed_record(&journal, "token-success", 0, asset);
        let block_hash = hash(3);
        let valid = receipt(&signed, 100, &block_hash, true);
        let result = value(
            journal
                .observe_receipt("token-success".into(), valid.to_string(), block_hash, 100)
                .unwrap(),
        );
        assert_eq!(result["execution"], "success");
        assert_eq!(result["phase"], "finalized");
    }
    for mutation in [
        "missing",
        "contract",
        "recipient",
        "sender",
        "amount",
        "two_transfers",
    ] {
        let (_directory, journal, _path) = journal();
        let (signed, _) = signed_record(&journal, "token-unresolved", 0, "USDC");
        let block_hash = hash(4);
        let mut invalid = receipt(&signed, 100, &block_hash, true);
        match mutation {
            "missing" => invalid["logs"] = json!([]),
            "contract" => invalid["logs"][0]["address"] = json!(TO),
            "recipient" => invalid["logs"][0]["topics"][2] = json!(hash(5)),
            "sender" => invalid["logs"][0]["topics"][1] = json!(hash(5)),
            "amount" => invalid["logs"][0]["data"] = json!(hash(5)),
            "two_transfers" => {
                let mut duplicate = invalid["logs"][0].clone();
                duplicate["logIndex"] = json!("0x4");
                invalid["logs"].as_array_mut().unwrap().push(duplicate);
            }
            _ => unreachable!(),
        }
        let result = value(
            journal
                .observe_receipt(
                    "token-unresolved".into(),
                    invalid.to_string(),
                    block_hash,
                    100,
                )
                .unwrap(),
        );
        assert_eq!(result["phase"], "unresolved", "{mutation}");
        assert_eq!(result["execution"], "unknown", "{mutation}");
        assert_eq!(result["inclusion"], "finalized", "{mutation}");
        assert_eq!(result["receipt_status"], 1);
    }
}

#[test]
fn receipt_gas_charge_is_not_rejected_for_being_below_reserved_calldata_floor() {
    let (_directory, journal, _path) = journal();
    let (signed, _) = signed_record(&journal, "gas-charge-not-reservation", 0, "USDC");
    let block_hash = hash(18);
    let mut evidence = receipt(&signed, 100, &block_hash, true);
    // A synthetic RPC observation verifies our accounting boundary. It is not
    // an assertion that Circle's current transfer implementation uses 21k gas.
    // EIP-7623's gas-limit reservation must not become an unsupported receipt
    // rejection rule. Receipt identity, expected log and upper caps still apply.
    evidence["gasUsed"] = json!("0x5208");
    let result = value(
        journal
            .observe_receipt(
                "gas-charge-not-reservation".into(),
                evidence.to_string(),
                block_hash,
                100,
            )
            .unwrap(),
    );
    assert_eq!(result["execution"], "success");
    assert_eq!(result["actual_fee_wei"], "31500000000000");
}

#[test]
fn bounded_receipts_allow_contract_eth_receivers_with_many_logs() {
    let (_directory, journal, _path) = journal();
    let mut request = request("many-receiver-logs", 0, "ETH");
    request["gas_limit"] = json!(300_000);
    let prepared = crypto::prepare_payment(request.to_string()).unwrap();
    journal.record_prepared(prepared.clone()).unwrap();
    let signed = sign(&prepared);
    journal.record_signed(signed.clone()).unwrap();
    let signed_value: Value = serde_json::from_str(&signed).unwrap();
    let block_hash = hash(19);
    let mut evidence = receipt(&signed, 100, &block_hash, true);
    evidence["gasUsed"] = json!("0x445c0"); // 280,000, within the reviewed limit.
    evidence["logs"] = Value::Array(
        (0..600)
            .map(|index| {
                json!({
                    "address":TO,"topics":[],"data":"0x","transactionHash":signed_value["hash"],
                    "blockHash":block_hash,"blockNumber":"0x64","logIndex":format!("0x{index:x}")
                })
            })
            .collect(),
    );
    let result = value(
        journal
            .observe_receipt(
                "many-receiver-logs".into(),
                evidence.to_string(),
                block_hash.clone(),
                100,
            )
            .unwrap(),
    );
    assert_eq!(result["execution"], "success");
    // Bounded does not mean accepting arbitrarily large provider responses.
    let oversized = " ".repeat(1_048_577);
    assert!(journal
        .observe_receipt("many-receiver-logs".into(), oversized, block_hash, 100)
        .is_err());
}

#[test]
fn invalid_receipt_identity_fees_and_logs_do_not_change_records() {
    for mutation in [
        "hash",
        "from",
        "to",
        "canonical",
        "gas",
        "gas_one",
        "gas_below_intrinsic",
        "price",
        "status",
        "type",
        "removed",
        "log_hash",
        "log_block",
        "duplicate_index",
        "quantity",
    ] {
        let (_directory, journal, _path) = journal();
        let (signed, original) = signed_record(&journal, "reject-receipt", 0, "USDC");
        let block_hash = hash(6);
        let mut evidence = receipt(&signed, 100, &block_hash, true);
        let mut canonical = block_hash;
        match mutation {
            "hash" => evidence["transactionHash"] = json!(hash(7)),
            "from" => evidence["from"] = json!(TO),
            "to" => evidence["to"] = json!(TO),
            "canonical" => canonical = hash(7),
            "gas" => evidence["gasUsed"] = json!("0xfffff"),
            "gas_one" => evidence["gasUsed"] = json!("0x1"),
            "gas_below_intrinsic" => evidence["gasUsed"] = json!("0x5207"),
            "price" => evidence["effectiveGasPrice"] = json!("0xffffffffff"),
            "status" => evidence["status"] = json!("0x2"),
            "type" => evidence["type"] = json!("0x4"),
            "removed" => evidence["logs"][0]["removed"] = json!(true),
            "log_hash" => evidence["logs"][0]["transactionHash"] = json!(hash(7)),
            "log_block" => evidence["logs"][0]["blockHash"] = json!(hash(7)),
            "duplicate_index" => {
                let duplicate = evidence["logs"][0].clone();
                evidence["logs"].as_array_mut().unwrap().push(duplicate);
            }
            "quantity" => evidence["blockNumber"] = json!("0x064"),
            _ => unreachable!(),
        }
        assert!(
            journal
                .observe_receipt(
                    "reject-receipt".into(),
                    evidence.to_string(),
                    canonical,
                    100
                )
                .is_err(),
            "{mutation}"
        );
        assert_eq!(
            original,
            value(journal.get_payment("reject-receipt".into()).unwrap())
        );
        assert_eq!(event_count(&journal, "reject-receipt"), 2);
    }
}

#[test]
fn reorg_is_an_explicit_correction_with_old_evidence_and_nonce_preserved() {
    let (_directory, journal, _path) = journal();
    let (signed, original) = signed_record(&journal, "reorg", 0, "ETH");
    let old_hash = hash(8);
    let new_hash = hash(9);
    journal
        .observe_receipt(
            "reorg".into(),
            receipt(&signed, 100, &old_hash, true).to_string(),
            old_hash.clone(),
            99,
        )
        .unwrap();
    assert!(journal
        .observe_receipt(
            "reorg".into(),
            receipt(&signed, 101, &new_hash, true).to_string(),
            new_hash.clone(),
            99
        )
        .is_err());
    assert!(journal
        .mark_reorg("reorg".into(), new_hash.clone())
        .is_err());
    let corrected = journal
        .mark_reorg("reorg".into(), old_hash.clone())
        .unwrap();
    let record = value(corrected.clone());
    assert_eq!(record["phase"], "unknown");
    assert_eq!(record["execution"], "unknown");
    assert_eq!(record["inclusion"], "unseen");
    assert!(record["actual_fee_wei"].is_null());
    assert!(record["block_hash"].is_null());
    assert_eq!(record["raw_transaction"], original["raw_transaction"]);
    assert_eq!(
        corrected,
        journal.mark_reorg("reorg".into(), old_hash).unwrap()
    );
    assert!(journal
        .record_prepared(prepare("nonce-still-reserved", 0, "ETH"))
        .is_err());
    let events = value(journal.list_events("reorg".into()).unwrap());
    assert_eq!(events[3]["kind"], "reorg");
    assert_eq!(
        events[3]["payload"]["previous_record"]["actual_fee_wei"],
        "31500000000000"
    );
    let included = value(
        journal
            .observe_receipt(
                "reorg".into(),
                receipt(&signed, 101, &new_hash, true).to_string(),
                new_hash,
                100,
            )
            .unwrap(),
    );
    assert_eq!(included["phase"], "included");
}

#[test]
fn loss_of_finalized_evidence_is_flagged_without_automatic_rebroadcast_state() {
    let (_directory, journal, _path) = journal();
    let (signed, _) = signed_record(&journal, "finality-conflict", 0, "ETH");
    let block_hash = hash(10);
    journal
        .observe_receipt(
            "finality-conflict".into(),
            receipt(&signed, 100, &block_hash, true).to_string(),
            block_hash.clone(),
            100,
        )
        .unwrap();
    let record = journal
        .mark_reorg("finality-conflict".into(), block_hash)
        .unwrap();
    assert_eq!(value(record.clone())["phase"], "unresolved");
    assert_eq!(
        record,
        journal
            .mark_submission("finality-conflict".into(), "pending".into())
            .unwrap()
    );
}

#[test]
fn inconsistent_receipts_cannot_silently_overwrite_financial_evidence() {
    let (_directory, journal, _path) = journal();
    let (signed, _) = signed_record(&journal, "provider-conflict", 0, "USDC");
    let block_hash = hash(11);
    let evidence = receipt(&signed, 100, &block_hash, true);
    let original = journal
        .observe_receipt(
            "provider-conflict".into(),
            evidence.to_string(),
            block_hash.clone(),
            99,
        )
        .unwrap();
    let mut changed = evidence;
    changed["gasUsed"] = json!("0xbf68"); // 49,000: within cap, but different facts for this block.
    assert!(journal
        .observe_receipt(
            "provider-conflict".into(),
            changed.to_string(),
            block_hash,
            99
        )
        .is_err());
    assert_eq!(
        original,
        journal.get_payment("provider-conflict".into()).unwrap()
    );
}

fn balance(block: u64, block_hash: &str) -> Value {
    json!({"chain_id":CHAIN_ID,"address":FROM,"block_number":block,"block_hash":block_hash,
        "balances":{"ETH":"1000000000000000000","USDC":"1250000","EURC":"0"}})
}

#[test]
fn balances_are_observations_and_never_inferred_or_adjusted_by_payment_events() {
    let (_directory, journal, _path) = journal();
    assert_eq!(journal.latest_balance().unwrap(), "null");
    let snapshot = balance(100, &hash(12));
    let canonical = journal.record_balance(snapshot.to_string()).unwrap();
    assert_eq!(
        canonical,
        journal.record_balance(snapshot.to_string()).unwrap()
    );
    let (_signed, _) = signed_record(&journal, "does-not-adjust-balance", 0, "ETH");
    journal
        .mark_submission("does-not-adjust-balance".into(), "pending".into())
        .unwrap();
    assert_eq!(canonical, journal.latest_balance().unwrap());
    let mut inconsistent = snapshot.clone();
    inconsistent["balances"]["ETH"] = json!("999");
    assert!(journal.record_balance(inconsistent.to_string()).is_err());
    assert!(journal
        .record_balance(balance(99, &hash(13)).to_string())
        .is_err());
    assert_eq!(canonical, journal.latest_balance().unwrap());
    // Another canonical hash at the same height is separate observed evidence.
    let newer = journal
        .record_balance(balance(100, &hash(14)).to_string())
        .unwrap();
    assert_eq!(newer, journal.latest_balance().unwrap());
}

#[test]
fn balances_reject_unsupported_assets_accounts_and_unsafe_number_formats() {
    let (_directory, journal, _path) = journal();
    journal
        .record_balance(balance(100, &hash(15)).to_string())
        .unwrap();
    for mutation in [
        "chain",
        "account",
        "negative",
        "fraction",
        "leading_zero",
        "overflow",
        "extra_asset",
    ] {
        let mut invalid = balance(101, &hash(16));
        match mutation {
            "chain" => invalid["chain_id"] = json!(11_155_111),
            "account" => invalid["address"] = json!(TO),
            "negative" => invalid["balances"]["ETH"] = json!("-1"),
            "fraction" => invalid["balances"]["ETH"] = json!("1.5"),
            "leading_zero" => invalid["balances"]["ETH"] = json!("01"),
            "overflow" => invalid["balances"]["ETH"] = json!("9".repeat(78)),
            "extra_asset" => invalid["balances"]["BTC"] = json!("1"),
            _ => unreachable!(),
        }
        assert!(
            journal.record_balance(invalid.to_string()).is_err(),
            "{mutation}"
        );
    }
}

#[test]
fn concurrent_writers_cannot_reserve_one_nonce_twice() {
    let (_directory, first, path) = journal();
    let second = Journal::open(path).unwrap();
    let first_prepared = prepare("writer-a", 0, "ETH");
    let second_prepared = prepare("writer-b", 0, "ETH");
    let barrier = Arc::new(Barrier::new(2));
    let cloned = barrier.clone();
    let a = std::thread::spawn(move || {
        cloned.wait();
        first.record_prepared(first_prepared)
    });
    let b = std::thread::spawn(move || {
        barrier.wait();
        second.record_prepared(second_prepared)
    });
    let a = a.join().unwrap();
    let b = b.join().unwrap();
    assert_ne!(a.is_ok(), b.is_ok());
}

#[test]
fn cancellation_signing_race_has_one_valid_outcome() {
    let (_directory, journal, _path) = journal();
    let prepared = prepare("race", 0, "ETH");
    journal.record_prepared(prepared.clone()).unwrap();
    let signed = sign(&prepared);
    let journal = Arc::new(journal);
    let barrier = Arc::new(Barrier::new(2));
    let a_journal = journal.clone();
    let a_barrier = barrier.clone();
    let a = std::thread::spawn(move || {
        a_barrier.wait();
        a_journal.record_signed(signed)
    });
    let b_journal = journal.clone();
    let b = std::thread::spawn(move || {
        barrier.wait();
        b_journal.cancel_prepared("race".into())
    });
    let a = a.join().unwrap();
    let b = b.join().unwrap();
    assert_ne!(a.is_ok(), b.is_ok());
    let state = value(journal.get_payment("race".into()).unwrap());
    assert!(state["phase"] == "signed" || state["phase"] == "cancelled");
    assert_eq!(
        !state["raw_transaction"].is_null(),
        state["phase"] == "signed"
    );
}

#[test]
fn failed_audit_write_rolls_back_signature_and_never_returns_broadcast_bytes() {
    let (_directory, journal, path) = journal();
    let prepared = prepare("atomic-sign", 0, "ETH");
    journal.record_prepared(prepared.clone()).unwrap();
    let connection = Connection::open(path).unwrap();
    connection.execute_batch("CREATE TRIGGER inject_failure BEFORE INSERT ON payment_events WHEN NEW.kind = 'signed' BEGIN SELECT RAISE(ABORT, 'injected audit failure'); END;").unwrap();
    assert!(journal.record_signed(sign(&prepared)).is_err());
    let record = value(journal.get_payment("atomic-sign".into()).unwrap());
    assert_eq!(record["phase"], "prepared");
    assert!(record["raw_transaction"].is_null());
    assert!(record["hash"].is_null());
    assert_eq!(event_count(&journal, "atomic-sign"), 1);
    let signed: Option<String> = connection
        .query_row(
            "SELECT signed_json FROM payments WHERE intent_id = 'atomic-sign'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(signed.is_none());
}

#[test]
fn append_only_guards_and_read_validation_prevent_silent_history_changes() {
    let (_directory, journal, path) = journal();
    let (_signed, record) = signed_record(&journal, "immutable", 0, "ETH");
    journal
        .record_balance(balance(100, &hash(17)).to_string())
        .unwrap();
    let connection = Connection::open(path).unwrap();
    for statement in [
        "DELETE FROM payments",
        "UPDATE payments SET signed_json = '{}'",
        "UPDATE payments SET prepared_json = '{}'",
        "DELETE FROM payment_events",
        "UPDATE payment_events SET kind = 'forged'",
        "DELETE FROM balance_snapshots",
        "UPDATE balance_snapshots SET snapshot_json = '{}'",
        "UPDATE metadata SET value = 'forged'",
    ] {
        assert!(connection.execute(statement, []).is_err(), "{statement}");
    }
    // A projection is mutable by design; signatures and original approvals are
    // independently checked when the record is read back for possible broadcast.
    let mut corrupt = record;
    corrupt["to"] = json!(FROM);
    connection
        .execute(
            "UPDATE payments SET record_json = ?1 WHERE intent_id = ?2",
            params![corrupt.to_string(), "immutable"],
        )
        .unwrap();
    assert!(journal.get_payment("immutable".into()).is_err());
}
