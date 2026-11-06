//! Synthetic offline recovery tests. Fixture keys/phrases are public examples;
//! no user wallet, real proof, network, mainnet transaction or funds are used.
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use wallet_core::private_bridge::PrivatePaymentJournal;
use wallet_core::private_journal::{Journal, PrivateReceiptObservation, PrivateReorgObservation};
use wallet_core::private_payment::*;
use wallet_core::private_recovery::*;
use wallet_core::private_recovery_bridge::*;

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const OTHER_PHRASE: &str =
    "legal winner thank year wave sausage worth useful legal winner thank yellow";
const SELF_RECIPIENT: &str = "0zk-recovery-self-recipient-fixture-only";
fn hash(n: u8) -> String {
    format!("0x{}", hex::encode([n; 32]))
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
fn json<T: serde::Serialize>(t: &T) -> String {
    serde_json::to_string(t).unwrap()
}
fn value(s: String) -> Value {
    serde_json::from_str(&s).unwrap()
}
fn vault(phrase: &str, key: u8) -> String {
    value(wallet_core::restore_wallet(phrase.into(), vec![key; 32]).unwrap())["vault_hex"]
        .as_str()
        .unwrap()
        .into()
}
fn domain() -> PrivateDomain {
    PrivateDomain {
        protocol: CANDIDATE_PROTOCOL.into(),
        chain_id: CANDIDATE_CHAIN_ID,
        pool_contract: CANDIDATE_POOL.into(),
        token_contract: "0xaf88d065e77c8cc2239327c5edb3a432268e5831".into(),
        token_decimals: 6,
        verifier_digest: hash(200),
    }
}
fn note(id: u8) -> PrivateInputNote {
    PrivateInputNote {
        commitment: hash(id),
        nullifier: hash(id + 32),
        units: "100000000".into(),
    }
}
fn intent(id: u8, notes: Vec<PrivateInputNote>, recipient: &str) -> PrivateIntent {
    let sum: u128 = notes.iter().map(|n| n.units.parse::<u128>().unwrap()).sum();
    let mut intent = PrivateIntent {
        schema_version: 2,
        tree_number: 0,
        intent_id: hash(id),
        domain: domain(),
        recipient: recipient.into(),
        amount_units: (sum - 20000).to_string(),
        fee: FeeTerms {
            schedule_id: hash(201),
            fee_units: "20000".into(),
            fee_recipient_commitment: hash(202),
            fee_output_commitment: hash(id + 120),
            max_native_fee_wei: "1000000000000000".into(),
            gas_limit: 2_000_000,
            quote_valid_until: now() + 600,
        },
        input_notes: notes,
        state_root: hash(210),
        output_commitments: vec![hash(id + 100), hash(id + 120)],
        recipient_output_commitment: hash(id + 100),
        change_units: "0".into(),
        change_output_commitment: None,
        padding_output_commitments: vec![],
        created_at: now(),
        approval_digest: String::new(),
    };
    intent.approval_digest = intent_digest(&intent).unwrap();
    intent
}
fn envelope(p: &PrivateIntent) -> PrivateEnvelope {
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
    e
}
fn pending(j: &Journal, p: &PrivateIntent) -> PrivateEnvelope {
    let e = envelope(p);
    j.record_approved(json(p)).unwrap();
    j.record_ready(p.intent_id.clone(), json(&e)).unwrap();
    j.mark_released(p.intent_id.clone()).unwrap();
    j.mark_unknown(p.intent_id.clone()).unwrap();
    e
}
fn path(d: &tempfile::TempDir, name: &str) -> String {
    d.path().join(name).to_str().unwrap().into()
}
fn missing(d: &tempfile::TempDir) -> Journal {
    create_quarantined_journal(
        path(d, "missing.sqlite3"),
        hash(240),
        vec![8; 32],
        vault(PHRASE, 7),
        vec![7; 32],
    )
    .unwrap()
}
fn plan(j: &Journal, notes: Vec<PrivateInputNote>, fee: &str) -> RecoverySweepPlan {
    let status = value(j.recovery_status().unwrap());
    let id = status["recovery_id"].as_str().unwrap().to_string();
    j.record_recovery_cutoff(
        id.clone(),
        json(&RecoveryCutoff {
            schema_version: 1,
            recovery_id: id,
            domain: domain(),
            block_number: 500,
            block_hash: hash(211),
            observed_at: now(),
        }),
    )
    .unwrap();
    let mut scan = RecoveryScan {
        schema_version: 1,
        domain: domain(),
        wallet_binding: status["wallet_binding"].as_str().unwrap().into(),
        cutoff_block_number: 500,
        cutoff_block_hash: hash(211),
        scanned_through_block: 500,
        scanned_block_hash: hash(211),
        canonical_scanned_block_hash: hash(211),
        finalized_height: 500,
        state_root: hash(210),
        complete_from_genesis: true,
        spendable_notes: notes
            .into_iter()
            .map(|note| RecoveryNote {
                tree_number: 0,
                note,
                created_block_number: 400,
            })
            .collect(),
        scan_digest: String::new(),
    };
    scan.scan_digest = recovery_scan_digest(&scan).unwrap();
    let mut p = RecoverySweepPlan {
        schema_version: 1,
        recovery_id: status["recovery_id"].as_str().unwrap().into(),
        own_private_recipient: SELF_RECIPIENT.into(),
        initial_scan: scan,
        max_total_fee_units: fee.into(),
        approved_at: now(),
        plan_digest: String::new(),
    };
    p.plan_digest = recovery_plan_digest(&p).unwrap();
    p
}
fn receipt(e: &PrivateEnvelope, block: u64) -> PrivateReceiptObservation {
    PrivateReceiptObservation {
        operation_digest: e.operation_digest.clone(),
        chain_id: CANDIDATE_CHAIN_ID,
        pool_contract: CANDIDATE_POOL.into(),
        transaction_hash: hash((block % 250 + 1) as u8),
        block_hash: hash(((block + 1) % 250 + 1) as u8),
        canonical_block_hash: hash(((block + 1) % 250 + 1) as u8),
        block_number: block,
        finalized_height: 520,
        execution_success: true,
        nullifiers: e.nullifiers.clone(),
        output_commitments: e.output_commitments.clone(),
        fee_units_charged: e.fee.fee_units.clone(),
        native_fee_wei: "1000000".into(),
    }
}
fn resolved_scan(plan: &RecoverySweepPlan, outputs: &[(&PrivateIntent, u64)]) -> RecoveryScan {
    let mut scan = plan.initial_scan.clone();
    scan.scanned_through_block = 520;
    scan.scanned_block_hash = hash(212);
    scan.canonical_scanned_block_hash = hash(212);
    scan.finalized_height = 520;
    scan.spendable_notes = outputs
        .iter()
        .enumerate()
        .map(|(i, (intent, block))| RecoveryNote {
            tree_number: 0,
            note: PrivateInputNote {
                commitment: intent.recipient_output_commitment.clone(),
                nullifier: hash(170 + i as u8),
                units: intent.amount_units.clone(),
            },
            created_block_number: *block,
        })
        .collect();
    scan.scan_digest = recovery_scan_digest(&scan).unwrap();
    scan
}
fn resolution(
    plan: &RecoverySweepPlan,
    scan: RecoveryScan,
    receipts: Vec<(&PrivateIntent, PrivateReceiptObservation)>,
) -> RecoveryResolution {
    RecoveryResolution {
        schema_version: 1,
        recovery_id: plan.recovery_id.clone(),
        plan_digest: plan.plan_digest.clone(),
        final_scan: scan,
        finalized_receipts: receipts
            .into_iter()
            .map(|(p, receipt)| RecoveryFinalReceipt {
                intent_id: p.intent_id.clone(),
                receipt,
            })
            .collect(),
    }
}
fn sweep(
    j: &Journal,
    plan: &RecoverySweepPlan,
    p: &PrivateIntent,
) -> (PrivateEnvelope, PrivateReceiptObservation) {
    j.record_recovery_sweep_batch(plan.recovery_id.clone(), json(p))
        .unwrap();
    let e = envelope(p);
    j.record_ready(p.intent_id.clone(), json(&e)).unwrap();
    j.mark_released(p.intent_id.clone()).unwrap();
    j.mark_unknown(p.intent_id.clone()).unwrap();
    let r = receipt(&e, 510 + p.intent_id.as_bytes()[3] as u64 % 5);
    j.observe_receipt(p.intent_id.clone(), json(&r)).unwrap();
    (e, r)
}

#[test]
fn encrypted_backup_restores_exact_unknown_envelope_under_a_different_device_key() {
    let d = tempfile::tempdir().unwrap();
    let j = Journal::open(path(&d, "source.sqlite3"), hash(240), vec![1; 32]).unwrap();
    let p = intent(1, vec![note(21)], "0zk-private-destination-fixture");
    pending(&j, &p);
    let before = j.get_payment(p.intent_id.clone()).unwrap();
    let backup = export_backup(&j, vault(PHRASE, 2), vec![2; 32]).unwrap();
    for secret in [
        PHRASE,
        &p.recipient,
        &p.amount_units,
        &p.input_notes[0].commitment,
    ] {
        assert!(!backup
            .encrypted_bytes
            .windows(secret.len())
            .any(|w| w == secret.as_bytes()));
    }
    let restored = import_backup(
        backup.encrypted_bytes,
        vault(PHRASE, 3),
        vec![3; 32],
        path(&d, "new.sqlite3"),
        hash(240),
        vec![4; 32],
    )
    .unwrap();
    assert_eq!(restored.get_payment(p.intent_id.clone()).unwrap(), before);
    assert_eq!(
        value(restored.recovery_status().unwrap())["quarantined"],
        true
    );
    assert!(restored.cancel_approved(p.intent_id.clone()).is_err());
    let fresh = intent(2, vec![note(22)], SELF_RECIPIENT);
    assert!(restored.record_approved(json(&fresh)).is_err());
    restored.mark_released(p.intent_id.clone()).unwrap();
    assert_eq!(
        value(restored.get_payment(p.intent_id.clone()).unwrap())["envelope_json"],
        value(before)["envelope_json"]
    );
    drop(restored);
    assert!(Journal::open(path(&d, "new.sqlite3"), hash(240), vec![1; 32]).is_err());
    let reopened = Journal::open(path(&d, "new.sqlite3"), hash(240), vec![4; 32]).unwrap();
    assert_eq!(
        value(reopened.recovery_status().unwrap())["quarantined"],
        true
    );
}

#[test]
fn journal_authentication_requires_the_current_vault_and_never_exports_or_changes_history() {
    let directory = tempfile::tempdir().unwrap();
    let journal = PrivatePaymentJournal::new(
        path(&directory, "authenticated.sqlite3"),
        hash(240),
        vec![1; 32],
    )
    .unwrap();
    let original_vault = vault(PHRASE, 2);
    assert!(
        private_authenticate_journal(journal.clone(), original_vault.clone(), vec![2; 32]).is_err(),
        "authentication cannot silently bind an unbound journal"
    );
    let initial_status = private_recovery_status(journal.clone()).unwrap();
    assert_eq!(value(initial_status)["wallet_binding"], Value::Null);
    // Establish a binding before there are any proof bytes to export. Subsequent
    // authentication must not act like another backup export for this READY item.
    private_export_recovery_backup(journal.clone(), original_vault.clone(), vec![2; 32]).unwrap();
    let p = intent(1, vec![note(21)], SELF_RECIPIENT);
    journal.record_approved(json(&p)).unwrap();
    journal
        .record_ready(p.intent_id.clone(), json(&envelope(&p)))
        .unwrap();
    let before = journal.get_payment(p.intent_id.clone()).unwrap();
    let events_before = journal.list_events(p.intent_id.clone()).unwrap();
    let status_before = private_recovery_status(journal.clone()).unwrap();
    assert_eq!(
        private_authenticate_journal(journal.clone(), original_vault.clone(), vec![2; 32]).unwrap(),
        status_before
    );
    // Same seed encrypted under a replacement device key is the same wallet.
    assert_eq!(
        private_authenticate_journal(journal.clone(), vault(PHRASE, 3), vec![3; 32]).unwrap(),
        status_before
    );
    assert!(
        private_authenticate_journal(journal.clone(), vault(OTHER_PHRASE, 2), vec![2; 32]).is_err(),
        "a valid differently seeded vault must be rejected"
    );
    assert!(
        private_authenticate_journal(journal.clone(), original_vault.clone(), vec![9; 32]).is_err()
    );
    assert_eq!(journal.get_payment(p.intent_id.clone()).unwrap(), before);
    assert_eq!(
        journal.list_events(p.intent_id.clone()).unwrap(),
        events_before
    );
    assert_eq!(value(before)["record"]["envelope_backed_up"], false);
    drop(journal);
    let reopened = PrivatePaymentJournal::new(
        path(&directory, "authenticated.sqlite3"),
        hash(240),
        vec![1; 32],
    )
    .unwrap();
    assert_eq!(
        private_authenticate_journal(reopened.clone(), original_vault, vec![2; 32]).unwrap(),
        status_before
    );
    assert_eq!(
        value(reopened.cancel_approved(p.intent_id.clone()).unwrap())["phase"],
        "cancelled"
    );
}

#[test]
fn created_and_imported_quarantined_journals_authenticate_without_clearing_quarantine() {
    let directory = tempfile::tempdir().unwrap();
    let original_vault = vault(PHRASE, 2);
    let created = private_create_recovery_journal(
        path(&directory, "created.sqlite3"),
        hash(240),
        vec![1; 32],
        original_vault.clone(),
        vec![2; 32],
    )
    .unwrap();
    let status =
        private_authenticate_journal(created.clone(), original_vault.clone(), vec![2; 32]).unwrap();
    assert_eq!(value(status.clone())["quarantined"], true);
    let backup =
        private_export_recovery_backup(created.clone(), original_vault, vec![2; 32]).unwrap();
    let restored_vault = vault(PHRASE, 3);
    let imported = private_import_recovery_backup(
        backup.encrypted_bytes,
        restored_vault.clone(),
        vec![3; 32],
        path(&directory, "imported.sqlite3"),
        hash(240),
        vec![4; 32],
    )
    .unwrap();
    let expected = private_recovery_status(imported.clone()).unwrap();
    assert_eq!(
        private_authenticate_journal(imported.clone(), restored_vault, vec![3; 32]).unwrap(),
        expected
    );
    assert_eq!(value(expected)["quarantined"], true);
    assert!(private_authenticate_journal(imported, vault(OTHER_PHRASE, 3), vec![3; 32]).is_err());
    assert_eq!(private_recovery_status(created).unwrap(), status);
}

#[test]
fn wrong_seed_tampering_wrong_identity_and_overwrite_are_rejected_before_import() {
    let d = tempfile::tempdir().unwrap();
    let j = Journal::open(path(&d, "source.sqlite3"), hash(240), vec![1; 32]).unwrap();
    let p = intent(1, vec![note(21)], SELF_RECIPIENT);
    pending(&j, &p);
    let v = vault(PHRASE, 2);
    let b = export_backup(&j, v.clone(), vec![2; 32]).unwrap();
    assert!(inspect_backup(
        b.encrypted_bytes.clone(),
        vault(OTHER_PHRASE, 2),
        vec![2; 32]
    )
    .is_err());
    assert!(
        export_backup(&j, vault(OTHER_PHRASE, 2), vec![2; 32]).is_err(),
        "journal seed binding cannot be replaced"
    );
    for index in [0, 10, 45, 59, b.encrypted_bytes.len() - 1] {
        let mut bytes = b.encrypted_bytes.clone();
        bytes[index] ^= 1;
        assert!(import_backup(
            bytes,
            v.clone(),
            vec![2; 32],
            path(&d, "bad.sqlite3"),
            hash(240),
            vec![4; 32]
        )
        .is_err());
        assert!(!std::path::Path::new(&path(&d, "bad.sqlite3")).exists());
    }
    assert!(import_backup(
        b.encrypted_bytes.clone(),
        v.clone(),
        vec![2; 32],
        path(&d, "wrong-id.sqlite3"),
        hash(241),
        vec![4; 32]
    )
    .is_err());
    assert!(!std::path::Path::new(&path(&d, "wrong-id.sqlite3")).exists());
    assert!(import_backup(
        b.encrypted_bytes,
        v,
        vec![2; 32],
        path(&d, "source.sqlite3"),
        hash(240),
        vec![4; 32]
    )
    .is_err());
    assert_eq!(
        value(j.get_payment(p.intent_id.clone()).unwrap())["record"]["phase"],
        "unknown"
    );
}

#[test]
fn independently_retained_checkpoint_detects_an_older_valid_backup() {
    let d = tempfile::tempdir().unwrap();
    let v = vault(PHRASE, 2);
    let j = Journal::open(path(&d, "source.sqlite3"), hash(240), vec![1; 32]).unwrap();
    let first = intent(1, vec![note(21)], SELF_RECIPIENT);
    pending(&j, &first);
    let old = export_backup(&j, v.clone(), vec![2; 32]).unwrap();
    let second = intent(2, vec![note(22)], SELF_RECIPIENT);
    pending(&j, &second);
    let latest = export_backup(&j, v.clone(), vec![2; 32]).unwrap();
    let stale = import_backup(
        old.encrypted_bytes,
        v.clone(),
        vec![2; 32],
        path(&d, "stale.sqlite3"),
        hash(240),
        vec![4; 32],
    )
    .unwrap();
    let id = value(stale.recovery_status().unwrap())["recovery_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(stale
        .accept_recovery_checkpoint(id, latest.checkpoint_json.clone())
        .is_err());
    assert_eq!(value(stale.recovery_status().unwrap())["quarantined"], true);
    let current = import_backup(
        latest.encrypted_bytes,
        v,
        vec![2; 32],
        path(&d, "current.sqlite3"),
        hash(240),
        vec![4; 32],
    )
    .unwrap();
    let id = value(current.recovery_status().unwrap())["recovery_id"]
        .as_str()
        .unwrap()
        .to_string();
    current
        .accept_recovery_checkpoint(id, latest.checkpoint_json)
        .unwrap();
    assert_eq!(
        value(current.recovery_status().unwrap())["quarantined"],
        false
    );
    assert!(current.cancel_approved(first.intent_id.clone()).is_err());
    assert!(current
        .record_approved(json(&intent(3, vec![note(21)], SELF_RECIPIENT)))
        .is_err());
    current
        .record_approved(json(&intent(4, vec![note(23)], SELF_RECIPIENT)))
        .unwrap();
}

#[test]
fn restored_ready_proof_cannot_be_cancelled_without_complete_release_history() {
    let d = tempfile::tempdir().unwrap();
    let v = vault(PHRASE, 2);
    let j = Journal::open(path(&d, "source.sqlite3"), hash(240), vec![1; 32]).unwrap();
    let p = intent(1, vec![note(21)], SELF_RECIPIENT);
    j.record_approved(json(&p)).unwrap();
    j.record_ready(p.intent_id.clone(), json(&envelope(&p)))
        .unwrap();
    let b = export_backup(&j, v.clone(), vec![2; 32]).unwrap();
    j.mark_released(p.intent_id.clone()).unwrap();
    let r = import_backup(
        b.encrypted_bytes,
        v,
        vec![2; 32],
        path(&d, "restored.sqlite3"),
        hash(240),
        vec![4; 32],
    )
    .unwrap();
    assert!(r.cancel_approved(p.intent_id.clone()).is_err());
    assert_eq!(
        value(r.get_payment(p.intent_id.clone()).unwrap())["record"]["phase"],
        "ready"
    );
    let id = value(r.recovery_status().unwrap())["recovery_id"]
        .as_str()
        .unwrap()
        .to_string();
    r.accept_recovery_checkpoint(id, b.checkpoint_json).unwrap();
    assert!(
        r.cancel_approved(p.intent_id.clone()).is_err(),
        "a trusted checkpoint never revokes a restored proof"
    );
    assert_eq!(
        value(r.get_payment(p.intent_id.clone()).unwrap())["record"]["release_history_incomplete"],
        true
    );
}

#[test]
fn exporting_a_ready_proof_durably_prevents_source_cancellation_without_claiming_submission() {
    let d = tempfile::tempdir().unwrap();
    let source_path = path(&d, "source.sqlite3");
    let j = Journal::open(source_path.clone(), hash(240), vec![1; 32]).unwrap();
    let p = intent(1, vec![note(21)], SELF_RECIPIENT);
    j.record_approved(json(&p)).unwrap();
    j.record_ready(p.intent_id.clone(), json(&envelope(&p)))
        .unwrap();
    let b = export_backup(&j, vault(PHRASE, 2), vec![2; 32]).unwrap();
    let saved = value(j.get_payment(p.intent_id.clone()).unwrap());
    assert_eq!(saved["record"]["phase"], "ready");
    assert_eq!(saved["record"]["release_count"], 0);
    assert_eq!(saved["record"]["envelope_backed_up"], true);
    assert!(j.cancel_approved(p.intent_id.clone()).is_err());
    drop(j);
    let source = Journal::open(source_path, hash(240), vec![1; 32]).unwrap();
    assert!(source.cancel_approved(p.intent_id.clone()).is_err());
    let restored = import_backup(
        b.encrypted_bytes,
        vault(PHRASE, 3),
        vec![3; 32],
        path(&d, "restored.sqlite3"),
        hash(240),
        vec![4; 32],
    )
    .unwrap();
    assert_eq!(
        value(restored.get_payment(p.intent_id.clone()).unwrap())["envelope_json"],
        saved["envelope_json"]
    );
    restored.mark_released(p.intent_id.clone()).unwrap();
    assert!(source
        .record_approved(json(&intent(2, vec![note(21)], SELF_RECIPIENT)))
        .is_err());
    // A copied intent without any completed proof is not an authorization.
    let unsigned = intent(3, vec![note(22)], SELF_RECIPIENT);
    source.record_approved(json(&unsigned)).unwrap();
    export_backup(&source, vault(PHRASE, 2), vec![2; 32]).unwrap();
    source.cancel_approved(unsigned.intent_id.clone()).unwrap();
}

#[test]
fn missing_history_and_unspent_scan_never_silently_clear_quarantine() {
    let d = tempfile::tempdir().unwrap();
    let j = missing(&d);
    let p = plan(&j, vec![note(21)], "20000");
    j.approve_recovery_sweep(json(&p)).unwrap();
    let no_consumption = resolution(&p, resolved_scan(&p, &[]), vec![]);
    assert!(j.complete_recovery_sweep(json(&no_consumption)).is_err());
    let still_unspent = resolution(&p, p.initial_scan.clone(), vec![]);
    assert!(j.complete_recovery_sweep(json(&still_unspent)).is_err());
    let mut bad = p.initial_scan.clone();
    bad.complete_from_genesis = false;
    assert!(recovery_scan_digest(&bad).is_err());
    let invented = RecoveryCheckpoint {
        schema_version: 1,
        journal_identity: hash(240),
        wallet_binding: p.initial_scan.wallet_binding.clone(),
        source_sequence: 0,
        source_tip: format!("0x{}", "00".repeat(32)),
        backup_digest: hash(242),
    };
    assert!(j
        .accept_recovery_checkpoint(p.recovery_id.clone(), json(&invented))
        .is_err());
    assert_eq!(value(j.recovery_status().unwrap())["quarantined"], true);
}

#[test]
fn recovery_batches_require_self_recipient_fresh_outputs_disjoint_notes_and_total_fee() {
    let d = tempfile::tempdir().unwrap();
    let j = missing(&d);
    let p = plan(&j, vec![note(21), note(22)], "20000");
    j.approve_recovery_sweep(json(&p)).unwrap();
    let wrong = intent(1, vec![note(21)], "0zk-another-private-recipient");
    assert!(j
        .record_recovery_sweep_batch(p.recovery_id.clone(), json(&wrong))
        .is_err());
    let outside = intent(1, vec![note(23)], SELF_RECIPIENT);
    assert!(j
        .record_recovery_sweep_batch(p.recovery_id.clone(), json(&outside))
        .is_err());
    let a = intent(1, vec![note(21)], SELF_RECIPIENT);
    j.record_recovery_sweep_batch(p.recovery_id.clone(), json(&a))
        .unwrap();
    j.record_recovery_sweep_batch(p.recovery_id.clone(), json(&a))
        .unwrap();
    assert!(j
        .record_recovery_sweep_batch(
            p.recovery_id.clone(),
            json(&intent(2, vec![note(21)], SELF_RECIPIENT))
        )
        .is_err());
    assert!(
        j.record_recovery_sweep_batch(
            p.recovery_id.clone(),
            json(&intent(2, vec![note(22)], SELF_RECIPIENT))
        )
        .is_err(),
        "aggregate fee cap applies across batches"
    );
    j.cancel_approved(a.intent_id.clone()).unwrap();
    j.record_recovery_sweep_batch(
        p.recovery_id.clone(),
        json(&intent(2, vec![note(22)], SELF_RECIPIENT)),
    )
    .unwrap();
}

#[test]
fn multi_batch_recovery_requires_every_input_final_and_rescans_fresh_self_outputs() {
    let d = tempfile::tempdir().unwrap();
    let j = missing(&d);
    let plan = plan(&j, vec![note(21), note(22)], "40000");
    j.approve_recovery_sweep(json(&plan)).unwrap();
    let a = intent(1, vec![note(21)], SELF_RECIPIENT);
    let (_, ra) = sweep(&j, &plan, &a);
    let partial = resolution(
        &plan,
        resolved_scan(&plan, &[(&a, ra.block_number)]),
        vec![(&a, ra.clone())],
    );
    assert!(j.complete_recovery_sweep(json(&partial)).is_err());
    let b = intent(2, vec![note(22)], SELF_RECIPIENT);
    let (_, rb) = sweep(&j, &plan, &b);
    let outputs = resolved_scan(&plan, &[(&a, ra.block_number), (&b, rb.block_number)]);
    let mut wrong = outputs.clone();
    wrong.spendable_notes[0].note.units = "1".into();
    wrong.scan_digest = recovery_scan_digest(&wrong).unwrap();
    assert!(j
        .complete_recovery_sweep(json(&resolution(
            &plan,
            wrong,
            vec![(&a, ra.clone()), (&b, rb.clone())]
        )))
        .is_err());
    j.complete_recovery_sweep(json(&resolution(
        &plan,
        outputs.clone(),
        vec![(&a, ra), (&b, rb)],
    )))
    .unwrap();
    assert_eq!(value(j.recovery_status().unwrap())["quarantined"], false);
    j.record_approved(json(&intent(
        3,
        vec![outputs.spendable_notes[0].note.clone()],
        "0zk-next-private-recipient",
    )))
    .unwrap();
}

#[test]
fn sweep_supersedes_older_unknown_proofs_and_finality_conflicts_requarantine() {
    let d = tempfile::tempdir().unwrap();
    let v = vault(PHRASE, 2);
    let source = Journal::open(path(&d, "source.sqlite3"), hash(240), vec![1; 32]).unwrap();
    let old = intent(1, vec![note(21)], "0zk-old-private-recipient");
    pending(&source, &old);
    let backup = export_backup(&source, v.clone(), vec![2; 32]).unwrap();
    let j = import_backup(
        backup.encrypted_bytes,
        v,
        vec![2; 32],
        path(&d, "recovered.sqlite3"),
        hash(240),
        vec![4; 32],
    )
    .unwrap();
    let plan = plan(&j, vec![note(21)], "20000");
    j.approve_recovery_sweep(json(&plan)).unwrap();
    let fresh = intent(2, vec![note(21)], SELF_RECIPIENT);
    let (_, r) = sweep(&j, &plan, &fresh);
    let scan = resolved_scan(&plan, &[(&fresh, r.block_number)]);
    j.complete_recovery_sweep(json(&resolution(&plan, scan, vec![(&fresh, r.clone())])))
        .unwrap();
    assert_eq!(
        value(j.get_payment(old.intent_id.clone()).unwrap())["record"]["phase"],
        "superseded"
    );
    assert!(j.mark_released(old.intent_id.clone()).is_err());
    j.mark_reorg(
        fresh.intent_id.clone(),
        json(&PrivateReorgObservation {
            block_number: r.block_number,
            previous_block_hash: r.block_hash,
            canonical_block_hash: hash(230),
            finalized_height: 520,
        }),
    )
    .unwrap();
    let status = value(j.recovery_status().unwrap());
    assert_eq!(status["quarantined"], true);
    assert_eq!(status["incident"], true);
    assert!(j
        .record_approved(json(&intent(3, vec![note(23)], SELF_RECIPIENT)))
        .is_err());
    drop(j);
    let reopened = Journal::open(path(&d, "recovered.sqlite3"), hash(240), vec![4; 32]).unwrap();
    assert_eq!(value(reopened.recovery_status().unwrap())["incident"], true);
}

#[test]
fn exact_prior_authorization_receipt_can_resolve_an_old_input_without_sweeping_it_again() {
    let d = tempfile::tempdir().unwrap();
    let v = vault(PHRASE, 2);
    let source = Journal::open(path(&d, "source.sqlite3"), hash(240), vec![1; 32]).unwrap();
    let old = intent(1, vec![note(21)], "0zk-already-approved-recipient");
    let e = pending(&source, &old);
    let b = export_backup(&source, v.clone(), vec![2; 32]).unwrap();
    let j = import_backup(
        b.encrypted_bytes,
        v,
        vec![2; 32],
        path(&d, "restore.sqlite3"),
        hash(240),
        vec![4; 32],
    )
    .unwrap();
    let p = plan(&j, vec![note(21)], "0");
    j.approve_recovery_sweep(json(&p)).unwrap();
    let r = receipt(&e, 512);
    j.observe_receipt(old.intent_id.clone(), json(&r)).unwrap();
    j.complete_recovery_sweep(json(&resolution(
        &p,
        resolved_scan(&p, &[]),
        vec![(&old, r)],
    )))
    .unwrap();
    assert_eq!(value(j.recovery_status().unwrap())["quarantined"], false);
    assert_eq!(
        value(j.get_payment(old.intent_id.clone()).unwrap())["record"]["phase"],
        "finalized"
    );
}

#[test]
fn native_bridge_roundtrip_preserves_the_exact_externally_usable_envelope() {
    let d = tempfile::tempdir().unwrap();
    let source =
        PrivatePaymentJournal::new(path(&d, "ffi-source.sqlite3"), hash(240), vec![1; 32]).unwrap();
    let p = intent(1, vec![note(21)], SELF_RECIPIENT);
    let e = envelope(&p);
    source.record_approved(json(&p)).unwrap();
    source.record_ready(p.intent_id.clone(), json(&e)).unwrap();
    source.mark_released(p.intent_id.clone()).unwrap();
    source.mark_unknown(p.intent_id.clone()).unwrap();
    let backup =
        private_export_recovery_backup(source.clone(), vault(PHRASE, 2), vec![2; 32]).unwrap();
    let replacement_vault = vault(PHRASE, 3);
    assert_eq!(
        private_inspect_recovery_backup(
            backup.encrypted_bytes.clone(),
            replacement_vault.clone(),
            vec![3; 32]
        )
        .unwrap(),
        backup.checkpoint_json
    );
    let restored = private_import_recovery_backup(
        backup.encrypted_bytes,
        replacement_vault,
        vec![3; 32],
        path(&d, "ffi-new.sqlite3"),
        hash(240),
        vec![4; 32],
    )
    .unwrap();
    assert_eq!(
        value(restored.get_payment(p.intent_id.clone()).unwrap())["envelope_json"],
        canonical_envelope(&e).unwrap()
    );
    assert_eq!(
        value(private_recovery_status(restored.clone()).unwrap())["quarantined"],
        true
    );
    assert_eq!(
        value(restored.get_payment(p.intent_id.clone()).unwrap())["record"]["phase"],
        "unknown"
    );
}

#[test]
fn empty_enrollment_must_finalize_the_durably_observed_latest_cutoff() {
    let d = tempfile::tempdir().unwrap();
    let j = missing(&d);
    let mut p = plan(&j, vec![], "0");
    let cutoff = RecoveryCutoff {
        schema_version: 1,
        recovery_id: p.recovery_id.clone(),
        domain: domain(),
        block_number: 600,
        block_hash: hash(215),
        observed_at: now(),
    };
    j.record_recovery_cutoff(p.recovery_id.clone(), json(&cutoff))
        .unwrap();
    assert!(
        j.approve_recovery_sweep(json(&p)).is_err(),
        "an older finalized empty balance cannot cover the latest start head"
    );
    let mut lowered = cutoff.clone();
    lowered.block_number = 599;
    assert!(j
        .record_recovery_cutoff(p.recovery_id.clone(), json(&lowered))
        .is_err());
    p.initial_scan.cutoff_block_number = 600;
    p.initial_scan.cutoff_block_hash = hash(215);
    p.initial_scan.scanned_through_block = 600;
    p.initial_scan.scanned_block_hash = hash(215);
    p.initial_scan.canonical_scanned_block_hash = hash(215);
    p.initial_scan.finalized_height = 599;
    assert!(recovery_scan_digest(&p.initial_scan).is_err());
    p.initial_scan.finalized_height = 600;
    p.initial_scan.scan_digest = recovery_scan_digest(&p.initial_scan).unwrap();
    p.plan_digest = recovery_plan_digest(&p).unwrap();
    j.approve_recovery_sweep(json(&p)).unwrap();
    let r = resolution(&p, p.initial_scan.clone(), vec![]);
    j.complete_recovery_sweep(json(&r)).unwrap();
    assert_eq!(value(j.recovery_status().unwrap())["quarantined"], false);
}

#[test]
fn equal_nullifier_bytes_in_different_trees_do_not_complete_the_other_sweep() {
    let d = tempfile::tempdir().unwrap();
    let j = missing(&d);
    let mut p = plan(&j, vec![note(21), note(22)], "40000");
    p.initial_scan.spendable_notes[1].tree_number = 1;
    p.initial_scan.spendable_notes[1].note.nullifier =
        p.initial_scan.spendable_notes[0].note.nullifier.clone();
    p.initial_scan.scan_digest = recovery_scan_digest(&p.initial_scan).unwrap();
    p.plan_digest = recovery_plan_digest(&p).unwrap();
    j.approve_recovery_sweep(json(&p)).unwrap();
    let a = intent(
        1,
        vec![p.initial_scan.spendable_notes[0].note.clone()],
        SELF_RECIPIENT,
    );
    let (_, ra) = sweep(&j, &p, &a);
    assert!(j
        .complete_recovery_sweep(json(&resolution(
            &p,
            resolved_scan(&p, &[(&a, ra.block_number)]),
            vec![(&a, ra.clone())]
        )))
        .is_err());
    let mut b = intent(
        2,
        vec![p.initial_scan.spendable_notes[1].note.clone()],
        SELF_RECIPIENT,
    );
    b.tree_number = 1;
    b.approval_digest = intent_digest(&b).unwrap();
    let (_, rb) = sweep(&j, &p, &b);
    j.complete_recovery_sweep(json(&resolution(
        &p,
        resolved_scan(&p, &[(&a, ra.block_number), (&b, rb.block_number)]),
        vec![(&a, ra), (&b, rb)],
    )))
    .unwrap();
}

#[test]
fn duplicate_funded_commitments_and_recovery_resume_are_exact() {
    let d = tempfile::tempdir().unwrap();
    let j = missing(&d);
    let first = note(21);
    let mut second = first.clone();
    second.nullifier = hash(98);
    let p = plan(&j, vec![first.clone(), second.clone()], "40000");
    j.approve_recovery_sweep(json(&p)).unwrap();
    let batch = intent(1, vec![first.clone()], SELF_RECIPIENT);
    j.record_recovery_sweep_batch(p.recovery_id.clone(), json(&batch))
        .unwrap();
    let e = envelope(&batch);
    j.record_ready(batch.intent_id.clone(), json(&e)).unwrap();
    j.mark_released(batch.intent_id.clone()).unwrap();
    j.mark_unknown(batch.intent_id.clone()).unwrap();
    let before = j.recovery_workspace().unwrap();
    let view = value(before.clone());
    assert_eq!(view["plan_json"], json(&p));
    assert_eq!(view["batches"][0]["phase"], "unknown");
    assert_eq!(
        view["remaining_note_commitments"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        view["remaining_note_commitments"][0]["nullifier"],
        second.nullifier
    );
    assert_eq!(
        view["remaining_note_commitments"][0]["commitment"],
        first.commitment
    );
    drop(j);
    let reopened = Journal::open(path(&d, "missing.sqlite3"), hash(240), vec![8; 32]).unwrap();
    assert_eq!(reopened.recovery_workspace().unwrap(), before);
    let batch2 = intent(2, vec![second], SELF_RECIPIENT);
    reopened
        .record_recovery_sweep_batch(p.recovery_id.clone(), json(&batch2))
        .unwrap();
    assert!(
        value(reopened.recovery_workspace().unwrap())["remaining_note_commitments"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    reopened.cancel_approved(batch2.intent_id.clone()).unwrap();
    assert_eq!(
        value(reopened.recovery_workspace().unwrap())["remaining_note_commitments"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        value(reopened.recovery_status().unwrap())["quarantined"],
        true
    );
}

#[test]
fn finalized_reverted_sweep_can_complete_via_exact_retry_and_old_finality_remains_a_dependency() {
    let d = tempfile::tempdir().unwrap();
    let j = missing(&d);
    let p = plan(&j, vec![note(21)], "20000");
    j.approve_recovery_sweep(json(&p)).unwrap();
    let batch = intent(1, vec![note(21)], SELF_RECIPIENT);
    j.record_recovery_sweep_batch(p.recovery_id.clone(), json(&batch))
        .unwrap();
    let e = envelope(&batch);
    j.record_ready(batch.intent_id.clone(), json(&e)).unwrap();
    j.mark_released(batch.intent_id.clone()).unwrap();
    let mut failed = receipt(&e, 505);
    failed.execution_success = false;
    failed.nullifiers.clear();
    failed.output_commitments.clear();
    failed.fee_units_charged = "0".into();
    j.observe_receipt(batch.intent_id.clone(), json(&failed))
        .unwrap();
    assert!(
        value(j.recovery_workspace().unwrap())["remaining_note_commitments"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let success = receipt(&e, 515);
    j.observe_receipt(batch.intent_id.clone(), json(&success))
        .unwrap();
    j.complete_recovery_sweep(json(&resolution(
        &p,
        resolved_scan(&p, &[(&batch, success.block_number)]),
        vec![(&batch, success)],
    )))
    .unwrap();
    assert_eq!(value(j.recovery_status().unwrap())["quarantined"], false);
    let reorg = PrivateReorgObservation {
        block_number: failed.block_number,
        previous_block_hash: failed.block_hash,
        canonical_block_hash: hash(199),
        finalized_height: 520,
    };
    j.mark_reorg(batch.intent_id.clone(), json(&reorg)).unwrap();
    let status = value(j.recovery_status().unwrap());
    assert_eq!(status["quarantined"], true);
    assert_eq!(status["incident"], true);
}
