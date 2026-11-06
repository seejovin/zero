package app.localwallet.mainnet

import org.json.JSONArray
import org.json.JSONObject
import java.math.BigInteger

/** A read-only, short-lived draft observation. It never authorizes signing or reserves a nonce. */
data class MaxTransferQuote internal constructor(
    val from: String,
    val network: WalletNetwork,
    val symbol: String,
    val recipient: String,
    val amount: String,
    val units: BigInteger,
    val balanceUnits: BigInteger,
    val fee: FeePreview,
    val block: Long,
    val blockHash: String,
    val observedAtElapsed: Long
) {
    fun matches(from: String, network: WalletNetwork, symbol: String, recipient: String,
                amount: String, nowElapsed: Long): Boolean =
        observedAtElapsed >= 0 && nowElapsed >= observedAtElapsed && nowElapsed - observedAtElapsed < 30_000L &&
            this.from.equals(from, ignoreCase = true) && this.network == network && this.symbol == symbol &&
            this.recipient.equals(recipient, ignoreCase = true) && this.amount == amount

    fun requireCurrent(from: String, network: WalletNetwork, symbol: String, recipient: String,
                       amount: String, nowElapsed: Long) {
        if (!matches(from, network, symbol, recipient, amount, nowElapsed)) {
            throw WalletFailure("Max is no longer current for this transfer. Tap Max again before reviewing.")
        }
    }
}

internal data class MaxTransferCalculation(val amount: String, val units: BigInteger, val balanceUnits: BigInteger, val fee: FeePreview)

/** Exact integer arithmetic only. No price, vault, journal, signing or broadcast dependency. */
internal object MaxTransferCalculator {
    fun calculate(network: WalletNetwork, from: String, symbol: String, recipient: String,
                  block: JSONObject, values: Map<String, BigInteger>, rpc: NetworkClient,
                  priorityFeePerGas: BigInteger? = null, nonce: Long? = null): MaxTransferCalculation {
        Amounts.address(from)
        Amounts.address(recipient)
        val asset = network.asset(symbol)
        val balance = values[symbol] ?: throw WalletFailure("Refresh the asset balance before using Max.")
        val nativeBalance = values[network.nativeSymbol] ?: throw WalletFailure("Refresh the ${network.nativeSymbol} balance before using Max.")
        // Reject malformed quantities rather than treating unknown or negative balances as zero.
        Amounts.hex(balance)
        Amounts.hex(nativeBalance)
        if (balance == BigInteger.ZERO) throw WalletFailure("There is no $symbol available to send on ${network.name}.")
        val priority = priorityFeePerGas ?: FeeEstimator.priorityFee(network, rpc)
        fun quote(units: BigInteger): FeePreview = FeeEstimator.estimate(network, from, symbol, recipient,
            Amounts.display(units, asset.decimals), block, values, rpc, requireSimulation = true,
            priorityFeePerGas = priority, nonce = nonce, includeFeeFields = asset.contract == null || nonce != null)

        if (asset.contract != null) {
            val fee = quote(balance)
            return MaxTransferCalculation(fee.amount, balance, balance, fee)
        }

        // The recipient may be a contract whose gas use depends on msg.value. Re-simulate every
        // candidate until the exact amount and its reserve agree. An unstable calculation fails
        // closed instead of quietly guessing 21,000 gas or rounding away any wei.
        var candidate = BigInteger.ONE
        val visited = HashSet<BigInteger>()
        repeat(6) {
            if (!visited.add(candidate)) throw WalletFailure("This recipient's gas requirement changed while calculating Max. Try a specific amount instead.")
            val fee = quote(candidate)
            val next = balance.subtract(fee.reserve)
            if (next <= BigInteger.ZERO) throw WalletFailure("The ${network.nativeSymbol} balance does not cover this transfer's fee reserve.")
            if (next == candidate) return MaxTransferCalculation(fee.amount, candidate, balance, fee)
            candidate = next
        }
        throw WalletFailure("The network could not produce a stable Max amount. Refresh or enter a specific amount.")
    }

    fun requireFreshBlock(block: JSONObject, nowMillis: Long) {
        val timestamp = Amounts.longQuantity(block.getString("timestamp"))
        val now = nowMillis / 1000L
        if (timestamp < now - 60L || timestamp > now + 120L) {
            throw WalletFailure("The latest network block is not fresh enough to calculate Max. Refresh and check your device clock.")
        }
    }
}
