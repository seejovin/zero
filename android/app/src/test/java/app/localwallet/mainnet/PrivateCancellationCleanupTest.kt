package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test
import java.math.BigInteger

/** Local replay fixtures. These do not enable the engine, private networking or release gates. */
class PrivateCancellationCleanupTest {
    private fun hash(value: Int) = "0x" + value.toString(16).padStart(64, '0')
    private fun payment(
        id: Int,
        phase: PrivateJournalPhase,
        notes: List<Int>,
        tree: Int = 0,
        sequence: Long = id.toLong(),
        released: Long = 0,
        backedUp: Boolean = false,
        imported: Boolean = false,
        recovery: Boolean = false,
        receipt: Boolean = false
    ): PrivateLocalPayment {
        val hasEnvelope = phase !in setOf(PrivateJournalPhase.APPROVED, PrivateJournalPhase.CANCELLED, PrivateJournalPhase.SUPERSEDED)
        val entry = PrivateJournalEntry(hash(id), phase, released, 1000,
            if (hasEnvelope) hash(id + 100) else null, if (hasEnvelope) "{}" else null,
            if (receipt) "{}" else null, releaseHistoryIncomplete = imported, envelopeBackedUp = backedUp)
        return PrivateLocalPayment(entry, sequence, if (recovery) hash(999) else null, tree, "{}",
            BigInteger.ONE, "0zk-fixture", notes.map(::hash))
    }

    @Test fun restartDoesNotReleaseAnOldCancellationInputNowOwnedByANewerIntent() {
        val old = payment(1, PrivateJournalPhase.CANCELLED, listOf(10))
        val current = payment(2, PrivateJournalPhase.READY, listOf(10))
        // Startup first merges the live native reservation into an engine cache. Cleanup must
        // preserve that exact reservation even though its previous intent was cancelled.
        val restored = mutableSetOf(current.treeNumber to hash(10))
        val cleanup = PrivateCancellationCleanup.pending(listOf(old, current), 0, emptySet())
        cleanup.forEach { release -> release.nullifiers.forEach { restored.remove(release.payment.treeNumber to it) } }
        assertTrue(cleanup.isEmpty())
        assertEquals(setOf(0 to hash(10)), restored)
    }

    @Test fun partialCleanupReleasesOnlyUnclaimedInputsAndRemainsRetryable() {
        val old = payment(1, PrivateJournalPhase.CANCELLED, listOf(10, 11))
        val current = payment(2, PrivateJournalPhase.APPROVED, listOf(10))
        val first = PrivateCancellationCleanup.pending(listOf(old, current), 0, emptySet()).single()
        assertEquals(listOf(hash(11)), first.nullifiers)
        assertFalse(first.completesCleanup)
        val cancelledCurrent = payment(2, PrivateJournalPhase.CANCELLED, listOf(10))
        val next = PrivateCancellationCleanup.pending(listOf(old, cancelledCurrent), 0, emptySet())
            .first { it.payment.entry.intentId == old.entry.intentId }
        assertEquals(listOf(hash(10), hash(11)), next.nullifiers)
        assertTrue(next.completesCleanup)
    }

    @Test fun backupImportedRecoveryAndUncertainLiveClaimsAllProtectReusedInputs() {
        val old = payment(1, PrivateJournalPhase.CANCELLED, listOf(10))
        val live = listOf(
            payment(2, PrivateJournalPhase.READY, listOf(10), backedUp = true),
            payment(2, PrivateJournalPhase.APPROVED, listOf(10), imported = true),
            payment(2, PrivateJournalPhase.READY, listOf(10), recovery = true),
            payment(2, PrivateJournalPhase.UNKNOWN, listOf(10), released = 1),
            payment(2, PrivateJournalPhase.FAILED, listOf(10), released = 1, receipt = true)
        )
        live.forEach { assertTrue(PrivateCancellationCleanup.pending(listOf(old, it), 0, emptySet()).isEmpty()) }
    }

    @Test fun exposedImportedAndRecoveryCancellationsCannotAuthorizeCacheDeletion() {
        val candidates = listOf(
            payment(1, PrivateJournalPhase.CANCELLED, listOf(10), backedUp = true),
            payment(2, PrivateJournalPhase.CANCELLED, listOf(11), imported = true),
            payment(3, PrivateJournalPhase.CANCELLED, listOf(12), recovery = true),
            payment(4, PrivateJournalPhase.CANCELLED, listOf(13), released = 1),
            payment(5, PrivateJournalPhase.CANCELLED, listOf(14), receipt = true)
        )
        assertTrue(PrivateCancellationCleanup.pending(candidates, 0, emptySet()).isEmpty())
        val fromBackup = payment(6, PrivateJournalPhase.CANCELLED, listOf(15), sequence = 6)
        assertTrue(PrivateCancellationCleanup.pending(listOf(fromBackup), 6, emptySet()).isEmpty())
        // Ineligible cancellations also protect their selectors when an independent eligible
        // old cancellation happens to reference them. Missing exposure history is not clearance.
        candidates.forEachIndexed { index, reservedPayment ->
            val old = payment(10, PrivateJournalPhase.CANCELLED, listOf(index + 10))
            assertTrue(PrivateCancellationCleanup.pending(listOf(old, reservedPayment), 0, emptySet()).isEmpty())
        }
    }

    @Test fun nullifierIdentityIncludesItsTreeAndCompletedCleanupIsNotReplayed() {
        val old = payment(1, PrivateJournalPhase.CANCELLED, listOf(10), tree = 7)
        val otherTree = payment(2, PrivateJournalPhase.READY, listOf(10), tree = 8)
        assertEquals(listOf(hash(10)), PrivateCancellationCleanup.pending(listOf(old, otherTree), 0, emptySet()).single().nullifiers)
        assertTrue(PrivateCancellationCleanup.pending(listOf(old, otherTree), 0, setOf(old.entry.intentId)).isEmpty())
    }
}
