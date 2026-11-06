package app.localwallet.mainnet

import org.json.JSONObject
import org.json.JSONArray
import uniffi.wallet_core.privatePrepareIntent
import uniffi.wallet_core.privatePrepareRecoveryScan
import uniffi.wallet_core.privateSealEnvelope
import uniffi.wallet_core.privateVerifyFeeSchedule
import java.math.BigInteger
import java.util.concurrent.ConcurrentHashMap

/** Compiled and independently reviewed release material, not user input or remote feature flags. */
internal class PrivateEngineConfiguration(
    val asset: PrivatePaymentAsset,
    val poolContract: String,
    val verifierDigest: String,
    val artifactManifestSha256: String,
    val feeRecipientAddress: String,
    val feeRecipientCommitment: String,
    feeSchedulePublicKey: ByteArray,
    val deploymentBlock: Long,
    val relay: PrivateRelayConfiguration
) {
    private val feeKey = feeSchedulePublicKey.copyOf()
    init {
        if (asset.chainId != 42161L || asset.symbol != "USDC" || asset.decimals != 6 ||
            asset.tokenContract != "0xaf88d065e77c8cc2239327c5edb3a432268e5831" ||
            poolContract != "0xfa7093cdd9ee6932b4eb2c9e1cde7ce00b1fa4b9" ||
            !Regex("[0-9a-f]{64}").matches(artifactManifestSha256) ||
            !PrivateAddressRules.looksPrivate(feeRecipientAddress) || feeRecipientAddress.length !in 8..256 ||
            feeKey.size != 32 || deploymentBlock <= 0 || deploymentBlock > 9_007_199_254_740_991L) {
            throw PrivatePaymentFailure(PrivateFailure.UNAVAILABLE)
        }
        PrivateIdentifiers.require(verifierDigest)
        PrivateIdentifiers.require(feeRecipientCommitment)
    }
    fun domain(): JSONObject = JSONObject().put("protocol", PrivateEngineWire.PROTOCOL).put("chain_id", asset.chainId)
        .put("pool_contract", poolContract).put("token_contract", asset.tokenContract).put("token_decimals", asset.decimals)
        .put("verifier_digest", verifierDigest)
    fun feeVerificationKey(): ByteArray = feeKey.copyOf()
    companion object {
        /** No approved proving-artifact/relay deployment has been installed into this release. */
        fun reviewedRelease(): PrivateEngineConfiguration = throw PrivatePaymentFailure(PrivateFailure.UNAVAILABLE)
    }
}

internal fun interface PrivateFeeScheduleSource { fun packet(): String }

/** The packet is authenticated natively before any fee or fee recipient enters a local proof. */
internal class PrivateVerifiedFeeScheduleSource(
    private val configuration: PrivateEngineConfiguration,
    private val source: PrivateFeeScheduleSource,
    private val now: () -> Long
) {
    fun current(): JSONObject {
        val time = now()
        if (time <= 0) throw PrivatePaymentFailure(PrivateFailure.QUOTE_EXPIRED)
        val pin = configuration.feeVerificationKey()
        val verified = try { privateVerifyFeeSchedule(source.packet(), pin, time.toULong()) }
            catch (_: Exception) { throw PrivatePaymentFailure(PrivateFailure.QUOTE_EXPIRED) }
            finally { pin.fill(0) }
        val schedule = RpcJson.parse(verified, maxChars = 16_384, maxNodes = 100)
        val domain = schedule.getJSONObject("domain")
        if (domain.toString() != configuration.domain().toString()) {
            // JSONObject order is not an authentication mechanism; native verification provides
            // authentication. Explicit field comparison below permits canonical field ordering.
            PrivateEngineWire.fields(domain, "protocol", "chain_id", "pool_contract", "token_contract", "token_decimals", "verifier_digest")
            val expected = configuration.domain()
            if (domain.keys().asSequence().any { domain.get(it).toString() != expected.get(it).toString() }) {
                throw PrivatePaymentFailure(PrivateFailure.UNAVAILABLE)
            }
        }
        if (schedule.getString("fee_recipient_address") != configuration.feeRecipientAddress ||
            schedule.getString("fee_recipient_commitment") != configuration.feeRecipientCommitment) {
            throw PrivatePaymentFailure(PrivateFailure.UNAVAILABLE)
        }
        return schedule
    }
}

internal data class PrivateScannedBalance(val units: BigInteger, val scanComplete: Boolean, val block: Long, val blockHash: String) {
    override fun toString() = "PrivateScannedBalance([private balance redacted])"
}

/** Real protocol adapter for the hash-verified local JS/WASM engine; it has no public RPC fallback. */
internal class PrivateWebLocalEngine(
    private val runtime: PrivateEngineRuntime,
    private val configuration: PrivateEngineConfiguration,
    private val feeSchedules: PrivateVerifiedFeeScheduleSource,
    private val advisoryStatus: (String) -> JSONObject,
    private val now: () -> Long
) : PrivateLocalEngine, AutoCloseable {
    private val reviews = ConcurrentHashMap<String, Pair<String, PrivatePaymentReview>>()
    private val operatorAdvice = ConcurrentHashMap<String, String>()
    private var initialized = false
    private var privateAddress: String? = null

    /** The 96 native-derived bytes are operation-scoped and wiped after this fixed local message. */
    fun initialize(walletIdentity: String, engineKeys: ByteArray): String {
        PrivatePaymentRelease.requireAvailable()
        PrivateIdentifiers.require(walletIdentity)
        if (engineKeys.size != 96 || initialized) { engineKeys.fill(0); throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE) }
        val spending = engineKeys.copyOfRange(0, 32)
        val viewing = engineKeys.copyOfRange(32, 64)
        val stateKey = engineKeys.copyOfRange(64, 96)
        engineKeys.fill(0)
        val payload = JSONObject()
        try {
            payload.put("wallet_identity", walletIdentity)
                .put("spending_private_key_hex", hex(spending)).put("viewing_private_key_hex", hex(viewing))
                .put("encrypted_state_key_hex", hex(stateKey)).put("domain", configuration.domain())
                .put("fee_recipient_address", configuration.feeRecipientAddress)
                .put("fee_recipient_commitment", configuration.feeRecipientCommitment)
                .put("creation_block", configuration.deploymentBlock)
            val result = runtime.call(PrivateEngineCommand.INITIALIZE, payload)
            PrivateEngineWire.fields(result, "wallet_identity", "private_address", "chain_id", "ready")
            val address = PrivateEngineWire.string(result, "private_address")
            if (PrivateEngineWire.string(result, "wallet_identity") != walletIdentity ||
                PrivateEngineWire.number(result, "chain_id", 42161, 42161) != configuration.asset.chainId ||
                !PrivateEngineWire.boolean(result, "ready") || !address.startsWith("0zk") || address.length !in 32..256 ||
                address.any { it.code !in 33..126 }) throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE)
            initialized = true
            privateAddress = address
            return address
        } finally {
            spending.fill(0); viewing.fill(0); stateKey.fill(0)
            payload.remove("spending_private_key_hex"); payload.remove("viewing_private_key_hex"); payload.remove("encrypted_state_key_hex")
        }
    }

    fun address(): String = privateAddress ?: throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE)

    /** Authenticated journal reservations are merged locally; this cannot clear cached proofs. */
    fun restoreReservations(reservations: JSONArray) {
        requireInitialized()
        val result = runtime.call(PrivateEngineCommand.RESTORE_RESERVATIONS, JSONObject().put("reservations", reservations))
        PrivateEngineWire.fields(result, "restored")
        if (!PrivateEngineWire.boolean(result, "restored")) throw PrivatePaymentFailure(PrivateFailure.STORAGE)
    }

    /** The caller must subtract every newer live claim from the native authoritative snapshot. */
    fun releaseCancelledReservations(release: PrivateCancellationRelease) {
        requireInitialized()
        val payment = release.payment
        val entry = payment.entry
        if (entry.phase != PrivateJournalPhase.CANCELLED || entry.releaseCount != 0L || entry.envelopeBackedUp ||
            entry.releaseHistoryIncomplete || entry.priorReceiptForLocalEngine() != null || payment.recoveryId != null ||
            release.nullifiers.isEmpty() || release.nullifiers.toSet().size != release.nullifiers.size ||
            release.nullifiers.any { it !in payment.nullifiers }) {
            throw PrivatePaymentFailure(PrivateFailure.CANCELLATION_UNAVAILABLE)
        }
        val result = runtime.call(PrivateEngineCommand.RELEASE_CANCELLED_RESERVATIONS,
            JSONObject().put("tree_number", payment.treeNumber).put("nullifiers", JSONArray(release.nullifiers)))
        PrivateEngineWire.fields(result, "released")
        if (!PrivateEngineWire.boolean(result, "released")) throw PrivatePaymentFailure(PrivateFailure.STORAGE)
    }

    fun synchronize(progress: (Int) -> Unit = {}): PrivateScannedBalance {
        requireInitialized()
        val sync = runtime.call(PrivateEngineCommand.SYNC, JSONObject().put("chain_id", configuration.asset.chainId)) { _, percent -> progress(percent) }
        PrivateEngineWire.fields(sync, "chain_id", "scan_complete", "scanned_block", "canonical_block_hash")
        validateScan(sync)
        return balance()
    }

    fun balance(): PrivateScannedBalance {
        requireInitialized()
        val result = runtime.call(PrivateEngineCommand.BALANCE,
            JSONObject().put("chain_id", configuration.asset.chainId).put("token_contract", configuration.asset.tokenContract))
        PrivateEngineWire.fields(result, "chain_id", "token_contract", "units", "scan_complete", "scanned_block", "canonical_block_hash")
        if (PrivateEngineWire.string(result, "token_contract") != configuration.asset.tokenContract) throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE)
        val (block, hash) = validateScan(result)
        return PrivateScannedBalance(PrivateUnits.fromCanonical(PrivateEngineWire.string(result, "units")),
            PrivateEngineWire.boolean(result, "scan_complete"), block, hash)
    }

    override fun prepareLocal(draft: PrivateTransferDraft): PrivatePaymentReview {
        requireInitialized()
        if (draft.asset != configuration.asset) throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
        val schedule = feeSchedules.current()
        val result = runtime.call(PrivateEngineCommand.PREPARE, JSONObject()
            .put("intent_id", draft.intentId).put("chain_id", draft.asset.chainId).put("token_contract", draft.asset.tokenContract)
            .put("recipient", draft.recipient).put("amount_units", draft.recipientUnits?.toString() ?: JSONObject.NULL)
            .put("maximum", draft.maximum).put("fee_schedule", schedule))
        return acceptReview(draft, schedule, result)
    }

    fun recoveryCutoff(recoveryId: String): String {
        requireInitialized(); PrivateIdentifiers.require(recoveryId)
        val result = runtime.call(PrivateEngineCommand.RECOVERY_CUTOFF, JSONObject().put("recovery_id", recoveryId))
        PrivateEngineWire.fields(result, "cutoff_json")
        return PrivateEngineWire.string(result, "cutoff_json")
    }

    fun recoveryScan(recoveryId: String, walletBinding: String, cutoff: JSONObject, finalScan: Boolean): String {
        requireInitialized(); PrivateIdentifiers.require(recoveryId); PrivateIdentifiers.require(walletBinding)
        val result = runtime.call(PrivateEngineCommand.RECOVERY_SCAN, JSONObject().put("recovery_id", recoveryId)
            .put("wallet_binding", walletBinding).put("cutoff_block_number", PrivateEngineWire.number(cutoff, "block_number", 1, 9_007_199_254_740_991))
            .put("cutoff_block_hash", PrivateIdentifiers.require(PrivateEngineWire.string(cutoff, "block_hash")))
            .put("final_scan", finalScan))
        PrivateEngineWire.fields(result, "scan_json")
        return try { privatePrepareRecoveryScan(PrivateEngineWire.string(result, "scan_json")) }
            catch (_: Exception) { throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION) }
    }

    fun recoveryFeeUnits(): BigInteger = units(feeSchedules.current(), "fee_units")

    fun prepareRecoveryBatch(planJson: String, remaining: JSONArray): PrivatePaymentReview {
        requireInitialized()
        val plan = RpcJson.parse(planJson, maxChars = 524_288)
        if (PrivateEngineWire.string(plan, "own_private_recipient") != address()) throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
        val intentId = PrivateIdentifiers.create()
        val schedule = feeSchedules.current()
        val result = runtime.call(PrivateEngineCommand.RECOVERY_PREPARE, JSONObject().put("plan_json", planJson)
            .put("intent_id", intentId).put("fee_schedule", schedule).put("remaining_note_commitments", remaining))
        val draft = PrivateTransferDraft.recovery(configuration.asset, address(), units(result, "recipient_units"), intentId)
        return acceptReview(draft, schedule, result)
    }

    private fun acceptReview(draft: PrivateTransferDraft, schedule: JSONObject, result: JSONObject): PrivatePaymentReview {
        PrivateEngineWire.fields(result, "review_id", "intent_json", "note_values", "service_fee_units", "protocol_fee_units", "recipient_units",
            "scan_complete", "includes_all_spendable_notes", "needs_consolidation", "quote_valid_until")
        val reviewId = PrivateIdentifiers.require(PrivateEngineWire.string(result, "review_id"))
        val notes = result.getJSONArray("note_values")
        if (notes.length() !in 1..16) throw PrivatePaymentFailure(PrivateFailure.CONSOLIDATION_REQUIRED)
        val serviceFee = units(result, "service_fee_units")
        val protocolFee = units(result, "protocol_fee_units")
        val until = PrivateEngineWire.number(result, "quote_valid_until", 1, Long.MAX_VALUE)
        if (serviceFee != units(schedule, "fee_units") || protocolFee != BigInteger.ZERO ||
            until != schedule.getLong("valid_until") || until <= now()) throw PrivatePaymentFailure(PrivateFailure.QUOTE_EXPIRED)
        val plan = PrivateSpendPlan((0 until notes.length()).map {
            val value = notes.get(it) as? String ?: throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE)
            PrivateUnits.fromCanonical(value, positive = true)
        }, serviceFee, protocolFee, PrivateEngineWire.boolean(result, "scan_complete"),
            PrivateEngineWire.boolean(result, "includes_all_spendable_notes"), PrivateEngineWire.boolean(result, "needs_consolidation"))
        val sealed = try { privatePrepareIntent(PrivateEngineWire.string(result, "intent_json")) }
            catch (_: Exception) { throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE) }
        val review = PrivatePaymentReview(draft, plan, units(result, "recipient_units"), until, sealed)
        // Native journal repeats these comparisons before durable approval. Keeping them here
        // prevents showing a review whose displayed recipient/amount differs from its intent.
        PrivateNativeJournal.checkReviewBinding(review)
        reviews[draft.intentId] = reviewId to review
        return review
    }

    override fun proveLocal(review: PrivatePaymentReview, progress: (Int) -> Unit): PrivateProvedOperation {
        requireInitialized()
        val saved = reviews[review.intentId] ?: throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
        if (saved.second !== review) throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
        val result = runtime.call(PrivateEngineCommand.PROVE,
            JSONObject().put("review_id", saved.first).put("intent_json", review.localIntentForJournal())) { _, percent -> progress(percent) }
        PrivateEngineWire.fields(result, "envelope_json", "verified")
        if (!PrivateEngineWire.boolean(result, "verified")) throw PrivatePaymentFailure(PrivateFailure.PROOF_INVALID)
        val envelope = try { privateSealEnvelope(PrivateEngineWire.string(result, "envelope_json")) }
            catch (_: Exception) { throw PrivatePaymentFailure(PrivateFailure.PROOF_INVALID) }
        verify(envelope)
        reviews.remove(review.intentId)
        return PrivateProvedOperation(envelope)
    }

    override fun discardLocalReview(review: PrivatePaymentReview) {
        requireInitialized()
        val saved = reviews.remove(review.intentId) ?: return
        if (saved.second !== review) throw PrivatePaymentFailure(PrivateFailure.STALE_REVIEW)
        val result = runtime.call(PrivateEngineCommand.CANCEL, JSONObject().put("review_id", saved.first))
        PrivateEngineWire.fields(result, "cancelled")
        if (!PrivateEngineWire.boolean(result, "cancelled")) throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE)
    }

    override fun verifyForSubmission(entry: PrivateJournalEntry) {
        requireInitialized()
        if (advisoryState(entry.operationDigest) == "retry_exhausted") throw PrivatePaymentFailure(PrivateFailure.RETRY_EXHAUSTED)
        verify(entry.envelopeForLocalEngine() ?: throw PrivatePaymentFailure(PrivateFailure.STORAGE))
    }

    private fun verify(envelope: String) {
        val verified = runtime.call(PrivateEngineCommand.VERIFY, JSONObject().put("envelope_json", envelope))
        PrivateEngineWire.fields(verified, "verified")
        if (!PrivateEngineWire.boolean(verified, "verified")) throw PrivatePaymentFailure(PrivateFailure.PROOF_INVALID)
    }

    override fun observeCanonical(entry: PrivateJournalEntry): PrivateChainObservation? {
        requireInitialized()
        val previous = entry.priorReceiptForLocalEngine()
        val advice = entry.operationDigest?.let { digest ->
            runCatching { advisoryStatus(digest) }.getOrNull()?.also { operatorAdvice[digest] = it.getString("state") }
        }
        val prior = previous?.let { RpcJson.parse(it) }
        val hash = if (prior != null && prior.getBoolean("execution_success")) prior.getString("transaction_hash")
            else advice?.optionalString("transaction_hash") ?: prior?.getString("transaction_hash")
        hash?.let(PrivateIdentifiers::require)
        val result = runtime.call(PrivateEngineCommand.OBSERVE, JSONObject()
            .put("envelope_json", entry.envelopeForLocalEngine() ?: throw PrivatePaymentFailure(PrivateFailure.STORAGE))
            .put("transaction_hash", hash ?: JSONObject.NULL).put("prior_receipt_json", previous ?: JSONObject.NULL))
        PrivateEngineWire.fields(result, "observation_type", "observation_json")
        return when (PrivateEngineWire.string(result, "observation_type")) {
            "none" -> { if (!result.isNull("observation_json")) throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE); null }
            "receipt" -> PrivateChainObservation.Receipt(PrivateEngineWire.string(result, "observation_json"))
            "reorg" -> PrivateChainObservation.Reorg(PrivateEngineWire.string(result, "observation_json"))
            else -> throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE)
        }
    }

    /** Advisory limits may disable a retry, but can never mark a payment confirmed. */
    fun advisoryState(operationDigest: String?): String? = operationDigest?.let(operatorAdvice::get)
    fun setForeground(value: Boolean) = runtime.setForeground(value)
    fun cancelLocalWork() = runtime.cancelLocalWork()
    private fun validateScan(value: JSONObject): Pair<Long, String> {
        PrivateEngineWire.number(value, "chain_id", configuration.asset.chainId, configuration.asset.chainId)
        val block = PrivateEngineWire.number(value, "scanned_block", configuration.deploymentBlock, 9_007_199_254_740_991)
        val hash = PrivateIdentifiers.require(PrivateEngineWire.string(value, "canonical_block_hash"))
        return block to hash
    }
    private fun requireInitialized() { if (!initialized) throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE) }
    private fun units(value: JSONObject, field: String) = PrivateUnits.fromCanonical(PrivateEngineWire.string(value, field))
    private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it.toInt() and 255) }
    override fun close() { reviews.clear(); operatorAdvice.clear(); privateAddress = null; initialized = false; runtime.close() }
}
