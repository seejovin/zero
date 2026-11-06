package app.localwallet.mainnet

import java.math.BigInteger
import java.util.Locale

/** These are release blockers, not switches that remote configuration or preferences can enable. */
internal enum class PrivateReleaseBlocker {
    REVIEWED_PROTOCOL_DEPLOYMENT,
    VERIFIED_LOCAL_PROVER,
    INDEPENDENT_PRIVACY_RELAY,
    ANDROID_AND_RECOVERY_VERIFICATION,
    WEBVIEW_PLATFORM_PRIVACY_REVIEW
}

internal class PrivatePaymentAvailability internal constructor() {
    val available: Boolean get() = false
    val supportedChainIds: Set<Long> get() = emptySet()
    val blockers: Set<PrivateReleaseBlocker> get() = PrivateReleaseBlocker.entries.toSet()
    val title: String get() = "Private transfers"
    val summary: String get() = "Not available in this build"
    val explanation: String get() = "Private transfers are not activated. The private payment engine, " +
        "privacy relay and recovery checks must be verified before this feature can send funds. " +
        "Current Send and Deposit use public blockchain addresses."
}

/**
 * The reviewed deployment allowlist is deliberately empty. Adding an SDK or setting a preference
 * cannot activate private sending. The future release must pin the exact protocol, verifier,
 * proving artifacts and independent transport before this gate is changed and reviewed.
 */
internal object PrivatePaymentRelease {
    fun availability(): PrivatePaymentAvailability = PrivatePaymentAvailability()

    fun requireAvailable() {
        if (!availability().available) throw PrivatePaymentFailure(PrivateFailure.UNAVAILABLE)
    }
}

internal enum class PrivateFailure(val publicMessage: String) {
    UNAVAILABLE("Private transfers are not available in this build. No new private payment was submitted."),
    PRIVATE_DESTINATION_IN_PUBLIC_FLOW("This is a private-payment destination. Private transfers are not available in this build. Use a public address for a public transfer."),
    INVALID_INPUT("The private transfer details are invalid. Review them again."),
    SYNC_INCOMPLETE("The private balance is still synchronizing. Wait for a verified balance before sending."),
    CONSOLIDATION_REQUIRED("These private funds need consolidation. Review its cost before using Max."),
    INSUFFICIENT_FUNDS("The private balance does not cover the payment and its fee."),
    QUOTE_EXPIRED("The private transfer fee expired. Review a new fee before submitting."),
    BACKUP_REQUIRED("Complete private-wallet recovery verification before sending."),
    RECOVERY_WAITING_FOR_FINALITY("Recovery is waiting for its saved blockchain checkpoint to finalize. Try verification again later."),
    RECOVERY_CUTOFF_REORG("The saved recovery checkpoint changed on the canonical chain. Recovery remains locked for review."),
    RECOVERY_CAPACITY("This private history exceeds the verified recovery capacity. Its existing records have been preserved."),
    RECOVERY_FEE_EXCEEDS_BALANCE("Some recovered private notes do not cover the self-transfer fee. Recovery remains locked."),
    PROOF_INVALID("The local private proof or its approved transfer details could not be verified. Nothing new was submitted."),
    LOCAL_CANCELLED("Local preparation was stopped. Check the saved private transfer before preparing another one."),
    BUSY("Resolve the existing private transfer before starting another one."),
    FOREGROUND_REQUIRED("Open the wallet to continue this private transfer."),
    STALE_REVIEW("The private transfer changed. Review it again."),
    LOCAL_ENGINE("The private payment could not be prepared locally. Nothing new was submitted."),
    STORAGE("The private payment record could not be saved. Submission has been stopped."),
    UNKNOWN_OUTCOME("The submission outcome is unknown. Check the existing payment before trying again."),
    RETRY_EXHAUSTED("The service's retry budget for this exact operation is exhausted. The saved proof remains tracked and its inputs remain reserved."),
    INVALID_ACKNOWLEDGEMENT("The service response did not match this private operation. Its outcome remains unresolved."),
    RECONCILIATION("The private payment outcome could not be verified. Its inputs remain reserved."),
    CANCELLATION_UNAVAILABLE("This private payment may already have been released. Check its outcome before replacing it.")
}

/** Do not include raw provider errors, wallet addresses, notes or proofs in UI error strings. */
internal class PrivatePaymentFailure(val failure: PrivateFailure) : WalletFailure(failure.publicMessage)

internal object PrivateAddressRules {
    /** A routing guard only; a protocol adapter must fully decode and validate private addresses. */
    fun looksPrivate(input: String): Boolean {
        val value = input.trim().lowercase(Locale.ROOT)
        return value.startsWith("0zk") || value.startsWith("railgun:") ||
            value.startsWith("private:") || value.startsWith("aztec:")
    }

    fun requirePublicText(input: String): String {
        if (looksPrivate(input)) throw PrivatePaymentFailure(PrivateFailure.PRIVATE_DESTINATION_IN_PUBLIC_FLOW)
        return input.trim()
    }

    /** This must run before creating an RPC client or issuing a public balance/fee query. */
    fun requirePublicEvmRecipient(input: String): String = Amounts.address(requirePublicText(input))
}

internal object PrivateUnits {
    private val maximum = BigInteger.ONE.shiftLeft(256).subtract(BigInteger.ONE)

    fun checked(value: BigInteger, positive: Boolean = false): BigInteger {
        if (value.signum() < 0 || value > maximum || (positive && value.signum() == 0)) {
            throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
        }
        return value
    }

    fun fromCanonical(value: String, positive: Boolean = false): BigInteger {
        if (value.length !in 1..78 || !Regex("0|[1-9][0-9]*").matches(value)) {
            throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
        }
        return checked(BigInteger(value), positive)
    }

    fun add(left: BigInteger, right: BigInteger): BigInteger = checked(checked(left) + checked(right))
}

/** Note selection is local. Neither identifiers nor note values belong in a fee-quote request. */
internal class PrivateSpendPlan(
    noteValues: List<BigInteger>,
    val serviceFeeUnits: BigInteger,
    val protocolFeeUnits: BigInteger,
    val scanComplete: Boolean,
    val includesAllSpendableNotes: Boolean,
    val needsConsolidation: Boolean
) {
    private val values = noteValues.toList().also { notes ->
        if (notes.size > 1_024) throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
        notes.forEach { PrivateUnits.checked(it, positive = true) }
        PrivateUnits.checked(serviceFeeUnits)
        PrivateUnits.checked(protocolFeeUnits)
    }
    val noteCount: Int get() = values.size
    val inputUnits: BigInteger = values.fold(BigInteger.ZERO, PrivateUnits::add)
    val feeUnits: BigInteger = PrivateUnits.add(serviceFeeUnits, protocolFeeUnits)

    fun maximumUnits(): BigInteger {
        if (!scanComplete) throw PrivatePaymentFailure(PrivateFailure.SYNC_INCOMPLETE)
        if (!includesAllSpendableNotes || needsConsolidation || noteCount > 16) {
            throw PrivatePaymentFailure(PrivateFailure.CONSOLIDATION_REQUIRED)
        }
        val maximum = inputUnits - feeUnits
        if (maximum.signum() <= 0) throw PrivatePaymentFailure(PrivateFailure.INSUFFICIENT_FUNDS)
        return PrivateUnits.checked(maximum, positive = true)
    }

    fun changeFor(recipientUnits: BigInteger): BigInteger {
        if (!scanComplete) throw PrivatePaymentFailure(PrivateFailure.SYNC_INCOMPLETE)
        if (needsConsolidation || noteCount > 16) throw PrivatePaymentFailure(PrivateFailure.CONSOLIDATION_REQUIRED)
        val outgoing = PrivateUnits.add(PrivateUnits.checked(recipientUnits, positive = true), feeUnits)
        if (outgoing > inputUnits) throw PrivatePaymentFailure(PrivateFailure.INSUFFICIENT_FUNDS)
        return inputUnits - outgoing
    }

    override fun toString(): String = "PrivateSpendPlan([local values redacted])"
}
