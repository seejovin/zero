package app.localwallet.mainnet

import java.math.BigInteger
import java.security.SecureRandom

/** Public asset metadata. A reviewed engine must additionally pin the protocol and deployment. */
internal data class PrivatePaymentAsset(val chainId: Long, val tokenContract: String, val symbol: String, val decimals: Int) {
    init {
        if (chainId <= 0 || decimals !in 0..18 || symbol !in setOf("USDC", "USDT", "EURC") ||
            !Regex("0x[0-9a-f]{40}").matches(tokenContract) || tokenContract.drop(2).all { it == '0' }) {
            throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
        }
    }
}

/** Device-local input. It deliberately has no JSON or network serialization method. */
internal class PrivateTransferDraft private constructor(
    val intentId: String,
    val asset: PrivatePaymentAsset,
    val recipient: String,
    val recipientUnits: BigInteger?,
    val maximum: Boolean
) {
    companion object {
        /** Used only by the trusted local recovery engine for one exact, explicitly reviewed batch. */
        internal fun recovery(asset: PrivatePaymentAsset, recipient: String, units: BigInteger, intentId: String): PrivateTransferDraft {
            val text = units.toString().padStart(asset.decimals + 1, '0').let { if (asset.decimals == 0) it else it.dropLast(asset.decimals) + "." + it.takeLast(asset.decimals) }
            val checked = create(asset, recipient, text, maximum = false)
            PrivateIdentifiers.require(intentId)
            if (checked.recipientUnits != units) throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
            return PrivateTransferDraft(intentId, asset, checked.recipient, units, maximum = false)
        }

        fun create(asset: PrivatePaymentAsset, recipient: String, amount: String, maximum: Boolean = false): PrivateTransferDraft {
            val destination = recipient.trim()
            if (destination.length !in 8..256 || !PrivateAddressRules.looksPrivate(destination) ||
                destination.any { it.code !in 33..126 }) {
                throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
            }
            // This is routing validation only. The local protocol engine must decode/check the
            // address and reject wrong networks, bad checksums and unsupported privacy systems.
            val units = if (maximum) null else try { Amounts.parse(amount.trim(), asset.decimals) }
                catch (_: Exception) { throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT) }
            val identity = ByteArray(32).also { bytes ->
                do { SecureRandom().nextBytes(bytes) } while (bytes.all { it == 0.toByte() })
            }
            val intentId = "0x" + identity.joinToString("") { "%02x".format(it.toInt() and 255) }
            identity.fill(0)
            return PrivateTransferDraft(intentId, asset, destination, units, maximum)
        }
    }

    override fun toString(): String = "PrivateTransferDraft([private input redacted])"
}

/** A locally prepared review. The native journal repeats all amount and intent binding checks. */
internal class PrivatePaymentReview(
    val draft: PrivateTransferDraft,
    val plan: PrivateSpendPlan,
    val recipientUnits: BigInteger,
    val quoteValidUntilEpochSeconds: Long,
    private val canonicalIntentJson: String
) {
    val intentId: String get() = draft.intentId
    init {
        if (quoteValidUntilEpochSeconds <= 0 || canonicalIntentJson.length !in 2..262_144 ||
            (draft.maximum && recipientUnits != plan.maximumUnits()) ||
            (!draft.maximum && recipientUnits != draft.recipientUnits)) {
            throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
        }
        plan.changeFor(recipientUnits)
    }

    fun localIntentForJournal(): String = canonicalIntentJson
    override fun toString(): String = "PrivatePaymentReview([private input redacted])"
}

/** Locally generated proof data. Only a strict, durable native journal may release it for transport. */
internal class PrivateProvedOperation(private val canonicalEnvelopeJson: String) {
    init {
        if (canonicalEnvelopeJson.length !in 2..2_097_152) throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
    }
    fun localEnvelopeForJournal(): String = canonicalEnvelopeJson
    override fun toString(): String = "PrivateProvedOperation([authorized operation redacted])"
}

internal enum class PrivateJournalPhase {
    APPROVED, READY, RELEASED, UNKNOWN, INCLUDED, FINALIZED, FAILED, REORG, CONFLICT, CANCELLED, SUPERSEDED
}

/**
 * Returned only by the device-local encrypted journal adapter. The adapter must parse the native
 * core result, not an API response. It validates/digest-binds the strict envelope before READY.
 */
internal class PrivateJournalEntry(
    val intentId: String,
    val phase: PrivateJournalPhase,
    val releaseCount: Long,
    val quoteValidUntilEpochSeconds: Long,
    val operationDigest: String?,
    private val canonicalEnvelopeJson: String?,
    private val priorReceiptJson: String? = null,
    val releaseHistoryIncomplete: Boolean = false,
    val finalizedRevert: Boolean = false,
    val envelopeBackedUp: Boolean = false
) {
    init {
        if (!Regex("0x[0-9a-f]{64}").matches(intentId) || intentId.drop(2).all { it == '0' } ||
            releaseCount < 0 || quoteValidUntilEpochSeconds <= 0 ||
            (operationDigest != null && !Regex("0x[0-9a-f]{64}").matches(operationDigest)) ||
            (canonicalEnvelopeJson != null && canonicalEnvelopeJson.length !in 2..2_097_152) ||
            ((operationDigest == null) != (canonicalEnvelopeJson == null)) ||
            (phase !in setOf(PrivateJournalPhase.APPROVED, PrivateJournalPhase.CANCELLED, PrivateJournalPhase.SUPERSEDED) && operationDigest == null) ||
            (phase in setOf(PrivateJournalPhase.RELEASED, PrivateJournalPhase.UNKNOWN) && releaseCount == 0L)) {
            throw PrivatePaymentFailure(PrivateFailure.STORAGE)
        }
    }

    fun sameOperation(other: PrivateJournalEntry): Boolean = intentId == other.intentId &&
        operationDigest == other.operationDigest && canonicalEnvelopeJson == other.canonicalEnvelopeJson &&
        quoteValidUntilEpochSeconds == other.quoteValidUntilEpochSeconds && releaseHistoryIncomplete == other.releaseHistoryIncomplete

    fun envelopeForLocalEngine(): String? = canonicalEnvelopeJson
    fun priorReceiptForLocalEngine(): String? = priorReceiptJson

    fun releasedOperation(): PrivateReleasedOperation {
        if (phase !in setOf(PrivateJournalPhase.RELEASED, PrivateJournalPhase.UNKNOWN) || releaseCount <= 0) {
            throw PrivatePaymentFailure(PrivateFailure.CANCELLATION_UNAVAILABLE)
        }
        return PrivateReleasedOperation.fromJournal(operationDigest!!, canonicalEnvelopeJson!!)
    }

    override fun toString(): String = "PrivateJournalEntry(phase=$phase, operation=[redacted])"
}

/** The ONLY value accepted by the submission interface; never a local draft, mnemonic or review. */
internal class PrivateReleasedOperation private constructor(
    val operationDigest: String,
    private val canonicalEnvelopeJson: String
) {
    companion object {
        internal fun fromJournal(digest: String, envelope: String): PrivateReleasedOperation =
            PrivateReleasedOperation(digest, envelope)
    }
    fun transportBytes(): ByteArray = canonicalEnvelopeJson.toByteArray(Charsets.UTF_8)
    override fun toString(): String = "PrivateReleasedOperation([authorized operation redacted])"
}

/** Service acceptance is not proof of chain inclusion or recipient payment. */
internal data class PrivateSubmissionAcknowledgement(val operationDigest: String, val accepted: Boolean)

internal sealed class PrivateChainObservation {
    /** Must be assembled and checked by the local protocol adapter against canonical chain data. */
    class Receipt(private val evidence: String) : PrivateChainObservation() {
        fun evidenceForJournal(): String = evidence
        override fun toString(): String = "PrivateChainObservation.Receipt([evidence redacted])"
    }
    class Reorg(private val evidence: String) : PrivateChainObservation() {
        fun evidenceForJournal(): String = evidence
        override fun toString(): String = "PrivateChainObservation.Reorg([evidence redacted])"
    }
}

internal interface PrivateLocalEngine {
    /** May read common encrypted history; may not upload the draft or a viewing/spending key. */
    fun prepareLocal(draft: PrivateTransferDraft): PrivatePaymentReview

    /** Must be entirely local. No usable proof may be externally simulated or sent here. */
    fun proveLocal(review: PrivatePaymentReview, progress: (Int) -> Unit): PrivateProvedOperation

    /** Discards an unsubmitted local review; never removes a durable reservation or released proof. */
    fun discardLocalReview(review: PrivatePaymentReview)

    /** Independently rechecks a saved proof and its private-only calldata before any egress. */
    fun verifyForSubmission(entry: PrivateJournalEntry)

    /** Matches the exact operation/outputs, not merely a spent nullifier or service assertion. */
    fun observeCanonical(entry: PrivateJournalEntry): PrivateChainObservation?
}

internal interface PrivateOperationJournal {
    fun recordApproved(review: PrivatePaymentReview): PrivateJournalEntry
    fun recordReady(intentId: String, operation: PrivateProvedOperation): PrivateJournalEntry
    fun markReleased(intentId: String): PrivateJournalEntry
    fun markUnknown(intentId: String): PrivateJournalEntry
    fun observeReceipt(intentId: String, observation: PrivateChainObservation.Receipt): PrivateJournalEntry
    fun markReorg(intentId: String, observation: PrivateChainObservation.Reorg): PrivateJournalEntry
    /** Native core allows APPROVED/READY cancellation only when no release or receipt exists. */
    fun cancelUnreleased(intentId: String): PrivateJournalEntry
    fun get(intentId: String): PrivateJournalEntry
}

internal interface PrivateSubmissionTransport {
    /** Must use a reviewed independent privacy relay. There is no public RPC fallback method. */
    fun submit(operation: PrivateReleasedOperation): PrivateSubmissionAcknowledgement
}

internal enum class PrivatePaymentStage {
    EMPTY, PREPARING, REVIEW, PROVING, READY, SUBMITTING, AWAITING_CONFIRMATION,
    UNKNOWN, INCLUDED, FINALIZED, FAILED, CONFLICT, CANCELLED, SUPERSEDED, APPROVED_UNFINISHED, ERROR
}

/** No recipient, principal, note identity or proof can accidentally enter a UI diagnostic export. */
internal data class PrivatePaymentState(
    val stage: PrivatePaymentStage,
    val intentId: String?,
    val provingProgress: Int?,
    val failure: PrivateFailure?
)

/**
 * Single-payment coordinator. Commands run on the wallet's serialized work executor; UI foreground
 * changes are safe from another thread. A pause never discards a proof or an unresolved payment.
 * No engine or transport implementation is instantiated by this source candidate's application.
 */
internal class PrivatePaymentCoordinator internal constructor(
    private val engine: PrivateLocalEngine,
    private val journal: PrivateOperationJournal,
    private val transport: PrivateSubmissionTransport,
    private val nowEpochSeconds: () -> Long
) {
    private enum class Work { PREPARE, PROVE, SUBMIT, RECONCILE, RESTORE, CANCEL }
    private val lock = Any()
    private var foreground = true
    private var authorizationRevision = 0L
    private var work: Work? = null
    private var review: PrivatePaymentReview? = null
    private var entry: PrivateJournalEntry? = null
    // A failed durable write may have committed. Only re-reading this identity may clear it.
    private var uncertainStorageIntentId: String? = null
    private var uncertainStorageWrite = false
    private var state = PrivatePaymentState(PrivatePaymentStage.EMPTY, null, null, null)

    fun snapshot(): PrivatePaymentState = synchronized(lock) { state }
    fun authorizationStamp(): Long = synchronized(lock) { requireForeground(); authorizationRevision }
    fun authorizationCurrent(stamp: Long): Boolean = synchronized(lock) { foreground && authorizationRevision == stamp }

    fun setForeground(value: Boolean) = synchronized(lock) {
        if (!value) authorizationRevision++
        foreground = value
    }

    fun currentReview(): PrivatePaymentReview? = synchronized(lock) {
        review?.takeIf { foreground && work == null && state.stage == PrivatePaymentStage.REVIEW }
    }

    fun prepare(draft: PrivateTransferDraft): PrivatePaymentState {
        var replacedReview: PrivatePaymentReview? = null
        var preparedReview: PrivatePaymentReview? = null
        synchronized(lock) {
            requireForeground()
            requireIdle()
            requireKnownStorage()
            if (entry?.phase !in listOf(null, PrivateJournalPhase.FINALIZED, PrivateJournalPhase.CANCELLED, PrivateJournalPhase.SUPERSEDED)) {
                throw PrivatePaymentFailure(PrivateFailure.BUSY)
            }
            work = Work.PREPARE
            replacedReview = review
            review = null
            entry = null
            state = PrivatePaymentState(PrivatePaymentStage.PREPARING, draft.intentId, null, null)
        }
        try {
            replacedReview?.let(engine::discardLocalReview)
            val prepared = engine.prepareLocal(draft).also { preparedReview = it }
            if (prepared.draft !== draft || prepared.intentId != draft.intentId) {
                throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
            }
            requireFresh(prepared.quoteValidUntilEpochSeconds)
            synchronized(lock) {
                review = prepared
                state = PrivatePaymentState(PrivatePaymentStage.REVIEW, draft.intentId, null, null)
            }
        } catch (error: Exception) {
            preparedReview?.let { runCatching { engine.discardLocalReview(it) } }
            fail(error, PrivateFailure.LOCAL_ENGINE)
        } finally { synchronized(lock) { work = null } }
        return snapshot()
    }

    /** A reviewed recovery adapter may supply a prepared exact self-transfer for the same gate. */
    internal fun reviewPrepared(prepared: PrivatePaymentReview): PrivatePaymentState = synchronized(lock) {
        requireForeground(); requireIdle(); requireKnownStorage()
        if (entry?.phase !in listOf(null, PrivateJournalPhase.FINALIZED, PrivateJournalPhase.CANCELLED, PrivateJournalPhase.SUPERSEDED)) {
            throw PrivatePaymentFailure(PrivateFailure.BUSY)
        }
        requireFresh(prepared.quoteValidUntilEpochSeconds)
        review = prepared; entry = null
        state = PrivatePaymentState(PrivatePaymentStage.REVIEW, prepared.intentId, null, null)
        state
    }

    /** User approval is explicit. The resulting proof is saved locally and is not sent here. */
    fun approveAndProve(approved: PrivatePaymentReview): PrivatePaymentState {
        synchronized(lock) {
            requireForeground()
            requireIdle()
            requireKnownStorage()
            if (review !== approved || state.stage != PrivatePaymentStage.REVIEW) {
                throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
            }
            requireFresh(approved.quoteValidUntilEpochSeconds)
            work = Work.PROVE
            state = PrivatePaymentState(PrivatePaymentStage.PROVING, approved.intentId, 0, null)
        }
        try {
            val stored = mutation(approved.intentId) { journal.recordApproved(approved) }
            checkIdentity(approved.intentId, stored)
            synchronized(lock) { entry = stored }
            val proved = engine.proveLocal(approved) { percent ->
                synchronized(lock) {
                    if (work == Work.PROVE && state.intentId == approved.intentId) {
                        state = state.copy(provingProgress = percent.coerceIn(0, 100))
                    }
                }
            }
            val ready = mutation(approved.intentId) { journal.recordReady(approved.intentId, proved) }
            checkIdentity(approved.intentId, ready)
            if (ready.phase != PrivateJournalPhase.READY || ready.releaseCount != 0L) {
                throw PrivatePaymentFailure(PrivateFailure.STORAGE)
            }
            accept(ready)
            if (!fresh(ready.quoteValidUntilEpochSeconds)) synchronized(lock) {
                state = state.copy(failure = PrivateFailure.QUOTE_EXPIRED)
            }
        } catch (error: Exception) {
            fail(error, PrivateFailure.LOCAL_ENGINE)
        } finally { synchronized(lock) { work = null } }
        return snapshot()
    }

    /**
     * One outbound attempt, always using the durable envelope. Repeated calls can only resend that
     * exact operation. A quote expiring after release never establishes that it cannot execute.
     */
    fun submitExisting(): PrivatePaymentState {
        var approvedRevision = -1L
        val saved = synchronized(lock) {
            requireForeground()
            requireIdle()
            requireKnownStorage()
            val current = entry ?: throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
            if (current.phase !in setOf(PrivateJournalPhase.READY, PrivateJournalPhase.RELEASED, PrivateJournalPhase.UNKNOWN, PrivateJournalPhase.FAILED)) {
                throw PrivatePaymentFailure(PrivateFailure.CANCELLATION_UNAVAILABLE)
            }
            if (current.releaseCount == 0L && !current.releaseHistoryIncomplete) requireFresh(current.quoteValidUntilEpochSeconds)
            work = Work.SUBMIT
            approvedRevision = authorizationRevision
            state = PrivatePaymentState(PrivatePaymentStage.SUBMITTING, current.intentId, null, null)
            current
        }
        try {
            if (saved.phase == PrivateJournalPhase.FAILED) {
                // A reverted outer attempt is not a cancelled private proof. An explicit retry
                // may use only this exact proof, after fresh canonical finalized-revert evidence.
                val observation = engine.observeCanonical(saved)
                val checked = when (observation) {
                    is PrivateChainObservation.Receipt -> mutation(saved.intentId) { journal.observeReceipt(saved.intentId, observation) }
                    is PrivateChainObservation.Reorg -> {
                        mutation(saved.intentId) { journal.markReorg(saved.intentId, observation) }.also(::accept)
                        throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
                    }
                    null -> throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
                }
                requireSameOperation(saved, checked); accept(checked)
                if (checked.phase in setOf(PrivateJournalPhase.INCLUDED, PrivateJournalPhase.FINALIZED)) return snapshot()
                if (checked.phase != PrivateJournalPhase.FAILED || !checked.finalizedRevert) throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
            }
            engine.verifyForSubmission(saved)
            if (!synchronized(lock) { foreground && authorizationRevision == approvedRevision }) throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED)
            val released = mutation(saved.intentId) { journal.markReleased(saved.intentId) }
            requireSameOperation(saved, released)
            accept(released)
            // This state commits before any transport call, including a remote simulation.
            val unresolved = mutation(saved.intentId) { journal.markUnknown(saved.intentId) }
            requireSameOperation(saved, unresolved)
            accept(unresolved)
            if (!synchronized(lock) { foreground && authorizationRevision == approvedRevision }) return snapshot()
            val acknowledgement = try { transport.submit(unresolved.releasedOperation()) }
                catch (_: Exception) {
                    synchronized(lock) { state = state.copy(failure = PrivateFailure.UNKNOWN_OUTCOME) }
                    return snapshot()
                }
            val latest = storage { journal.get(saved.intentId) }
            requireSameOperation(saved, latest)
            accept(latest)
            if (latest.phase in setOf(PrivateJournalPhase.RELEASED, PrivateJournalPhase.UNKNOWN)) {
                synchronized(lock) {
                    state = when {
                        acknowledgement.operationDigest != saved.operationDigest -> state.copy(failure = PrivateFailure.INVALID_ACKNOWLEDGEMENT)
                        acknowledgement.accepted -> state.copy(stage = PrivatePaymentStage.AWAITING_CONFIRMATION, failure = null)
                        else -> state.copy(failure = PrivateFailure.UNKNOWN_OUTCOME)
                    }
                }
            }
        } catch (error: Exception) {
            fail(error, PrivateFailure.STORAGE)
        } finally { synchronized(lock) { work = null } }
        return snapshot()
    }

    /** A service acknowledgement is never supplied to this method as chain evidence. */
    fun reconcile(): PrivatePaymentState {
        val saved = synchronized(lock) {
            requireIdle()
            requireKnownStorage()
            val current = entry ?: throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
            if (current.operationDigest == null) throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
            work = Work.RECONCILE
            current
        }
        try {
            val observed = when (val observation = engine.observeCanonical(saved)) {
                null -> storage { journal.get(saved.intentId) }
                is PrivateChainObservation.Receipt -> mutation(saved.intentId) { journal.observeReceipt(saved.intentId, observation) }
                is PrivateChainObservation.Reorg -> mutation(saved.intentId) { journal.markReorg(saved.intentId, observation) }
            }
            requireSameOperation(saved, observed)
            accept(observed)
        } catch (error: Exception) {
            // The last durable entry is retained. No retry, cancellation or fresh spend follows.
            fail(error, PrivateFailure.RECONCILIATION)
        } finally { synchronized(lock) { work = null } }
        return snapshot()
    }

    /** Process-death recovery reads only the local encrypted journal and never auto-submits. */
    fun restore(intentId: String): PrivatePaymentState {
        synchronized(lock) {
            requireIdle()
            if (!Regex("0x[0-9a-f]{64}").matches(intentId) || intentId.drop(2).all { it == '0' }) {
                throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
            }
            if (uncertainStorageIntentId != null && uncertainStorageIntentId != intentId) {
                throw PrivatePaymentFailure(PrivateFailure.STORAGE)
            }
            // Selecting another durable history item changes only this UI cursor. The earlier
            // authorization and its reservations remain in the encrypted journal and SDK cache.
            work = Work.RESTORE
            review = null
            if (entry?.intentId != intentId) entry = null
            // Until this requested identity is read successfully, a missing/unreadable pending
            // record must not be treated as absence of a transfer or permission for a new spend.
            uncertainStorageIntentId = intentId
            state = PrivatePaymentState(PrivatePaymentStage.APPROVED_UNFINISHED, intentId, null, null)
        }
        try {
            val stored = storage { journal.get(intentId) }
            checkIdentity(intentId, stored)
            accept(stored)
            synchronized(lock) { uncertainStorageIntentId = null; uncertainStorageWrite = false }
        } catch (error: Exception) {
            fail(error, PrivateFailure.STORAGE)
        } finally { synchronized(lock) { work = null } }
        return snapshot()
    }

    /** Local abandonment only. A released proof cannot be cancelled by clearing this UI. */
    fun cancelUnreleased(): PrivatePaymentState {
        val saved = synchronized(lock) {
            requireForeground()
            requireIdle()
            requireKnownStorage()
            val current = entry
            if (current != null && (current.releaseHistoryIncomplete || current.envelopeBackedUp || current.releaseCount > 0 || current.phase !in setOf(PrivateJournalPhase.APPROVED, PrivateJournalPhase.READY))) {
                throw PrivatePaymentFailure(PrivateFailure.CANCELLATION_UNAVAILABLE)
            }
            work = Work.CANCEL
            current
        }
        try {
            if (saved == null) {
                synchronized(lock) { review }?.let(engine::discardLocalReview)
                synchronized(lock) {
                    review = null
                    state = PrivatePaymentState(PrivatePaymentStage.CANCELLED, state.intentId, null, null)
                }
            } else {
                val cancelled = mutation(saved.intentId) { journal.cancelUnreleased(saved.intentId) }
                checkIdentity(saved.intentId, cancelled)
                if (cancelled.phase != PrivateJournalPhase.CANCELLED || cancelled.releaseCount != 0L || cancelled.releaseHistoryIncomplete || cancelled.envelopeBackedUp) {
                    throw PrivatePaymentFailure(PrivateFailure.STORAGE)
                }
                accept(cancelled)
                synchronized(lock) { review }?.let(engine::discardLocalReview)
                synchronized(lock) { review = null }
            }
        } catch (error: Exception) {
            fail(error, PrivateFailure.STORAGE)
        } finally { synchronized(lock) { work = null } }
        return snapshot()
    }

    private fun requireForeground() {
        if (!foreground) throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED)
    }

    private fun requireIdle() {
        if (work != null) throw PrivatePaymentFailure(PrivateFailure.BUSY)
    }

    private fun requireKnownStorage() {
        if (uncertainStorageIntentId != null) throw PrivatePaymentFailure(PrivateFailure.STORAGE)
    }

    private fun fresh(validUntil: Long): Boolean {
        val now = nowEpochSeconds()
        return now > 0 && validUntil > now
    }

    private fun requireFresh(validUntil: Long) {
        if (!fresh(validUntil)) throw PrivatePaymentFailure(PrivateFailure.QUOTE_EXPIRED)
    }

    private fun checkIdentity(expected: String, actual: PrivateJournalEntry) {
        if (actual.intentId != expected) throw PrivatePaymentFailure(PrivateFailure.STORAGE)
    }

    private fun requireSameOperation(expected: PrivateJournalEntry, actual: PrivateJournalEntry) {
        if (!expected.sameOperation(actual)) throw PrivatePaymentFailure(PrivateFailure.STORAGE)
    }

    private fun accept(saved: PrivateJournalEntry) = synchronized(lock) {
        entry = saved
        state = PrivatePaymentState(displayStage(saved), saved.intentId, null, null)
    }

    private fun displayStage(saved: PrivateJournalEntry): PrivatePaymentStage =
        if (saved.releaseHistoryIncomplete && saved.phase in setOf(PrivateJournalPhase.APPROVED, PrivateJournalPhase.READY)) PrivatePaymentStage.UNKNOWN
        else stageFor(saved.phase)

    private fun stageFor(phase: PrivateJournalPhase): PrivatePaymentStage = when (phase) {
            PrivateJournalPhase.APPROVED -> PrivatePaymentStage.APPROVED_UNFINISHED
            PrivateJournalPhase.READY -> PrivatePaymentStage.READY
            PrivateJournalPhase.RELEASED, PrivateJournalPhase.UNKNOWN, PrivateJournalPhase.REORG -> PrivatePaymentStage.UNKNOWN
            PrivateJournalPhase.INCLUDED -> PrivatePaymentStage.INCLUDED
            PrivateJournalPhase.FINALIZED -> PrivatePaymentStage.FINALIZED
            PrivateJournalPhase.FAILED -> PrivatePaymentStage.FAILED
            PrivateJournalPhase.CONFLICT -> PrivatePaymentStage.CONFLICT
            PrivateJournalPhase.CANCELLED -> PrivatePaymentStage.CANCELLED
            PrivateJournalPhase.SUPERSEDED -> PrivatePaymentStage.SUPERSEDED
    }

    private fun fail(error: Exception, fallback: PrivateFailure) = synchronized(lock) {
        val reason = (error as? PrivatePaymentFailure)?.failure ?: fallback
        // A read failure cannot invent a reorg or revoke observed finality. A failed write may
        // have committed and must be re-read before any new operation or local cancellation.
        val stage = if (uncertainStorageIntentId != null && (uncertainStorageWrite || entry == null)) PrivatePaymentStage.UNKNOWN
            else entry?.let(::displayStage) ?: PrivatePaymentStage.ERROR
        state = PrivatePaymentState(stage, state.intentId, null, reason)
    }

    private inline fun mutation(intentId: String, block: () -> PrivateJournalEntry): PrivateJournalEntry {
        synchronized(lock) { uncertainStorageIntentId = intentId; uncertainStorageWrite = true }
        val saved = storage(block)
        checkIdentity(intentId, saved)
        synchronized(lock) { uncertainStorageIntentId = null; uncertainStorageWrite = false }
        return saved
    }

    private inline fun <T> storage(block: () -> T): T = try { block() }
        catch (error: PrivatePaymentFailure) { throw error }
        catch (_: Exception) { throw PrivatePaymentFailure(PrivateFailure.STORAGE) }
}

/** The application must use this factory. Tests exercise the coordinator with explicit fixtures. */
internal object PrivatePaymentFeature {
    fun create(
        engine: PrivateLocalEngine,
        journal: PrivateOperationJournal,
        transport: PrivateSubmissionTransport,
        nowEpochSeconds: () -> Long
    ): PrivatePaymentCoordinator {
        PrivatePaymentRelease.requireAvailable()
        return PrivatePaymentCoordinator(engine, journal, transport, nowEpochSeconds)
    }
}
