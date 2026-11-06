package app.localwallet.mainnet

import java.math.BigDecimal
import java.math.BigInteger
import java.text.Normalizer
import java.util.Locale

data class PortfolioAssetKey(val chainId: Long, val symbol: String)
data class PortfolioAsset(val network: WalletNetwork, val asset: AssetInfo) {
    val key: PortfolioAssetKey get() = PortfolioAssetKey(network.chainId, asset.symbol)
    val label: String get() = "${asset.symbol} (${network.name})"
}
data class PortfolioTotal(val usdcUnits: BigInteger?, val stale: Boolean)
data class PortfolioHolding(val position: PortfolioAsset, val units: BigInteger, val stale: Boolean)
enum class PortfolioEmptyState { NONE, ZERO, SMALL_BALANCES_HIDDEN, UNKNOWN }
data class PortfolioHoldings(
    val visible: List<PortfolioHolding>,
    val unknown: Set<PortfolioAssetKey>,
    val hiddenSmallCount: Int,
    val stale: Boolean,
    val emptyState: PortfolioEmptyState
)

object WalletPortfolio {
    val chainOrder = NetworkPolicy.chainOrder
    const val SMALL_BALANCE_DESCRIPTION = "Hide each asset below your USD threshold using current Coinbase prices. Assets with unavailable or older prices remain visible."

    private fun validUnits(value: BigInteger?): Boolean = value != null && value.signum() >= 0 && value.bitLength() <= 256

    fun assets(networks: List<WalletNetwork>): List<PortfolioAsset> {
        if (networks.map { it.chainId }.toSet() != chainOrder.toSet() || networks.size != chainOrder.size) {
            throw WalletFailure("The portfolio network registry is invalid.")
        }
        return AssetSelection.assets(networks).flatMap { it.positions }
    }

    /** Home holdings only. Deposit and send pickers retain every supported asset/network pair. */
    fun holdings(networks: List<WalletNetwork>, balances: Map<Long, BalanceView>, states: Map<Long, String>,
                 hideSmallBalances: Boolean = false, thresholdUsd: BigDecimal = BigDecimal.ONE,
                 quotes: Map<String, UsdQuote> = emptyMap(), nowMillis: Long = System.currentTimeMillis()): PortfolioHoldings {
        val threshold = UsdPrices.checkedThreshold(thresholdUsd)
        val visible = mutableListOf<PortfolioHolding>()
        val unknown = linkedSetOf<PortfolioAssetKey>()
        var hiddenSmallCount = 0
        var stale = false
        for (position in assets(networks)) {
            val snapshot = balances[position.network.chainId]
            val units = snapshot?.balances?.get(position.asset.symbol)
            if (!validUnits(units)) { unknown.add(position.key); continue }
            val lastChecked = states[position.network.chainId] != "live" || snapshot!!.observedAt <= 0L
            stale = stale || lastChecked
            if (units == BigInteger.ZERO) continue
            val usd = if (hideSmallBalances) usdValue(position, units, quotes, nowMillis) else null
            if (!lastChecked && usd?.freshness == PriceFreshness.FRESH && usd.amount != null && usd.amount < threshold) hiddenSmallCount++
            else visible.add(PortfolioHolding(position, units!!, lastChecked))
        }
        val emptyState = when {
            visible.isNotEmpty() -> PortfolioEmptyState.NONE
            hiddenSmallCount > 0 -> PortfolioEmptyState.SMALL_BALANCES_HIDDEN
            unknown.isNotEmpty() -> PortfolioEmptyState.UNKNOWN
            else -> PortfolioEmptyState.ZERO
        }
        return PortfolioHoldings(visible, unknown, hiddenSmallCount, stale, emptyState)
    }

    fun usdValue(position: PortfolioAsset, units: BigInteger?, quotes: Map<String, UsdQuote>, nowMillis: Long): UsdValue =
        UsdPrices.value(position, units, quotes, nowMillis)

    /** Entire unfiltered portfolio. No price, failed read, or missing chain is silently valued at zero. */
    fun totalUsd(networks: List<WalletNetwork>, balances: Map<Long, BalanceView>, states: Map<Long, String>,
                 quotes: Map<String, UsdQuote>, nowMillis: Long): PortfolioUsdTotal {
        var subtotal = BigDecimal.ZERO
        var stale = false
        var valuedPositivePositions = 0
        var oldestQuote: Long? = null
        val missingBalances = linkedSetOf<PortfolioAssetKey>()
        val missingPrices = linkedSetOf<PortfolioAssetKey>()
        for (position in assets(networks)) {
            val snapshot = balances[position.network.chainId]
            val units = snapshot?.balances?.get(position.asset.symbol)
            if (!validUnits(units)) { missingBalances.add(position.key); continue }
            stale = stale || states[position.network.chainId] != "live" || snapshot!!.observedAt <= 0L
            val value = usdValue(position, units, quotes, nowMillis)
            if (value.amount == null) { missingPrices.add(position.key); continue }
            subtotal = subtotal.add(value.amount)
            if (units!!.signum() > 0) valuedPositivePositions++
            stale = stale || value.freshness == PriceFreshness.STALE
            value.fetchedAtMillis?.let { oldestQuote = oldestQuote?.coerceAtMost(it) ?: it }
        }
        val complete = missingBalances.isEmpty() && missingPrices.isEmpty()
        val coverage = when {
            complete -> UsdCoverage.COMPLETE
            valuedPositivePositions > 0 -> UsdCoverage.PARTIAL
            else -> UsdCoverage.UNAVAILABLE
        }
        return PortfolioUsdTotal(if (complete) subtotal else null, subtotal, coverage, stale, oldestQuote, missingBalances, missingPrices)
    }

    /** A token sum, never a USD valuation. A missing chain is never treated as zero. */
    fun totalUsdc(balances: Map<Long, BalanceView>, states: Map<Long, String>): PortfolioTotal {
        val usdcChains = chainOrder.filter { NetworkPolicy.supports(it, "USDC") }
        val snapshots = usdcChains.map { balances[it] }
        val amounts = snapshots.map { it?.balances?.get("USDC") }
        if (amounts.any { !validUnits(it) }) return PortfolioTotal(null, false)
        val total = amounts.fold(BigInteger.ZERO) { sum, value -> sum + value!! }
        val stale = usdcChains.any { states[it] != "live" || balances[it]?.observedAt == 0L }
        return PortfolioTotal(total, stale)
    }
}

/** A late read may update only the same unlocked account session, never a new screen's transfer context. */
data class PortfolioRefreshContext(val address: String, val epoch: Int) {
    fun accepts(chainId: Long, currentAddress: String?, currentEpoch: Int, unlocked: Boolean, resumed: Boolean): Boolean =
        chainId in WalletPortfolio.chainOrder && currentAddress != null && address.equals(currentAddress, true) &&
            currentEpoch == epoch && unlocked && resumed
}

data class TransferDraft(val recipient: String = "", val amount: String = "")
class TransferDrafts {
    private val values = HashMap<Pair<String, PortfolioAssetKey>, TransferDraft>()
    fun get(address: String, asset: PortfolioAssetKey): TransferDraft = values[address.lowercase(Locale.ROOT) to asset] ?: TransferDraft()
    fun save(address: String, asset: PortfolioAssetKey, draft: TransferDraft) { values[address.lowercase(Locale.ROOT) to asset] = draft }
}

data class DisplayNames(val wallet: String = "Wallet 1", val account: String = "Account 1")
object DisplayNameRules {
    fun normalize(input: String): String {
        if (input.length > 256) throw WalletFailure("Use a name of 1–32 characters.")
        var index = 0
        while (index < input.length) {
            val codePoint = input.codePointAt(index)
            if (Character.getType(codePoint) in setOf(Character.CONTROL.toInt(), Character.FORMAT.toInt(),
                    Character.SURROGATE.toInt(), Character.LINE_SEPARATOR.toInt(), Character.PARAGRAPH_SEPARATOR.toInt())) {
                throw WalletFailure("Names cannot contain control, direction-changing, or line-break characters.")
            }
            index += Character.charCount(codePoint)
        }
        val value = Normalizer.normalize(input.trim(), Normalizer.Form.NFC)
        if (value.codePointCount(0, value.length) !in 1..32) throw WalletFailure("Use a name of 1–32 characters.")
        return value
    }
    fun scope(address: String): String = Amounts.address(address).lowercase(Locale.ROOT)
    fun normalized(names: DisplayNames): DisplayNames = DisplayNames(normalize(names.wallet), normalize(names.account))
}

internal interface DisplayNamePersistence {
    fun read(addressScope: String): DisplayNames?
    fun write(addressScope: String, names: DisplayNames): Boolean
    fun readWallet(addressScope: String): String?
    fun writeWallet(addressScope: String, wallet: String): Boolean
}

/** Names are cosmetic metadata. No nickname enters address derivation, vault encryption, or signing. */
internal class WalletDisplayNames(private val persistence: DisplayNamePersistence) {
    /** Wallet-only UI must not depend on, validate, or overwrite hidden legacy account labels. */
    fun loadWallet(address: String): String {
        val scope = DisplayNameRules.scope(address)
        return runCatching { persistence.readWallet(scope)?.let(DisplayNameRules::normalize) }.getOrNull() ?: DisplayNames().wallet
    }
    fun saveWallet(address: String, wallet: String): String {
        val normalized = DisplayNameRules.normalize(wallet)
        val scope = DisplayNameRules.scope(address)
        if (!runCatching { persistence.writeWallet(scope, normalized) }.getOrDefault(false)) {
            throw WalletFailure("Wallet name could not be saved. Try again.")
        }
        return normalized
    }

    // Retained for compatibility with existing two-label metadata and older callers.
    fun load(address: String): DisplayNames {
        val scope = DisplayNameRules.scope(address)
        return runCatching { persistence.read(scope)?.let(DisplayNameRules::normalized) }.getOrNull() ?: DisplayNames()
    }
    fun save(address: String, names: DisplayNames): DisplayNames {
        val normalized = DisplayNameRules.normalized(names)
        if (!persistence.write(DisplayNameRules.scope(address), normalized)) throw WalletFailure("Display names could not be saved. Try again.")
        return normalized
    }
}

internal interface PortfolioPreferencePersistence {
    fun read(addressScope: String): Boolean?
    fun write(addressScope: String, hideSmallBalances: Boolean): Boolean
    fun readThreshold(addressScope: String): String?
    fun writeSettings(addressScope: String, hideSmallBalances: Boolean, thresholdUsd: String): Boolean
}

data class PortfolioDisplaySettings(val hideSmallBalances: Boolean = false, val thresholdUsd: BigDecimal = BigDecimal.ONE)

/** Cosmetic filtering is independent of vault creation, protection and transaction records. */
internal class WalletPortfolioPreferences(private val persistence: PortfolioPreferencePersistence) {
    fun loadSettings(address: String): PortfolioDisplaySettings {
        val scope = DisplayNameRules.scope(address)
        // Preserve the existing v0.6.1 toggle. The new explicit USD threshold defaults to $1.
        val enabled = runCatching { persistence.read(scope) }.getOrNull() ?: false
        val threshold = runCatching { persistence.readThreshold(scope)?.let(UsdPrices::parseThreshold) }.getOrNull() ?: BigDecimal.ONE
        return PortfolioDisplaySettings(enabled, threshold)
    }

    fun saveSettings(address: String, hideSmallBalances: Boolean, thresholdInput: String): PortfolioDisplaySettings {
        val scope = DisplayNameRules.scope(address)
        val threshold = UsdPrices.parseThreshold(thresholdInput)
        if (!runCatching { persistence.writeSettings(scope, hideSmallBalances, threshold.toPlainString()) }.getOrDefault(false)) {
            throw WalletFailure("The display setting could not be saved. Try again.")
        }
        return PortfolioDisplaySettings(hideSmallBalances, threshold)
    }

    // Compatibility with the older boolean-only preference API.
    fun load(address: String): Boolean {
        val scope = DisplayNameRules.scope(address)
        return runCatching { persistence.read(scope) }.getOrNull() ?: false
    }
    fun save(address: String, hideSmallBalances: Boolean): Boolean {
        val scope = DisplayNameRules.scope(address)
        if (!runCatching { persistence.write(scope, hideSmallBalances) }.getOrDefault(false)) {
            throw WalletFailure("The display setting could not be saved. Try again.")
        }
        return hideSmallBalances
    }
}
