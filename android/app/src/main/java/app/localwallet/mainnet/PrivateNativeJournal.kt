package app.localwallet.mainnet

import org.json.JSONObject
import org.json.JSONArray
import uniffi.wallet_core.PrivatePaymentJournal
import uniffi.wallet_core.privateValidateEnvelope
import java.math.BigInteger
import java.util.Locale

/** A device-local view, never admitted to an outbound transport or diagnostic log. */
internal class PrivateLocalPayment(
    val entry: PrivateJournalEntry,
    val sequence: Long,
    val recoveryId: String?,
    val treeNumber: Int,
    val localIntentJson: String,
    val amountUnits: BigInteger,
    val recipient: String,
    val nullifiers: List<String>
) {
    override fun toString() = "PrivateLocalPayment([local history redacted])"
}

/**
 * Concrete local UniFFI adapter. The Rust journal encrypts intents/envelopes, reserves input notes,
 * verifies its append-only history and repeats approval binding. It does not verify ZK proofs.
 * Only the authenticated PrivateWalletSession factory creates this adapter. Missing journals are
 * always enrolled through native recovery quarantine; there is no bare create-or-open shortcut.
 */
internal class PrivateNativeJournal internal constructor(
    private val native: PrivatePaymentJournal
) : PrivateOperationJournal, AutoCloseable {
    companion object {
        /** Review fields are explicitly compared before the native journal sees any authorization. */
        fun checkReviewBinding(review: PrivatePaymentReview) {
            val intent = RpcJson.parse(review.localIntentForJournal(), maxChars = 524_288)
            val domain = intent.getJSONObject("domain")
            val fee = intent.getJSONObject("fee")
            val asset = review.draft.asset
            requireStorage(intent.getString("intent_id") == review.intentId &&
                intent.getString("recipient") == review.draft.recipient &&
                units(intent, "amount_units") == review.recipientUnits &&
                domain.getLong("chain_id") == asset.chainId && domain.getString("token_contract") == asset.tokenContract &&
                domain.getInt("token_decimals") == asset.decimals)
            // The initial protocol candidate has one broadcaster fee. A separate protocol fee must
            // gain its own native binding and review before a later deployment can support it.
            requireStorage(review.plan.protocolFeeUnits == BigInteger.ZERO &&
                units(fee, "fee_units") == review.plan.serviceFeeUnits &&
                fee.getLong("quote_valid_until") == review.quoteValidUntilEpochSeconds)
            val notes = intent.getJSONArray("input_notes").objects()
            requireStorage(notes.size == review.plan.noteCount &&
                notes.map { units(it, "units") }.fold(BigInteger.ZERO, PrivateUnits::add) == review.plan.inputUnits &&
                units(intent, "change_units") == review.plan.changeFor(review.recipientUnits))
        }

        private fun units(value: JSONObject, field: String): BigInteger = PrivateUnits.fromCanonical(value.getString(field))
        private fun requireStorage(condition: Boolean) {
            if (!condition) throw PrivatePaymentFailure(PrivateFailure.STORAGE)
        }

    }

    override fun recordApproved(review: PrivatePaymentReview): PrivateJournalEntry = checked {
        checkReviewBinding(review)
        native.recordApproved(review.localIntentForJournal())
        get(review.intentId)
    }

    override fun recordReady(intentId: String, operation: PrivateProvedOperation): PrivateJournalEntry = checked {
        native.recordReady(intentId, operation.localEnvelopeForJournal())
        get(intentId)
    }

    override fun markReleased(intentId: String): PrivateJournalEntry = checked {
        native.markReleased(intentId)
        get(intentId)
    }

    override fun markUnknown(intentId: String): PrivateJournalEntry = checked {
        native.markUnknown(intentId)
        get(intentId)
    }

    override fun observeReceipt(intentId: String, observation: PrivateChainObservation.Receipt): PrivateJournalEntry = checked {
        native.observeReceipt(intentId, observation.evidenceForJournal())
        get(intentId)
    }

    override fun markReorg(intentId: String, observation: PrivateChainObservation.Reorg): PrivateJournalEntry = checked {
        native.markReorg(intentId, observation.evidenceForJournal())
        get(intentId)
    }

    override fun cancelUnreleased(intentId: String): PrivateJournalEntry = checked {
        native.cancelApproved(intentId)
        get(intentId)
    }

    override fun get(intentId: String): PrivateJournalEntry = localPayment(intentId).entry

    fun localPayment(intentId: String): PrivateLocalPayment = checked {
        val stored = RpcJson.parse(native.getPayment(intentId), maxChars = 2_097_152)
        val record = stored.getJSONObject("record")
        val intent = RpcJson.parse(stored.getString("intent_json"), maxChars = 524_288)
        val domain = intent.getJSONObject("domain")
        val fee = intent.getJSONObject("fee")
        requireStorage(record.getString("intent_id") == intentId && intent.getString("intent_id") == intentId)
        requireStorage(record.getLong("chain_id") == domain.getLong("chain_id") &&
            record.getString("token_contract") == domain.getString("token_contract"))
        requireStorage(record.getString("proof_verification") == "not_performed_by_journal")
        val envelope = stored.optionalString("envelope_json")
        val digest = record.optionalString("operation_digest")
        if (envelope != null) {
            // Closed-schema and digest validation only. Never present this as proof verification.
            val validated = privateValidateEnvelope(envelope)
            requireStorage(validated == envelope)
            val contents = RpcJson.parse(validated, maxChars = 524_288)
            requireStorage(contents.getString("operation_digest") == digest &&
                contents.getJSONObject("fee").getLong("quote_valid_until") == fee.getLong("quote_valid_until"))
        }
        val receipt = if (record.isNull("receipt")) null else record.getJSONObject("receipt")
        val finalizedRevert = receipt != null && !receipt.getBoolean("execution_success") &&
            receipt.getLong("finalized_height") >= receipt.getLong("block_number") &&
            receipt.getString("canonical_block_hash") == receipt.getString("block_hash")
        val entry = PrivateJournalEntry(intentId,
            PrivateJournalPhase.valueOf(record.getString("phase").uppercase(Locale.ROOT)),
            record.getLong("release_count"), fee.getLong("quote_valid_until"), digest, envelope,
            receipt?.toString(), record.getBoolean("release_history_incomplete"), finalizedRevert, record.getBoolean("envelope_backed_up"))
        val tree = PrivateEngineWire.number(intent, "tree_number", 0, 65_535).toInt()
        val nullifiers = intent.getJSONArray("input_notes").objects().map {
            PrivateIdentifiers.require(PrivateEngineWire.string(it, "nullifier"))
        }
        if (nullifiers.isEmpty() || nullifiers.size > 16 || nullifiers.toSet().size != nullifiers.size) requireStorage(false)
        PrivateLocalPayment(entry, PrivateEngineWire.number(record, "sequence", 1, Long.MAX_VALUE),
            record.optionalString("recovery_id")?.let(PrivateIdentifiers::require), tree, stored.getString("intent_json"),
            units(intent, "amount_units"), intent.getString("recipient"), nullifiers)
    }

    /** The authoritative encrypted journal is re-read; cached UI history is never a spend ledger. */
    fun localPayments(): List<PrivateLocalPayment> = checked {
        val rows = RpcJson.parse("{\"records\":" + native.listPayments() + "}", maxChars = 4_194_304, maxNodes = 200_000)
            .getJSONArray("records")
        if (rows.length() > 10_000) requireStorage(false)
        rows.objects().map { localPayment(PrivateIdentifiers.require(it.getString("intent_id"))) }.sortedBy { it.sequence }
    }

    fun reservations(payments: List<PrivateLocalPayment>): JSONArray {
        val seen = HashSet<Pair<Int, String>>()
        val result = JSONArray()
        payments.filter { it.entry.phase !in setOf(PrivateJournalPhase.CANCELLED, PrivateJournalPhase.SUPERSEDED) }.forEach { payment ->
            payment.nullifiers.forEach { nullifier ->
                if (seen.add(payment.treeNumber to nullifier)) result.put(JSONObject().put("tree_number", payment.treeNumber).put("nullifier", nullifier))
            }
        }
        return result
    }

    private inline fun <T> checked(block: () -> T): T = try { block() }
        catch (_: Exception) { throw PrivatePaymentFailure(PrivateFailure.STORAGE) }

    override fun close() { native.close() }
    internal fun recoveryHandle(): PrivatePaymentJournal = native
}
