package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test
import java.math.BigDecimal
import java.math.BigInteger

class UsdPricesTest {
    private val networks = TestNetworks.all
    private val live = TestNetworks.live
    private val now = 1_000_000L
    private val rollupZeros = TestNetworks.rollupZeros(now)
    private fun balance(usdc: String = "0", eurc: String = "0", eth: String = "0") =
        BalanceView(100, "0x" + "a".repeat(64), mapOf("USDC" to BigInteger(usdc), "EURC" to BigInteger(eurc), "ETH" to BigInteger(eth), "USDT" to BigInteger.ZERO), now)
    private fun quote(symbol: String, amount: String, received: Long = now) = UsdPrices.quote(symbol, amount, received)
    private fun asset(symbol: String) = WalletPortfolio.assets(networks).first { it.asset.symbol == symbol }
    private fun exact(expected: String, actual: BigDecimal?) = assertEquals(0, BigDecimal(expected).compareTo(actual))

    @Test fun bothNetworksAndAllThreeAssetsContributeExactDollarValuesWithoutAssumingAStablecoinPeg() {
        val balances = rollupZeros + mapOf(8453L to balance("10000", "1000000", "400000000000000"),
            1L to balance("1000000", "2000000", "800000000000000"))
        val quotes = mapOf("USDC" to quote("USDC", "0.98"), "EURC" to quote("EURC", "1.12"), "ETH" to quote("ETH", "2500"))
        val total = WalletPortfolio.totalUsd(networks, balances, live, quotes, now)
        exact("7.3498", total.amount)
        exact("7.3498", total.knownSubtotal)
        assertEquals(UsdCoverage.COMPLETE, total.coverage)
        assertFalse(total.incomplete)
        assertFalse(total.stale)
        assertEquals(now, total.oldestQuoteAtMillis)
        exact("0.0098", WalletPortfolio.usdValue(asset("USDC"), BigInteger("10000"), quotes, now).amount)
    }

    @Test fun tinyWeiAndSixDecimalStablecoinUnitsRemainExactUntilDisplayFormatting() {
        val quotes = mapOf("ETH" to quote("ETH", "2247.38"), "EURC" to quote("EURC", "1.1224251796805455"))
        exact("0.00000000000000224738", WalletPortfolio.usdValue(asset("ETH"), BigInteger.ONE, quotes, now).amount)
        exact("0.0000011224251796805455", WalletPortfolio.usdValue(asset("EURC"), BigInteger.ONE, quotes, now).amount)
        assertEquals("<\$0.01", UsdPrices.formatUsd(BigDecimal("0.009999999999999999")))
        assertEquals("\$0.00", UsdPrices.formatUsd(BigDecimal.ZERO))
        assertEquals("\$0.01", UsdPrices.formatUsd(BigDecimal("0.010000001")))
        assertEquals("\$12.35", UsdPrices.formatUsd(BigDecimal("12.345")))
    }

    @Test fun zeroBalancesNeedNoQuoteButMissingBalancesOrUnpricedPositiveAssetsNeverBecomeZeroTotals() {
        val zeros = rollupZeros + mapOf(8453L to balance(), 1L to balance())
        val empty = WalletPortfolio.totalUsd(networks, zeros, live, emptyMap(), now)
        exact("0", empty.amount)
        assertEquals(UsdCoverage.COMPLETE, empty.coverage)
        assertNull(empty.oldestQuoteAtMillis)

        val missingNetwork = WalletPortfolio.totalUsd(networks, rollupZeros + mapOf(8453L to balance()), live, emptyMap(), now)
        assertNull(missingNetwork.amount)
        assertEquals(UsdCoverage.UNAVAILABLE, missingNetwork.coverage)
        assertEquals(TestNetworks.ethereum.assets.size, missingNetwork.missingBalances.size)

        val unpriced = WalletPortfolio.totalUsd(networks, zeros + (8453L to balance(usdc = "10000")), live, emptyMap(), now)
        assertNull(unpriced.amount)
        assertEquals(UsdCoverage.UNAVAILABLE, unpriced.coverage)
        assertEquals(setOf(PortfolioAssetKey(8453L, "USDC")), unpriced.missingPrices)
    }

    @Test fun incompletePortfolioReturnsAnExplicitKnownSubtotalAndRetainsFreshnessOfItsEvidence() {
        val quotes = mapOf("USDC" to quote("USDC", "0.99", now - UsdPrices.FRESH_MILLIS - 1))
        val partial = WalletPortfolio.totalUsd(networks, rollupZeros + mapOf(8453L to balance("1000000", "1")), live, quotes, now)
        assertNull(partial.amount)
        exact("0.99", partial.knownSubtotal)
        assertEquals(UsdCoverage.PARTIAL, partial.coverage)
        assertTrue(partial.incomplete)
        assertTrue(partial.stale)
        assertEquals(now - UsdPrices.FRESH_MILLIS - 1, partial.oldestQuoteAtMillis)
        assertEquals(TestNetworks.ethereum.assets.size, partial.missingBalances.size)
        assertEquals(setOf(PortfolioAssetKey(8453L, "EURC")), partial.missingPrices)

        val oldBalances = WalletPortfolio.totalUsd(networks, rollupZeros + mapOf(8453L to balance(), 1L to balance()),
            live + (8453L to "stale"), emptyMap(), now)
        assertTrue(oldBalances.stale)
        exact("0", oldBalances.amount)
    }

    @Test fun quoteAgeHasExplicitFreshStaleExpiredAndFutureBoundaries() {
        val quote = quote("USDC", "0.9999")
        assertEquals(PriceFreshness.FRESH, UsdPrices.freshness(quote, now + UsdPrices.FRESH_MILLIS))
        assertEquals(PriceFreshness.STALE, UsdPrices.freshness(quote, now + UsdPrices.FRESH_MILLIS + 1))
        assertEquals(PriceFreshness.STALE, UsdPrices.freshness(quote, now + UsdPrices.MAX_CACHE_MILLIS))
        assertEquals(PriceFreshness.UNAVAILABLE, UsdPrices.freshness(quote, now + UsdPrices.MAX_CACHE_MILLIS + 1))
        assertEquals(PriceFreshness.UNAVAILABLE, UsdPrices.freshness(quote, now - 1))
        assertNull(WalletPortfolio.usdValue(asset("USDC"), BigInteger.ONE, mapOf("USDC" to quote), now - 1).amount)
    }

    @Test fun filterUsesUnroundedUsdValueAndAnAdjustableExclusiveThresholdRatherThanTokenQuantity() {
        val quotes = mapOf("USDC" to quote("USDC", "0.999999"), "EURC" to quote("EURC", "1.000001"), "ETH" to quote("ETH", "2000"))
        val balances = rollupZeros + mapOf(8453L to balance("1000000", "1000000", "500000000000000"), 1L to balance())
        val result = WalletPortfolio.holdings(networks, balances, live, true, BigDecimal.ONE, quotes, now)
        assertEquals(1, result.hiddenSmallCount)
        assertEquals(listOf("EURC", "ETH"), result.visible.map { it.position.asset.symbol })
        assertEquals("\$1.00", UsdPrices.formatUsd(WalletPortfolio.usdValue(asset("USDC"), BigInteger("1000000"), quotes, now).amount!!))
        assertEquals(3, WalletPortfolio.holdings(networks, balances, live, false, BigDecimal.ONE, quotes, now).visible.size)
        assertEquals(PortfolioEmptyState.SMALL_BALANCES_HIDDEN,
            WalletPortfolio.holdings(networks, balances, live, true, BigDecimal("2"), quotes, now).emptyState)
        exact("3", WalletPortfolio.totalUsd(networks, balances, live, quotes, now).amount)
    }

    @Test fun unknownOrStalePricesCannotHidePositiveAssetsOrCreateAnEmptyWalletClaim() {
        val balances = rollupZeros + mapOf(8453L to balance("10000", "1", "1"), 1L to balance())
        val quotes = mapOf("USDC" to quote("USDC", "1"), "ETH" to quote("ETH", "2000", now - UsdPrices.FRESH_MILLIS - 1))
        val filtered = WalletPortfolio.holdings(networks, balances, live, true, BigDecimal.ONE, quotes, now)
        assertEquals(listOf("EURC", "ETH"), filtered.visible.map { it.position.asset.symbol })
        assertEquals(1, filtered.hiddenSmallCount)
        assertEquals(PortfolioEmptyState.NONE, filtered.emptyState)
        assertEquals(3, WalletPortfolio.holdings(networks, balances, live, true, BigDecimal.ONE, emptyMap(), now).visible.size)
    }

    @Test fun staleBalancesRemainVisibleEvenWhenAFreshPriceWouldPutThemBelowTheThreshold() {
        val balances = rollupZeros + mapOf(8453L to balance(usdc = "10000"), 1L to balance())
        val quotes = mapOf("USDC" to quote("USDC", "1"))
        for (state in listOf("stale", "loading", "unavailable", "storage")) {
            val filtered = WalletPortfolio.holdings(networks, balances, live + (8453L to state), true, BigDecimal.ONE, quotes, now)
            assertEquals(PortfolioAssetKey(8453L, "USDC"), filtered.visible.single().position.key)
            assertTrue(filtered.visible.single().stale)
            assertEquals(0, filtered.hiddenSmallCount)
            assertEquals(PortfolioEmptyState.NONE, filtered.emptyState)
        }
    }

    @Test fun malformedPricesWrongAssetBindingsAndInvalidUnitsRemainUnavailable() {
        for (invalid in listOf("0", "-1", "1e0", "NaN", "Infinity", "01", "1.0000000000000000001", "1000000000001", "9".repeat(200))) {
            assertThrows(WalletFailure::class.java) { quote("USDC", invalid) }
        }
        assertThrows(WalletFailure::class.java) { quote("USD", "1") }
        val wrongAsset = mapOf("USDC" to quote("EURC", "1.12"))
        assertNull(WalletPortfolio.usdValue(asset("USDC"), BigInteger.ONE, wrongAsset, now).amount)
        for (invalid in listOf(BigInteger.valueOf(-1), BigInteger.ONE.shiftLeft(256))) {
            assertNull(WalletPortfolio.usdValue(asset("USDC"), invalid, emptyMap(), now).amount)
        }
        val max = BigInteger.ONE.shiftLeft(256).subtract(BigInteger.ONE)
        assertNotNull(WalletPortfolio.usdValue(asset("USDC"), max, mapOf("USDC" to quote("USDC", "1")), now).amount)
    }
}
