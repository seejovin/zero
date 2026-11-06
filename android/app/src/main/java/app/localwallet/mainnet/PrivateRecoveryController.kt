package app.localwallet.mainnet

import org.json.JSONArray
import org.json.JSONObject
import uniffi.wallet_core.privateApproveRecoverySweep
import uniffi.wallet_core.privateCompleteRecoverySweep
import uniffi.wallet_core.privatePrepareRecoverySweep
import uniffi.wallet_core.privateRecordRecoveryCutoff
import uniffi.wallet_core.privateRecordRecoverySweepBatch
import uniffi.wallet_core.privateRecoveryWorkspace
import java.math.BigInteger

/** Plaintext here is local-only and never accepted by the OHTTP submission interface. */
internal class PrivateRecoveryReview(
    val recoveryId: String,
    val noteCount: Int,
    val inputUnits: BigInteger,
    val maximumFeeUnits: BigInteger,
    val planJson: String
) {
    override fun toString() = "PrivateRecoveryReview([local plan redacted])"
}

internal class PrivateRecoveryWorkspace(
    val recoveryId: String?,
    val walletBinding: String,
    val quarantined: Boolean,
    val incident: Boolean,
    val cutoffJson: String?,
    val planJson: String?,
    val batches: List<PrivateJournalEntry>,
    val reconciled: List<PrivateJournalEntry>,
    val remaining: JSONArray
) {
    override fun toString() = "PrivateRecoveryWorkspace(quarantined=$quarantined, incident=$incident)"
}

/**
 * Real local recovery orchestration. Native encrypted history owns the cutoff, plan, batches,
 * reservations and final quarantine decision. No UI preference can mark recovery complete.
 */
internal class PrivateRecoveryController(
    private val engine: PrivateWebLocalEngine,
    private val journal: PrivateNativeJournal,
    private val transport: PrivateSubmissionTransport,
    private val now: () -> Long
) {
    @Volatile private var foreground = true
    @Volatile var workspace: PrivateRecoveryWorkspace? = null
        private set
    @Volatile var proposedPlan: PrivateRecoveryReview? = null
        private set
    private var batchCoordinator: PrivatePaymentCoordinator? = null

    fun setForeground(value: Boolean) { foreground = value; batchCoordinator?.setForeground(value) }
    fun currentBatch(): PrivatePaymentState? = batchCoordinator?.snapshot()
    fun currentBatchReview(): PrivatePaymentReview? = batchCoordinator?.currentReview()

    fun reload(): PrivateRecoveryWorkspace {
        val document = RpcJson.parse(privateRecoveryWorkspace(journal.recoveryHandle()), maxChars = 2_097_152, maxNodes = 100_000)
        PrivateEngineWire.fields(document, "status", "cutoff_json", "plan_json", "batches", "reconciled_records", "remaining_note_commitments")
        val status = document.getJSONObject("status")
        val id = status.optionalString("recovery_id")?.let(PrivateIdentifiers::require)
        val binding = PrivateIdentifiers.require(PrivateEngineWire.string(status, "wallet_binding"))
        fun entries(name: String): List<PrivateJournalEntry> = document.getJSONArray(name).objects().map {
            journal.get(PrivateIdentifiers.require(PrivateEngineWire.string(it, "intent_id")))
        }
        val remaining = document.getJSONArray("remaining_note_commitments")
        if (remaining.length() > 1024) throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
        val unique = HashSet<Pair<Long, String>>()
        remaining.objects().forEach { note ->
            PrivateEngineWire.fields(note, "tree_number", "commitment", "nullifier")
            val tree = PrivateEngineWire.number(note, "tree_number", 0, 65_535)
            PrivateIdentifiers.require(PrivateEngineWire.string(note, "commitment"))
            val nullifier = PrivateIdentifiers.require(PrivateEngineWire.string(note, "nullifier"))
            if (!unique.add(tree to nullifier)) throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
        }
        return PrivateRecoveryWorkspace(id, binding, PrivateEngineWire.boolean(status, "quarantined"),
            PrivateEngineWire.boolean(status, "incident"), document.optionalString("cutoff_json"),
            document.optionalString("plan_json"), entries("batches"), entries("reconciled_records"), remaining).also {
            workspace = it
        }
    }

    /** Captures a latest tip durably first, then waits for complete finalized coverage of it. */
    fun preparePlan(): PrivateRecoveryReview {
        requireForeground()
        var current = requireQuarantine(reload())
        if (current.planJson != null) throw PrivatePaymentFailure(PrivateFailure.BUSY)
        val id = current.recoveryId!!
        if (current.cutoffJson == null) {
            val cutoff = engine.recoveryCutoff(id)
            privateRecordRecoveryCutoff(journal.recoveryHandle(), id, cutoff)
            current = requireQuarantine(reload())
        }
        val cutoff = RpcJson.parse(current.cutoffJson ?: throw PrivatePaymentFailure(PrivateFailure.STORAGE), maxChars = 16_384)
        val scan = RpcJson.parse(engine.recoveryScan(id, current.walletBinding, cutoff, finalScan = false), maxChars = 524_288)
        val notes = scan.getJSONArray("spendable_notes")
        var total = BigInteger.ZERO
        notes.objects().forEach { total = PrivateUnits.add(total, PrivateUnits.fromCanonical(it.getJSONObject("note").getString("units"), positive = true)) }
        // A conservative visible cap permits at most one shared fee per selected note. Each actual
        // batch has its own exact review; native history enforces their accumulated fee cap.
        val fee = if (notes.length() == 0) BigInteger.ZERO else engine.recoveryFeeUnits()
        var cap = BigInteger.ZERO
        repeat(notes.length()) { cap = PrivateUnits.add(cap, fee) }
        val rawPlan = JSONObject().put("schema_version", 1).put("recovery_id", id)
            .put("own_private_recipient", engine.address()).put("initial_scan", scan)
            .put("max_total_fee_units", cap.toString()).put("approved_at", now()).put("plan_digest", "")
        val plan = privatePrepareRecoverySweep(rawPlan.toString())
        return PrivateRecoveryReview(id, notes.length(), total, cap, plan).also { proposedPlan = it }
    }

    /** Approval records a self-sweep plan, never a recipient supplied by a service or UI input. */
    fun approvePlan(review: PrivateRecoveryReview): PrivateRecoveryWorkspace {
        requireForeground()
        if (proposedPlan !== review) throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
        val current = requireQuarantine(reload())
        if (current.recoveryId != review.recoveryId || current.planJson != null) throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
        privateApproveRecoverySweep(journal.recoveryHandle(), review.planJson)
        proposedPlan = null
        return reload()
    }

    /** One batch at a time; a timeout or unknown batch never leads to a new authorization. */
    fun prepareNextBatch(): PrivatePaymentState {
        requireForeground()
        val current = requireQuarantine(reload())
        if (current.batches.any { it.phase !in setOf(PrivateJournalPhase.FINALIZED, PrivateJournalPhase.CANCELLED, PrivateJournalPhase.SUPERSEDED) }) {
            throw PrivatePaymentFailure(PrivateFailure.BUSY)
        }
        if (current.remaining.length() == 0) throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
        val plan = current.planJson ?: throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
        val prepared = engine.prepareRecoveryBatch(plan, current.remaining)
        val coordinator = newCoordinator(current.recoveryId!!)
        batchCoordinator = coordinator
        return coordinator.reviewPrepared(prepared)
    }

    fun resumeBatch(intentId: String): PrivatePaymentState {
        val current = requireQuarantine(reload())
        if (current.batches.none { it.intentId == intentId }) throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
        return newCoordinator(current.recoveryId!!).also { batchCoordinator = it }.restore(intentId)
    }

    fun approveBatchAndSubmit(stillAuthorized: () -> Boolean): PrivatePaymentState {
        requireForeground()
        val coordinator = batchCoordinator ?: throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
        val review = coordinator.currentReview() ?: throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
        val stamp = coordinator.authorizationStamp()
        val result = coordinator.approveAndProve(review)
        return if (result.stage == PrivatePaymentStage.READY && stillAuthorized() && coordinator.authorizationCurrent(stamp)) {
            coordinator.submitExisting().also { reload() }
        } else result.also { reload() }
    }

    fun retryBatch(stillAuthorized: () -> Boolean): PrivatePaymentState {
        requireForeground()
        if (!stillAuthorized()) throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED)
        val coordinator = requireBatch()
        val id = coordinator.snapshot().intentId ?: throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
        coordinator.restore(id)
        return coordinator.submitExisting().also { reload() }
    }

    fun reconcileBatch(): PrivatePaymentState {
        val coordinator = requireBatch()
        val id = coordinator.snapshot().intentId ?: throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
        coordinator.restore(id)
        return coordinator.reconcile().also { reload() }
    }

    fun cancelUnreleasedBatch(): PrivatePaymentState {
        requireForeground()
        return requireBatch().cancelUnreleased().also { reload() }
    }

    /** Exact canonical receipts and a fresh full scan are rechecked before native quarantine ends. */
    fun finish(): PrivateRecoveryWorkspace {
        requireForeground()
        var current = requireQuarantine(reload())
        if (current.remaining.length() != 0 || current.batches.any {
                it.phase !in setOf(PrivateJournalPhase.FINALIZED, PrivateJournalPhase.CANCELLED, PrivateJournalPhase.SUPERSEDED)
            }) throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
        val selected = (current.batches + current.reconciled).filter { it.phase == PrivateJournalPhase.FINALIZED }.distinctBy { it.intentId }
        selected.forEach { saved ->
            when (val observation = engine.observeCanonical(saved)) {
                is PrivateChainObservation.Receipt -> journal.observeReceipt(saved.intentId, observation)
                is PrivateChainObservation.Reorg -> { journal.markReorg(saved.intentId, observation); throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION) }
                null -> throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
            }
        }
        current = requireQuarantine(reload())
        val plan = RpcJson.parse(current.planJson ?: throw PrivatePaymentFailure(PrivateFailure.STORAGE), maxChars = 524_288)
        val cutoff = RpcJson.parse(current.cutoffJson ?: throw PrivatePaymentFailure(PrivateFailure.STORAGE), maxChars = 16_384)
        val scan = RpcJson.parse(engine.recoveryScan(current.recoveryId!!, current.walletBinding, cutoff, finalScan = true), maxChars = 524_288)
        val receipts = JSONArray()
        (current.batches + current.reconciled).filter { it.phase == PrivateJournalPhase.FINALIZED }.distinctBy { it.intentId }.forEach { entry ->
            val receipt = entry.priorReceiptForLocalEngine() ?: throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
            receipts.put(JSONObject().put("intent_id", entry.intentId).put("receipt", RpcJson.parse(receipt, maxChars = 524_288)))
        }
        val resolution = JSONObject().put("schema_version", 1).put("recovery_id", current.recoveryId)
            .put("plan_digest", plan.getString("plan_digest")).put("final_scan", scan).put("finalized_receipts", receipts)
        privateCompleteRecoverySweep(journal.recoveryHandle(), resolution.toString())
        return reload()
    }

    private fun newCoordinator(recoveryId: String): PrivatePaymentCoordinator {
        val recoveryJournal = object : PrivateOperationJournal by journal {
            override fun recordApproved(review: PrivatePaymentReview): PrivateJournalEntry {
                PrivateNativeJournal.checkReviewBinding(review)
                privateRecordRecoverySweepBatch(journal.recoveryHandle(), recoveryId, review.localIntentForJournal())
                return journal.get(review.intentId)
            }
        }
        return PrivatePaymentFeature.create(engine, recoveryJournal, transport, now).also { it.setForeground(foreground) }
    }
    private fun requireBatch() = batchCoordinator ?: throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
    private fun requireQuarantine(value: PrivateRecoveryWorkspace): PrivateRecoveryWorkspace {
        if (!value.quarantined || value.incident || value.recoveryId == null) throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
        return value
    }
    private fun requireForeground() { if (!foreground) throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED) }
}
