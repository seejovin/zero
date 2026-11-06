package app.localwallet.mainnet

import java.math.BigDecimal
import java.math.BigInteger
import java.math.RoundingMode

enum class PriceFreshness { FRESH, STALE, UNAVAILABLE }
enum class UsdCoverage { COMPLETE, PARTIAL, UNAVAILABLE }

/** A public spot quote. fetchedAtMillis records receipt, not an exchange execution timestamp. */
data class UsdQuote(val symbol: String, val usdPerUnit: BigDecimal, val fetchedAtMillis: Long) {
    val source: String get() = UsdPrices.SOURCE
}

data class UsdValue(val amount: BigDecimal?, val freshness: PriceFreshness, val fetchedAtMillis: Long?) {
    val source: String get() = UsdPrices.SOURCE
}

data class PortfolioUsdTotal(
    val amount: BigDecimal?,
    val knownSubtotal: BigDecimal,
    val coverage: UsdCoverage,
    val stale: Boolean,
    val oldestQuoteAtMillis: Long?,
    val missingBalances: Set<PortfolioAssetKey>,
    val missingPrices: Set<PortfolioAssetKey>
) {
    val incomplete: Boolean get() = coverage != UsdCoverage.COMPLETE
}

object UsdPrices {
    const val SOURCE = "Coinbase"
    const val FRESH_MILLIS = 120_000L
    const val MAX_CACHE_MILLIS = 86_400_000L
    val symbols = NetworkPolicy.assetSymbols
    private val amountPattern = Regex("(?:0|[1-9][0-9]{0,12})(?:\\.[0-9]{1,18})?")
    private val thresholdPattern = Regex("(?:0|[1-9][0-9]{0,6})(?:\\.[0-9]{1,2})?")
    private val largestQuote = BigDecimal("1000000000000")
    private val smallestThreshold = BigDecimal("0.01")
    private val largestThreshold = BigDecimal("1000000")

    fun quote(symbol: String, amount: String, fetchedAtMillis: Long): UsdQuote {
        if (symbol !in symbols || amount.length > 48 || !amountPattern.matches(amount) || fetchedAtMillis <= 0) {
            throw WalletFailure("Coinbase returned an invalid USD quote.")
        }
        val value = BigDecimal(amount)
        if (!validAmount(value)) throw WalletFailure("Coinbase returned an invalid USD quote.")
        return UsdQuote(symbol, value, fetchedAtMillis)
    }

    private fun validAmount(value: BigDecimal): Boolean =
        value.scale() in 0..18 && value.precision() <= 31 && value.signum() > 0 && value <= largestQuote

    fun freshness(quote: UsdQuote?, nowMillis: Long): PriceFreshness {
        if (quote == null || quote.symbol !in symbols || !validAmount(quote.usdPerUnit) ||
            quote.fetchedAtMillis <= 0 || nowMillis < quote.fetchedAtMillis) return PriceFreshness.UNAVAILABLE
        val age = nowMillis - quote.fetchedAtMillis
        return when {
            age <= FRESH_MILLIS -> PriceFreshness.FRESH
            age <= MAX_CACHE_MILLIS -> PriceFreshness.STALE
            else -> PriceFreshness.UNAVAILABLE
        }
    }

    fun value(position: PortfolioAsset, units: BigInteger?, quotes: Map<String, UsdQuote>, nowMillis: Long): UsdValue {
        if (units == null || units.signum() < 0 || units.bitLength() > 256 ||
            position.asset.symbol !in symbols || !NetworkPolicy.supports(position.network.chainId, position.asset.symbol) ||
            position.asset.decimals != NetworkPolicy.decimals(position.network.chainId, position.asset.symbol)) {
            return UsdValue(null, PriceFreshness.UNAVAILABLE, null)
        }
        // A known zero quantity is worth zero without needing a market price.
        if (units == BigInteger.ZERO) return UsdValue(BigDecimal.ZERO, PriceFreshness.FRESH, null)
        val quote = quotes[position.asset.symbol]?.takeIf { it.symbol == position.asset.symbol }
        val freshness = freshness(quote, nowMillis)
        if (freshness == PriceFreshness.UNAVAILABLE) return UsdValue(null, freshness, null)
        val amount = BigDecimal(units, position.asset.decimals).multiply(quote!!.usdPerUnit)
        return UsdValue(amount, freshness, quote.fetchedAtMillis)
    }

    fun parseThreshold(input: String): BigDecimal {
        if (input.length > 32 || input.any { it.isISOControl() }) throw WalletFailure("Enter a USD amount from 0.01 to 1,000,000, with up to two decimal places.")
        val text = input.trim()
        if (!thresholdPattern.matches(text)) throw WalletFailure("Enter a USD amount from 0.01 to 1,000,000, with up to two decimal places.")
        return checkedThreshold(BigDecimal(text))
    }

    fun checkedThreshold(value: BigDecimal): BigDecimal {
        if (value.scale() !in 0..2 || value.precision() > 9 || value < smallestThreshold || value > largestThreshold) {
            throw WalletFailure("Enter a USD amount from 0.01 to 1,000,000, with up to two decimal places.")
        }
        val normalized = value.stripTrailingZeros()
        return if (normalized.scale() < 0) normalized.setScale(0) else normalized
    }

    /** Display rounding never feeds a balance, transfer amount, or hide-small comparison. */
    fun formatUsd(value: BigDecimal): String {
        require(value.signum() >= 0)
        if (value.signum() > 0 && value < smallestThreshold) return "<\$0.01"
        return "\$" + value.setScale(2, RoundingMode.HALF_UP).toPlainString()
    }
}
