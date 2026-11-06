package app.localwallet.mainnet

/** A local cache mutation derived from one authenticated journal snapshot. Never sent to a provider. */
internal class PrivateCancellationRelease internal constructor(
    val payment: PrivateLocalPayment,
    val nullifiers: List<String>
) {
    val completesCleanup: Boolean get() = nullifiers.size == payment.nullifiers.size
    override fun toString() = "PrivateCancellationRelease([local reservations redacted])"
}

/**
 * An old cancellation authorizes releasing only its still-unclaimed cache reservations. A newer
 * authorization can reuse the same note, so replaying every cancelled entry after restart must not
 * delete that newer authorization's reservation. Tree identity is part of every comparison.
 */
internal object PrivateCancellationCleanup {
    fun pending(
        records: List<PrivateLocalPayment>,
        importedThrough: Long,
        completedIntentIds: Set<String>
    ): List<PrivateCancellationRelease> {
        val claimed = HashSet<Pair<Int, String>>()
        records.filter { payment ->
            payment.entry.phase != PrivateJournalPhase.SUPERSEDED &&
                (payment.entry.phase != PrivateJournalPhase.CANCELLED || !eligible(payment, importedThrough))
        }.forEach { payment ->
            payment.nullifiers.forEach { claimed.add(payment.treeNumber to it) }
        }
        return records.filter { eligible(it, importedThrough) && it.entry.intentId !in completedIntentIds }
            .mapNotNull { payment ->
                val unclaimed = payment.nullifiers.filter { (payment.treeNumber to it) !in claimed }
                if (unclaimed.isEmpty()) null else PrivateCancellationRelease(payment, unclaimed)
            }
    }

    private fun eligible(payment: PrivateLocalPayment, importedThrough: Long): Boolean {
        val entry = payment.entry
        return entry.phase == PrivateJournalPhase.CANCELLED && entry.releaseCount == 0L && !entry.envelopeBackedUp &&
            !entry.releaseHistoryIncomplete && entry.priorReceiptForLocalEngine() == null && payment.recoveryId == null &&
            payment.sequence > importedThrough
    }
}
