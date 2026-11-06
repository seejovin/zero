//! These are offline policy fixtures, not ZK proofs or deployable operations.
//! No SDK verifier, network provider, signing key, or wallet secret is used.
use primitive_types::U256;
use serde_json::{json, Value};
use wallet_core::private_payment::*;

fn hash(n: u8) -> String {
    format!("0x{}", hex::encode([n; 32]))
}
fn domain() -> PrivateDomain {
    PrivateDomain {
        protocol: CANDIDATE_PROTOCOL.into(),
        chain_id: CANDIDATE_CHAIN_ID,
        pool_contract: CANDIDATE_POOL.into(),
        token_contract: "0xaf88d065e77c8cc2239327c5edb3a432268e5831".into(),
        token_decimals: 6,
        verifier_digest: hash(1),
    }
}
fn fee() -> FeeTerms {
    FeeTerms {
        schedule_id: hash(2),
        fee_units: "20000".into(),
        fee_recipient_commitment: hash(3),
        fee_output_commitment: hash(4),
        max_native_fee_wei: "1000000000000000".into(),
        gas_limit: 2_000_000,
        quote_valid_until: 2000,
    }
}
fn note(n: u8, units: &str) -> PrivateInputNote {
    PrivateInputNote {
        commitment: hash(n),
        nullifier: hash(n + 32),
        units: units.into(),
    }
}
fn intent() -> PrivateIntent {
    let mut p = PrivateIntent {
        schema_version: 2,
        tree_number: 0,
        intent_id: hash(5),
        domain: domain(),
        recipient: "0zk-private-fixture-not-an-address".into(),
        amount_units: "100103456".into(),
        fee: fee(),
        input_notes: vec![note(6, "100123456")],
        state_root: hash(7),
        output_commitments: vec![hash(8), hash(4)],
        recipient_output_commitment: hash(8),
        change_units: "0".into(),
        change_output_commitment: None,
        padding_output_commitments: vec![],
        created_at: 1000,
        approval_digest: String::new(),
    };
    p.approval_digest = intent_digest(&p).unwrap();
    p
}
fn envelope(p: &PrivateIntent) -> PrivateEnvelope {
    let mut e = PrivateEnvelope {
        schema_version: 2,
        tree_number: p.tree_number,
        domain: p.domain.clone(),
        state_root: p.state_root.clone(),
        nullifiers: p.input_notes.iter().map(|n| n.nullifier.clone()).collect(),
        output_commitments: p.output_commitments.clone(),
        encrypted_outputs: p
            .output_commitments
            .iter()
            .map(|_| format!("0x{}", "ab".repeat(64)))
            .collect(),
        proof: format!("0x{}", "cd".repeat(256)),
        call_data: format!("0x{}", "ef".repeat(128)),
        fee: p.fee.clone(),
        operation_digest: String::new(),
    };
    e.operation_digest = envelope_digest(&e).unwrap();
    e
}

#[test]
fn exact_maximum_uses_all_atomic_units_and_actual_note_plan() {
    let req = PrivateMaxRequest {
        input_notes: vec![note(1, "100000000"), note(2, "123456")],
        fee_units: "20000".into(),
        max_inputs: 2,
    };
    let max = maximum_transfer(&req).unwrap();
    assert_eq!(max.total_input_units, "100123456");
    assert_eq!(max.recipient_units, "100103456");
    assert_eq!(max.change_units, "0");
    let mut req = req;
    req.max_inputs = 1;
    assert!(
        maximum_transfer(&req).is_err(),
        "must not silently drop the second note"
    );
}

#[test]
fn maximum_never_accepts_duplicate_notes_or_overflow_or_insufficient_fee() {
    let make = |notes, fee: &str| PrivateMaxRequest {
        input_notes: notes,
        fee_units: fee.into(),
        max_inputs: 16,
    };
    assert!(maximum_transfer(&make(vec![note(1, "2"), note(1, "2")], "1")).is_err());
    let mut duplicate_nullifier = note(2, "3");
    duplicate_nullifier.nullifier = note(1, "3").nullifier.clone();
    assert!(maximum_transfer(&make(vec![note(1, "3"), duplicate_nullifier], "1")).is_err());
    let max = U256::MAX.to_string();
    assert!(maximum_transfer(&make(vec![note(1, &max), note(2, "1")], "1")).is_err());
    for fee in ["0", "03", "-1", "1.5", "2", "3"] {
        assert!(maximum_transfer(&make(vec![note(1, "2")], fee)).is_err());
    }
    assert!(maximum_transfer(&make(vec![], "1")).is_err());
    assert!(maximum_transfer(&make(vec![note(1, "0")], "1")).is_err());
    let result = maximum_transfer(&make(vec![note(1, &max)], "1")).unwrap();
    assert_eq!(
        result.recipient_units,
        (U256::MAX - U256::one()).to_string()
    );
}

#[test]
fn identical_commitments_at_distinct_positions_have_distinct_nullifiers() {
    let first = note(1, "123456789");
    let mut second = first.clone();
    second.nullifier = hash(90);
    let maximum = maximum_transfer(&PrivateMaxRequest {
        input_notes: vec![first, second],
        fee_units: "1".into(),
        max_inputs: 2,
    })
    .unwrap();
    assert_eq!(maximum.recipient_units, "246913577");
}

#[test]
fn intent_requires_exact_conservation_and_complete_output_set() {
    let p = intent();
    validate_intent(&p).unwrap();
    let mut wrong = p.clone();
    wrong.amount_units = "100103455".into();
    assert!(intent_digest(&wrong).is_err());
    wrong.change_units = "1".into();
    wrong.change_output_commitment = Some(hash(9));
    wrong.output_commitments.push(hash(9));
    wrong.approval_digest = intent_digest(&wrong).unwrap();
    validate_intent(&wrong).unwrap();
    wrong.output_commitments.push(hash(10));
    assert!(
        intent_digest(&wrong).is_err(),
        "unapproved output must fail"
    );
    wrong.padding_output_commitments.push(hash(10));
    wrong.approval_digest = intent_digest(&wrong).unwrap();
    validate_intent(&wrong).unwrap();
    wrong.padding_output_commitments.push(hash(10));
    assert!(intent_digest(&wrong).is_err());
}

#[test]
fn local_approval_digest_binds_recipient_amount_and_fee() {
    let p = intent();
    let mut changed = p.clone();
    changed.recipient = "0zk-different-private-recipient".into();
    assert!(validate_intent(&changed).is_err());
    assert_ne!(intent_digest(&changed).unwrap(), p.approval_digest);
    let mut changed = p.clone();
    changed.amount_units = "100093456".into();
    changed.fee.fee_units = "30000".into();
    assert!(validate_intent(&changed).is_err());
    assert_ne!(intent_digest(&changed).unwrap(), p.approval_digest);
}

#[test]
fn daily_shared_fee_deadline_survives_native_intent_and_review_binding() {
    // The service signs one common deadline, which Android preserves exactly.
    // Native review must accept a freshly published daily schedule, not only
    // its final hour. A longer, backwards or overflowing window remains invalid.
    for duration in [1, 3_600, 86_400] {
        let mut p = intent();
        p.fee.quote_valid_until = p.created_at + duration;
        p.approval_digest = intent_digest(&p).unwrap();
        let mut approval = PrivateTransferApproval {
            schema_version: 2,
            intent_id: p.intent_id.clone(),
            domain: p.domain.clone(),
            recipient: p.recipient.clone(),
            amount_units: p.amount_units.clone(),
            schedule_id: p.fee.schedule_id.clone(),
            fee_units: p.fee.fee_units.clone(),
            fee_recipient_commitment: p.fee.fee_recipient_commitment.clone(),
            max_native_fee_wei: p.fee.max_native_fee_wei.clone(),
            gas_limit: p.fee.gas_limit,
            quote_valid_until: p.fee.quote_valid_until,
            created_at: p.created_at,
            approval_digest: String::new(),
        };
        approval.approval_digest = transfer_approval_digest(&approval).unwrap();
        validate_intent_for_approval(&approval, &p).unwrap();
        let e = envelope(&p);
        validate_envelope_for_intent(&p, &e).unwrap();
        assert_eq!(e.fee.quote_valid_until, p.created_at + duration);

        for invalid_deadline in [p.created_at - 1, p.created_at + 86_401, u64::MAX] {
            let mut invalid_intent = p.clone();
            invalid_intent.fee.quote_valid_until = invalid_deadline;
            assert!(intent_digest(&invalid_intent).is_err());
            let mut invalid_approval = approval.clone();
            invalid_approval.quote_valid_until = invalid_deadline;
            assert!(transfer_approval_digest(&invalid_approval).is_err());
        }
    }
}

#[test]
fn public_address_cannot_be_passed_as_private_recipient() {
    for recipient in [
        "0x1234567890123456789012345678901234567890",
        "",
        "a short recipient",
        "private\nrecipient",
    ] {
        let mut p = intent();
        p.recipient = recipient.into();
        assert!(intent_digest(&p).is_err());
    }
}

#[test]
fn exact_envelope_binding_rejects_changed_fees_outputs_and_input_order() {
    let p = intent();
    let e = envelope(&p);
    validate_envelope_for_intent(&p, &e).unwrap();
    for alteration in 0..4 {
        let mut changed = e.clone();
        match alteration {
            0 => changed.fee.fee_units = "20001".into(),
            1 => changed.state_root = hash(9),
            2 => changed.nullifiers[0] = hash(10),
            _ => changed.output_commitments.swap(0, 1),
        }
        changed.operation_digest = envelope_digest(&changed).unwrap();
        assert!(validate_envelope_for_intent(&p, &changed).is_err());
    }
}

#[test]
fn operation_digest_covers_proof_ciphertext_and_exact_contract_calldata() {
    let e = envelope(&intent());
    for alteration in 0..6 {
        let mut changed = e.clone();
        match alteration {
            0 => changed.proof = format!("0x{}", "aa".repeat(256)),
            1 => changed.encrypted_outputs[0] = format!("0x{}", "bb".repeat(64)),
            2 => changed.call_data = format!("0x{}", "cc".repeat(128)),
            3 => changed.fee.gas_limit += 1,
            4 => changed.domain.verifier_digest = hash(11),
            _ => changed.fee.quote_valid_until += 1,
        }
        assert!(validate_envelope(&changed).is_err());
        assert_ne!(envelope_digest(&changed).unwrap(), e.operation_digest);
    }
}

#[test]
fn strict_json_rejects_sensitive_extra_fields_and_critical_duplicates() {
    let e = envelope(&intent());
    let text = canonical_envelope(&e).unwrap();
    let mut json: Value = serde_json::from_str(&text).unwrap();
    for field in [
        "recipient",
        "amount_units",
        "mnemonic",
        "viewing_key",
        "wallet_id",
        "memo",
    ] {
        json[field] = json!("must-never-be-sent");
        assert!(validate_envelope_json(&json.to_string()).is_err());
        json.as_object_mut().unwrap().remove(field);
    }
    let duplicate = text.replacen(
        "\"fee_units\":\"20000\"",
        "\"fee_units\":\"1\",\"fee_units\":\"20000\"",
        1,
    );
    assert!(validate_envelope_json(&duplicate).is_err());
    let duplicate = text.replacen("\"chain_id\":42161", "\"chain_id\":1,\"chain_id\":42161", 1);
    assert!(validate_envelope_json(&duplicate).is_err());
    let duplicate = text.replacen(
        "\"schema_version\":2",
        "\"schema_version\":2,\"schema_version\":2",
        1,
    );
    assert!(validate_envelope_json(&duplicate).is_err());
    json["fee"]["customer_id"] = json!("tracking");
    assert!(validate_envelope_json(&json.to_string()).is_err());
}

#[test]
fn canonical_serialization_is_independent_of_input_json_key_order() {
    let e = envelope(&intent());
    let ordered = canonical_envelope(&e).unwrap();
    let value: Value = serde_json::from_str(&ordered).unwrap();
    let reordered = value.to_string();
    assert_eq!(
        canonical_envelope(&validate_envelope_json(&reordered).unwrap()).unwrap(),
        ordered
    );
    let mut unsealed = e.clone();
    unsealed.operation_digest.clear();
    assert_eq!(
        seal_envelope_json(&serde_json::to_string(&unsealed).unwrap()).unwrap(),
        ordered
    );
    unsealed.operation_digest = hash(99);
    assert!(
        seal_envelope_json(&serde_json::to_string(&unsealed).unwrap()).is_err(),
        "existing wrong digest cannot be silently replaced"
    );
}

#[test]
fn domain_and_encoding_are_canonical_and_restricted() {
    let e = envelope(&intent());
    for alteration in 0..8 {
        let mut bad = e.clone();
        match alteration {
            0 => bad.domain.chain_id = 8453,
            1 => bad.domain.protocol = "railgun-v3".into(),
            2 => bad.domain.token_decimals = 18,
            3 => bad.domain.token_contract = "0xff970a61a04b1ca14834a43f5de4533ebddb5cc8".into(),
            4 => bad.state_root = format!("0x{}", "AB".repeat(32)),
            5 => bad.nullifiers.push(bad.nullifiers[0].clone()),
            6 => bad.encrypted_outputs.pop().map(|_| ()).unwrap(),
            _ => bad.fee.fee_output_commitment = hash(88),
        }
        assert!(envelope_digest(&bad).is_err());
    }
    let mut e = e;
    e.proof = "0xdeadbeef".into();
    assert!(envelope_digest(&e).is_err());
}

#[test]
fn shape_acceptance_never_opens_production_or_claims_to_verify_a_proof() {
    let fabricated = envelope(&intent());
    validate_envelope(&fabricated).unwrap();
    assert!(require_private_release().is_err());
    assert!(!format!("{:?}", intent()).contains("100103456"));
    assert!(!format!("{:?}", intent()).contains("private-fixture"));
    assert!(!format!("{fabricated:?}").contains(&fabricated.proof));
}

#[test]
fn oversized_payloads_and_wrong_integer_types_are_rejected() {
    let e = envelope(&intent());
    let mut bad = e.clone();
    bad.call_data = format!("0x{}", "ab".repeat(MAX_CALL_DATA_BYTES + 1));
    assert!(envelope_digest(&bad).is_err());
    let mut bad: Value = serde_json::to_value(&e).unwrap();
    bad["fee"]["fee_units"] = json!(20000);
    assert!(validate_envelope_json(&bad.to_string()).is_err());
    bad["fee"]["fee_units"] = json!("020000");
    assert!(validate_envelope_json(&bad.to_string()).is_err());
    assert!(validate_envelope_json(&" ".repeat(MAX_JSON_BYTES + 1)).is_err());
}

#[test]
fn preproof_review_binds_semantics_while_allowing_later_randomized_commitments() {
    let p = intent();
    let mut approval = PrivateTransferApproval {
        schema_version: 2,
        intent_id: p.intent_id.clone(),
        domain: p.domain.clone(),
        recipient: p.recipient.clone(),
        amount_units: p.amount_units.clone(),
        schedule_id: p.fee.schedule_id.clone(),
        fee_units: p.fee.fee_units.clone(),
        fee_recipient_commitment: p.fee.fee_recipient_commitment.clone(),
        max_native_fee_wei: p.fee.max_native_fee_wei.clone(),
        gas_limit: p.fee.gas_limit,
        quote_valid_until: p.fee.quote_valid_until,
        created_at: p.created_at,
        approval_digest: String::new(),
    };
    approval.approval_digest = transfer_approval_digest(&approval).unwrap();
    validate_intent_for_approval(&approval, &p).unwrap();
    let mut concrete = p.clone();
    concrete.recipient_output_commitment = hash(90);
    concrete.output_commitments[0] = hash(90);
    concrete.approval_digest = intent_digest(&concrete).unwrap();
    validate_intent_for_approval(&approval, &concrete).unwrap();
    concrete.recipient = "0zk-a-different-private-recipient".into();
    concrete.approval_digest = intent_digest(&concrete).unwrap();
    assert!(validate_intent_for_approval(&approval, &concrete).is_err());
    let mut concrete = p.clone();
    concrete.fee.gas_limit += 1;
    concrete.approval_digest = intent_digest(&concrete).unwrap();
    assert!(validate_intent_for_approval(&approval, &concrete).is_err());
    let mut concrete = p.clone();
    concrete.created_at += 1;
    concrete.approval_digest = intent_digest(&concrete).unwrap();
    assert!(validate_intent_for_approval(&approval, &concrete).is_err());
    let text = serde_json::to_string(&approval).unwrap().replace(
        "\"recipient\":",
        "\"unexpected_tracking\":\"no\",\"recipient\":",
    );
    assert!(seal_transfer_approval_json(&text).is_err());
}

#[test]
fn tree_identity_is_required_bound_and_never_guessed_for_legacy_envelopes() {
    let p = intent();
    let e = envelope(&p);
    let mut other = e.clone();
    other.tree_number = 1;
    assert_ne!(envelope_digest(&other).unwrap(), e.operation_digest);
    other.operation_digest = envelope_digest(&other).unwrap();
    assert!(validate_envelope_for_intent(&p, &other).is_err());
    let mut body: Value = serde_json::from_str(&canonical_envelope(&e).unwrap()).unwrap();
    body.as_object_mut().unwrap().remove("tree_number");
    body["schema_version"] = json!(1);
    assert!(validate_envelope_json(&body.to_string()).is_err());
    body["schema_version"] = json!(2);
    body["tree_number"] = json!(65536);
    assert!(validate_envelope_json(&body.to_string()).is_err());
}
