package app.localwallet.mainnet

import java.math.BigInteger

internal enum class WithdrawalAmountMode { MANUAL, MAX }

/** Public transfer input only; this type grants no signing or submission authority. */
internal class WithdrawalRequest private constructor(
    val from: String,
    val position: PortfolioAsset,
    val recipient: String,
    val amount: String?,
    val mode: WithdrawalAmountMode
) {
    val network: WalletNetwork get() = position.network
    val symbol: String get() = position.asset.symbol

    companion object {
        fun create(networks: List<WalletNetwork>, from: String, chainId: Long?, symbol: String,
                   recipientInput: String, amountInput: String,
                   mode: WithdrawalAmountMode = WithdrawalAmountMode.MANUAL): WithdrawalRequest {
            val position = AssetSelection.requireSelected(networks, symbol, chainId)
            val sender = Amounts.address(from.trim())
            val recipient = PrivateAddressRules.requirePublicEvmRecipient(recipientInput)
            // The core repeats this policy, including EIP-55 validation, before final review.
            // Do not lowercase an entered mixed-case address and accidentally erase its checksum.
            if (networks.flatMap { it.assets }.any { it.contract?.equals(recipient, ignoreCase = true) == true }) {
                throw WalletFailure("Enter the recipient's wallet address, not a token contract.")
            }
            val amount = if (mode == WithdrawalAmountMode.MAX) null else
                Amounts.display(Amounts.parse(amountInput.trim(), position.asset.decimals), position.asset.decimals)
            return WithdrawalRequest(sender, position, recipient, amount, mode)
        }
    }

    override fun equals(other: Any?): Boolean = other is WithdrawalRequest && from == other.from &&
        position == other.position && recipient == other.recipient && amount == other.amount && mode == other.mode

    override fun hashCode(): Int {
        var result = from.hashCode()
        result = 31 * result + position.hashCode()
        result = 31 * result + recipient.hashCode()
        result = 31 * result + (amount?.hashCode() ?: 0)
        return 31 * result + mode.hashCode()
    }
}

internal class WithdrawalQuoteTicket internal constructor(
    val request: WithdrawalRequest,
    internal val revision: Long,
    internal val startedElapsed: Long
)

/** A checked read-only quote; fresh preparation and user approval remain separate steps. */
internal class WithdrawalQuote internal constructor(
    internal val ticket: WithdrawalQuoteTicket,
    val fee: FeePreview,
    val maximum: MaxTransferQuote?,
    val recipientUnits: BigInteger
) {
    val request: WithdrawalRequest get() = ticket.request
    val amount: String get() = Amounts.display(recipientUnits, request.position.asset.decimals)
    val assetAvailable: BigInteger get() = fee.assetAvailable!!
    val readyToPrepare: Boolean get() = fee.transferAvailable && fee.shortfall == BigInteger.ZERO

    fun recipientUsd(quotes: Map<String, UsdQuote>, nowMillis: Long): UsdValue =
        UsdPrices.value(request.position, recipientUnits, quotes, nowMillis)

    fun estimatedFeeUsd(quotes: Map<String, UsdQuote>, nowMillis: Long): UsdValue =
        UsdPrices.value(PortfolioAsset(request.network, request.network.nativeAsset), fee.totalEstimate, quotes, nowMillis)

    fun reserveUsd(quotes: Map<String, UsdQuote>, nowMillis: Long): UsdValue =
        UsdPrices.value(PortfolioAsset(request.network, request.network.nativeAsset), fee.reserve, quotes, nowMillis)

    fun remainingMillis(nowElapsed: Long): Long = if (fresh(ticket.startedElapsed, nowElapsed))
        QUOTE_LIFETIME_MILLIS - (nowElapsed - ticket.startedElapsed) else 0

    internal companion object {
        const val QUOTE_LIFETIME_MILLIS = 30_000L
        fun fresh(startedElapsed: Long, nowElapsed: Long): Boolean = startedElapsed >= 0 &&
            nowElapsed >= startedElapsed && nowElapsed - startedElapsed < QUOTE_LIFETIME_MILLIS

        fun checked(ticket: WithdrawalQuoteTicket, fee: FeePreview, maximum: MaxTransferQuote?, nowElapsed: Long): WithdrawalQuote {
            if (!fresh(ticket.startedElapsed, nowElapsed)) throw WalletFailure("The fee estimate expired. Refresh it before reviewing.")
            val request = ticket.request
            if (fee.network != request.network || fee.symbol != request.symbol ||
                !fee.recipient.equals(request.recipient, ignoreCase = true)) invalidQuote()
            val units = Amounts.parse(fee.amount, request.position.asset.decimals)
            val assetAvailable = fee.assetAvailable ?: invalidQuote()
            val fields = listOf(fee.executionEstimate, fee.executionLimit, fee.l1Estimate, fee.operatorEstimate,
                fee.reserve, fee.ethAvailable, fee.ethNeeded, fee.totalEstimate, assetAvailable)
            if (fields.any { it.signum() < 0 || it.bitLength() > 256 } ||
                fee.gasLimit !in 21_000L..300_000L || fee.executionLimit <= BigInteger.ZERO ||
                fee.executionEstimate > fee.executionLimit ||
                fee.reserve != fee.executionLimit + (fee.l1Estimate + fee.operatorEstimate) * BigInteger.valueOf(2) ||
                fee.ethNeeded != fee.reserve + (if (request.position.asset.contract == null) units else BigInteger.ZERO) ||
                (request.network.feeProfile != NetworkFeeProfile.OP_STACK && (fee.l1Estimate != BigInteger.ZERO || fee.operatorEstimate != BigInteger.ZERO)) ||
                fee.transferAvailable != (assetAvailable >= units) ||
                (request.position.asset.contract == null && assetAvailable != fee.ethAvailable)) invalidQuote()
            if (fee.reserve > NetworkPolicy.maximumFeeReserve(request.network.chainId)) {
                throw WalletFailure("The fee reserve exceeds this wallet's transfer policy. Try again when network fees are lower.")
            }
            when (request.mode) {
                WithdrawalAmountMode.MANUAL -> {
                    if (maximum != null || request.amount == null ||
                        Amounts.parse(request.amount, request.position.asset.decimals) != units) invalidQuote()
                }
                WithdrawalAmountMode.MAX -> {
                    if (maximum == null || maximum.fee != fee || maximum.units != units ||
                        maximum.balanceUnits.signum() <= 0 || maximum.balanceUnits.bitLength() > 256 ||
                        maximum.observedAtElapsed < ticket.startedElapsed ||
                        !maximum.matches(request.from, request.network, request.symbol, request.recipient, fee.amount, nowElapsed)) invalidQuote()
                    val expectedBalance = if (request.position.asset.contract == null) units + fee.reserve else units
                    if (maximum.balanceUnits != expectedBalance || maximum.balanceUnits != assetAvailable ||
                        (request.position.asset.contract == null && fee.ethAvailable != maximum.balanceUnits)) invalidQuote()
                }
            }
            return WithdrawalQuote(ticket, fee, maximum, units)
        }

        private fun invalidQuote(): Nothing = throw WalletFailure("The fee response did not match this transfer. Refresh the estimate.")
    }
}

/**
 * UI-thread state for one withdrawal form. Quote jobs contain public inputs only and are
 * coalesced by the caller. Object identity consumes replies exactly once, while revisions
 * prevent an old A reply from being accepted after the form changed A -> B -> A.
 */
internal class WithdrawalFlow {
    private var revision = 0L
    private var request: WithdrawalRequest? = null
    private var activeTicket: WithdrawalQuoteTicket? = null
    private var accepted: WithdrawalQuote? = null
    private var preparation: WithdrawalQuote? = null
    private var closed = false

    fun update(request: WithdrawalRequest?) {
        if (closed || this.request == request) return
        revision++
        this.request = request
        activeTicket = null
        accepted = null
        // A running preparation is not released by editing. Its caller must resolve its
        // result before another preparation is possible, including a stale success.
    }

    fun beginQuote(nowElapsed: Long): WithdrawalQuoteTicket? {
        val input = request ?: return null
        if (closed || preparation != null || activeTicket != null || nowElapsed < 0) return null
        accepted = null
        return WithdrawalQuoteTicket(input, revision, nowElapsed).also { activeTicket = it }
    }

    fun complete(ticket: WithdrawalQuoteTicket, fee: FeePreview, maximum: MaxTransferQuote?, nowElapsed: Long): Boolean {
        if (!currentTicket(ticket)) return false
        activeTicket = null
        accepted = null
        accepted = WithdrawalQuote.checked(ticket, fee, maximum, nowElapsed)
        return true
    }

    /** Only the current request may replace its own UI with a read failure. */
    fun fail(ticket: WithdrawalQuoteTicket): Boolean {
        if (!currentTicket(ticket)) return false
        activeTicket = null
        accepted = null
        return true
    }

    fun currentQuote(nowElapsed: Long): WithdrawalQuote? = accepted?.takeIf {
        !closed && it.ticket.revision == revision && it.request == request && it.remainingMillis(nowElapsed) > 0
    }

    /** One claim per accepted observation; it reserves no nonce and never accesses a key. */
    fun claimPreparation(nowElapsed: Long): WithdrawalQuote? {
        if (closed || preparation != null) return null
        return currentQuote(nowElapsed)?.takeIf { it.readyToPrepare }?.also {
            preparation = it
            activeTicket = null
        }
    }

    fun preparationCurrent(quote: WithdrawalQuote): Boolean = !closed && preparation === quote &&
        quote.ticket.revision == revision && quote.request == request

    /** A failed preparation requires a new observation, not a replay of its prior claim. */
    fun releasePreparation() {
        preparation = null
        revision++
        activeTicket = null
        accepted = null
    }

    fun invalidate() {
        closed = true
        revision++
        request = null
        activeTicket = null
        accepted = null
        preparation = null
    }

    private fun currentTicket(ticket: WithdrawalQuoteTicket): Boolean = !closed &&
        activeTicket === ticket && ticket.revision == revision && ticket.request == request && preparation == null
}
