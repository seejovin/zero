package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test
import java.math.BigInteger

/**
 * Explicit in-memory fixtures exercise orchestration failures; fixture JSON is not a valid proof
 * or native envelope. These tests must never be described as an end-to-end private-payment test.
 */
class PrivatePaymentCoordinatorTest {
    private class Harness {
        val events = mutableListOf<String>()
        var now = 100L
        val asset = PrivatePaymentAsset(42161, "0xaf88d065e77c8cc2239327c5edb3a432268e5831", "USDC", 6)
        val digest = "0x" + "a".repeat(64)
        val envelope = "{\"fixture\":\"not-a-proof\"}"
        val journal = MemoryJournal(events, digest)
        var preparations = 0
        var proofs = 0
        var duringProof: (() -> Unit)? = null
        var duringPreparation: (() -> Unit)? = null
        var duringVerification: (() -> Unit)? = null
        var failVerification = false
        var observeFailure = false
        var observation: PrivateChainObservation? = null
        val engine = object : PrivateLocalEngine {
            override fun prepareLocal(draft: PrivateTransferDraft): PrivatePaymentReview {
                preparations++
                events.add("prepare-local")
                duringPreparation?.invoke()
                val plan = PrivateSpendPlan(listOf(BigInteger("1000003")), BigInteger("2"), BigInteger.ZERO, true, true, false)
                return PrivatePaymentReview(draft, plan, draft.recipientUnits ?: plan.maximumUnits(), 200L, "{\"fixture\":\"local-intent\"}")
            }
            override fun proveLocal(review: PrivatePaymentReview, progress: (Int) -> Unit): PrivateProvedOperation {
                proofs++
                events.add("prove-local")
                progress(25)
                duringProof?.invoke()
                progress(100)
                return PrivateProvedOperation(envelope)
            }
            override fun observeCanonical(entry: PrivateJournalEntry): PrivateChainObservation? {
                events.add("observe")
                if (observeFailure) throw IllegalStateException("fixture provider secret must be redacted")
                return observation
            }
            override fun discardLocalReview(review: PrivatePaymentReview) { events.add("discard-local") }
            override fun verifyForSubmission(entry: PrivateJournalEntry) {
                events.add("verify-local")
                duringVerification?.invoke()
                if (failVerification) throw PrivatePaymentFailure(PrivateFailure.PROOF_INVALID)
            }
        }
        val submitted = mutableListOf<String>()
        var failSubmission = false
        var ackDigest = digest
        val transport = object : PrivateSubmissionTransport {
            override fun submit(operation: PrivateReleasedOperation): PrivateSubmissionAcknowledgement {
                events.add("transport")
                assertEquals(PrivateJournalPhase.UNKNOWN, journal.saved!!.phase)
                assertTrue(journal.saved!!.releaseCount > 0)
                submitted.add(String(operation.transportBytes(), Charsets.UTF_8))
                if (failSubmission) throw IllegalStateException("fixture timeout after possible delivery")
                return PrivateSubmissionAcknowledgement(ackDigest, true)
            }
        }
        val coordinator = PrivatePaymentCoordinator(engine, journal, transport) { now }
        fun draft() = PrivateTransferDraft.create(asset, "0zkfixture-recipient", "0.000001")
        fun review(): PrivatePaymentReview {
            assertEquals(PrivatePaymentStage.REVIEW, coordinator.prepare(draft()).stage)
            return coordinator.currentReview()!!
        }
        fun ready() {
            assertEquals(PrivatePaymentStage.READY, coordinator.approveAndProve(review()).stage)
        }
        fun restored() = PrivatePaymentCoordinator(engine, journal, transport) { now }
    }

    private class MemoryJournal(private val events: MutableList<String>, private val digest: String) : PrivateOperationJournal {
        var saved: PrivateJournalEntry? = null
        var failAfterWrite: String? = null
        var failRead = false
        var receiptPhase = PrivateJournalPhase.FINALIZED
        var receiptFinalizedRevert = false
        var afterWrite: ((String) -> Unit)? = null

        private fun write(method: String, next: PrivateJournalEntry): PrivateJournalEntry {
            events.add(method)
            saved = next
            afterWrite?.invoke(method)
            if (failAfterWrite == method) throw IllegalStateException("fixture committed write followed by read failure")
            return next
        }
        private fun next(phase: PrivateJournalPhase, releases: Long = saved!!.releaseCount) = saved!!.let {
            PrivateJournalEntry(it.intentId, phase, releases, it.quoteValidUntilEpochSeconds, it.operationDigest, it.envelopeForLocalEngine(),
                releaseHistoryIncomplete = it.releaseHistoryIncomplete, finalizedRevert = phase == PrivateJournalPhase.FAILED && receiptFinalizedRevert,
                envelopeBackedUp = it.envelopeBackedUp)
        }
        override fun recordApproved(review: PrivatePaymentReview): PrivateJournalEntry = write("approved",
            PrivateJournalEntry(review.intentId, PrivateJournalPhase.APPROVED, 0, review.quoteValidUntilEpochSeconds, null, null))
        override fun recordReady(intentId: String, operation: PrivateProvedOperation): PrivateJournalEntry = write("ready",
            PrivateJournalEntry(intentId, PrivateJournalPhase.READY, 0, saved!!.quoteValidUntilEpochSeconds, digest, operation.localEnvelopeForJournal()))
        override fun markReleased(intentId: String): PrivateJournalEntry = write("released", next(PrivateJournalPhase.RELEASED, saved!!.releaseCount + 1))
        override fun markUnknown(intentId: String): PrivateJournalEntry = write("unknown", next(PrivateJournalPhase.UNKNOWN))
        override fun observeReceipt(intentId: String, observation: PrivateChainObservation.Receipt): PrivateJournalEntry = write("receipt", next(receiptPhase))
        override fun markReorg(intentId: String, observation: PrivateChainObservation.Reorg): PrivateJournalEntry = write("reorg", next(PrivateJournalPhase.REORG))
        override fun cancelUnreleased(intentId: String): PrivateJournalEntry {
            check(saved!!.releaseCount == 0L && saved!!.phase in setOf(PrivateJournalPhase.APPROVED, PrivateJournalPhase.READY))
            return write("cancelled", next(PrivateJournalPhase.CANCELLED))
        }
        override fun get(intentId: String): PrivateJournalEntry {
            events.add("get")
            if (failRead) throw IllegalStateException("fixture read failure")
            check(saved!!.intentId == intentId)
            return saved!!
        }
    }

    @Test fun factoryRefusesBeforePreparingProvingSavingOrSending() {
        val h = Harness()
        assertThrows(PrivatePaymentFailure::class.java) { PrivatePaymentFeature.create(h.engine, h.journal, h.transport) { h.now } }
        assertTrue(h.events.isEmpty())
    }

    @Test fun approvalAndProvingAreLocalAndBothReleaseMarkersPrecedeTransport() {
        val h = Harness()
        h.ready()
        assertEquals(listOf("prepare-local", "approved", "prove-local", "ready"), h.events)
        assertTrue(h.submitted.isEmpty())
        val state = h.coordinator.submitExisting()
        assertEquals(PrivatePaymentStage.AWAITING_CONFIRMATION, state.stage)
        assertEquals(listOf("verify-local", "released", "unknown", "transport", "get"), h.events.takeLast(5))
        assertEquals(PrivateJournalPhase.UNKNOWN, h.journal.saved!!.phase)
        assertEquals(listOf(h.envelope), h.submitted)
        assertFalse(h.submitted.single().contains("0zkfixture-recipient"))
    }

    @Test fun anOldReviewCannotAuthorizeAChangedDraft() {
        val h = Harness()
        val first = h.review()
        val second = h.review()
        assertNotEquals(first.intentId, second.intentId)
        assertEquals(PrivateFailure.STALE_REVIEW,
            assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.approveAndProve(first) }.failure)
        assertEquals(0, h.proofs)
        assertNull(h.journal.saved)
    }

    @Test fun backgroundHidesReviewAndProvingCompletionNeverSendsAutomatically() {
        val h = Harness()
        val approved = h.review()
        h.coordinator.setForeground(false)
        assertNull(h.coordinator.currentReview())
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.approveAndProve(approved) }
        h.coordinator.setForeground(true)
        assertSame(approved, h.coordinator.currentReview())
        h.duringProof = { h.coordinator.setForeground(false) }
        assertEquals(PrivatePaymentStage.READY, h.coordinator.approveAndProve(approved).stage)
        assertTrue(h.submitted.isEmpty())
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.submitExisting() }
        h.coordinator.setForeground(true)
        h.coordinator.submitExisting()
        assertEquals(1, h.proofs)
        assertEquals(1, h.submitted.size)
    }

    @Test fun overlappingCommandsAreRejectedWhileLocalProofRuns() {
        val h = Harness()
        h.duringProof = {
            assertEquals(PrivateFailure.BUSY, assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.prepare(h.draft()) }.failure)
            assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.cancelUnreleased() }
        }
        h.ready()
        assertEquals(1, h.proofs)
    }

    @Test fun timeoutRetainsUnknownAndExplicitRetryUsesOnlyTheExactDurableOperation() {
        val h = Harness()
        h.ready()
        h.failSubmission = true
        assertEquals(PrivateFailure.UNKNOWN_OUTCOME, h.coordinator.submitExisting().failure)
        assertEquals(PrivatePaymentStage.UNKNOWN, h.coordinator.snapshot().stage)
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.prepare(h.draft()) }
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.cancelUnreleased() }
        h.now = 500L // Expiry cannot revoke a proof already released.
        h.coordinator.submitExisting()
        assertEquals(listOf(h.envelope, h.envelope), h.submitted)
        assertEquals(1, h.proofs)
        assertEquals(1, h.preparations)
    }

    @Test fun processRestartRestoresUnknownWithoutSubmittingOrProving() {
        val h = Harness()
        h.ready()
        h.failSubmission = true
        h.coordinator.submitExisting()
        val restarted = h.restored()
        assertEquals(PrivatePaymentStage.UNKNOWN, restarted.restore(h.journal.saved!!.intentId).stage)
        assertEquals(1, h.submitted.size)
        assertEquals(1, h.proofs)
        restarted.submitExisting()
        assertEquals(listOf(h.envelope, h.envelope), h.submitted)
    }

    @Test fun committedReleaseFollowedByStorageFailureCannotSendOrCancelUntilRestored() {
        val h = Harness()
        h.ready()
        h.journal.failAfterWrite = "released"
        assertEquals(PrivateFailure.STORAGE, h.coordinator.submitExisting().failure)
        assertEquals(PrivatePaymentStage.UNKNOWN, h.coordinator.snapshot().stage)
        assertTrue(h.submitted.isEmpty())
        assertEquals(PrivateJournalPhase.RELEASED, h.journal.saved!!.phase)
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.submitExisting() }
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.cancelUnreleased() }
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.prepare(h.draft()) }
        h.journal.failAfterWrite = null
        h.coordinator.restore(h.journal.saved!!.intentId)
        h.coordinator.submitExisting()
        assertEquals(1, h.submitted.size)
    }

    @Test fun approvalWriteUncertaintyKeepsTheIdentityReservedUntilReadBack() {
        val h = Harness()
        val approved = h.review()
        h.journal.failAfterWrite = "approved"
        assertEquals(PrivateFailure.STORAGE, h.coordinator.approveAndProve(approved).failure)
        assertEquals(0, h.proofs)
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.prepare(h.draft()) }
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.cancelUnreleased() }
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.restore(h.draft().intentId) }
        h.journal.failAfterWrite = null
        assertEquals(PrivatePaymentStage.APPROVED_UNFINISHED, h.coordinator.restore(approved.intentId).stage)
        assertEquals(PrivatePaymentStage.CANCELLED, h.coordinator.cancelUnreleased().stage)
    }

    @Test fun expiredUnreleasedProofCanBeLocallyAbandonedButNeverSubmitted() {
        val h = Harness()
        h.ready()
        h.now = 200L
        assertEquals(PrivateFailure.QUOTE_EXPIRED, assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.submitExisting() }.failure)
        assertTrue(h.submitted.isEmpty())
        assertEquals(PrivatePaymentStage.CANCELLED, h.coordinator.cancelUnreleased().stage)
        assertEquals(0L, h.journal.saved!!.releaseCount)
    }

    @Test fun aMismatchedServiceAcknowledgementCannotShowSuccess() {
        val h = Harness()
        h.ready()
        h.ackDigest = "0x" + "b".repeat(64)
        val result = h.coordinator.submitExisting()
        assertEquals(PrivatePaymentStage.UNKNOWN, result.stage)
        assertEquals(PrivateFailure.INVALID_ACKNOWLEDGEMENT, result.failure)
        assertEquals(PrivateJournalPhase.UNKNOWN, h.journal.saved!!.phase)
    }

    @Test fun canonicalReceiptIsRequiredForFinalityAndReadFailuresDoNotInventAReorg() {
        val h = Harness()
        h.ready()
        h.coordinator.submitExisting()
        assertEquals(PrivatePaymentStage.UNKNOWN, h.coordinator.reconcile().stage)
        h.observation = PrivateChainObservation.Receipt("fixture canonical observation, not raw RPC")
        assertEquals(PrivatePaymentStage.FINALIZED, h.coordinator.reconcile().stage)
        h.observeFailure = true
        val afterFailure = h.coordinator.reconcile()
        assertEquals(PrivatePaymentStage.FINALIZED, afterFailure.stage)
        assertEquals(PrivateFailure.RECONCILIATION, afterFailure.failure)
        assertFalse(afterFailure.toString().contains("fixture provider secret"))
        h.journal.failRead = true
        assertEquals(PrivatePaymentStage.FINALIZED, h.coordinator.restore(h.journal.saved!!.intentId).stage)
    }

    @Test fun reorgReturnsToUnknownAndNeverMakesPreviouslyReleasedNotesSpendable() {
        val h = Harness()
        h.ready()
        h.coordinator.submitExisting()
        h.journal.receiptPhase = PrivateJournalPhase.INCLUDED
        h.observation = PrivateChainObservation.Receipt("fixture receipt")
        assertEquals(PrivatePaymentStage.INCLUDED, h.coordinator.reconcile().stage)
        h.observation = PrivateChainObservation.Reorg("fixture reorg observation")
        assertEquals(PrivatePaymentStage.UNKNOWN, h.coordinator.reconcile().stage)
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.prepare(h.draft()) }
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.cancelUnreleased() }
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.submitExisting() }
    }

    @Test fun preparingAnExpiredQuoteCannotLeaveAnApprovableReview() {
        val h = Harness()
        h.duringPreparation = { h.now = 200 }
        assertEquals(PrivateFailure.QUOTE_EXPIRED, h.coordinator.prepare(h.draft()).failure)
        assertNull(h.coordinator.currentReview())
        assertNull(h.journal.saved)
        assertTrue(h.submitted.isEmpty())
    }

    @Test fun failedRestorationCannotCopyAnOlderPaymentsFinalityOntoANewIdentity() {
        val h = Harness()
        h.ready()
        h.coordinator.submitExisting()
        h.observation = PrivateChainObservation.Receipt("fixture receipt")
        assertEquals(PrivatePaymentStage.FINALIZED, h.coordinator.reconcile().stage)
        val anotherId = h.draft().intentId
        h.journal.failRead = true
        val result = h.coordinator.restore(anotherId)
        assertEquals(anotherId, result.intentId)
        assertEquals(PrivatePaymentStage.UNKNOWN, result.stage)
        assertEquals(PrivateFailure.STORAGE, result.failure)
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.prepare(h.draft()) }
    }

    @Test fun failedLocalProofVerificationPreventsReleaseAndTransport() {
        val h = Harness(); h.ready(); h.failVerification = true
        assertEquals(PrivateFailure.PROOF_INVALID, h.coordinator.submitExisting().failure)
        assertEquals(0L, h.journal.saved!!.releaseCount)
        assertEquals(PrivateJournalPhase.READY, h.journal.saved!!.phase)
        assertTrue(h.submitted.isEmpty())
    }

    @Test fun pausingAndResumingDuringVerificationInvalidatesTheOriginalSendApproval() {
        val h = Harness(); h.ready()
        h.duringVerification = { h.coordinator.setForeground(false); h.coordinator.setForeground(true) }
        assertEquals(PrivateFailure.FOREGROUND_REQUIRED, h.coordinator.submitExisting().failure)
        assertEquals(0L, h.journal.saved!!.releaseCount)
        assertTrue(h.submitted.isEmpty())
    }

    @Test fun pauseAfterDurableReleaseKeepsUnknownButDoesNotTransmitOnAutomaticResume() {
        val h = Harness(); h.ready()
        h.journal.afterWrite = { method ->
            if (method == "unknown") { h.coordinator.setForeground(false); h.coordinator.setForeground(true) }
        }
        assertEquals(PrivatePaymentStage.UNKNOWN, h.coordinator.submitExisting().stage)
        assertEquals(1L, h.journal.saved!!.releaseCount)
        assertTrue(h.submitted.isEmpty())
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.cancelUnreleased() }
    }

    @Test fun aRestoredUnminedProofNeverBecomesCancellableBecauseBackupReleaseCountWasZero() {
        val h = Harness(); h.ready()
        val previous = h.journal.saved!!
        h.journal.saved = PrivateJournalEntry(previous.intentId, PrivateJournalPhase.READY, 0, previous.quoteValidUntilEpochSeconds,
            previous.operationDigest, previous.envelopeForLocalEngine(), releaseHistoryIncomplete = true)
        val restored = h.restored()
        assertEquals(PrivatePaymentStage.UNKNOWN, restored.restore(previous.intentId).stage)
        assertThrows(PrivatePaymentFailure::class.java) { restored.cancelUnreleased() }
        assertThrows(PrivatePaymentFailure::class.java) { restored.prepare(h.draft()) }
        assertTrue(h.submitted.isEmpty())
    }

    @Test fun restoredApprovalWithoutEnvelopeCannotSilentlyGenerateAnotherProof() {
        val h = Harness(); val review = h.review()
        h.journal.saved = PrivateJournalEntry(review.intentId, PrivateJournalPhase.APPROVED, 0, review.quoteValidUntilEpochSeconds,
            null, null, releaseHistoryIncomplete = true)
        val restored = h.restored()
        assertEquals(PrivatePaymentStage.UNKNOWN, restored.restore(review.intentId).stage)
        assertNull(restored.currentReview())
        assertThrows(PrivatePaymentFailure::class.java) { restored.approveAndProve(review) }
        assertThrows(PrivatePaymentFailure::class.java) { restored.cancelUnreleased() }
        assertEquals(0, h.proofs)
    }

    @Test fun aRevertedOuterAttemptRequiresFreshFinalizedEvidenceBeforeExactRetry() {
        val h = Harness(); h.ready(); h.coordinator.submitExisting()
        h.journal.receiptPhase = PrivateJournalPhase.FAILED
        h.observation = PrivateChainObservation.Receipt("fixture canonical revert")
        assertEquals(PrivatePaymentStage.FAILED, h.coordinator.reconcile().stage)
        assertEquals(PrivateFailure.RECONCILIATION, h.coordinator.submitExisting().failure)
        assertEquals(1, h.submitted.size)
        h.journal.receiptFinalizedRevert = true
        assertEquals(PrivatePaymentStage.AWAITING_CONFIRMATION, h.coordinator.submitExisting().stage)
        assertEquals(listOf(h.envelope, h.envelope), h.submitted)
        assertEquals(1, h.proofs)
        assertEquals(1, h.preparations)
    }

    @Test fun unknownRevertObservationCannotAuthorizeAnotherOuterAttempt() {
        val h = Harness(); h.ready(); h.coordinator.submitExisting()
        h.journal.receiptPhase = PrivateJournalPhase.FAILED
        h.journal.receiptFinalizedRevert = true
        h.observation = PrivateChainObservation.Receipt("fixture canonical revert")
        h.coordinator.reconcile()
        h.observation = null
        assertEquals(PrivateFailure.RECONCILIATION, h.coordinator.submitExisting().failure)
        assertEquals(1, h.submitted.size)
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.cancelUnreleased() }
    }

    @Test fun discardOfAReviewClearsOnlyLocalPreparationAndNeverSubmits() {
        val h = Harness(); h.review()
        assertEquals(PrivatePaymentStage.CANCELLED, h.coordinator.cancelUnreleased().stage)
        assertTrue(h.events.contains("discard-local"))
        assertNull(h.journal.saved)
        assertTrue(h.submitted.isEmpty())
    }


    @Test fun exportedReadyEnvelopeCannotBeCancelledOnItsOriginalDevice() {
        val h = Harness(); h.ready()
        val saved = h.journal.saved!!
        h.journal.saved = PrivateJournalEntry(saved.intentId, saved.phase, saved.releaseCount, saved.quoteValidUntilEpochSeconds,
            saved.operationDigest, saved.envelopeForLocalEngine(), envelopeBackedUp = true)
        h.coordinator.restore(saved.intentId)
        assertThrows(PrivatePaymentFailure::class.java) { h.coordinator.cancelUnreleased() }
        assertEquals(PrivatePaymentStage.AWAITING_CONFIRMATION, h.coordinator.submitExisting().stage)
        assertEquals(listOf(h.envelope), h.submitted)
    }

    @Test fun freshSuccessDuringRevertRetryStopsWithoutAnotherBroadcast() {
        val h = Harness(); h.ready(); h.coordinator.submitExisting()
        h.journal.receiptPhase = PrivateJournalPhase.FAILED
        h.journal.receiptFinalizedRevert = true
        h.observation = PrivateChainObservation.Receipt("fixture old finalized revert")
        h.coordinator.reconcile()
        h.journal.receiptPhase = PrivateJournalPhase.FINALIZED
        h.observation = PrivateChainObservation.Receipt("fixture successful exact-envelope retry")
        assertEquals(PrivatePaymentStage.FINALIZED, h.coordinator.submitExisting().stage)
        assertEquals(1, h.submitted.size)
        assertEquals(1, h.proofs)
    }

}
