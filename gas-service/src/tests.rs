//! Offline correctness fixtures only. These are not ZK proofs and never reach a
//! network. The only bypass of the release gate is private to this cfg(test)
//! module. Outer transactions use a public development key and real chain ID;
//! they must never be broadcast or funded.
use super::*;
use crate::transaction::{fixture_address, sign_fixture};
use std::sync::atomic::{AtomicUsize, Ordering};
use wallet_core::private_payment::{
    envelope_digest, FeeTerms, PrivateDomain, CANDIDATE_CHAIN_ID, CANDIDATE_POOL,
    CANDIDATE_PROTOCOL,
};

pub(crate) fn hash(byte: u8) -> String {
    format!("0x{}", hex::encode([byte; 32]))
}

pub(crate) const FEE_ADDRESS: &str = "0zk1sharedfeeaddress";
pub(crate) fn policy() -> AdmissionPolicy {
    AdmissionPolicy {
        schema_version: 1,
        domain: PrivateDomain {
            protocol: CANDIDATE_PROTOCOL.into(),
            chain_id: CANDIDATE_CHAIN_ID,
            pool_contract: CANDIDATE_POOL.into(),
            token_contract: "0xaf88d065e77c8cc2239327c5edb3a432268e5831".into(),
            token_decimals: 6,
            verifier_digest: hash(7),
        },
        broadcaster_address: fixture_address(),
        schedule_id: hash(8),
        fee_recipient_commitment: crate::fee_schedule::recipient_commitment(FEE_ADDRESS).unwrap(),
        fee_units: "20000".into(),
        gas_limit: 100_000,
        max_native_fee_wei: "1000000".into(),
        valid_from: 1000,
        valid_until: 2000,
        max_pending_operations: 2,
        max_pending_native_wei: "2000000".into(),
        max_total_native_spend_wei: "5000000".into(),
        max_envelope_bytes: 400_000,
        // Synthetic test selector. No claimed deployed protocol ABI support.
        allowed_selectors: vec!["0x12345678".into()],
    }
}

pub(crate) fn envelope(number: u8) -> PrivateEnvelope {
    let p = policy();
    let mut envelope = PrivateEnvelope {
        schema_version: 2,
        tree_number: 0,
        domain: p.domain,
        state_root: hash(10),
        nullifiers: vec![hash(number)],
        output_commitments: vec![hash(100), hash(101)],
        encrypted_outputs: vec![hash(102), hash(103)],
        proof: hash(104),
        call_data: format!("0x12345678{}", hex::encode([number; 32])),
        fee: FeeTerms {
            schedule_id: p.schedule_id,
            fee_units: p.fee_units,
            fee_recipient_commitment: p.fee_recipient_commitment,
            fee_output_commitment: hash(101),
            max_native_fee_wei: p.max_native_fee_wei,
            gas_limit: p.gas_limit,
            quote_valid_until: p.valid_until,
        },
        operation_digest: hash(105),
    };
    envelope.operation_digest = envelope_digest(&envelope).unwrap();
    envelope
}

pub(crate) fn verified(envelope: &PrivateEnvelope) -> VerifiedOperation {
    VerifiedOperation {
        operation_digest: envelope.operation_digest.clone(),
        tree_number: u64::from(envelope.tree_number),
        call_data: envelope.call_data.clone(),
        fee_output_commitment: envelope.fee.fee_output_commitment.clone(),
        additional_native_fee_cap_wei: "1000".into(),
    }
}

pub(crate) struct FixtureVerifier;
impl ProofVerifier for FixtureVerifier {
    fn verify(&self, envelope: &PrivateEnvelope) -> Result<VerifiedOperation> {
        Ok(verified(envelope))
    }
}

pub(crate) struct FixtureChain;
impl ChainEvidenceVerifier for FixtureChain {
    fn verify_treasury(&self, _: &TreasuryAnchor) -> Result<()> {
        Ok(())
    }
    fn verify_pending_exposure(&self, _: &str) -> Result<()> {
        Ok(())
    }
    fn verify_admission(&self, _: &PrivateEnvelope, _: &VerifiedOperation) -> Result<()> {
        Ok(())
    }
    fn verify_receipt(
        &self,
        _: &ReceiptEvidence,
        _: &PrivateEnvelope,
        _: &VerifiedOperation,
    ) -> Result<()> {
        Ok(())
    }
    fn verify_reorg(&self, _: &ReorgEvidence) -> Result<()> {
        Ok(())
    }
}

pub(crate) fn service(path: &Path) -> GasService {
    GasService::open_harness(
        path,
        policy(),
        Arc::new(FixtureVerifier),
        Arc::new(FixtureChain),
    )
    .unwrap()
}

pub(crate) fn funded(path: &Path) -> GasService {
    let service = service(path);
    service
        .initialize_treasury(&TreasuryAnchor {
            chain_id: CANDIDATE_CHAIN_ID,
            broadcaster_address: fixture_address(),
            balance_wei: "10000000".into(),
            block_number: 100,
            block_hash: hash(120),
        })
        .unwrap();
    service
}

fn admit_and_sign(service: &GasService, n: u8, nonce: u64) -> (PrivateEnvelope, BroadcastTicket) {
    let envelope = envelope(n);
    let id = hash(n);
    service
        .admit(&id, &encode(&envelope).unwrap(), 1500)
        .unwrap();
    let raw = sign_fixture(&policy(), &verified(&envelope), nonce, 2);
    service.record_signed_outer(&id, &raw).unwrap();
    let ticket = service.release_for_broadcast(&id).unwrap();
    (envelope, ticket)
}

fn receipt(
    env: &PrivateEnvelope,
    ticket: &BroadcastTicket,
    finalized: bool,
    accepted: bool,
) -> ReceiptEvidence {
    ReceiptEvidence {
        request_id: ticket.request_id.clone(),
        transaction_hash: ticket.transaction_hash.clone(),
        chain_id: CANDIDATE_CHAIN_ID,
        block_number: 110,
        block_hash: hash(121),
        canonical_block_hash: hash(121),
        finalized_height: if finalized { 110 } else { 109 },
        gas_used: 50_000,
        effective_gas_price_wei: "2".into(),
        additional_native_fee_wei: "500".into(),
        execution: if accepted {
            PrivateExecution::Accepted {
                operation_digest: env.operation_digest.clone(),
                tree_number: 0,
                nullifiers: env.nullifiers.clone(),
                output_commitments: env.output_commitments.clone(),
                fee_output_commitment: env.fee.fee_output_commitment.clone(),
            }
        } else {
            PrivateExecution::Reverted
        },
    }
}

#[test]
fn production_gate_precedes_injected_verifier_or_funding() {
    struct CountingVerifier(Arc<AtomicUsize>);
    impl ProofVerifier for CountingVerifier {
        fn verify(&self, env: &PrivateEnvelope) -> Result<VerifiedOperation> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(verified(env))
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let counter = Arc::new(AtomicUsize::new(0));
    let service = GasService::open(
        &temp.path().join("journal.sqlite"),
        policy(),
        Arc::new(CountingVerifier(counter.clone())),
        Arc::new(FixtureChain),
    )
    .unwrap();
    assert_eq!(
        service.admit(&hash(11), &encode(&envelope(11)).unwrap(), 1500),
        Err(Error::ReleaseUnavailable)
    );
    assert_eq!(
        service.release_for_broadcast(&hash(11)),
        Err(Error::ReleaseUnavailable)
    );
    assert_eq!(counter.load(Ordering::SeqCst), 0);
    assert_eq!(service.summary().unwrap().jobs, 0);
    assert!(!capabilities().production_sponsorship_available);
}

#[test]
fn unknown_and_duplicate_customer_fields_are_rejected() {
    let mut value = serde_json::to_value(policy()).unwrap();
    value["customer_id"] = "forbidden".into();
    assert!(AdmissionPolicy::parse(&value.to_string()).is_err());
    let text = encode(&policy())
        .unwrap()
        .replacen("{", "{\"fee_units\":\"1\",", 1);
    assert!(AdmissionPolicy::parse(&text).is_err());
    let mut env = serde_json::to_value(envelope(11)).unwrap();
    env["recipient"] = "forbidden".into();
    let temp = tempfile::tempdir().unwrap();
    let service = funded(&temp.path().join("j.sqlite"));
    assert_eq!(
        service.admit(&hash(11), &env.to_string(), 1500),
        Err(Error::InvalidEncoding)
    );
    let text =
        encode(&envelope(11))
            .unwrap()
            .replacen("\"fee\":{", "\"fee\":{\"fee_units\":\"1\",", 1);
    assert_eq!(
        service.admit(&hash(11), &text, 1500),
        Err(Error::InvalidEncoding)
    );
}

#[test]
fn exact_u256_budget_does_not_pass_through_sqlite_float_or_i64() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(&temp.path().join("j.sqlite"));
    let large = "340282366920938463463374607431768211457";
    service
        .initialize_treasury(&TreasuryAnchor {
            chain_id: CANDIDATE_CHAIN_ID,
            broadcaster_address: fixture_address(),
            balance_wei: large.into(),
            block_number: 100,
            block_hash: hash(120),
        })
        .unwrap();
    service
        .admit(&hash(11), &encode(&envelope(11)).unwrap(), 1500)
        .unwrap();
    let totals = service.summary().unwrap();
    assert_eq!(
        totals.available_native_wei,
        (decimal(large).unwrap() - U256::from(1_000_000u64)).to_string()
    );
    assert_eq!(decimal("01"), Err(Error::InvalidQuantity));
    assert_eq!(decimal("1.0"), Err(Error::InvalidQuantity));
    assert_eq!(
        decimal("115792089237316195423570985008687907853269984665640564039457584007913129639936"),
        Err(Error::InvalidQuantity)
    );
}

#[test]
fn proof_and_fee_rejection_leaves_no_reservations() {
    struct WrongFee;
    impl ProofVerifier for WrongFee {
        fn verify(&self, env: &PrivateEnvelope) -> Result<VerifiedOperation> {
            let mut result = verified(env);
            result.fee_output_commitment = hash(99);
            Ok(result)
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("j.sqlite");
    let service =
        GasService::open_harness(&path, policy(), Arc::new(WrongFee), Arc::new(FixtureChain))
            .unwrap();
    assert_eq!(
        service.admit(&hash(11), &encode(&envelope(11)).unwrap(), 1500),
        Err(Error::VerificationFailed)
    );
    assert_eq!(service.summary().unwrap().jobs, 0);
    assert_eq!(service.summary().unwrap().reserved_native_wei, "0");
    let unavailable = UnavailableProofVerifier;
    assert_eq!(
        unavailable.verify(&envelope(11)),
        Err(Error::IntegrationUnavailable)
    );
}

#[test]
fn proof_adapter_cannot_substitute_unbound_calldata() {
    let env = envelope(11);
    let mut result = verified(&env);
    result.call_data.push_str("ff");
    assert_eq!(
        result.validate(&env, &policy()),
        Err(Error::VerificationFailed)
    );
}

#[test]
fn idempotent_admission_after_quote_expiry_preserves_one_reservation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("j.sqlite");
    let service = funded(&path);
    let env = encode(&envelope(11)).unwrap();
    let one = service.admit(&hash(11), &env, 1500).unwrap();
    let two = service.admit(&hash(11), &env, 5000).unwrap();
    assert_eq!(one, two);
    assert_eq!(service.summary().unwrap().jobs, 1);
    assert_eq!(service.summary().unwrap().reserved_native_wei, "1000000");
    assert_eq!(
        service.admit(&hash(12), &encode(&envelope(12)).unwrap(), 5000),
        Err(Error::PolicyMismatch)
    );
    assert_eq!(
        service.admit(&hash(11), &encode(&envelope(12)).unwrap(), 1500),
        Err(Error::Conflict)
    );
}

#[test]
fn overlapping_nullifiers_and_alias_request_ids_cannot_duplicate_payment() {
    let temp = tempfile::tempdir().unwrap();
    let service = funded(&temp.path().join("j.sqlite"));
    let env = envelope(11);
    service
        .admit(&hash(11), &encode(&env).unwrap(), 1500)
        .unwrap();
    assert_eq!(
        service.admit(&hash(12), &encode(&env).unwrap(), 1500),
        Err(Error::Conflict)
    );
    let mut alternate = envelope(12);
    alternate.nullifiers.push(hash(11));
    alternate.operation_digest = envelope_digest(&alternate).unwrap();
    assert_eq!(
        service.admit(&hash(13), &encode(&alternate).unwrap(), 1500),
        Err(Error::Conflict)
    );
    assert_eq!(service.summary().unwrap().jobs, 1);
}

#[test]
fn concurrent_admissions_share_atomic_budget_reservations() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("j.sqlite");
    let initial = funded(&path);
    let a = service(&path);
    let b = service(&path);
    let c = service(&path);
    let results = std::thread::scope(|scope| {
        let x = scope.spawn(|| a.admit(&hash(11), &encode(&envelope(11)).unwrap(), 1500));
        let y = scope.spawn(|| b.admit(&hash(12), &encode(&envelope(12)).unwrap(), 1500));
        let z = scope.spawn(|| c.admit(&hash(13), &encode(&envelope(13)).unwrap(), 1500));
        [x.join().unwrap(), y.join().unwrap(), z.join().unwrap()]
    });
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 2);
    assert_eq!(
        results
            .iter()
            .filter(|r| **r == Err(Error::BudgetExceeded))
            .count(),
        1
    );
    assert_eq!(initial.summary().unwrap().reserved_native_wei, "2000000");
}

#[test]
fn unknown_submission_survives_reopen_and_releases_identical_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("j.sqlite");
    let first = funded(&path);
    let (_, ticket) = admit_and_sign(&first, 11, 0);
    assert_eq!(first.get_job(&hash(11)).unwrap().phase, JobPhase::Unknown);
    drop(first);
    let reopened = service(&path);
    assert_eq!(
        reopened.get_job(&hash(11)).unwrap().phase,
        JobPhase::Unknown
    );
    assert_eq!(reopened.release_for_broadcast(&hash(11)).unwrap(), ticket);
    assert_eq!(reopened.summary().unwrap().reserved_native_wei, "1000000");
    reopened
        .acknowledge_submission(&hash(11), &ticket.transaction_hash)
        .unwrap();
    assert_eq!(
        reopened.get_job(&hash(11)).unwrap().phase,
        JobPhase::Pending
    );
    assert_eq!(
        reopened.acknowledge_submission(&hash(11), &hash(200)),
        Err(Error::InvalidEvidence)
    );
}

#[test]
fn signing_requires_exact_target_calldata_sender_nonce_and_gas_budget() {
    let temp = tempfile::tempdir().unwrap();
    let service = funded(&temp.path().join("j.sqlite"));
    let env = envelope(11);
    service
        .admit(&hash(11), &encode(&env).unwrap(), 1500)
        .unwrap();
    let verified = verified(&env);
    let mut wrong = verified.clone();
    wrong.call_data.push_str("ff");
    let raw = sign_fixture(&policy(), &wrong, 0, 2);
    assert_eq!(
        service.record_signed_outer(&hash(11), &raw),
        Err(Error::InvalidOuterTransaction)
    );
    let too_expensive = sign_fixture(&policy(), &verified, 0, 11);
    assert_eq!(
        service.record_signed_outer(&hash(11), &too_expensive),
        Err(Error::InvalidOuterTransaction)
    );
    let mut wrong_chain = policy();
    wrong_chain.domain.chain_id = 1;
    let raw = sign_fixture(&wrong_chain, &verified, 0, 2);
    assert_eq!(
        service.record_signed_outer(&hash(11), &raw),
        Err(Error::InvalidOuterTransaction)
    );
    let valid = sign_fixture(&policy(), &verified, 0, 2);
    let mut bytes = hex::decode(&valid[2..]).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    assert_eq!(
        service.record_signed_outer(&hash(11), &format!("0x{}", hex::encode(bytes))),
        Err(Error::InvalidOuterTransaction)
    );
    service.record_signed_outer(&hash(11), &valid).unwrap();
    assert_eq!(
        service.record_signed_outer(&hash(11), &sign_fixture(&policy(), &verified, 1, 2)),
        Err(Error::Conflict)
    );
    let second = envelope(12);
    service
        .admit(&hash(12), &encode(&second).unwrap(), 1500)
        .unwrap();
    assert_eq!(
        service.record_signed_outer(
            &hash(12),
            &sign_fixture(&policy(), &super::tests::verified(&second), 0, 2)
        ),
        Err(Error::Conflict)
    );
}

#[test]
fn finalized_success_collects_only_approved_fee_and_actual_native_charge() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("j.sqlite");
    let service = funded(&path);
    let (env, ticket) = admit_and_sign(&service, 11, 0);
    let provisional = receipt(&env, &ticket, false, true);
    let included = service.observe_receipt(&provisional).unwrap();
    assert_eq!(included.phase, JobPhase::Included);
    assert_eq!(included.earned_fee_units, "0");
    assert_eq!(included.reserved_native_wei, "1000000");
    let final_receipt = receipt(&env, &ticket, true, true);
    let finalized = service.observe_receipt(&final_receipt).unwrap();
    assert_eq!(finalized.phase, JobPhase::Finalized);
    assert_eq!(finalized.earned_fee_units, "20000");
    assert_eq!(finalized.finalized_native_charge_wei, "100500");
    assert_eq!(finalized.reserved_native_wei, "0");
    assert_eq!(service.observe_receipt(&final_receipt).unwrap(), finalized);
    assert_eq!(service.summary().unwrap().available_native_wei, "9899500");
    assert_eq!(
        service.release_for_broadcast(&hash(11)),
        Err(Error::InvalidTransition)
    );
    assert_eq!(inspect_database(&path).unwrap(), service.summary().unwrap());
}

#[test]
fn finalized_revert_costs_company_native_gas_but_earns_no_fee() {
    let temp = tempfile::tempdir().unwrap();
    let service = funded(&temp.path().join("j.sqlite"));
    let (env, ticket) = admit_and_sign(&service, 11, 0);
    let failed = service
        .observe_receipt(&receipt(&env, &ticket, true, false))
        .unwrap();
    assert_eq!(failed.phase, JobPhase::FinalizedRevert);
    assert_eq!(failed.earned_fee_units, "0");
    assert_eq!(failed.finalized_native_charge_wei, "100500");
    assert_eq!(service.summary().unwrap().available_native_wei, "9899500");
    // A reverted outer transaction does not revoke the already released private
    // authorization. No fresh intent can reuse its nullifiers automatically.
    let mut alternate = envelope(12);
    alternate.nullifiers = env.nullifiers;
    alternate.operation_digest = envelope_digest(&alternate).unwrap();
    assert_eq!(
        service.admit(&hash(12), &encode(&alternate).unwrap(), 1500),
        Err(Error::Conflict)
    );
}

#[test]
fn retry_cannot_reset_cumulative_cost_cap_and_survives_reopen_between_attempts() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("retry.sqlite");
    let mut p = policy();
    p.max_native_fee_wei = "250000".into();
    let service = GasService::open_harness(
        &path,
        p.clone(),
        Arc::new(FixtureVerifier),
        Arc::new(FixtureChain),
    )
    .unwrap();
    service
        .initialize_treasury(&TreasuryAnchor {
            chain_id: p.domain.chain_id,
            broadcaster_address: p.broadcaster_address.clone(),
            balance_wei: "10000000".into(),
            block_number: 100,
            block_hash: hash(120),
        })
        .unwrap();
    let mut env = envelope(11);
    env.fee.max_native_fee_wei = p.max_native_fee_wei.clone();
    env.operation_digest = envelope_digest(&env).unwrap();
    let id = hash(11);
    service.admit(&id, &encode(&env).unwrap(), 1500).unwrap();
    service
        .record_signed_outer(&id, &sign_fixture(&p, &verified(&env), 0, 2))
        .unwrap();
    let ticket = service.release_for_broadcast(&id).unwrap();
    service
        .observe_receipt(&receipt(&env, &ticket, true, false))
        .unwrap();
    let retry = service.admit(&id, &encode(&env).unwrap(), 10_000).unwrap();
    assert_eq!(retry.attempt_number, 2);
    assert_eq!(retry.reserved_native_wei, "149500");
    assert_eq!(retry.finalized_native_charge_wei, "100500");
    drop(service);
    let service = GasService::open_harness(
        &path,
        p.clone(),
        Arc::new(FixtureVerifier),
        Arc::new(FixtureChain),
    )
    .unwrap();
    assert_eq!(service.get_job(&id).unwrap(), retry);
    // Individually under the original cap, but no longer within what remains.
    assert_eq!(
        service.record_signed_outer(&id, &sign_fixture(&p, &verified(&env), 1, 2)),
        Err(Error::CostExceeded)
    );
    assert_eq!(service.get_job(&id).unwrap(), retry);
    service
        .record_signed_outer(&id, &sign_fixture(&p, &verified(&env), 1, 1))
        .unwrap();
    let second = service.release_for_broadcast(&id).unwrap();
    let mut success = receipt(&env, &second, true, true);
    success.block_number = 111;
    success.finalized_height = 111;
    success.effective_gas_price_wei = "1".into();
    let final_view = service.observe_receipt(&success).unwrap();
    assert_eq!(final_view.finalized_native_charge_wei, "151000");
    assert_eq!(final_view.earned_fee_units, "20000");
    assert_eq!(final_view.attempt_number, 2);
    assert_eq!(inspect_database(&path).unwrap(), service.summary().unwrap());
}

#[test]
fn concurrent_exact_retry_creates_one_new_attempt_and_one_remaining_reservation() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(funded(&temp.path().join("retry")));
    let (env, ticket) = admit_and_sign(&service, 11, 0);
    service
        .observe_receipt(&receipt(&env, &ticket, true, false))
        .unwrap();
    let body = encode(&env).unwrap();
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let service = service.clone();
            let body = body.clone();
            std::thread::spawn(move || service.admit(&hash(11), &body, 2500).unwrap())
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap().attempt_number, 2);
    }
    let summary = service.summary().unwrap();
    assert_eq!(summary.jobs, 1);
    assert_eq!(summary.pending_jobs, 1);
    assert_eq!(summary.reserved_native_wei, "899500");
    assert_eq!(summary.finalized_native_charge_wei, "100500");
    assert_eq!(summary.earned_fee_units, "0");
}

#[test]
fn spent_nullifiers_without_expected_outputs_do_not_prove_success() {
    let temp = tempfile::tempdir().unwrap();
    let service = funded(&temp.path().join("j.sqlite"));
    let (env, ticket) = admit_and_sign(&service, 11, 0);
    let mut forged = receipt(&env, &ticket, true, true);
    if let PrivateExecution::Accepted {
        output_commitments, ..
    } = &mut forged.execution
    {
        output_commitments[0] = hash(88);
    }
    assert_eq!(
        service.observe_receipt(&forged),
        Err(Error::InvalidEvidence)
    );
    assert_eq!(service.get_job(&hash(11)).unwrap().phase, JobPhase::Unknown);
    assert_eq!(service.summary().unwrap().earned_fee_units, "0");
}

#[test]
fn receipt_relabel_or_finality_regression_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let service = funded(&temp.path().join("j.sqlite"));
    let (env, ticket) = admit_and_sign(&service, 11, 0);
    service
        .observe_receipt(&receipt(&env, &ticket, true, true))
        .unwrap();
    assert_eq!(
        service.observe_receipt(&receipt(&env, &ticket, false, true)),
        Err(Error::ConflictingEvidence)
    );
    assert_eq!(
        service.observe_receipt(&receipt(&env, &ticket, true, false)),
        Err(Error::ConflictingEvidence)
    );
    let mut wrong = receipt(&env, &ticket, true, true);
    wrong.transaction_hash = hash(85);
    assert_eq!(service.observe_receipt(&wrong), Err(Error::InvalidEvidence));
}

#[test]
fn provisional_reorg_keeps_reserve_and_same_operation_rebroadcastable() {
    let temp = tempfile::tempdir().unwrap();
    let service = funded(&temp.path().join("j.sqlite"));
    let (env, ticket) = admit_and_sign(&service, 11, 0);
    service
        .observe_receipt(&receipt(&env, &ticket, false, true))
        .unwrap();
    service
        .observe_reorg(&ReorgEvidence {
            request_id: hash(11),
            chain_id: CANDIDATE_CHAIN_ID,
            previous_block_number: 110,
            previous_block_hash: hash(121),
            canonical_block_hash: hash(122),
        })
        .unwrap();
    assert_eq!(service.get_job(&hash(11)).unwrap().phase, JobPhase::Reorged);
    assert_eq!(service.summary().unwrap().reserved_native_wei, "1000000");
    assert_eq!(service.release_for_broadcast(&hash(11)).unwrap(), ticket);
}

#[test]
fn finalized_reorg_reverses_accounting_and_halts_new_spending() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("j.sqlite");
    let service = funded(&path);
    let (env, ticket) = admit_and_sign(&service, 11, 0);
    service
        .observe_receipt(&receipt(&env, &ticket, true, true))
        .unwrap();
    service
        .observe_reorg(&ReorgEvidence {
            request_id: hash(11),
            chain_id: CANDIDATE_CHAIN_ID,
            previous_block_number: 110,
            previous_block_hash: hash(121),
            canonical_block_hash: hash(122),
        })
        .unwrap();
    let totals = service.summary().unwrap();
    assert!(totals.halted);
    assert_eq!(totals.earned_fee_units, "0");
    assert_eq!(totals.finalized_native_charge_wei, "0");
    assert_eq!(totals.reserved_native_wei, "1000000");
    assert_eq!(
        service.admit(&hash(12), &encode(&envelope(12)).unwrap(), 1500),
        Err(Error::Halted)
    );
    assert_eq!(service.release_for_broadcast(&hash(11)), Err(Error::Halted));
    drop(service);
    assert!(super::tests::service(&path).summary().unwrap().halted);
}

#[test]
fn native_cost_overrun_freezes_operation_and_persists_incident() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("j.sqlite");
    let service = funded(&path);
    let (env, ticket) = admit_and_sign(&service, 11, 0);
    let mut evidence = receipt(&env, &ticket, true, true);
    evidence.additional_native_fee_wei = "1001".into();
    assert_eq!(service.observe_receipt(&evidence), Err(Error::CostExceeded));
    assert_eq!(
        service.get_job(&hash(11)).unwrap().phase,
        JobPhase::Incident
    );
    assert!(service.summary().unwrap().halted);
    assert_eq!(service.summary().unwrap().earned_fee_units, "0");
    drop(service);
    assert!(inspect_database(&path).unwrap().halted);
}

#[test]
fn journal_is_policy_bound_append_only_and_preserves_foreign_database() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("j.sqlite");
    let service = funded(&path);
    service
        .admit(&hash(11), &encode(&envelope(11)).unwrap(), 1500)
        .unwrap();
    drop(service);
    let connection = Connection::open(&path).unwrap();
    assert!(connection.execute("DELETE FROM jobs", []).is_err());
    assert!(connection
        .execute("UPDATE metadata SET policy_digest='changed'", [])
        .is_err());
    drop(connection);
    let mut changed = policy();
    changed.fee_units = "20001".into();
    assert!(matches!(
        GasService::open(
            &path,
            changed,
            Arc::new(FixtureVerifier),
            Arc::new(FixtureChain)
        ),
        Err(Error::JournalMismatch)
    ));
    let foreign = temp.path().join("foreign.sqlite");
    let connection = Connection::open(&foreign).unwrap();
    connection
        .execute("CREATE TABLE keep_me(value TEXT)", [])
        .unwrap();
    connection
        .execute("INSERT INTO keep_me VALUES('preserved')", [])
        .unwrap();
    drop(connection);
    assert!(matches!(
        GasService::open(
            &foreign,
            policy(),
            Arc::new(FixtureVerifier),
            Arc::new(FixtureChain)
        ),
        Err(Error::JournalMismatch)
    ));
    let connection = Connection::open(foreign).unwrap();
    let value: String = connection
        .query_row("SELECT value FROM keep_me", [], |r| r.get(0))
        .unwrap();
    assert_eq!(value, "preserved");
}

#[test]
fn diagnostics_do_not_export_request_payloads_or_identifiers() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("j.sqlite");
    let service = funded(&path);
    let (env, ticket) = admit_and_sign(&service, 11, 0);
    let text = encode(&inspect_database(&path).unwrap()).unwrap();
    assert!(!text.contains(&env.proof));
    assert!(!text.contains(&env.encrypted_outputs[0]));
    assert!(!text.contains(&ticket.raw_transaction));
    assert!(!text.contains(&ticket.request_id));
    assert!(!text.contains(&policy().broadcaster_address));
    assert!(text.contains("\"production_sponsorship_available\":false"));
}

#[test]
fn failed_commit_never_releases_broadcast_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("j.sqlite");
    let service = funded(&path);
    let env = envelope(11);
    service
        .admit(&hash(11), &encode(&env).unwrap(), 1500)
        .unwrap();
    let raw = sign_fixture(&policy(), &verified(&env), 0, 2);
    let before = service.record_signed_outer(&hash(11), &raw).unwrap();
    // Force the release transaction's COMMIT (not its INSERT) to fail using a
    // deferred foreign-key violation. This injection exists only in this test.
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            r#"
        CREATE TABLE deferred_failure (
            job TEXT REFERENCES jobs(request_id) DEFERRABLE INITIALLY DEFERRED
        );
        CREATE TRIGGER reject_release_commit AFTER INSERT ON events
        WHEN json_extract(NEW.body,'$.kind') = 'Released'
        BEGIN INSERT INTO deferred_failure(job) VALUES('missing'); END;
    "#,
        )
        .unwrap();
    assert_eq!(
        service.release_for_broadcast(&hash(11)),
        Err(Error::Storage)
    );
    assert_eq!(service.get_job(&hash(11)).unwrap(), before);
    assert_eq!(service.summary().unwrap().reserved_native_wei, "1000000");
    connection
        .execute_batch("DROP TRIGGER reject_release_commit;")
        .unwrap();
    assert_eq!(
        service
            .release_for_broadcast(&hash(11))
            .unwrap()
            .raw_transaction,
        raw
    );
    assert_eq!(service.get_job(&hash(11)).unwrap().phase, JobPhase::Unknown);
}

#[test]
fn unfunded_insufficient_pending_and_lifetime_budgets_are_separate_checks() {
    let temp = tempfile::tempdir().unwrap();
    let unfunded = service(&temp.path().join("unfunded.sqlite"));
    let payload = encode(&envelope(11)).unwrap();
    assert_eq!(
        unfunded.admit(&hash(11), &payload, 1500),
        Err(Error::TreasuryNotInitialized)
    );
    unfunded
        .initialize_treasury(&TreasuryAnchor {
            chain_id: CANDIDATE_CHAIN_ID,
            broadcaster_address: fixture_address(),
            balance_wei: "999999".into(),
            block_number: 100,
            block_hash: hash(120),
        })
        .unwrap();
    assert_eq!(
        unfunded.admit(&hash(11), &payload, 1500),
        Err(Error::BudgetExceeded)
    );
    assert_eq!(unfunded.summary().unwrap().jobs, 0);
    assert_eq!(unfunded.summary().unwrap().available_native_wei, "999999");

    let mut pending_policy = policy();
    pending_policy.max_pending_native_wei = "1000000".into();
    let limited = GasService::open_harness(
        &temp.path().join("pending.sqlite"),
        pending_policy,
        Arc::new(FixtureVerifier),
        Arc::new(FixtureChain),
    )
    .unwrap();
    limited
        .initialize_treasury(&TreasuryAnchor {
            chain_id: CANDIDATE_CHAIN_ID,
            broadcaster_address: fixture_address(),
            balance_wei: "10000000".into(),
            block_number: 100,
            block_hash: hash(120),
        })
        .unwrap();
    limited.admit(&hash(11), &payload, 1500).unwrap();
    assert_eq!(
        limited.admit(&hash(12), &encode(&envelope(12)).unwrap(), 1500),
        Err(Error::BudgetExceeded)
    );
    assert_eq!(limited.summary().unwrap().pending_jobs, 1);

    let mut total_policy = policy();
    total_policy.max_total_native_spend_wei = "1000000".into();
    let limited = GasService::open_harness(
        &temp.path().join("total.sqlite"),
        total_policy,
        Arc::new(FixtureVerifier),
        Arc::new(FixtureChain),
    )
    .unwrap();
    limited
        .initialize_treasury(&TreasuryAnchor {
            chain_id: CANDIDATE_CHAIN_ID,
            broadcaster_address: fixture_address(),
            balance_wei: "10000000".into(),
            block_number: 100,
            block_hash: hash(120),
        })
        .unwrap();
    let (env, ticket) = admit_and_sign(&limited, 11, 0);
    limited
        .observe_receipt(&receipt(&env, &ticket, true, true))
        .unwrap();
    assert_eq!(limited.summary().unwrap().pending_jobs, 0);
    assert_eq!(
        limited.admit(&hash(12), &encode(&envelope(12)).unwrap(), 1500),
        Err(Error::BudgetExceeded)
    );
    assert_eq!(limited.summary().unwrap().available_native_wei, "9899500");
}
