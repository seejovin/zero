//! Offline encrypted-journal and lifecycle checks. The fabricated envelopes in
//! this file are not valid ZK proofs. No network or user wallet is accessed.
use rusqlite::{params, Connection};
use serde_json::Value;
use std::sync::{Arc, Barrier};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;
use wallet_core::private_journal::{Journal, PrivateReceiptObservation, PrivateReorgObservation};
use wallet_core::private_payment::*;

fn hash(n: u8) -> String {
    format!("0x{}", hex::encode([n; 32]))
}
fn time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
fn value(text: String) -> Value {
    serde_json::from_str(&text).unwrap()
}
fn encoded<T: serde::Serialize>(t: &T) -> String {
    serde_json::to_string(t).unwrap()
}
fn fixture(seed: u8) -> (PrivateIntent, PrivateEnvelope) {
    let mut p = PrivateIntent {
        schema_version: 2,
        tree_number: 0,
        intent_id: hash(seed + 1),
        domain: PrivateDomain {
            protocol: CANDIDATE_PROTOCOL.into(),
            chain_id: CANDIDATE_CHAIN_ID,
            pool_contract: CANDIDATE_POOL.into(),
            token_contract: "0xaf88d065e77c8cc2239327c5edb3a432268e5831".into(),
            token_decimals: 6,
            verifier_digest: hash(200),
        },
        recipient: "0zk-secret-private-recipient-fixture-only".into(),
        amount_units: "100103456".into(),
        fee: FeeTerms {
            schedule_id: hash(201),
            fee_units: "20000".into(),
            fee_recipient_commitment: hash(202),
            fee_output_commitment: hash(seed + 80),
            max_native_fee_wei: "1000000000000000".into(),
            gas_limit: 2_000_000,
            quote_valid_until: time() + 600,
        },
        input_notes: vec![PrivateInputNote {
            commitment: hash(seed + 20),
            nullifier: hash(seed + 40),
            units: "100123456".into(),
        }],
        state_root: hash(203),
        output_commitments: vec![hash(seed + 60), hash(seed + 80)],
        recipient_output_commitment: hash(seed + 60),
        change_units: "0".into(),
        change_output_commitment: None,
        padding_output_commitments: vec![],
        created_at: time(),
        approval_digest: String::new(),
    };
    p.approval_digest = intent_digest(&p).unwrap();
    let mut e = PrivateEnvelope {
        schema_version: 2,
        tree_number: p.tree_number,
        domain: p.domain.clone(),
        state_root: p.state_root.clone(),
        nullifiers: p.input_notes.iter().map(|n| n.nullifier.clone()).collect(),
        output_commitments: p.output_commitments.clone(),
        encrypted_outputs: vec![format!("0x{}", "ab".repeat(64)); 2],
        proof: format!("0x{}", "cd".repeat(256)),
        call_data: format!("0x{}", "ef".repeat(128)),
        fee: p.fee.clone(),
        operation_digest: String::new(),
    };
    e.operation_digest = envelope_digest(&e).unwrap();
    (p, e)
}
fn db() -> (TempDir, String, Journal) {
    let d = tempfile::tempdir().unwrap();
    let path = d
        .path()
        .join("private.sqlite3")
        .to_str()
        .unwrap()
        .to_owned();
    let j = Journal::open(path.clone(), hash(240), vec![0x42; 32]).unwrap();
    (d, path, j)
}
fn reopen(path: &str) -> Journal {
    Journal::open(path.into(), hash(240), vec![0x42; 32]).unwrap()
}
fn ready(j: &Journal, p: &PrivateIntent, e: &PrivateEnvelope) {
    j.record_approved(encoded(p)).unwrap();
    j.record_ready(p.intent_id.clone(), encoded(e)).unwrap();
}
fn release(j: &Journal, p: &PrivateIntent, e: &PrivateEnvelope) {
    ready(j, p, e);
    j.mark_released(p.intent_id.clone()).unwrap();
}
fn receipt(e: &PrivateEnvelope, success: bool, finalized: bool) -> PrivateReceiptObservation {
    PrivateReceiptObservation {
        operation_digest: e.operation_digest.clone(),
        chain_id: e.domain.chain_id,
        pool_contract: e.domain.pool_contract.clone(),
        transaction_hash: hash(210),
        block_hash: hash(211),
        canonical_block_hash: hash(211),
        block_number: 500,
        finalized_height: if finalized { 500 } else { 499 },
        execution_success: success,
        nullifiers: if success {
            e.nullifiers.clone()
        } else {
            vec![]
        },
        output_commitments: if success {
            e.output_commitments.clone()
        } else {
            vec![]
        },
        fee_units_charged: if success {
            e.fee.fee_units.clone()
        } else {
            "0".into()
        },
        native_fee_wei: "50000000000".into(),
    }
}
fn same_notes_new_intent(p: &PrivateIntent) -> PrivateIntent {
    let mut p = p.clone();
    p.intent_id = hash(239);
    p.approval_digest = intent_digest(&p).unwrap();
    p
}

#[test]
fn reservations_use_tree_scoped_nullifiers_not_commitment_hashes() {
    let (_dir, path, j) = db();
    let (first, envelope) = fixture(1);
    release(&j, &first, &envelope);
    let (mut second, _) = fixture(2);
    second.input_notes[0].commitment = first.input_notes[0].commitment.clone();
    second.approval_digest = intent_digest(&second).unwrap();
    j.record_approved(encoded(&second)).unwrap();
    let mut duplicate = same_notes_new_intent(&second);
    duplicate.input_notes[0].commitment = hash(230);
    duplicate.approval_digest = intent_digest(&duplicate).unwrap();
    assert!(j.record_approved(encoded(&duplicate)).is_err());
    drop(j);
    assert_eq!(
        value(reopen(&path).list_payments().unwrap())
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn approved_and_ready_data_are_encrypted_on_disk_and_survive_reopen() {
    let (dir, path, j) = db();
    let (p, e) = fixture(1);
    ready(&j, &p, &e);
    let before = j.get_payment(p.intent_id.clone()).unwrap();
    let saved = value(before.clone());
    assert_eq!(saved["intent_json"], encoded(&p));
    assert_eq!(saved["envelope_json"], canonical_envelope(&e).unwrap());
    assert_eq!(saved["record"]["phase"], "ready");
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        for text in [
            &p.recipient,
            &p.amount_units,
            &p.input_notes[0].commitment,
            &e.proof,
        ] {
            assert!(
                !bytes.windows(text.len()).any(|w| w == text.as_bytes()),
                "SQLite and WAL must contain no plaintext private payload"
            );
        }
    }
    drop(j);
    assert_eq!(
        reopen(&path).get_payment(p.intent_id.clone()).unwrap(),
        before
    );
}

#[test]
fn wrong_key_or_wallet_identity_cannot_open_or_replace_existing_records() {
    let (_d, path, j) = db();
    let (p, e) = fixture(1);
    release(&j, &p, &e);
    let before = j.get_payment(p.intent_id.clone()).unwrap();
    drop(j);
    assert!(Journal::open(path.clone(), hash(240), vec![0x43; 32]).is_err());
    assert!(Journal::open(path.clone(), hash(241), vec![0x42; 32]).is_err());
    assert!(Journal::open(path.clone(), hash(240), vec![0x42; 31]).is_err());
    assert_eq!(
        reopen(&path).get_payment(p.intent_id.clone()).unwrap(),
        before
    );
}

#[test]
fn audit_views_do_not_export_recipient_principal_proof_or_ciphertexts() {
    let (_d, _path, j) = db();
    let (p, e) = fixture(1);
    release(&j, &p, &e);
    for text in [
        j.list_payments().unwrap(),
        j.list_events(p.intent_id.clone()).unwrap(),
    ] {
        for secret in [
            &p.recipient,
            &p.amount_units,
            &p.input_notes[0].commitment,
            &e.proof,
            &e.encrypted_outputs[0],
        ] {
            assert!(!text.contains(secret));
        }
        assert!(!text.contains("intent_json"));
        assert!(!text.contains("envelope_json"));
    }
}

#[test]
fn concurrent_duplicate_approval_is_idempotent_and_note_reservations_are_atomic() {
    let (_d, path, j) = db();
    let (p, _e) = fixture(1);
    let other = reopen(&path);
    let barrier = Arc::new(Barrier::new(2));
    let b = barrier.clone();
    let input = encoded(&p);
    let other_input = input.clone();
    let t = std::thread::spawn(move || {
        b.wait();
        other.record_approved(other_input).unwrap()
    });
    barrier.wait();
    let a = j.record_approved(input).unwrap();
    assert_eq!(a, t.join().unwrap());
    assert_eq!(
        value(j.list_events(p.intent_id.clone()).unwrap())
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(j
        .record_approved(encoded(&same_notes_new_intent(&p)))
        .is_err());
    let mut changed = p.clone();
    changed.recipient = "0zk-replaced-private-recipient".into();
    changed.approval_digest = intent_digest(&changed).unwrap();
    assert!(j.record_approved(encoded(&changed)).is_err());
}

#[test]
fn independent_connections_cannot_reserve_overlapping_inputs() {
    let (_d, path, j) = db();
    let (p, _) = fixture(1);
    let other = reopen(&path);
    let q = same_notes_new_intent(&p);
    let barrier = Arc::new(Barrier::new(2));
    let b = barrier.clone();
    let t = std::thread::spawn(move || {
        b.wait();
        other.record_approved(encoded(&q)).is_ok()
    });
    barrier.wait();
    let a = j.record_approved(encoded(&p)).is_ok();
    let b = t.join().unwrap();
    assert_ne!(a, b, "exactly one conflicting approval may commit");
    assert_eq!(
        value(j.list_payments().unwrap()).as_array().unwrap().len(),
        1
    );
}

#[test]
fn released_unknown_operations_cannot_cancel_or_release_reserved_notes() {
    let (_d, path, j) = db();
    let (p, e) = fixture(1);
    release(&j, &p, &e);
    j.mark_unknown(p.intent_id.clone()).unwrap();
    assert!(j.cancel_approved(p.intent_id.clone()).is_err());
    assert!(j
        .record_approved(encoded(&same_notes_new_intent(&p)))
        .is_err());
    drop(j);
    let j = reopen(&path);
    let before = value(j.get_payment(p.intent_id.clone()).unwrap());
    assert_eq!(before["record"]["phase"], "unknown");
    j.mark_released(p.intent_id.clone()).unwrap();
    let after = value(j.get_payment(p.intent_id.clone()).unwrap());
    assert_eq!(after["envelope_json"], before["envelope_json"]);
    assert_eq!(after["record"]["release_count"], 2);
}

#[test]
fn local_ready_cancellation_is_allowed_only_before_any_egress() {
    let (_d, _path, j) = db();
    let (p, e) = fixture(1);
    ready(&j, &p, &e);
    assert_eq!(
        value(j.cancel_approved(p.intent_id.clone()).unwrap())["phase"],
        "cancelled"
    );
    assert!(j.mark_released(p.intent_id.clone()).is_err());
    j.record_approved(encoded(&same_notes_new_intent(&p)))
        .unwrap();
}

#[test]
fn ready_operation_cannot_be_overwritten_with_changed_proof_or_fee() {
    let (_d, _path, j) = db();
    let (p, e) = fixture(1);
    ready(&j, &p, &e);
    let first = j.record_ready(p.intent_id.clone(), encoded(&e)).unwrap();
    let mut changed = e.clone();
    changed.proof = format!("0x{}", "aa".repeat(256));
    changed.operation_digest = envelope_digest(&changed).unwrap();
    assert!(j
        .record_ready(p.intent_id.clone(), encoded(&changed))
        .is_err());
    changed = e.clone();
    changed.fee.fee_units = "20001".into();
    changed.operation_digest = envelope_digest(&changed).unwrap();
    assert!(j
        .record_ready(p.intent_id.clone(), encoded(&changed))
        .is_err());
    assert_eq!(
        value(j.get_payment(p.intent_id.clone()).unwrap())["record"],
        value(first)
    );
}

#[test]
fn receipt_requires_the_exact_outputs_not_only_spent_nullifiers() {
    let (_d, _path, j) = db();
    let (p, e) = fixture(1);
    release(&j, &p, &e);
    let mut r = receipt(&e, true, false);
    r.output_commitments[0] = hash(220);
    assert!(j.observe_receipt(p.intent_id.clone(), encoded(&r)).is_err());
    r = receipt(&e, true, false);
    r.fee_units_charged = "30000".into();
    assert!(j.observe_receipt(p.intent_id.clone(), encoded(&r)).is_err());
    r = receipt(&e, true, false);
    r.canonical_block_hash = hash(220);
    assert!(j.observe_receipt(p.intent_id.clone(), encoded(&r)).is_err());
    r = receipt(&e, true, false);
    r.chain_id = 8453;
    assert!(j.observe_receipt(p.intent_id.clone(), encoded(&r)).is_err());
    assert_eq!(
        value(j.get_payment(p.intent_id.clone()).unwrap())["record"]["phase"],
        "released"
    );
    assert_eq!(
        value(
            j.observe_receipt(p.intent_id.clone(), encoded(&receipt(&e, true, false)))
                .unwrap()
        )["phase"],
        "included"
    );
    assert_eq!(
        value(j.mark_unknown(p.intent_id.clone()).unwrap())["phase"],
        "included"
    );
}

#[test]
fn finality_and_reorg_retain_reservations_and_conflicting_finality_blocks_use() {
    let (_d, path, j) = db();
    let (p, e) = fixture(1);
    release(&j, &p, &e);
    j.observe_receipt(p.intent_id.clone(), encoded(&receipt(&e, true, false)))
        .unwrap();
    let reorg = PrivateReorgObservation {
        block_number: 500,
        previous_block_hash: hash(211),
        canonical_block_hash: hash(222),
        finalized_height: 499,
    };
    assert_eq!(
        value(j.mark_reorg(p.intent_id.clone(), encoded(&reorg)).unwrap())["phase"],
        "reorg"
    );
    assert!(j.cancel_approved(p.intent_id.clone()).is_err());
    assert!(j
        .record_approved(encoded(&same_notes_new_intent(&p)))
        .is_err());
    let mut r = receipt(&e, true, true);
    r.block_hash = hash(222);
    r.canonical_block_hash = hash(222);
    assert_eq!(
        value(j.observe_receipt(p.intent_id.clone(), encoded(&r)).unwrap())["phase"],
        "finalized"
    );
    let reorg = PrivateReorgObservation {
        block_number: 500,
        previous_block_hash: hash(222),
        canonical_block_hash: hash(223),
        finalized_height: 500,
    };
    assert_eq!(
        value(j.mark_reorg(p.intent_id.clone(), encoded(&reorg)).unwrap())["phase"],
        "conflict"
    );
    assert!(j.mark_released(p.intent_id.clone()).is_err());
    drop(j);
    assert_eq!(
        value(reopen(&path).get_payment(p.intent_id.clone()).unwrap())["record"]["phase"],
        "conflict"
    );
}

#[test]
fn contradictory_inclusion_is_recorded_as_conflict_without_overwriting_prior_evidence() {
    let (_d, _path, j) = db();
    let (p, e) = fixture(1);
    release(&j, &p, &e);
    let old = receipt(&e, true, false);
    j.observe_receipt(p.intent_id.clone(), encoded(&old))
        .unwrap();
    let mut other = old.clone();
    other.transaction_hash = hash(225);
    let record = value(
        j.observe_receipt(p.intent_id.clone(), encoded(&other))
            .unwrap(),
    );
    assert_eq!(record["phase"], "conflict");
    assert_eq!(record["receipt"]["transaction_hash"], old.transaction_hash);
    assert!(j
        .record_approved(encoded(&same_notes_new_intent(&p)))
        .is_err());
}

#[test]
fn failed_atomic_execution_cannot_collect_fee_and_same_operation_can_retry() {
    let (_d, _path, j) = db();
    let (p, e) = fixture(1);
    release(&j, &p, &e);
    let mut r = receipt(&e, false, true);
    r.fee_units_charged = e.fee.fee_units.clone();
    assert!(j.observe_receipt(p.intent_id.clone(), encoded(&r)).is_err());
    r = receipt(&e, false, true);
    assert_eq!(
        value(j.observe_receipt(p.intent_id.clone(), encoded(&r)).unwrap())["phase"],
        "failed"
    );
    assert!(j.cancel_approved(p.intent_id.clone()).is_err());
    assert!(j
        .record_approved(encoded(&same_notes_new_intent(&p)))
        .is_err());
    j.mark_released(p.intent_id.clone()).unwrap();
    let mut success = receipt(&e, true, true);
    success.transaction_hash = hash(226);
    success.block_hash = hash(227);
    success.canonical_block_hash = hash(227);
    success.block_number = 501;
    success.finalized_height = 501;
    assert_eq!(
        value(
            j.observe_receipt(p.intent_id.clone(), encoded(&success))
                .unwrap()
        )["phase"],
        "finalized"
    );
}

#[test]
fn automatic_exact_proof_retry_keeps_prior_finalized_reverts_and_ignores_delayed_duplicates() {
    let (_d, path, j) = db();
    let (p, e) = fixture(1);
    release(&j, &p, &e);
    let old = receipt(&e, false, true);
    j.observe_receipt(p.intent_id.clone(), encoded(&old))
        .unwrap();
    let mut next = receipt(&e, true, true);
    next.transaction_hash = hash(225);
    next.block_number = 510;
    next.finalized_height = 510;
    next.block_hash = hash(226);
    next.canonical_block_hash = next.block_hash.clone();
    let success = j
        .observe_receipt(p.intent_id.clone(), encoded(&next))
        .unwrap();
    assert_eq!(value(success.clone())["phase"], "finalized");
    assert_eq!(
        j.observe_receipt(p.intent_id.clone(), encoded(&old))
            .unwrap(),
        success
    );
    drop(j);
    let j = reopen(&path);
    assert_eq!(
        j.observe_receipt(p.intent_id.clone(), encoded(&old))
            .unwrap(),
        success
    );
    assert_eq!(
        value(j.get_payment(p.intent_id.clone()).unwrap())["envelope_json"],
        canonical_envelope(&e).unwrap()
    );
    let reorg = PrivateReorgObservation {
        block_number: old.block_number,
        previous_block_hash: old.block_hash,
        canonical_block_hash: hash(229),
        finalized_height: 510,
    };
    assert_eq!(
        value(j.mark_reorg(p.intent_id.clone(), encoded(&reorg)).unwrap())["phase"],
        "conflict"
    );
}

#[test]
fn unfinalized_revert_cannot_authorize_a_retry_or_replace_its_receipt() {
    let (_d, _path, j) = db();
    let (p, e) = fixture(1);
    release(&j, &p, &e);
    let old = receipt(&e, false, false);
    j.observe_receipt(p.intent_id.clone(), encoded(&old))
        .unwrap();
    assert!(j.mark_released(p.intent_id.clone()).is_err());
    let mut other = receipt(&e, true, true);
    other.transaction_hash = hash(225);
    assert_eq!(
        value(
            j.observe_receipt(p.intent_id.clone(), encoded(&other))
                .unwrap()
        )["phase"],
        "conflict"
    );
    assert!(j.cancel_approved(p.intent_id.clone()).is_err());
}

#[test]
fn receipt_duplicate_fields_and_unearned_outputs_are_rejected() {
    let (_d, _path, j) = db();
    let (p, e) = fixture(1);
    release(&j, &p, &e);
    let r = encoded(&receipt(&e, true, false));
    let ambiguous = r.replace(
        "\"execution_success\":true",
        "\"execution_success\":false,\"execution_success\":true",
    );
    assert!(j.observe_receipt(p.intent_id.clone(), ambiguous).is_err());
    let mut r = receipt(&e, false, true);
    r.output_commitments = e.output_commitments.clone();
    assert!(j.observe_receipt(p.intent_id.clone(), encoded(&r)).is_err());
}

fn mutate_db(path: &str, kind: &str) {
    let c = Connection::open(path).unwrap();
    let name = if kind == "truncate" {
        "events_no_delete"
    } else {
        "events_no_update"
    };
    let trigger: String = c
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
            [name],
            |r| r.get(0),
        )
        .unwrap();
    c.execute_batch(&format!("DROP TRIGGER {name}")).unwrap();
    match kind {
        "ciphertext" => {
            c.execute(
                "UPDATE events SET ciphertext=zeroblob(length(ciphertext)) WHERE seq=1",
                [],
            )
            .unwrap();
        }
        "truncate" => {
            c.execute(
                "DELETE FROM events WHERE seq=(SELECT MAX(seq) FROM events)",
                [],
            )
            .unwrap();
        }
        "swap" => {
            let (nonce, cipher, prev, hash): (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) = c
                .query_row(
                    "SELECT nonce,ciphertext,previous_hash,event_hash FROM events WHERE seq=1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .unwrap();
            c.execute("UPDATE events SET nonce=?1,ciphertext=?2,previous_hash=?3,event_hash=?4 WHERE seq=2",params![nonce,cipher,prev,hash]).unwrap();
        }
        _ => panic!("unsupported mutation"),
    }
    c.execute_batch(&trigger).unwrap();
}

#[test]
fn ciphertext_corruption_row_substitution_and_suffix_truncation_are_detected() {
    for kind in ["ciphertext", "swap", "truncate"] {
        let (_d, path, j) = db();
        let (p, e) = fixture(1);
        release(&j, &p, &e);
        drop(j);
        mutate_db(&path, kind);
        assert!(
            Journal::open(path, hash(240), vec![0x42; 32]).is_err(),
            "mutation {kind} must fail closed"
        );
    }
}

#[test]
fn ciphertext_from_another_same_key_journal_cannot_be_transplanted() {
    let (_a, path_a, ja) = db();
    let (_b, path_b, jb) = db();
    let (p, e) = fixture(1);
    release(&ja, &p, &e);
    release(&jb, &p, &e);
    drop(ja);
    drop(jb);
    let a = Connection::open(&path_a).unwrap();
    let b = Connection::open(&path_b).unwrap();
    let (nonce, cipher, prev, event_digest): (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) = a
        .query_row(
            "SELECT nonce,ciphertext,previous_hash,event_hash FROM events WHERE seq=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    let trigger: String = b
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='events_no_update'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    b.execute_batch("DROP TRIGGER events_no_update").unwrap();
    b.execute(
        "UPDATE events SET nonce=?1,ciphertext=?2,previous_hash=?3,event_hash=?4 WHERE seq=1",
        params![nonce, cipher, prev, event_digest],
    )
    .unwrap();
    b.execute_batch(&trigger).unwrap();
    drop(a);
    drop(b);
    assert!(Journal::open(path_b, hash(240), vec![0x42; 32]).is_err());
    assert_eq!(
        value(reopen(&path_a).get_payment(p.intent_id.clone()).unwrap())["record"]["phase"],
        "released"
    );
}

#[test]
fn legacy_or_unrecognized_database_is_preserved() {
    let d = tempfile::tempdir().unwrap();
    let path = d
        .path()
        .join("existing.sqlite3")
        .to_str()
        .unwrap()
        .to_owned();
    let c = Connection::open(&path).unwrap();
    c.execute_batch("CREATE TABLE legacy(data TEXT); INSERT INTO legacy VALUES('preserve-me');")
        .unwrap();
    drop(c);
    assert!(Journal::open(path.clone(), hash(240), vec![0x42; 32]).is_err());
    let c = Connection::open(path).unwrap();
    let text: String = c
        .query_row("SELECT data FROM legacy", [], |r| r.get(0))
        .unwrap();
    assert_eq!(text, "preserve-me");
}

#[test]
fn expired_new_approval_is_rejected_without_writing_any_record() {
    let (_d, _path, j) = db();
    let (mut p, _) = fixture(1);
    p.created_at = time() - 600;
    p.fee.quote_valid_until = time() - 1;
    p.approval_digest = intent_digest(&p).unwrap();
    assert!(j.record_approved(encoded(&p)).is_err());
    assert_eq!(j.list_payments().unwrap(), "[]");
}

#[test]
fn reads_remain_consistent_while_another_connection_appends() {
    let (_d, path, j) = db();
    let other = reopen(&path);
    let (p, e) = fixture(1);
    ready(&j, &p, &e);
    let id = p.intent_id.clone();
    let t = std::thread::spawn(move || {
        for _ in 0..20 {
            other.mark_released(id.clone()).unwrap();
            other.mark_unknown(id.clone()).unwrap();
        }
    });
    for _ in 0..20 {
        let record = value(j.get_payment(p.intent_id.clone()).unwrap());
        assert_eq!(record["envelope_json"], canonical_envelope(&e).unwrap());
        j.list_payments().unwrap();
    }
    t.join().unwrap();
    assert_eq!(
        value(j.get_payment(p.intent_id.clone()).unwrap())["record"]["release_count"],
        20
    );
}

#[cfg(unix)]
#[test]
fn symlink_and_hardlink_paths_are_rejected() {
    let (d, path, j) = db();
    drop(j);
    let symlink = d.path().join("symlink.sqlite3");
    std::os::unix::fs::symlink(&path, &symlink).unwrap();
    assert!(Journal::open(symlink.to_str().unwrap().into(), hash(240), vec![0x42; 32]).is_err());
    let hardlink = d.path().join("hardlink.sqlite3");
    std::fs::hard_link(&path, &hardlink).unwrap();
    assert!(Journal::open(hardlink.to_str().unwrap().into(), hash(240), vec![0x42; 32]).is_err());
}

#[test]
fn aggregate_encrypted_history_is_rejected_before_blob_decryption_and_preserved() {
    let (_d, path, j) = db();
    let (p, e) = fixture(1);
    ready(&j, &p, &e);
    for _ in 0..32 {
        j.mark_released(p.intent_id.clone()).unwrap();
        j.mark_unknown(p.intent_id.clone()).unwrap();
    }
    drop(j);
    let c = Connection::open(&path).unwrap();
    let trigger: String = c
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='events_no_update'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // Keep the authenticated tip/count and exact schema intact. This creates
    // valid-size BLOBs whose aggregate exceeds the journal limit, so opening
    // must reject capacity before trying to decrypt the corrupted payloads.
    c.execute_batch("DROP TRIGGER events_no_update;").unwrap();
    c.execute("UPDATE events SET ciphertext=zeroblob(1048576)", [])
        .unwrap();
    c.execute_batch(&trigger).unwrap();
    let before: (i64, i64) = c
        .query_row(
            "SELECT COUNT(*),SUM(length(ciphertext)) FROM events",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(before.1 as usize > wallet_core::private_journal::MAX_ENCRYPTED_JOURNAL_BYTES);
    drop(c);
    let error = Journal::open(path.clone(), hash(240), vec![0x42; 32])
        .err()
        .unwrap();
    assert!(error.to_string().contains("capacity"));
    let c = Connection::open(path).unwrap();
    let after: (i64, i64) = c
        .query_row(
            "SELECT COUNT(*),SUM(length(ciphertext)) FROM events",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        before, after,
        "capacity failure must not clear history or release inputs"
    );
}

#[test]
fn reservations_scope_nullifiers_to_the_protocol_input_tree() {
    let (_d, _path, j) = db();
    let (p, _) = fixture(1);
    j.record_approved(encoded(&p)).unwrap();
    let mut different_tree = same_notes_new_intent(&p);
    different_tree.tree_number = 1;
    different_tree.approval_digest = intent_digest(&different_tree).unwrap();
    j.record_approved(encoded(&different_tree)).unwrap();
    let mut same_tree = p.clone();
    same_tree.intent_id = hash(238);
    same_tree.approval_digest = intent_digest(&same_tree).unwrap();
    assert!(j.record_approved(encoded(&same_tree)).is_err());
    assert_eq!(
        value(j.list_payments().unwrap()).as_array().unwrap().len(),
        2
    );
}

#[test]
fn legacy_tree_unaware_journal_is_rejected_without_overwriting_it() {
    let (_d, path, j) = db();
    let (p, _) = fixture(1);
    j.record_approved(encoded(&p)).unwrap();
    drop(j);
    let c = Connection::open(&path).unwrap();
    c.pragma_update(None, "user_version", 1).unwrap();
    let before: Vec<u8> = c
        .query_row("SELECT ciphertext FROM events WHERE seq=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    drop(c);
    let error = Journal::open(path.clone(), hash(240), vec![0x42; 32])
        .err()
        .unwrap();
    assert!(error.to_string().contains("migration"));
    let c = Connection::open(path).unwrap();
    let after: Vec<u8> = c
        .query_row("SELECT ciphertext FROM events WHERE seq=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
