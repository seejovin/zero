//! Durable operation driver. No new transaction is created after a release; an
//! uncertain RPC outcome retains the exact saved bytes, nonce and reservation.
use rusqlite::params;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use crate::rpc::ArbitrumRpc;
use crate::signer::NativeTreasurySigner;
use crate::{job, storage, Error, GasService, JobPhase, JobView, ReorgEvidence, Result};

pub struct OperatorWorker {
    service: Arc<GasService>,
    rpc: Arc<ArbitrumRpc>,
    signer: NativeTreasurySigner,
    cursor: AtomicUsize,
    audit_cursor: AtomicUsize,
    single_slot_audit: AtomicBool,
}

impl OperatorWorker {
    pub fn new(
        service: Arc<GasService>,
        rpc: Arc<ArbitrumRpc>,
        signer: NativeTreasurySigner,
    ) -> Result<Self> {
        if signer.address() != service.policy.broadcaster_address {
            return Err(Error::InvalidPolicy);
        }
        Ok(Self {
            service,
            rpc,
            signer,
            cursor: AtomicUsize::new(0),
            audit_cursor: AtomicUsize::new(0),
            single_slot_audit: AtomicBool::new(true),
        })
    }

    pub fn initialize_treasury(&self) -> Result<()> {
        self.service.require_active()?;
        {
            let connection = self.service.lock()?;
            if let Some(anchor) = crate::treasury(&connection)? {
                self.service.chain_verifier.verify_treasury(&anchor)?;
                return Ok(());
            }
        }
        self.service
            .initialize_treasury(&self.rpc.opening_treasury()?)?;
        Ok(())
    }

    pub fn process_once(&self, id: &str) -> Result<JobView> {
        self.service.require_active()?;
        let mut saved = self.snapshot(id)?;
        if saved.phase == JobPhase::Incident {
            return saved.view();
        }
        // Earlier reverted attempts remain part of the native charge projection
        // and must still be audited after a later attempt starts or succeeds.
        let receipts: Vec<_> = saved.receipts().cloned().collect();
        for previous in receipts {
            let canonical = self.rpc.canonical_block(previous.block_number)?;
            if canonical.hash != previous.block_hash {
                self.service.observe_reorg(&ReorgEvidence {
                    request_id: id.into(),
                    chain_id: self.service.policy.domain.chain_id,
                    previous_block_number: previous.block_number,
                    previous_block_hash: previous.block_hash.clone(),
                    canonical_block_hash: canonical.hash,
                })?;
                saved = self.snapshot(id)?;
                if saved.phase == JobPhase::Incident {
                    return saved.view();
                }
            }
        }
        if matches!(
            saved.phase,
            JobPhase::Finalized | JobPhase::FinalizedRevert | JobPhase::RetryExhausted
        ) {
            return saved.view();
        }
        if saved.phase == JobPhase::Admitted {
            let mut quote = self.rpc.quote(&saved.envelope, &saved.verified)?;
            let connection = self.service.lock()?;
            let mut statement = connection
                .prepare("SELECT nonce FROM outer_nonce_claims")
                .map_err(storage)?;
            for row in statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(storage)?
            {
                let text = row.map_err(storage)?;
                let nonce = text.parse::<u64>().map_err(|_| Error::JournalMismatch)?;
                if nonce.to_string() != text {
                    return Err(Error::JournalMismatch);
                }
                quote.nonce = quote
                    .nonce
                    .max(nonce.checked_add(1).ok_or(Error::Arithmetic)?);
            }
            drop(statement);
            drop(connection);
            self.service.sign_admitted(&self.signer, id, &quote)?;
            saved = self.snapshot(id)?;
        }
        let signed = saved.signed.as_ref().ok_or(Error::InvalidTransition)?;
        if let Some(receipt) = self.rpc.operation_receipt(
            id,
            &saved.envelope,
            &saved.verified,
            &signed.transaction_hash,
        )? {
            return self.service.observe_receipt(&receipt);
        }
        if saved.receipt.is_some() {
            return Err(Error::ConflictingEvidence);
        }
        // An uncertain attempt is reconciled automatically, but another release
        // requires an explicit identical request. Never manufacture a new nonce
        // or repeat an uncertain send merely because a timer elapsed.
        if !saved.rebroadcast_requested {
            return saved.view();
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::InvalidTransition)?
            .as_secs();
        let retry_delay = 30u64
            .checked_shl(saved.broadcast_attempts.saturating_sub(1).min(4))
            .ok_or(Error::Arithmetic)?;
        if saved.broadcast_attempts >= 3
            || saved
                .last_broadcast_release_at
                .is_some_and(|at| now < at.saturating_add(retry_delay))
        {
            return saved.view();
        }
        let ticket = self.service.release_for_broadcast(id)?;
        match self
            .rpc
            .submit_saved(&ticket.raw_transaction, &ticket.transaction_hash)
        {
            Ok(()) => self
                .service
                .acknowledge_submission(id, &ticket.transaction_hash),
            // Timeout, RPC error, or an unexpected hash cannot establish failure.
            // No new nonce, proof, recipient or fee is constructed here.
            Err(_) => self.service.get_job(id),
        }
    }

    /// Bounded queue scan. Per-request transport failures remain in the journal;
    /// no request identifiers, addresses, payloads or provider errors are logged.
    pub fn process_batch(&self, maximum: u16) -> Result<usize> {
        self.service.require_active()?;
        if maximum == 0 || maximum > 64 {
            return Err(Error::InvalidPolicy);
        }
        let (ids, audit) = {
            let connection = self.service.lock()?;
            let snapshot = connection.unchecked_transaction().map_err(storage)?;
            crate::require_not_halted(&snapshot)?;
            // Queue selection only. Every selected record is independently
            // replayed and validated by get_job/process_once before any action.
            const SELECT: &str = "WITH latest AS (SELECT j.request_id,j.admitted_at,(SELECT body FROM events e WHERE e.request_id=j.request_id ORDER BY seq DESC LIMIT 1) AS body FROM jobs j) ";
            const TERMINAL: &str = "json_extract(body,'$.kind')='Receipt' AND json_extract(body,'$.data.finalized_height')>=json_extract(body,'$.data.block_number')";
            let mut statement = snapshot
                .prepare(&format!("{SELECT} SELECT request_id FROM latest WHERE NOT COALESCE(({TERMINAL}),0) ORDER BY admitted_at,request_id LIMIT ?1"))
                .map_err(storage)?;
            let rows = statement
                .query_map(params![1000i64], |row| row.get::<_, String>(0))
                .map_err(storage)?;
            let ids = rows
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(storage)?;
            let count: i64 = snapshot
                .query_row(
                    &format!("{SELECT} SELECT COUNT(*) FROM latest WHERE {TERMINAL}"),
                    [],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            let audit = if count > 0 {
                let offset = self.audit_cursor.load(Ordering::Relaxed) % (count as usize);
                Some(snapshot.query_row(&format!("{SELECT} SELECT request_id FROM latest WHERE {TERMINAL} ORDER BY admitted_at,request_id LIMIT 1 OFFSET ?1"),[offset as i64],|row|row.get::<_,String>(0)).map_err(storage)?)
            } else {
                None
            };
            (ids, audit)
        };
        let mut processed = 0usize;
        // With a one-slot batch, alternate terminal audits and active work.
        // Clamping the live allowance to one would starve audits indefinitely
        // whenever active jobs remain. Larger runtime batches reserve one slot.
        let reserve_audit = audit.is_some()
            && (maximum > 1 || self.single_slot_audit.fetch_xor(true, Ordering::Relaxed));
        let live_limit = usize::from(maximum) - usize::from(reserve_audit);
        let start = if ids.is_empty() {
            0
        } else {
            self.cursor.fetch_add(live_limit, Ordering::Relaxed) % ids.len()
        };
        for id in ids
            .iter()
            .cycle()
            .skip(start)
            .take(if live_limit == 0 { 0 } else { ids.len() })
        {
            let phase = self.service.get_job(id)?.phase;
            if matches!(
                phase,
                JobPhase::Finalized
                    | JobPhase::FinalizedRevert
                    | JobPhase::RetryExhausted
                    | JobPhase::Incident
            ) {
                continue;
            }
            if let Err(Error::Halted) = self.process_once(id) {
                return Err(Error::Halted);
            }
            processed += 1;
            if processed >= live_limit {
                break;
            }
        }
        if processed < usize::from(maximum) {
            if let Some(id) = audit {
                self.audit_cursor.fetch_add(1, Ordering::Relaxed);
                if let Err(Error::Halted) = self.process_once(&id) {
                    return Err(Error::Halted);
                }
                processed += 1;
            }
        }
        Ok(processed)
    }

    fn snapshot(&self, id: &str) -> Result<crate::Job> {
        let connection = self.service.lock()?;
        let snapshot = connection.unchecked_transaction().map_err(storage)?;
        job(&snapshot, id, &self.service.policy)
    }
}
