//! Durable-state tests using deliberately public BIP39 fixtures. No RPC, wallet
//! secrets from a user, or submission to any network is involved.
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Barrier, OnceLock};
use tempfile::TempDir;
use wallet_core::{crypto, family_journal::Journal, solana, tron};

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const SOL_FROM: &str = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk";
const SOL_TO: &str = "GmaDrppBC7P5ARKV8g3djiwP89vz1jLK23V2GBjuAEGB";
const TRON_FROM: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";
const TRON_TO: &str = "TDp4x8kCJ6aHz8wnqZgkaVQAm2ajH1og3L";

fn value(text: String) -> Value {
    serde_json::from_str(&text).unwrap()
}
fn vault() -> &'static str {
    static VAULT: OnceLock<String> = OnceLock::new();
    VAULT.get_or_init(|| {
        value(crypto::restore_wallet(PHRASE.into(), vec![0x42; 32]).unwrap())["vault_hex"]
            .as_str()
            .unwrap()
            .into()
    })
}
fn db(network: &str, address: &str) -> (TempDir, String, Journal) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("family.sqlite3")
        .to_str()
        .unwrap()
        .to_owned();
    let journal = Journal::open(path.clone(), network.into(), address.into()).unwrap();
    (dir, path, journal)
}
fn reopen(path: &str, network: &str, address: &str) -> Journal {
    Journal::open(path.into(), network.into(), address.into()).unwrap()
}
fn sol_prepared(id: &str) -> solana::Prepared {
    solana::prepare(
        &json!({
            "intent_id":format!("family-journal-{id}"),"network":solana::NETWORK,"genesis_hash":solana::GENESIS_HASH,
            "from":SOL_FROM,"to":SOL_TO,"asset":"SOL","amount":"0.010000001",
            "recent_blockhash":bs58::encode(Sha256::digest(id.as_bytes())).into_string(),
            "last_valid_block_height":250000000,"context_slot":300000000,
            "fee_lamports":"5000","rent_lamports":"0","create_destination_ata":false,
            "source_accounts":[],"compute_unit_limit":10000,"compute_unit_price_micro_lamports":"0"
        })
        .to_string(),
    )
    .unwrap()
}
fn sol_signed(p: &solana::Prepared) -> solana::Signed {
    solana::sign(
        vault(),
        &[0x42; 32],
        &serde_json::to_string(p).unwrap(),
        &p.approval_digest,
    )
    .unwrap()
}
fn sol_evidence(p: &solana::Prepared, s: &solana::Signed, success: bool, finalized: bool) -> Value {
    let message = BASE64.decode(&p.unsigned_transaction).unwrap();
    let count = message[3] as usize;
    let keys: Vec<_> = (0..count)
        .map(|i| bs58::encode(&message[4 + i * 32..4 + (i + 1) * 32]).into_string())
        .collect();
    let sender = keys.iter().position(|k| k == SOL_FROM).unwrap();
    let recipient = keys.iter().position(|k| k == SOL_TO).unwrap();
    let mut pre = vec![0u64; count];
    let mut post = vec![0u64; count];
    pre[sender] = 1_000_000_000;
    post[sender] = 999_995_000;
    if success {
        post[sender] -= 10_000_001;
        post[recipient] = 10_000_001;
    }
    let err = if success {
        Value::Null
    } else {
        json!({"InstructionError":[2,"InsufficientFunds"]})
    };
    json!({"network":solana::NETWORK,"genesis_hash":solana::GENESIS_HASH,
        "requested_signature":s.hash,"canonical_block_slot":300000001,
        "transaction":{"slot":300000001,"transaction":[s.raw_transaction,"base64"],"version":"legacy",
            "meta":{"err":err,"fee":5000,"preBalances":pre,"postBalances":post,"preTokenBalances":[],"postTokenBalances":[],"innerInstructions":[]}},
        "signature_status":{"slot":300000001,"confirmations":if finalized {Value::Null}else{json!(1)},"err":err,"confirmationStatus":if finalized {"finalized"}else{"confirmed"}},
        "canonical_block":{"blockhash":bs58::encode([20u8;32]).into_string(),"previousBlockhash":bs58::encode([18u8;32]).into_string(),"parentSlot":300000000,"signatures":[s.hash]},
        "finalized_slot":if finalized {300000002}else{300000000}})
}
fn tron_prepared(id: &str, asset: &str) -> tron::Prepared {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    tron::prepare(&json!({"intent_id":id,"network":tron::NETWORK,"from":TRON_FROM,"to":TRON_TO,"asset":asset,
        "amount":"0.010001","maximum":false,"balance_sun":"1000000000","token_balance_units":"10001",
        "ref_block_id":format!("{:016x}{}",80000000u64,"12".repeat(24)),"ref_block_number":80000000,
        "ref_block_timestamp_ms":now-1000,"observed_at_ms":now,"recipient_active":true,"staked_bandwidth":0,"free_bandwidth":0,"available_energy":0,
        "bandwidth_price_sun":"1000","energy_price_sun":"100","creation_fee_sun":"1000000","creation_bandwidth_fee_sun":"100000",
        "creation_bandwidth_rate":1,"max_fee_limit_sun":"15000000000","max_energy_factor":34000,"energy_used":if asset=="USDT"{65000}else{0},"energy_penalty":if asset=="USDT"{35000}else{0}
    }).to_string()).unwrap()
}
fn tron_signed(p: &tron::Prepared) -> tron::Signed {
    tron::sign(
        vault().into(),
        vec![0x42; 32],
        serde_json::to_string(p).unwrap(),
        p.approval_digest.clone(),
    )
    .unwrap()
}
fn tron_evidence(p: &tron::Prepared, s: &tron::Signed, success: bool) -> Value {
    let height = 80000001u64;
    let stamp = p.created_at + 3000;
    let tx = json!({"txID":s.hash,"raw_data_hex":p.raw_data_hex,"signature":s.broadcast["signature"],"ret":[{"contractRet":if success{"SUCCESS"}else{"REVERT"}}]});
    let logs = if success && p.request.asset == "USDT" {
        vec![
            json!({"address":tron::address_hex(tron::USDT).unwrap()[2..],"topics":[
        "ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
        format!("{:0>64}",&tron::address_hex(TRON_FROM).unwrap()[2..]),format!("{:0>64}",&tron::address_hex(TRON_TO).unwrap()[2..])],"data":format!("{:064x}",10001)}),
        ]
    } else {
        vec![]
    };
    json!({"network":tron::NETWORK,"chain_id":tron::MAINNET_CHAIN_ID,"finality_source":"walletsolidity","transaction":tx,
        "info":{"id":s.hash,"fee":1000,"blockNumber":height,"blockTimeStamp":stamp,"receipt":{"net_fee":1000,"result":if success{"SUCCESS"}else{"REVERT"}},
            "contractResult":if success&&p.request.asset=="USDT"{vec![format!("{:064x}",1)]}else{vec![]},"log":logs},
        "block":{"blockID":format!("{height:016x}{}","34".repeat(24)),"block_header":{"raw_data":{"number":height,"timestamp":stamp}},"transactions":[tx]},"solid_height":height+10})
}
fn persist_sol(j: &Journal, p: &solana::Prepared, s: &solana::Signed) {
    j.record_prepared(serde_json::to_string(p).unwrap())
        .unwrap();
    j.record_signed(
        p.request.intent_id.clone(),
        serde_json::to_string(s).unwrap(),
    )
    .unwrap();
}
fn sol_balance(height: u64, block: u8, observed: u64, units: &str) -> Value {
    json!({"network":solana::NETWORK,"address":SOL_FROM,"block_number":height,"block_hash":bs58::encode([block;32]).into_string(),
        "balances":{"SOL":units,"USDC":"0","EURC":"0","USDT":"10001"},"observed_at":observed})
}

#[test]
fn signed_bytes_commit_before_use_survive_reopen_and_do_not_escape_audit() {
    let (_dir, path, journal) = db(solana::NETWORK, SOL_FROM);
    let p = sol_prepared("durable-sol");
    let s = sol_signed(&p);
    persist_sol(&journal, &p, &s);
    let before = journal.get_payment(p.request.intent_id.clone()).unwrap();
    let saved = value(before.clone());
    assert_eq!(
        value(saved["signed_json"].as_str().unwrap().into())["raw_transaction"],
        s.raw_transaction
    );
    assert_eq!(saved["record"]["amount"], "0.010000001");
    assert_eq!(saved["record"]["amount_units"], "10000001");
    for audit in [
        journal.list_payments().unwrap(),
        journal.list_events(p.request.intent_id.clone()).unwrap(),
    ] {
        assert!(!audit.contains(&s.raw_transaction));
        assert!(!audit.contains(PHRASE));
        assert!(!audit.contains("signed_json"));
    }
    drop(journal);
    assert_eq!(
        reopen(&path, solana::NETWORK, SOL_FROM)
            .get_payment(p.request.intent_id)
            .unwrap(),
        before
    );
}

#[test]
fn identical_intents_and_signatures_are_idempotent_even_across_connections() {
    let (_dir, path, journal) = db(solana::NETWORK, SOL_FROM);
    let p = sol_prepared("same-intent");
    let text = serde_json::to_string(&p).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let second = reopen(&path, solana::NETWORK, SOL_FROM);
    let b = barrier.clone();
    let other = text.clone();
    let task = std::thread::spawn(move || {
        b.wait();
        second.record_prepared(other).unwrap()
    });
    barrier.wait();
    let first = journal.record_prepared(text.clone()).unwrap();
    assert_eq!(first, task.join().unwrap());
    assert_eq!(
        value(journal.list_events(p.request.intent_id.clone()).unwrap())
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let s = sol_signed(&p);
    let signed = serde_json::to_string(&s).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let second = reopen(&path, solana::NETWORK, SOL_FROM);
    let b = barrier.clone();
    let other = signed.clone();
    let id = p.request.intent_id.clone();
    let task = std::thread::spawn(move || {
        b.wait();
        second.record_signed(id, other).unwrap()
    });
    barrier.wait();
    let first = journal
        .record_signed(p.request.intent_id.clone(), signed)
        .unwrap();
    assert_eq!(first, task.join().unwrap());
    assert_eq!(
        value(journal.list_events(p.request.intent_id.clone()).unwrap())
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(journal.record_prepared(text).unwrap(), first);
    let mut request = p.request;
    request.amount = "0.02".into();
    let altered = solana::prepare(&serde_json::to_string(&request).unwrap()).unwrap();
    assert!(journal
        .record_prepared(serde_json::to_string(&altered).unwrap())
        .is_err());
}

#[test]
fn racing_distinct_intents_cannot_reserve_two_payments() {
    let (_dir, path, journal) = db(solana::NETWORK, SOL_FROM);
    let a = sol_prepared("reserve-a");
    let b = sol_prepared("reserve-b");
    let barrier = Arc::new(Barrier::new(2));
    let second = reopen(&path, solana::NETWORK, SOL_FROM);
    let other = barrier.clone();
    let task = std::thread::spawn(move || {
        other.wait();
        second.record_prepared(serde_json::to_string(&b).unwrap())
    });
    barrier.wait();
    let first = journal.record_prepared(serde_json::to_string(&a).unwrap());
    let second = task.join().unwrap();
    assert_ne!(first.is_ok(), second.is_ok());
    assert_eq!(
        value(journal.list_payments().unwrap())
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn unknown_and_expired_signed_transactions_keep_the_reservation() {
    let (_dir, path, journal) = db(solana::NETWORK, SOL_FROM);
    let mut p = sol_prepared("historical-signed");
    let mut s = sol_signed(&p);
    // Model an authentic historical record without sleeping or changing the
    // system clock. The reviewed time is outside the signed protocol message.
    p.created_at = 1;
    p.expires_at = 1 + solana::APPROVAL_LIFETIME_SECONDS;
    p.approval_digest.clear();
    let mut digest = Sha256::new();
    digest.update(b"LocalWallet/Solana/transfer-review/v1\0");
    digest.update(serde_json::to_vec(&p).unwrap());
    p.approval_digest = hex::encode(digest.finalize());
    s.approval_digest = p.approval_digest.clone();
    assert!(solana::sign(
        vault(),
        &[0x42; 32],
        &serde_json::to_string(&p).unwrap(),
        &p.approval_digest
    )
    .is_err());
    persist_sol(&journal, &p, &s);
    let unknown = journal
        .mark_submission(p.request.intent_id.clone(), "unknown".into())
        .unwrap();
    assert_eq!(
        journal
            .mark_submission(p.request.intent_id.clone(), "unknown".into())
            .unwrap(),
        unknown
    );
    assert!(journal
        .cancel_prepared(p.request.intent_id.clone())
        .is_err());
    drop(journal);
    let journal = reopen(&path, solana::NETWORK, SOL_FROM);
    assert_eq!(
        value(journal.get_payment(p.request.intent_id).unwrap())["record"]["phase"],
        "unknown"
    );
    assert!(journal
        .record_prepared(serde_json::to_string(&sol_prepared("not-a-retry")).unwrap())
        .is_err());
}

#[test]
fn unsigned_cancellation_is_idempotent_and_never_cancels_signed_bytes() {
    let (_dir, _path, journal) = db(solana::NETWORK, SOL_FROM);
    let p = sol_prepared("cancel-review");
    let s = sol_signed(&p);
    journal
        .record_prepared(serde_json::to_string(&p).unwrap())
        .unwrap();
    let cancelled = journal
        .cancel_prepared(p.request.intent_id.clone())
        .unwrap();
    assert_eq!(
        journal
            .cancel_prepared(p.request.intent_id.clone())
            .unwrap(),
        cancelled
    );
    assert!(journal
        .record_signed(
            p.request.intent_id.clone(),
            serde_json::to_string(&s).unwrap()
        )
        .is_err());
    let next = sol_prepared("cancel-next");
    persist_sol(&journal, &next, &sol_signed(&next));
    assert!(journal.cancel_prepared(next.request.intent_id).is_err());
    assert_eq!(
        value(journal.list_events(p.request.intent_id).unwrap())
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn confirmations_and_finalized_failures_preserve_exact_status_and_actual_cost() {
    for success in [true, false] {
        let (_dir, path, journal) = db(solana::NETWORK, SOL_FROM);
        let p = sol_prepared("receipt");
        let s = sol_signed(&p);
        persist_sol(&journal, &p, &s);
        let included = sol_evidence(&p, &s, success, false);
        assert_eq!(
            value(
                journal
                    .observe_receipt(p.request.intent_id.clone(), included.to_string())
                    .unwrap()
            )["phase"],
            "included"
        );
        let final_proof = sol_evidence(&p, &s, success, true);
        let final_record = journal
            .observe_receipt(p.request.intent_id.clone(), final_proof.to_string())
            .unwrap();
        let result = value(final_record.clone());
        assert_eq!(result["phase"], "finalized");
        assert_eq!(result["actual_fee_wei"], "5000");
        assert_eq!(
            result["execution"],
            if success { "success" } else { "failed" }
        );
        assert_eq!(
            journal
                .observe_receipt(p.request.intent_id.clone(), final_proof.to_string())
                .unwrap(),
            final_record
        );
        assert_eq!(
            journal
                .observe_receipt(p.request.intent_id.clone(), included.to_string())
                .unwrap(),
            final_record
        );
        assert_eq!(
            value(journal.list_events(p.request.intent_id.clone()).unwrap())
                .as_array()
                .unwrap()
                .len(),
            4
        );
        drop(journal);
        let journal = reopen(&path, solana::NETWORK, SOL_FROM);
        assert_eq!(
            value(journal.get_payment(p.request.intent_id).unwrap())["record"],
            result
        );
        journal
            .record_prepared(serde_json::to_string(&sol_prepared("after-final")).unwrap())
            .unwrap();
    }
}

#[test]
fn late_finality_conflicts_block_an_already_prepared_next_signature() {
    let (_dir, path, journal) = db(solana::NETWORK, SOL_FROM);
    let p = sol_prepared("prior-final");
    let s = sol_signed(&p);
    persist_sol(&journal, &p, &s);
    let proof = sol_evidence(&p, &s, true, true);
    journal
        .observe_receipt(p.request.intent_id.clone(), proof.to_string())
        .unwrap();
    let next = sol_prepared("review-after-final");
    let signed_next = sol_signed(&next);
    assert_ne!(s.hash, signed_next.hash);
    journal
        .record_prepared(serde_json::to_string(&next).unwrap())
        .unwrap();
    let mut conflict = proof.clone();
    conflict["canonical_block"]["blockhash"] = json!(bs58::encode([21u8; 32]).into_string());
    let record = value(
        journal
            .observe_receipt(p.request.intent_id.clone(), conflict.to_string())
            .unwrap(),
    );
    assert_eq!(record["phase"], "unresolved");
    assert!(record["resolution_note"]
        .as_str()
        .unwrap()
        .contains("Conflicting"));
    assert!(journal
        .record_signed(
            next.request.intent_id.clone(),
            serde_json::to_string(&signed_next).unwrap()
        )
        .is_err());
    assert_eq!(
        value(
            journal
                .observe_receipt(p.request.intent_id.clone(), proof.to_string())
                .unwrap()
        )["phase"],
        "unresolved"
    );
    drop(journal);
    let journal = reopen(&path, solana::NETWORK, SOL_FROM);
    assert!(journal
        .record_signed(
            next.request.intent_id.clone(),
            serde_json::to_string(&signed_next).unwrap()
        )
        .is_err());
    journal.cancel_prepared(next.request.intent_id).unwrap();
    assert!(journal
        .record_prepared(serde_json::to_string(&sol_prepared("third")).unwrap())
        .is_err());
}

#[test]
fn malformed_or_wrong_receipt_never_changes_a_durable_payment() {
    let (_dir, _path, journal) = db(solana::NETWORK, SOL_FROM);
    let p = sol_prepared("bad-evidence");
    let s = sol_signed(&p);
    persist_sol(&journal, &p, &s);
    let before = journal.get_payment(p.request.intent_id.clone()).unwrap();
    let evidence = sol_evidence(&p, &s, true, true);
    let mut malformed = evidence.clone();
    malformed["canonical_block"]["signatures"] = json!([]);
    assert!(journal
        .observe_receipt(p.request.intent_id.clone(), malformed.to_string())
        .is_err());
    let text = evidence
        .to_string()
        .replace("\"fee\":5000", "\"fee\":9000,\"fee\":5000");
    assert!(journal
        .observe_receipt(p.request.intent_id.clone(), text)
        .is_err());
    assert_eq!(journal.get_payment(p.request.intent_id).unwrap(), before);
}

#[test]
fn tron_solid_receipts_and_large_canonical_blocks_survive_reopen() {
    for asset in ["TRX", "USDT"] {
        let (_dir, path, journal) = db(tron::NETWORK, TRON_FROM);
        let p = tron_prepared("tron-durable", asset);
        let s = tron_signed(&p);
        journal
            .record_prepared(serde_json::to_string(&p).unwrap())
            .unwrap();
        journal
            .record_signed(
                p.request.intent_id.clone(),
                serde_json::to_string(&s).unwrap(),
            )
            .unwrap();
        let mut proof = tron_evidence(&p, &s, true);
        // Canonical block RPCs can contain metadata larger than a preparation.
        proof["block"]["additional_rpc_metadata"] = json!("x".repeat(1_100_000));
        let record = value(
            journal
                .observe_receipt(p.request.intent_id.clone(), proof.to_string())
                .unwrap(),
        );
        assert_eq!(record["phase"], "finalized");
        assert_eq!(record["actual_fee_wei"], "1000");
        assert_eq!(record["amount_units"], "10001");
        drop(journal);
        assert_eq!(
            value(
                reopen(&path, tron::NETWORK, TRON_FROM)
                    .get_payment(p.request.intent_id)
                    .unwrap()
            )["record"],
            record
        );
    }
}

#[test]
fn unexpectedly_high_tron_fee_records_finality_and_surfaces_discrepancy() {
    let (_dir, _path, journal) = db(tron::NETWORK, TRON_FROM);
    let p = tron_prepared("tron-cost", "TRX");
    let s = tron_signed(&p);
    journal
        .record_prepared(serde_json::to_string(&p).unwrap())
        .unwrap();
    journal
        .record_signed(
            p.request.intent_id.clone(),
            serde_json::to_string(&s).unwrap(),
        )
        .unwrap();
    let mut proof = tron_evidence(&p, &s, true);
    let fee = p.fee_reserve_units.parse::<u64>().unwrap() + 1;
    proof["info"]["fee"] = json!(fee);
    proof["info"]["receipt"]["net_fee"] = json!(fee);
    let record = value(
        journal
            .observe_receipt(p.request.intent_id, proof.to_string())
            .unwrap(),
    );
    assert_eq!(record["phase"], "finalized");
    assert_eq!(record["actual_fee_wei"], fee.to_string());
    assert_eq!(record["fee_exceeds_reserve"], true);
    assert!(record["resolution_note"]
        .as_str()
        .unwrap()
        .contains("exceeded"));
}

#[test]
fn unexpectedly_high_solana_cost_keeps_the_actual_confirmed_result() {
    let (_dir, _path, journal) = db(solana::NETWORK, SOL_FROM);
    let p = sol_prepared("solana-actual-cost");
    let s = sol_signed(&p);
    persist_sol(&journal, &p, &s);
    let mut proof = sol_evidence(&p, &s, true, true);
    proof["transaction"]["meta"]["fee"] = json!(5_001);
    // A valid canonical receipt debits the actual fee, even when a quote was low.
    let post = &mut proof["transaction"]["meta"]["postBalances"][0];
    *post = json!(post.as_u64().unwrap() - 1);
    let record = value(
        journal
            .observe_receipt(p.request.intent_id, proof.to_string())
            .unwrap(),
    );
    assert_eq!(record["phase"], "finalized");
    assert_eq!(record["execution"], "success");
    assert_eq!(record["actual_fee_wei"], "5001");
    assert_eq!(record["actual_account_rent_units"], "0");
    assert_eq!(record["actual_total_cost_units"], "5001");
    assert_eq!(record["cost_exceeds_review"], true);
    assert!(record["resolution_note"]
        .as_str()
        .unwrap()
        .contains("exceeded"));
}

#[test]
fn balance_evidence_never_silently_overwrites_or_rolls_back() {
    let (_dir, path, journal) = db(solana::NETWORK, SOL_FROM);
    let first = sol_balance(100, 10, 1000, "1000000001");
    let second = sol_balance(101, 11, 1001, "2000000001");
    journal.record_balance(first.to_string()).unwrap();
    let newest = journal.record_balance(second.to_string()).unwrap();
    assert_eq!(journal.record_balance(first.to_string()).unwrap(), newest);
    assert_eq!(journal.record_balance(second.to_string()).unwrap(), newest);
    let mut conflict = second.clone();
    conflict["balances"]["SOL"] = json!("0");
    assert!(journal.record_balance(conflict.to_string()).is_err());
    assert!(journal
        .record_balance(sol_balance(99, 12, 1002, "0").to_string())
        .is_err());
    assert!(journal
        .record_balance(sol_balance(102, 13, 999, "0").to_string())
        .is_err());
    for bad in ["-1", "01", "18446744073709551616", "1.5", "1e6", ""] {
        let mut invalid = sol_balance(102, 13, 1002, bad);
        assert!(
            journal.record_balance(invalid.to_string()).is_err(),
            "{bad}"
        );
        invalid["balances"]["SOL"] = json!("1");
        invalid["balances"].as_object_mut().unwrap().remove("USDT");
        assert!(journal.record_balance(invalid.to_string()).is_err());
    }
    let duplicate = second
        .to_string()
        .replace("\"USDC\":\"0\"", "\"USDC\":\"1\",\"USDC\":\"0\"");
    assert!(journal.record_balance(duplicate).is_err());
    assert_eq!(journal.latest_balance().unwrap(), newest);
    drop(journal);
    assert_eq!(
        reopen(&path, solana::NETWORK, SOL_FROM)
            .latest_balance()
            .unwrap(),
        newest
    );
}

#[test]
fn activity_is_deduplicated_and_missing_or_conflicting_events_remain_visible() {
    let (_dir, path, journal) = db(solana::NETWORK, SOL_TO);
    let p = sol_prepared("incoming");
    let s = sol_signed(&p);
    let proof = sol_evidence(&p, &s, true, true);
    journal.track_transaction(s.hash.clone()).unwrap();
    let rows = journal
        .record_activity(s.hash.clone(), proof.to_string())
        .unwrap();
    assert_eq!(
        journal
            .record_activity(s.hash.clone(), proof.to_string())
            .unwrap(),
        rows
    );
    assert_eq!(value(rows)[0]["amount_units"], "10000001");
    assert_eq!(
        value(journal.list_tracked().unwrap())[0]["status"],
        "verified"
    );
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM activity", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(db);
    let failure = sol_evidence(&p, &s, false, true);
    journal
        .record_activity(s.hash.clone(), failure.to_string())
        .unwrap();
    let conflicted = value(journal.list_activity().unwrap());
    assert_eq!(conflicted[0]["verification_state"], "conflict");
    assert_eq!(conflicted[0]["amount_units"], "10000001");
    assert_eq!(
        value(journal.list_tracked().unwrap())[0]["status"],
        "conflict"
    );
    journal
        .record_activity(s.hash.clone(), proof.to_string())
        .unwrap();
    assert_eq!(value(journal.list_activity().unwrap()), conflicted);
    drop(journal);
    assert_eq!(
        value(
            reopen(&path, solana::NETWORK, SOL_TO)
                .list_activity()
                .unwrap()
        ),
        conflicted
    );
}

#[test]
fn empty_finalized_proofs_cannot_later_become_a_verified_deposit() {
    let (_dir, _path, journal) = db(solana::NETWORK, SOL_TO);
    let p = sol_prepared("empty-first");
    let s = sol_signed(&p);
    journal.track_transaction(s.hash.clone()).unwrap();
    let failed = sol_evidence(&p, &s, false, true);
    assert_eq!(
        journal
            .record_activity(s.hash.clone(), failed.to_string())
            .unwrap(),
        "[]"
    );
    assert_eq!(
        value(journal.list_tracked().unwrap())[0]["status"],
        "failed"
    );
    let wrong = bs58::encode([1u8; 64]).into_string();
    assert!(journal.record_activity(wrong, failed.to_string()).is_err());
    journal
        .record_activity(s.hash.clone(), sol_evidence(&p, &s, true, true).to_string())
        .unwrap();
    assert_eq!(
        value(journal.list_activity().unwrap())[0]["verification_state"],
        "conflict"
    );
    assert_eq!(
        value(journal.list_tracked().unwrap())[0]["status"],
        "conflict"
    );
}

#[test]
fn tron_activity_uses_verified_tx_identity_even_when_there_are_no_transfers() {
    let (_dir, _path, journal) = db(tron::NETWORK, TRON_TO);
    let p = tron_prepared("tron-incoming", "USDT");
    let s = tron_signed(&p);
    journal.track_transaction(s.hash.clone()).unwrap();
    let failed = tron_evidence(&p, &s, false);
    assert_eq!(
        journal
            .record_activity(s.hash.clone(), failed.to_string())
            .unwrap(),
        "[]"
    );
    assert!(journal
        .record_activity("11".repeat(32), failed.to_string())
        .is_err());
    journal
        .record_activity(s.hash.clone(), tron_evidence(&p, &s, true).to_string())
        .unwrap();
    assert_eq!(
        value(journal.list_tracked().unwrap())[0]["status"],
        "conflict"
    );
    assert_eq!(value(journal.list_activity().unwrap())[0]["asset"], "USDT");
}

#[test]
fn failed_storage_never_returns_a_signature_as_durable() {
    let (_dir, path, journal) = db(solana::NETWORK, SOL_FROM);
    let p = sol_prepared("disk-failure");
    let s = sol_signed(&p);
    journal
        .record_prepared(serde_json::to_string(&p).unwrap())
        .unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER simulate_full_disk BEFORE INSERT ON events WHEN NEW.kind='signed' BEGIN SELECT RAISE(ABORT,'write unavailable'); END;").unwrap();
    assert!(journal
        .record_signed(
            p.request.intent_id.clone(),
            serde_json::to_string(&s).unwrap()
        )
        .is_err());
    let saved = value(journal.get_payment(p.request.intent_id).unwrap());
    assert_eq!(saved["record"]["phase"], "prepared");
    assert!(saved["signed_json"].is_null());
    db.execute_batch("DROP TRIGGER simulate_full_disk;")
        .unwrap();
}

#[test]
fn append_only_schema_identity_and_signed_history_are_verified_on_open() {
    let (_dir, path, journal) = db(solana::NETWORK, SOL_FROM);
    let p = sol_prepared("immutable");
    let s = sol_signed(&p);
    persist_sol(&journal, &p, &s);
    let connection = Connection::open(&path).unwrap();
    assert!(connection.execute("DELETE FROM events", []).is_err());
    assert!(connection
        .execute("UPDATE identity SET address=?1", [SOL_TO])
        .is_err());
    assert!(Journal::open(path.clone(), solana::NETWORK.into(), SOL_TO.into()).is_err());
    drop(journal);
    connection
        .execute(
            "INSERT INTO events(intent,kind,body,at) VALUES(?1,'submission','\"unknown\"',-1)",
            [p.request.intent_id],
        )
        .unwrap();
    assert!(Journal::open(path.clone(), solana::NETWORK.into(), SOL_FROM.into()).is_err());
    let (_dir2, path2, journal2) = db(solana::NETWORK, SOL_FROM);
    drop(journal2);
    Connection::open(&path2)
        .unwrap()
        .execute_batch("DROP TRIGGER events_no_delete;")
        .unwrap();
    assert!(Journal::open(path2, solana::NETWORK.into(), SOL_FROM.into()).is_err());
}

#[cfg(unix)]
#[test]
fn world_readable_or_symlinked_journal_paths_are_rejected() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let (_dir, path, journal) = db(solana::NETWORK, SOL_FROM);
    drop(journal);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Journal::open(path.clone(), solana::NETWORK.into(), SOL_FROM.into()).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let alias = format!("{path}.alias");
    symlink(&path, &alias).unwrap();
    assert!(Journal::open(alias, solana::NETWORK.into(), SOL_FROM.into()).is_err());
    assert!(Journal::open(
        "relative.sqlite3".into(),
        solana::NETWORK.into(),
        SOL_FROM.into()
    )
    .is_err());
    let reopened = reopen(&path, solana::NETWORK, SOL_FROM);
    assert_eq!(reopened.list_payments().unwrap(), "[]");
}
