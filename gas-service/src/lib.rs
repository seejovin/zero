//! Private-payment gas-service domain and operator journal.
//!
//! Includes an OHTTP HTTP gateway, fixed proof-verifier subprocess adapter,
//! independent-provider RPC evidence adapter, restricted treasury signer and
//! append-only accounting. Mainnet economic paths remain blocked pending the
//! actual deployment/circuit/transport/device qualification; no environment,
//! feature or remote boolean bypasses these gates. Private test harnesses use
//! public fixture keys, invalid proofs and loopback HTTP only.
#![forbid(unsafe_code)]

pub mod fee_schedule;
pub mod gateway;
mod model;
pub mod proof_process;
pub mod relay;
pub mod rpc;
pub mod runtime;
pub mod secrets;
pub mod signer;
mod transaction;
pub mod worker;

pub use model::{
    AdmissionPolicy, BroadcastTicket, JobPhase, JobView, OperatorSummary, PrivateExecution,
    ReceiptEvidence, ReorgEvidence, TreasuryAnchor, VerifiedOperation,
};

use model::{decimal, decode, encode, fixed_hex, request_id};
use primitive_types::U256;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use transaction::{inspect_outer, SignedOuter};
use wallet_core::private_payment::{
    canonical_envelope, require_private_release, validate_envelope_json, PrivateEnvelope,
};

const APP_ID: i64 = 0x4c575a47;
const SCHEMA_VERSION: i64 = 4;
const SCHEMA: &str = include_str!("../migrations/001_operator_journal.sql");
const MAX_EVENTS_PER_JOB: usize = 10_000;
const MAX_LIFETIME_JOBS: usize = 100_000;
const MAX_OUTER_ATTEMPTS: usize = 3;

pub type Result<T> = std::result::Result<T, Error>;

/// Constant messages never interpolate untrusted requests, user data or keys.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("Private gas sponsorship is not released: reviewed protocol, proof, transport and device integrations are missing")]
    ReleaseUnavailable,
    #[error("Required reviewed integration is unavailable")]
    IntegrationUnavailable,
    #[error("The operator policy is invalid")]
    InvalidPolicy,
    #[error("The operation does not match the shared fee policy or admission window")]
    PolicyMismatch,
    #[error("The request encoding is invalid or ambiguous")]
    InvalidEncoding,
    #[error("The integer quantity is invalid")]
    InvalidQuantity,
    #[error("Private operation or fee verification failed")]
    VerificationFailed,
    #[error("The signed outer transaction does not match the authorized operation")]
    InvalidOuterTransaction,
    #[error("The chain evidence is invalid or does not match this operation")]
    InvalidEvidence,
    #[error("The evidence contradicts the saved transaction history")]
    ConflictingEvidence,
    #[error("The request, operation, nullifier or native nonce is already reserved")]
    Conflict,
    #[error("The operation is not in a state that permits this transition")]
    InvalidTransition,
    #[error("A verified opening treasury balance is required")]
    TreasuryNotInitialized,
    #[error("The native gas budget is exhausted")]
    BudgetExceeded,
    #[error("The chain cost exceeds the approved exposure; the service is halted")]
    CostExceeded,
    #[error("The service is halted pending operator investigation")]
    Halted,
    #[error("The request was not found")]
    NotFound,
    #[error("The operator journal could not be read or saved; existing records were preserved")]
    Storage,
    #[error("The operator journal identity, schema or evidence is invalid")]
    JournalMismatch,
    #[error("An exact integer calculation overflowed")]
    Arithmetic,
    #[error("Operator secret input is invalid, unavailable or not private")]
    SecretInput,
    #[error("RPC transport or chain verification failed")]
    Rpc,
    #[error("Private gateway request is invalid")]
    Gateway,
}

/// A real implementation must validate the private proof, accepted circuit and
/// ciphertext bindings, and decrypt/check ONLY the provider's own fee output.
/// Shape validation or successful deserialization is never a verifier.
pub trait ProofVerifier: Send + Sync {
    fn verify(&self, envelope: &PrivateEnvelope) -> Result<VerifiedOperation>;
}

/// A real adapter must check canonical chain/contract evidence and fresh state.
/// Caller-provided receipt JSON or a spent nullifier is not sufficient evidence.
pub trait ChainEvidenceVerifier: Send + Sync {
    fn verify_treasury(&self, anchor: &TreasuryAnchor) -> Result<()>;
    /// Check the live operator balance against ALL outstanding native exposure.
    /// This complements the immutable opening-balance projection; unexpected
    /// spending outside this journal must not authorize unfunded reservations.
    fn verify_pending_exposure(&self, required_native_wei: &str) -> Result<()>;
    fn verify_admission(
        &self,
        envelope: &PrivateEnvelope,
        verified: &VerifiedOperation,
    ) -> Result<()>;
    fn verify_receipt(
        &self,
        evidence: &ReceiptEvidence,
        envelope: &PrivateEnvelope,
        verified: &VerifiedOperation,
    ) -> Result<()>;
    fn verify_reorg(&self, evidence: &ReorgEvidence) -> Result<()>;
}

pub struct UnavailableProofVerifier;
impl ProofVerifier for UnavailableProofVerifier {
    fn verify(&self, _: &PrivateEnvelope) -> Result<VerifiedOperation> {
        Err(Error::IntegrationUnavailable)
    }
}

pub struct UnavailableChainVerifier;
impl ChainEvidenceVerifier for UnavailableChainVerifier {
    fn verify_treasury(&self, _: &TreasuryAnchor) -> Result<()> {
        Err(Error::IntegrationUnavailable)
    }
    fn verify_pending_exposure(&self, _: &str) -> Result<()> {
        Err(Error::IntegrationUnavailable)
    }
    fn verify_admission(&self, _: &PrivateEnvelope, _: &VerifiedOperation) -> Result<()> {
        Err(Error::IntegrationUnavailable)
    }
    fn verify_receipt(
        &self,
        _: &ReceiptEvidence,
        _: &PrivateEnvelope,
        _: &VerifiedOperation,
    ) -> Result<()> {
        Err(Error::IntegrationUnavailable)
    }
    fn verify_reorg(&self, _: &ReorgEvidence) -> Result<()> {
        Err(Error::IntegrationUnavailable)
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Capabilities {
    pub schema_version: u8,
    pub version: &'static str,
    pub production_sponsorship_available: bool,
    pub proof_verifier_available: bool,
    pub chain_evidence_adapter_available: bool,
    pub oblivious_transport_available: bool,
    pub signer_available: bool,
    pub durable_operator_journal_available: bool,
    pub storage_adapter: &'static str,
    pub ohttp_gateway_implementation: bool,
    pub opaque_relay_implementation: bool,
    pub restricted_native_signer_implementation: bool,
    pub independent_rpc_adapter_implementation: bool,
    pub pinned_verifier_process_implementation: bool,
    pub blockers: Vec<&'static str>,
}

pub fn capabilities() -> Capabilities {
    Capabilities {
        schema_version: 1,
        version: env!("CARGO_PKG_VERSION"),
        production_sponsorship_available: false,
        proof_verifier_available: false,
        chain_evidence_adapter_available: false,
        oblivious_transport_available: false,
        signer_available: false,
        durable_operator_journal_available: true,
        storage_adapter: "sqlite-wal-full",
        ohttp_gateway_implementation: true,
        opaque_relay_implementation: true,
        restricted_native_signer_implementation: true,
        independent_rpc_adapter_implementation: true,
        pinned_verifier_process_implementation: true,
        blockers: vec![
            "exact_protocol_deployment_and_circuit_review",
            "local_prover_and_fee_verifier_release_qualification",
            "independent_rpc_finality_and_deployment_pin_qualification",
            "independent_relay_gateway_operation_and_pin_distribution",
            "isolated_treasury_deployment_and_funding",
            "android_and_independent_security_acceptance",
        ],
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", deny_unknown_fields)]
enum Event {
    Signed(SignedOuter),
    Released {
        at: u64,
    },
    Acknowledged {
        transaction_hash: String,
    },
    Receipt(ReceiptEvidence),
    Reorg(ReorgEvidence),
    Retry {
        previous_transaction_hash: String,
        at: u64,
    },
    ReplayRequested,
    Incident {
        code: IncidentCode,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum IncidentCode {
    NativeCostExceeded,
}

struct Job {
    request_id: String,
    envelope: PrivateEnvelope,
    verified: VerifiedOperation,
    phase: JobPhase,
    signed: Option<SignedOuter>,
    receipt: Option<ReceiptEvidence>,
    event_count: u64,
    broadcast_attempts: u32,
    last_broadcast_release_at: Option<u64>,
    completed_reverts: Vec<CompletedAttempt>,
    rebroadcast_requested: bool,
}

struct CompletedAttempt {
    signed: SignedOuter,
    receipt: ReceiptEvidence,
}

impl Job {
    fn attempt_number(&self) -> u8 {
        (self.completed_reverts.len() + 1) as u8
    }

    fn prior_native_charge(&self) -> Result<U256> {
        self.completed_reverts
            .iter()
            .try_fold(U256::zero(), |sum, attempt| {
                sum.checked_add(attempt.receipt.native_charge()?)
                    .ok_or(Error::Arithmetic)
            })
    }

    fn remaining_native_budget(&self) -> Result<U256> {
        decimal(&self.envelope.fee.max_native_fee_wei)?
            .checked_sub(self.prior_native_charge()?)
            .ok_or(Error::CostExceeded)
    }

    fn validate_attempt_cost(&self, signed: &SignedOuter) -> Result<()> {
        let upper = decimal(&signed.max_fee_per_gas_wei)?
            .checked_mul(signed.gas_limit.into())
            .and_then(|value| {
                value.checked_add(decimal(&self.verified.additional_native_fee_cap_wei).ok()?)
            })
            .ok_or(Error::Arithmetic)?;
        if upper > self.remaining_native_budget()? {
            return Err(Error::CostExceeded);
        }
        Ok(())
    }

    fn receipts(&self) -> impl Iterator<Item = &ReceiptEvidence> {
        self.completed_reverts
            .iter()
            .map(|attempt| &attempt.receipt)
            .chain(self.receipt.iter())
    }

    fn view(&self) -> Result<JobView> {
        let finalized = matches!(
            self.phase,
            JobPhase::Finalized | JobPhase::FinalizedRevert | JobPhase::RetryExhausted
        );
        let charge = if self.phase == JobPhase::Incident {
            U256::zero()
        } else if finalized {
            self.receipt
                .as_ref()
                .ok_or(Error::JournalMismatch)?
                .native_charge()?
                .checked_add(self.prior_native_charge()?)
                .ok_or(Error::Arithmetic)?
        } else {
            self.prior_native_charge()?
        };
        Ok(JobView {
            request_id: self.request_id.clone(),
            operation_digest: self.envelope.operation_digest.clone(),
            phase: self.phase.clone(),
            transaction_hash: self.signed.as_ref().map(|s| s.transaction_hash.clone()),
            nonce: self.signed.as_ref().map(|s| s.nonce),
            reserved_native_wei: if self.phase == JobPhase::Incident {
                self.envelope.fee.max_native_fee_wei.clone()
            } else if finalized {
                "0".into()
            } else {
                self.remaining_native_budget()?.to_string()
            },
            finalized_native_charge_wei: charge.to_string(),
            earned_fee_units: if self.phase == JobPhase::Finalized {
                self.envelope.fee.fee_units.clone()
            } else {
                "0".into()
            },
            event_count: self.event_count,
            broadcast_attempts: self.broadcast_attempts,
            last_broadcast_release_at: self.last_broadcast_release_at,
            attempt_number: self.attempt_number(),
            maximum_attempts: MAX_OUTER_ATTEMPTS as u8,
        })
    }

    fn apply(&mut self, event: Event, policy: &AdmissionPolicy) -> Result<()> {
        match event {
            Event::Signed(signed) => {
                if self.phase != JobPhase::Admitted
                    || self.signed.is_some()
                    || inspect_outer(&signed.raw_transaction, policy, &self.verified)? != signed
                {
                    return Err(Error::JournalMismatch);
                }
                self.validate_attempt_cost(&signed)?;
                self.signed = Some(signed);
                self.phase = JobPhase::Signed;
                self.rebroadcast_requested = true;
            }
            Event::Released { at } => {
                if !matches!(
                    self.phase,
                    JobPhase::Signed | JobPhase::Unknown | JobPhase::Pending | JobPhase::Reorged
                ) || self.signed.is_none()
                {
                    return Err(Error::InvalidTransition);
                }
                if at == 0
                    || at > i64::MAX as u64
                    || self
                        .last_broadcast_release_at
                        .is_some_and(|previous| at < previous)
                {
                    return Err(Error::InvalidTransition);
                }
                self.broadcast_attempts = self
                    .broadcast_attempts
                    .checked_add(1)
                    .ok_or(Error::Arithmetic)?;
                self.last_broadcast_release_at = Some(at);
                self.phase = JobPhase::Unknown;
                self.rebroadcast_requested = false;
            }
            Event::Acknowledged { transaction_hash } => {
                if !matches!(self.phase, JobPhase::Unknown | JobPhase::Pending)
                    || self.signed.as_ref().map(|s| &s.transaction_hash) != Some(&transaction_hash)
                {
                    return Err(Error::InvalidEvidence);
                }
                self.phase = JobPhase::Pending;
            }
            Event::Receipt(evidence) => {
                self.validate_receipt(&evidence, policy)?;
                let accepted = matches!(evidence.execution, PrivateExecution::Accepted { .. });
                self.phase = match (evidence.is_finalized(), accepted) {
                    (true, true) => JobPhase::Finalized,
                    (true, false)
                        if usize::from(self.attempt_number()) >= MAX_OUTER_ATTEMPTS
                            || evidence.native_charge()? == self.remaining_native_budget()? =>
                    {
                        JobPhase::RetryExhausted
                    }
                    (true, false) => JobPhase::FinalizedRevert,
                    (false, true) => JobPhase::Included,
                    (false, false) => JobPhase::IncludedRevert,
                };
                self.receipt = Some(evidence);
                self.rebroadcast_requested = false;
            }
            Event::Reorg(evidence) => {
                let previous = self
                    .receipts()
                    .find(|previous| {
                        previous.block_number == evidence.previous_block_number
                            && previous.block_hash == evidence.previous_block_hash
                    })
                    .ok_or(Error::InvalidTransition)?;
                if evidence.request_id != self.request_id
                    || evidence.chain_id != policy.domain.chain_id
                    || evidence.previous_block_number != previous.block_number
                    || evidence.previous_block_hash != previous.block_hash
                    || evidence.canonical_block_hash == previous.block_hash
                {
                    return Err(Error::InvalidEvidence);
                }
                fixed_hex(&evidence.canonical_block_hash, 32)?;
                let finalized = previous.is_finalized();
                self.receipt = None;
                self.phase = if finalized {
                    JobPhase::Incident
                } else {
                    JobPhase::Reorged
                };
                self.rebroadcast_requested = false;
            }
            Event::Retry {
                previous_transaction_hash,
                at,
            } => {
                if self.phase != JobPhase::FinalizedRevert
                    || usize::from(self.attempt_number()) >= MAX_OUTER_ATTEMPTS
                    || at == 0
                    || at > i64::MAX as u64
                {
                    return Err(Error::InvalidTransition);
                }
                let receipt = self.receipt.take().ok_or(Error::JournalMismatch)?;
                let signed = self.signed.take().ok_or(Error::JournalMismatch)?;
                if previous_transaction_hash != signed.transaction_hash
                    || receipt.transaction_hash != signed.transaction_hash
                    || !receipt.is_finalized()
                    || receipt.execution != PrivateExecution::Reverted
                {
                    return Err(Error::InvalidEvidence);
                }
                self.completed_reverts
                    .push(CompletedAttempt { signed, receipt });
                if self.remaining_native_budget()?.is_zero() {
                    return Err(Error::CostExceeded);
                }
                self.phase = JobPhase::Admitted;
                self.broadcast_attempts = 0;
                self.last_broadcast_release_at = None;
                self.rebroadcast_requested = false;
            }
            Event::ReplayRequested => {
                if !matches!(
                    self.phase,
                    JobPhase::Unknown | JobPhase::Pending | JobPhase::Reorged
                ) || self.signed.is_none()
                    || self.rebroadcast_requested
                    || self.broadcast_attempts >= 3
                {
                    return Err(Error::InvalidTransition);
                }
                self.rebroadcast_requested = true;
            }
            Event::Incident { .. } => {
                self.phase = JobPhase::Incident;
            }
        }
        self.event_count = self.event_count.checked_add(1).ok_or(Error::Arithmetic)?;
        if self.event_count > MAX_EVENTS_PER_JOB as u64 {
            return Err(Error::JournalMismatch);
        }
        Ok(())
    }

    fn validate_receipt(&self, evidence: &ReceiptEvidence, policy: &AdmissionPolicy) -> Result<()> {
        let signed = self.signed.as_ref().ok_or(Error::InvalidTransition)?;
        if self.phase == JobPhase::Incident
            || evidence.request_id != self.request_id
            || evidence.chain_id != policy.domain.chain_id
            || evidence.transaction_hash != signed.transaction_hash
            || evidence.block_number == 0
            || evidence.canonical_block_hash != evidence.block_hash
            || evidence.gas_used < 21_000
            || evidence.gas_used > signed.gas_limit
        {
            return Err(Error::InvalidEvidence);
        }
        fixed_hex(&evidence.block_hash, 32)?;
        let effective_price = decimal(&evidence.effective_gas_price_wei)?;
        let additional = decimal(&evidence.additional_native_fee_wei)?;
        if effective_price > decimal(&signed.max_fee_per_gas_wei)?
            || additional > decimal(&self.verified.additional_native_fee_cap_wei)?
            || evidence.native_charge()? > decimal(&self.envelope.fee.max_native_fee_wei)?
            || evidence.native_charge()? > self.remaining_native_budget()?
        {
            return Err(Error::CostExceeded);
        }
        if let PrivateExecution::Accepted {
            operation_digest,
            tree_number,
            nullifiers,
            output_commitments,
            fee_output_commitment,
        } = &evidence.execution
        {
            if operation_digest != &self.envelope.operation_digest
                || tree_number != &self.verified.tree_number
                || nullifiers != &self.envelope.nullifiers
                || output_commitments != &self.envelope.output_commitments
                || fee_output_commitment != &self.envelope.fee.fee_output_commitment
            {
                return Err(Error::InvalidEvidence);
            }
        }
        if let Some(previous) = &self.receipt {
            let mut previous_identity = previous.clone();
            let mut next_identity = evidence.clone();
            previous_identity.finalized_height = 0;
            next_identity.finalized_height = 0;
            if previous_identity != next_identity
                || evidence.finalized_height < previous.finalized_height
            {
                return Err(Error::ConflictingEvidence);
            }
        }
        Ok(())
    }
}

pub struct GasService {
    connection: Mutex<Connection>,
    policy: AdmissionPolicy,
    proof_verifier: Arc<dyn ProofVerifier>,
    chain_verifier: Arc<dyn ChainEvidenceVerifier>,
    #[cfg(test)]
    test_harness: bool,
}

impl GasService {
    pub fn open(
        path: &Path,
        policy: AdmissionPolicy,
        proof_verifier: Arc<dyn ProofVerifier>,
        chain_verifier: Arc<dyn ChainEvidenceVerifier>,
    ) -> Result<Self> {
        policy.validate()?;
        let connection = open_journal(path, &policy)?;
        let service = Self {
            connection: Mutex::new(connection),
            policy,
            proof_verifier,
            chain_verifier,
            #[cfg(test)]
            test_harness: false,
        };
        service.summary()?;
        Ok(service)
    }

    fn require_active(&self) -> Result<()> {
        #[cfg(test)]
        if self.test_harness {
            return Ok(());
        }
        require_private_release().map_err(|_| Error::ReleaseUnavailable)?;
        // A core release gate alone cannot enable a missing service integration.
        Err(Error::IntegrationUnavailable)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| Error::Storage)
    }

    pub fn initialize_treasury(&self, anchor: &TreasuryAnchor) -> Result<OperatorSummary> {
        self.require_active()?;
        anchor.validate(&self.policy)?;
        self.chain_verifier.verify_treasury(anchor)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        if let Some(saved) = treasury(&tx)? {
            if saved != *anchor {
                return Err(Error::Conflict);
            }
        } else {
            tx.execute(
                "INSERT INTO treasury_anchor(id,body) VALUES(1,?1)",
                [encode(anchor)?],
            )
            .map_err(storage)?;
        }
        tx.commit().map_err(storage)?;
        read_summary(&connection, &self.policy)
    }

    pub fn admit(&self, id: &str, envelope_json: &str, now: u64) -> Result<JobView> {
        self.require_active()?;
        request_id(id)?;
        if envelope_json.len() > self.policy.max_envelope_bytes as usize || now > i64::MAX as u64 {
            return Err(Error::InvalidEncoding);
        }
        let envelope = validate_envelope_json(envelope_json).map_err(|_| Error::InvalidEncoding)?;
        let canonical = canonical_envelope(&envelope).map_err(|_| Error::InvalidEncoding)?;

        // Idempotent retrieval remains valid after a quote expires. It does not
        // authorize another send or change a previously accepted fee schedule.
        let operation_policy = {
            let connection = self.lock()?;
            let snapshot = connection.unchecked_transaction().map_err(storage)?;
            if let Some(saved) = optional_job(&snapshot, id, &self.policy)? {
                if canonical_envelope(&saved.envelope).map_err(|_| Error::JournalMismatch)?
                    != canonical
                {
                    return Err(Error::Conflict);
                }
                if saved.phase == JobPhase::FinalizedRevert {
                    drop(snapshot);
                    drop(connection);
                    return self.retry_finalized_revert(id, now);
                }
                if matches!(
                    saved.phase,
                    JobPhase::Unknown | JobPhase::Pending | JobPhase::Reorged
                ) {
                    drop(snapshot);
                    drop(connection);
                    return self.request_identical_rebroadcast(id);
                }
                return saved.view();
            }
            require_not_halted(&snapshot)?;
            schedule_policy(&snapshot, &envelope.fee.schedule_id, &self.policy)?
        };
        operation_policy.matches(&envelope, now)?;
        let verified = self.proof_verifier.verify(&envelope)?;
        verified.validate(&envelope, &self.policy)?;
        self.chain_verifier.verify_admission(&envelope, &verified)?;

        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        if let Some(saved) = optional_job(&tx, id, &self.policy)? {
            if saved.envelope != envelope {
                return Err(Error::Conflict);
            }
            return saved.view();
        }
        let totals = summary(&tx, &self.policy)?;
        if totals.halted {
            return Err(Error::Halted);
        }
        if treasury(&tx)?.is_none() {
            return Err(Error::TreasuryNotInitialized);
        }
        if totals.jobs >= MAX_LIFETIME_JOBS as u64
            || totals.pending_jobs >= self.policy.max_pending_operations as u64
        {
            return Err(Error::BudgetExceeded);
        }
        let reserve = decimal(&envelope.fee.max_native_fee_wei)?;
        let reserved = decimal(&totals.reserved_native_wei)?
            .checked_add(reserve)
            .ok_or(Error::Arithmetic)?;
        if reserve > decimal(&totals.available_native_wei)?
            || reserved > decimal(&self.policy.max_pending_native_wei)?
            || reserved
                .checked_add(decimal(&totals.finalized_native_charge_wei)?)
                .ok_or(Error::Arithmetic)?
                > decimal(&self.policy.max_total_native_spend_wei)?
        {
            return Err(Error::BudgetExceeded);
        }
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM jobs WHERE operation_digest=?1)",
                [&envelope.operation_digest],
                |row| row.get(0),
            )
            .map_err(storage)?;
        if exists {
            return Err(Error::Conflict);
        }
        for nullifier in &envelope.nullifiers {
            let claimed: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM nullifier_claims WHERE tree_number=?1 AND nullifier=?2)",
                    params![verified.tree_number as i64, nullifier],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            if claimed {
                return Err(Error::Conflict);
            }
        }
        self.chain_verifier
            .verify_pending_exposure(&reserved.to_string())?;
        tx.execute("INSERT INTO jobs(request_id,operation_digest,envelope_json,verified_operation_json,admitted_at) VALUES(?1,?2,?3,?4,?5)",
            params![id, envelope.operation_digest, canonical, encode(&verified)?, now as i64]).map_err(storage)?;
        for nullifier in &envelope.nullifiers {
            tx.execute(
                "INSERT INTO nullifier_claims(tree_number,nullifier,request_id) VALUES(?1,?2,?3)",
                params![verified.tree_number as i64, nullifier, id],
            )
            .map_err(storage)?;
        }
        let view = job(&tx, id, &self.policy)?.view()?;
        tx.commit().map_err(storage)?;
        Ok(view)
    }

    /// A separately isolated signer supplies bytes; this method independently
    /// recovers their signer and checks chain, target, calldata, nonce and caps.
    /// No raw bytes are returned until release_for_broadcast commits UNKNOWN.
    pub fn record_signed_outer(&self, id: &str, raw_transaction: &str) -> Result<JobView> {
        self.require_active()?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        require_not_halted(&tx)?;
        let mut saved = job(&tx, id, &self.policy)?;
        let signed = inspect_outer(raw_transaction, &self.policy, &saved.verified)?;
        if let Some(previous) = &saved.signed {
            if *previous != signed {
                return Err(Error::Conflict);
            }
            return saved.view();
        }
        let nonce_claimed: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM outer_nonce_claims WHERE nonce=?1)",
                [signed.nonce.to_string()],
                |row| row.get(0),
            )
            .map_err(storage)?;
        if nonce_claimed {
            return Err(Error::Conflict);
        }
        let event = Event::Signed(signed.clone());
        saved.apply(event.clone(), &self.policy)?;
        tx.execute(
            "INSERT INTO outer_nonce_claims(nonce,request_id,attempt) VALUES(?1,?2,?3)",
            params![
                signed.nonce.to_string(),
                id,
                i64::from(saved.attempt_number())
            ],
        )
        .map_err(storage)?;
        append(&tx, id, &event)?;
        let view = saved.view()?;
        tx.commit().map_err(storage)?;
        Ok(view)
    }

    /// Reconstruct and sign only an already admitted operation. This method does
    /// not accept a destination, native value or calldata from its caller.
    pub fn sign_admitted(
        &self,
        signer: &signer::NativeTreasurySigner,
        id: &str,
        quote: &signer::GasQuote,
    ) -> Result<JobView> {
        self.require_active()?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        require_not_halted(&tx)?;
        let mut saved = job(&tx, id, &self.policy)?;
        if saved.signed.is_some() {
            return saved.view();
        }
        if saved.phase != JobPhase::Admitted {
            return Err(Error::InvalidTransition);
        }
        self.chain_verifier
            .verify_admission(&saved.envelope, &saved.verified)?;
        let signed = signer.sign(&self.policy, &saved.verified, quote)?;
        saved.validate_attempt_cost(&signed)?;
        let claimed: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM outer_nonce_claims WHERE nonce=?1)",
                [signed.nonce.to_string()],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if claimed {
            return Err(Error::Conflict);
        }
        let event = Event::Signed(signed.clone());
        saved.apply(event.clone(), &self.policy)?;
        tx.execute(
            "INSERT INTO outer_nonce_claims(nonce,request_id,attempt) VALUES(?1,?2,?3)",
            params![
                signed.nonce.to_string(),
                id,
                i64::from(saved.attempt_number())
            ],
        )
        .map_err(storage)?;
        append(&tx, id, &event)?;
        let view = saved.view()?;
        tx.commit().map_err(storage)?;
        Ok(view)
    }

    /// Start one additional OUTER attempt for the SAME admitted private proof.
    /// No new client fee, envelope, authorization or nullifier claim is created.
    /// This is only reached by an explicit identical request after a verified
    /// canonical finalized revert; unknown outcomes never enter this transition.
    pub fn retry_finalized_revert(&self, id: &str, now: u64) -> Result<JobView> {
        self.require_active()?;
        request_id(id)?;
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::InvalidEncoding);
        }
        let before = {
            let connection = self.lock()?;
            let snapshot = connection.unchecked_transaction().map_err(storage)?;
            require_not_halted(&snapshot)?;
            job(&snapshot, id, &self.policy)?
        };
        if before.phase != JobPhase::FinalizedRevert {
            // A concurrent caller may already have committed this retry.
            return if before.attempt_number() > 1
                && matches!(
                    before.phase,
                    JobPhase::Admitted
                        | JobPhase::Signed
                        | JobPhase::Unknown
                        | JobPhase::Pending
                        | JobPhase::Included
                        | JobPhase::IncludedRevert
                        | JobPhase::Reorged
                ) {
                before.view()
            } else {
                Err(Error::InvalidTransition)
            };
        }
        // Verify all prior native charges still belong to finalized canonical
        // attempts before reserving more operator exposure.
        for receipt in before.receipts() {
            self.chain_verifier
                .verify_receipt(receipt, &before.envelope, &before.verified)?;
        }
        self.chain_verifier
            .verify_admission(&before.envelope, &before.verified)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        require_not_halted(&tx)?;
        let mut saved = job(&tx, id, &self.policy)?;
        if saved.phase != JobPhase::FinalizedRevert || saved.event_count != before.event_count {
            return saved.view();
        }
        let totals = summary(&tx, &self.policy)?;
        let event = Event::Retry {
            previous_transaction_hash: saved
                .signed
                .as_ref()
                .ok_or(Error::JournalMismatch)?
                .transaction_hash
                .clone(),
            at: now,
        };
        saved.apply(event.clone(), &self.policy)?;
        let reserve = decimal(&saved.view()?.reserved_native_wei)?;
        let reserved = decimal(&totals.reserved_native_wei)?
            .checked_add(reserve)
            .ok_or(Error::Arithmetic)?;
        if totals.pending_jobs >= u64::from(self.policy.max_pending_operations)
            || reserve > decimal(&totals.available_native_wei)?
            || reserved > decimal(&self.policy.max_pending_native_wei)?
            || reserved
                .checked_add(decimal(&totals.finalized_native_charge_wei)?)
                .ok_or(Error::Arithmetic)?
                > decimal(&self.policy.max_total_native_spend_wei)?
        {
            return Err(Error::BudgetExceeded);
        }
        self.chain_verifier
            .verify_pending_exposure(&reserved.to_string())?;
        append(&tx, id, &event)?;
        let view = saved.view()?;
        tx.commit().map_err(storage)?;
        Ok(view)
    }

    fn request_identical_rebroadcast(&self, id: &str) -> Result<JobView> {
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        require_not_halted(&tx)?;
        let mut saved = job(&tx, id, &self.policy)?;
        if saved.rebroadcast_requested
            || saved.broadcast_attempts >= 3
            || !matches!(
                saved.phase,
                JobPhase::Unknown | JobPhase::Pending | JobPhase::Reorged
            )
        {
            return saved.view();
        }
        let event = Event::ReplayRequested;
        saved.apply(event.clone(), &self.policy)?;
        append(&tx, id, &event)?;
        let view = saved.view()?;
        tx.commit().map_err(storage)?;
        Ok(view)
    }

    pub fn release_for_broadcast(&self, id: &str) -> Result<BroadcastTicket> {
        self.require_active()?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        require_not_halted(&tx)?;
        let mut saved = job(&tx, id, &self.policy)?;
        self.chain_verifier
            .verify_admission(&saved.envelope, &saved.verified)?;
        let exposure = summary(&tx, &self.policy)?.reserved_native_wei;
        self.chain_verifier.verify_pending_exposure(&exposure)?;
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::InvalidTransition)?
            .as_secs();
        let event = Event::Released { at };
        saved.apply(event.clone(), &self.policy)?;
        append(&tx, id, &event)?;
        let signed = saved.signed.ok_or(Error::JournalMismatch)?;
        let ticket = BroadcastTicket {
            request_id: id.into(),
            operation_digest: saved.envelope.operation_digest,
            transaction_hash: signed.transaction_hash,
            raw_transaction: signed.raw_transaction,
        };
        tx.commit().map_err(storage)?;
        Ok(ticket)
    }

    pub fn acknowledge_submission(&self, id: &str, hash: &str) -> Result<JobView> {
        self.require_active()?;
        fixed_hex(hash, 32)?;
        self.append_transition(
            id,
            Event::Acknowledged {
                transaction_hash: hash.into(),
            },
        )
    }

    fn append_transition(&self, id: &str, event: Event) -> Result<JobView> {
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut saved = job(&tx, id, &self.policy)?;
        saved.apply(event.clone(), &self.policy)?;
        append(&tx, id, &event)?;
        let view = saved.view()?;
        tx.commit().map_err(storage)?;
        Ok(view)
    }

    pub fn observe_receipt(&self, evidence: &ReceiptEvidence) -> Result<JobView> {
        self.require_active()?;
        let (envelope, verified) = {
            let connection = self.lock()?;
            let snapshot = connection.unchecked_transaction().map_err(storage)?;
            let saved = job(&snapshot, &evidence.request_id, &self.policy)?;
            (saved.envelope, saved.verified)
        };
        self.chain_verifier
            .verify_receipt(evidence, &envelope, &verified)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut saved = job(&tx, &evidence.request_id, &self.policy)?;
        let anchor = treasury(&tx)?.ok_or(Error::TreasuryNotInitialized)?;
        if evidence.block_number <= anchor.block_number {
            return Err(Error::InvalidEvidence);
        }
        if saved.receipt.as_ref() == Some(evidence) {
            return saved.view();
        }
        match saved.validate_receipt(evidence, &self.policy) {
            Err(Error::CostExceeded) => {
                let event = Event::Incident {
                    code: IncidentCode::NativeCostExceeded,
                };
                saved.apply(event.clone(), &self.policy)?;
                append(&tx, &evidence.request_id, &event)?;
                tx.execute(
                    "INSERT INTO incidents(request_id,code) VALUES(?1,'native_cost_exceeded')",
                    [&evidence.request_id],
                )
                .map_err(storage)?;
                tx.commit().map_err(storage)?;
                return Err(Error::CostExceeded);
            }
            other => other?,
        }
        let event = Event::Receipt(evidence.clone());
        saved.apply(event.clone(), &self.policy)?;
        append(&tx, &evidence.request_id, &event)?;
        let view = saved.view()?;
        tx.commit().map_err(storage)?;
        Ok(view)
    }

    pub fn observe_reorg(&self, evidence: &ReorgEvidence) -> Result<JobView> {
        self.require_active()?;
        self.chain_verifier.verify_reorg(evidence)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut saved = job(&tx, &evidence.request_id, &self.policy)?;
        let event = Event::Reorg(evidence.clone());
        saved.apply(event.clone(), &self.policy)?;
        append(&tx, &evidence.request_id, &event)?;
        if saved.phase == JobPhase::Incident {
            tx.execute(
                "INSERT INTO incidents(request_id,code) VALUES(?1,'finality_contradiction')",
                [&evidence.request_id],
            )
            .map_err(storage)?;
        }
        let view = saved.view()?;
        tx.commit().map_err(storage)?;
        Ok(view)
    }

    pub fn get_job(&self, id: &str) -> Result<JobView> {
        request_id(id)?;
        let connection = self.lock()?;
        let snapshot = connection.unchecked_transaction().map_err(storage)?;
        job(&snapshot, id, &self.policy)?.view()
    }

    pub fn get_operation(&self, digest: &str) -> Result<JobView> {
        fixed_hex(digest, 32)?;
        let connection = self.lock()?;
        let snapshot = connection.unchecked_transaction().map_err(storage)?;
        let id: Option<String> = snapshot
            .query_row(
                "SELECT request_id FROM jobs WHERE operation_digest=?1",
                [digest],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage)?;
        job(&snapshot, &id.ok_or(Error::NotFound)?, &self.policy)?.view()
    }

    /// Local operator configuration installation, never an HTTP request. Signed
    /// schedule rotation preserves every nonce/nullifier/gas record. It changes
    /// prices and epochs only; release gates and treasury limits remain intact.
    pub fn install_fee_schedule(
        &self,
        packet: &fee_schedule::SignedFeeSchedule,
        pinned_key: [u8; 32],
    ) -> Result<()> {
        packet.verify(&pinned_key)?;
        let mut next = self.policy.clone();
        next.schedule_id = packet.schedule.schedule_id.clone();
        next.fee_units = packet.schedule.fee_units.clone();
        next.valid_from = packet.schedule.valid_from;
        next.valid_until = packet.schedule.valid_until;
        packet.verify_policy(&pinned_key, &next)?;
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let public = format!("0x{}", hex::encode(pinned_key));
        let current: Option<String> = tx
            .query_row("SELECT public_key FROM fee_publisher WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(storage)?;
        match current {
            Some(saved) if saved != public => return Err(Error::Conflict),
            None => {
                tx.execute(
                    "INSERT INTO fee_publisher(id,public_key) VALUES(1,?1)",
                    [public],
                )
                .map_err(storage)?;
            }
            _ => {}
        }
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM fee_schedules WHERE schedule_id=?1)",
                [&next.schedule_id],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if exists {
            if schedule_policy(&tx, &next.schedule_id, &self.policy)? != next {
                return Err(Error::Conflict);
            }
        } else {
            let count: u32 = tx
                .query_row("SELECT COUNT(*) FROM fee_schedules", [], |r| r.get(0))
                .map_err(storage)?;
            if count >= 10_000 {
                return Err(Error::BudgetExceeded);
            }
            let mut statement = tx
                .prepare("SELECT policy_json FROM fee_schedules")
                .map_err(storage)?;
            for row in statement
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(storage)?
            {
                let existing = AdmissionPolicy::parse(&row.map_err(storage)?)
                    .map_err(|_| Error::JournalMismatch)?;
                if next.valid_from < existing.valid_until {
                    return Err(Error::PolicyMismatch);
                }
            }
            drop(statement);
            tx.execute(
                "INSERT INTO fee_schedules(schedule_id,policy_json,policy_digest) VALUES(?1,?2,?3)",
                params![next.schedule_id, encode(&next)?, next.digest()?],
            )
            .map_err(storage)?;
        }
        let canonical = encode(packet)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT packet_json FROM fee_schedule_packets WHERE schedule_id=?1",
                [&next.schedule_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        if let Some(previous) = previous {
            if previous != canonical {
                return Err(Error::Conflict);
            }
        } else {
            tx.execute(
                "INSERT INTO fee_schedule_packets(schedule_id,packet_json) VALUES(?1,?2)",
                params![next.schedule_id, canonical],
            )
            .map_err(storage)?;
        }
        tx.commit().map_err(storage)?;
        Ok(())
    }

    pub fn summary(&self) -> Result<OperatorSummary> {
        let connection = self.lock()?;
        read_summary(&connection, &self.policy)
    }

    #[cfg(test)]
    fn open_harness(
        path: &Path,
        policy: AdmissionPolicy,
        proof: Arc<dyn ProofVerifier>,
        chain: Arc<dyn ChainEvidenceVerifier>,
    ) -> Result<Self> {
        let mut service = Self::open(path, policy, proof, chain)?;
        service.test_harness = true;
        Ok(service)
    }
}

fn append(connection: &Connection, id: &str, event: &Event) -> Result<()> {
    connection
        .execute(
            "INSERT INTO events(request_id,body) VALUES(?1,?2)",
            params![id, encode(event)?],
        )
        .map_err(storage)?;
    Ok(())
}

fn optional_job(
    connection: &Connection,
    id: &str,
    policy: &AdmissionPolicy,
) -> Result<Option<Job>> {
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM jobs WHERE request_id=?1)",
            [id],
            |row| row.get(0),
        )
        .map_err(storage)?;
    if exists {
        Ok(Some(job(connection, id, policy)?))
    } else {
        Ok(None)
    }
}

fn job(connection: &Connection, id: &str, policy: &AdmissionPolicy) -> Result<Job> {
    request_id(id)?;
    let row: Option<(String, String, String, i64)> = connection.query_row(
        "SELECT operation_digest,envelope_json,verified_operation_json,admitted_at FROM jobs WHERE request_id=?1", [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).optional().map_err(storage)?;
    let (digest, envelope_json, verified_json, admitted_at) = row.ok_or(Error::NotFound)?;
    let envelope = validate_envelope_json(&envelope_json).map_err(|_| Error::JournalMismatch)?;
    if digest != envelope.operation_digest
        || canonical_envelope(&envelope).map_err(|_| Error::JournalMismatch)? != envelope_json
        || admitted_at <= 0
    {
        return Err(Error::JournalMismatch);
    }
    schedule_policy(connection, &envelope.fee.schedule_id, policy)?
        .matches(&envelope, admitted_at as u64)
        .map_err(|_| Error::JournalMismatch)?;
    let verified: VerifiedOperation = decode(&verified_json).map_err(|_| Error::JournalMismatch)?;
    verified
        .validate(&envelope, policy)
        .map_err(|_| Error::JournalMismatch)?;
    if encode(&verified)? != verified_json {
        return Err(Error::JournalMismatch);
    }
    let mut claims = connection
        .prepare("SELECT tree_number,nullifier FROM nullifier_claims WHERE request_id=?1 ORDER BY nullifier")
        .map_err(storage)?;
    let claimed: Vec<(u64, String)> = claims
        .query_map([id], |row| {
            Ok((u64::from(row.get::<_, u16>(0)?), row.get(1)?))
        })
        .map_err(storage)?
        .collect::<std::result::Result<_, _>>()
        .map_err(storage)?;
    let mut expected: Vec<(u64, String)> = envelope
        .nullifiers
        .iter()
        .map(|nullifier| (verified.tree_number, nullifier.clone()))
        .collect();
    expected.sort();
    if claimed != expected {
        return Err(Error::JournalMismatch);
    }
    let mut saved = Job {
        request_id: id.into(),
        envelope,
        verified,
        phase: JobPhase::Admitted,
        signed: None,
        receipt: None,
        event_count: 0,
        broadcast_attempts: 0,
        last_broadcast_release_at: None,
        completed_reverts: Vec::new(),
        rebroadcast_requested: false,
    };
    let mut statement = connection
        .prepare("SELECT body FROM events WHERE request_id=?1 ORDER BY seq")
        .map_err(storage)?;
    let rows = statement
        .query_map([id], |row| row.get::<_, String>(0))
        .map_err(storage)?;
    for row in rows {
        let body = row.map_err(storage)?;
        let event: Event = decode(&body).map_err(|_| Error::JournalMismatch)?;
        if encode(&event)? != body {
            return Err(Error::JournalMismatch);
        }
        saved
            .apply(event, policy)
            .map_err(|_| Error::JournalMismatch)?;
    }
    let mut statement = connection
        .prepare(
            "SELECT attempt,nonce FROM outer_nonce_claims WHERE request_id=?1 ORDER BY attempt",
        )
        .map_err(storage)?;
    let claims: Vec<(u8, String)> = statement
        .query_map([id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(storage)?
        .collect::<std::result::Result<_, _>>()
        .map_err(storage)?;
    let expected: Vec<(u8, String)> = saved
        .completed_reverts
        .iter()
        .map(|attempt| &attempt.signed)
        .chain(saved.signed.iter())
        .enumerate()
        .map(|(index, signed)| ((index + 1) as u8, signed.nonce.to_string()))
        .collect();
    if claims != expected {
        return Err(Error::JournalMismatch);
    }
    Ok(saved)
}

fn treasury(connection: &Connection) -> Result<Option<TreasuryAnchor>> {
    let body: Option<String> = connection
        .query_row("SELECT body FROM treasury_anchor WHERE id=1", [], |row| {
            row.get(0)
        })
        .optional()
        .map_err(storage)?;
    body.map(|text| decode(&text).map_err(|_| Error::JournalMismatch))
        .transpose()
}

fn read_summary(connection: &Connection, policy: &AdmissionPolicy) -> Result<OperatorSummary> {
    // A mutex covers this connection only. The deferred read transaction keeps
    // all replay queries on one snapshot while other processes commit events.
    let snapshot = connection.unchecked_transaction().map_err(storage)?;
    summary(&snapshot, policy)
}

fn summary(connection: &Connection, policy: &AdmissionPolicy) -> Result<OperatorSummary> {
    let anchor = treasury(connection)?;
    if let Some(anchor) = &anchor {
        anchor
            .validate(policy)
            .map_err(|_| Error::JournalMismatch)?;
    }
    let opening = anchor
        .as_ref()
        .map(|a| decimal(&a.balance_wei))
        .transpose()?
        .unwrap_or_default();
    let mut reserved = U256::zero();
    let mut spent = U256::zero();
    let mut fees = U256::zero();
    let mut pending = 0u64;
    let mut jobs = 0u64;
    let mut halted = has_incident(connection)?;
    let mut statement = connection
        .prepare("SELECT request_id FROM jobs ORDER BY request_id")
        .map_err(storage)?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(storage)?;
    for id in ids {
        jobs = jobs.checked_add(1).ok_or(Error::Arithmetic)?;
        if jobs > MAX_LIFETIME_JOBS as u64 {
            return Err(Error::JournalMismatch);
        }
        let saved = job(connection, &id.map_err(storage)?, policy)?;
        let view = saved.view()?;
        let reserve = decimal(&view.reserved_native_wei)?;
        if !reserve.is_zero() {
            pending += 1;
        }
        reserved = reserved.checked_add(reserve).ok_or(Error::Arithmetic)?;
        spent = spent
            .checked_add(decimal(&view.finalized_native_charge_wei)?)
            .ok_or(Error::Arithmetic)?;
        fees = fees
            .checked_add(decimal(&view.earned_fee_units)?)
            .ok_or(Error::Arithmetic)?;
        halted |= saved.phase == JobPhase::Incident;
    }
    let available = opening
        .checked_sub(spent)
        .and_then(|balance| balance.checked_sub(reserved));
    if available.is_none() {
        halted = true;
    }
    if jobs > 0 && anchor.is_none() {
        return Err(Error::JournalMismatch);
    }
    Ok(OperatorSummary {
        schema_version: 1,
        chain_id: policy.domain.chain_id,
        jobs,
        pending_jobs: pending,
        opening_balance_wei: opening.to_string(),
        reserved_native_wei: reserved.to_string(),
        finalized_native_charge_wei: spent.to_string(),
        available_native_wei: available.unwrap_or_default().to_string(),
        earned_fee_units: fees.to_string(),
        halted,
        production_sponsorship_available: false,
    })
}

fn has_incident(connection: &Connection) -> Result<bool> {
    connection
        .query_row("SELECT EXISTS(SELECT 1 FROM incidents)", [], |row| {
            row.get(0)
        })
        .map_err(storage)
}

fn require_not_halted(connection: &Connection) -> Result<()> {
    if has_incident(connection)? {
        Err(Error::Halted)
    } else {
        Ok(())
    }
}

fn storage(_: rusqlite::Error) -> Error {
    Error::Storage
}

fn file_flags(readonly: bool) -> OpenFlags {
    let access = if readonly {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    access | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_NOFOLLOW
}

fn regular_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| Error::Storage)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Error::JournalMismatch);
    }
    Ok(())
}

fn verify_identity(connection: &Connection) -> Result<()> {
    let app: i64 = connection
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(storage)?;
    let schema: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(storage)?;
    if app != APP_ID || schema != SCHEMA_VERSION {
        return Err(Error::JournalMismatch);
    }
    let check: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(storage)?;
    if check != "ok" {
        return Err(Error::JournalMismatch);
    }
    Ok(())
}

fn stored_policy(connection: &Connection) -> Result<AdmissionPolicy> {
    let (text, digest): (String, String) = connection
        .query_row(
            "SELECT policy_json,policy_digest FROM metadata WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(storage)?;
    let policy = AdmissionPolicy::parse(&text).map_err(|_| Error::JournalMismatch)?;
    if policy.digest()? != digest || encode(&policy)? != text {
        return Err(Error::JournalMismatch);
    }
    Ok(policy)
}

fn schedule_policy(
    connection: &Connection,
    id: &str,
    base: &AdmissionPolicy,
) -> Result<AdmissionPolicy> {
    fixed_hex(id, 32)?;
    let row: Option<(String, String)> = connection
        .query_row(
            "SELECT policy_json,policy_digest FROM fee_schedules WHERE schedule_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    let (text, digest) = row.ok_or(Error::PolicyMismatch)?;
    let policy = AdmissionPolicy::parse(&text).map_err(|_| Error::JournalMismatch)?;
    if policy.schedule_id != id || policy.digest()? != digest || encode(&policy)? != text {
        return Err(Error::JournalMismatch);
    }
    let mut fixed = policy.clone();
    fixed.schedule_id = base.schedule_id.clone();
    fixed.fee_units = base.fee_units.clone();
    fixed.valid_from = base.valid_from;
    fixed.valid_until = base.valid_until;
    if fixed != *base {
        return Err(Error::JournalMismatch);
    }
    let packet: Option<String> = connection
        .query_row(
            "SELECT packet_json FROM fee_schedule_packets WHERE schedule_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    if let Some(packet) = packet {
        let public: String = connection
            .query_row("SELECT public_key FROM fee_publisher WHERE id=1", [], |r| {
                r.get(0)
            })
            .map_err(storage)?;
        let key: [u8; 32] = model::bounded_hex(&public, 32)?
            .try_into()
            .map_err(|_| Error::JournalMismatch)?;
        fee_schedule::SignedFeeSchedule::parse(&packet)?
            .verify_policy(&key, &policy)
            .map_err(|_| Error::JournalMismatch)?;
    } else if id != base.schedule_id {
        return Err(Error::JournalMismatch);
    }
    Ok(policy)
}

fn open_journal(path: &Path, policy: &AdmissionPolicy) -> Result<Connection> {
    if path.as_os_str().is_empty() || path == Path::new(":memory:") {
        return Err(Error::JournalMismatch);
    }
    let created = match fs::symlink_metadata(path) {
        Ok(_) => {
            regular_file(path)?;
            false
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(path).map_err(|_| Error::Storage)?;
            true
        }
        Err(_) => return Err(Error::Storage),
    };
    let mut connection = Connection::open_with_flags(path, file_flags(false)).map_err(storage)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(storage)?;
    if !created {
        verify_identity(&connection)?;
        if stored_policy(&connection)? != *policy {
            return Err(Error::JournalMismatch);
        }
    }
    connection
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")
        .map_err(storage)?;
    if created {
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        tx.execute_batch(SCHEMA).map_err(storage)?;
        tx.pragma_update(None, "application_id", APP_ID)
            .map_err(storage)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(storage)?;
        tx.execute(
            "INSERT INTO metadata(id,policy_json,policy_digest) VALUES(1,?1,?2)",
            params![encode(policy)?, policy.digest()?],
        )
        .map_err(storage)?;
        tx.execute(
            "INSERT INTO fee_schedules(schedule_id,policy_json,policy_digest) VALUES(?1,?2,?3)",
            params![policy.schedule_id, encode(policy)?, policy.digest()?],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
    }
    verify_identity(&connection)?;
    Ok(connection)
}

/// Read-only operational totals: no job IDs, addresses, raw transactions or
/// encrypted user payloads are exported. A schema/policy mismatch is an error.
pub fn inspect_database(path: &Path) -> Result<OperatorSummary> {
    regular_file(path)?;
    let connection = Connection::open_with_flags(path, file_flags(true)).map_err(storage)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(storage)?;
    let snapshot = connection.unchecked_transaction().map_err(storage)?;
    verify_identity(&snapshot)?;
    let policy = stored_policy(&snapshot)?;
    summary(&snapshot, &policy)
}

#[cfg(test)]
mod rpc_tests;
#[cfg(test)]
mod runtime_tests;
#[cfg(test)]
mod subprocess_tests;
#[cfg(test)]
mod tests;
