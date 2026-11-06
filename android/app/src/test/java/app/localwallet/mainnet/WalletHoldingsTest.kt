package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test
import java.math.BigInteger

class WalletHoldingsTest {
    private val networks = TestNetworks.all
    private val live = TestNetworks.live
    private val rollupZeros = TestNetworks.rollupZeros()
    private val quotes = mapOf("USDC" to UsdPrices.quote("USDC", "1", 1000L), "EURC" to UsdPrices.quote("EURC", "1", 1000L),
        "ETH" to UsdPrices.quote("ETH", "10000", 1000L))
    private fun balance(usdc: String = "0", eurc: String = "0", eth: String = "0", observedAt: Long = 1000L) =
        BalanceView(100, "0x" + "a".repeat(64), mapOf("USDC" to BigInteger(usdc), "EURC" to BigInteger(eurc), "ETH" to BigInteger(eth), "USDT" to BigInteger.ZERO), observedAt)
    private fun holdings(base: BalanceView? = balance(), ethereum: BalanceView? = balance(), hideSmall: Boolean = false,
                         states: Map<Long, String> = live): PortfolioHoldings =
        WalletPortfolio.holdings(networks, rollupZeros + listOfNotNull(base?.let { 8453L to it }, ethereum?.let { 1L to it }).toMap(), live + states,
            hideSmall, quotes = quotes, nowMillis = 1000L)

    @Test fun defaultListOmitsZerosButRetainsThePointZeroOneDepositAndEveryPositiveAtomicUnit() {
        val result = holdings(base = balance(usdc = "10000", eurc = "1", eth = "1"))
        assertEquals(listOf(PortfolioAssetKey(8453, "USDC"), PortfolioAssetKey(8453, "EURC"), PortfolioAssetKey(8453, "ETH")), result.visible.map { it.position.key })
        assertEquals(listOf(BigInteger("10000"), BigInteger.ONE, BigInteger.ONE), result.visible.map { it.units })
        assertEquals("0.01", Amounts.display(result.visible[0].units, 6))
        assertEquals("0.000001", Amounts.display(result.visible[1].units, 6))
        assertEquals("0.000000000000000001", Amounts.display(result.visible[2].units, 18))
        assertEquals(0, result.hiddenSmallCount)
        assertTrue(result.unknown.isEmpty())
        assertEquals(PortfolioEmptyState.NONE, result.emptyState)
        assertFalse(result.stale)
    }

    @Test fun oneDollarThresholdIsExactExclusiveAndAppliedToEachNetworkPositionUsingItsPrice() {
        val result = holdings(base = balance("999999", "1000000", "99999999999999"),
            ethereum = balance("1000000", "999999", "100000000000000"), hideSmall = true)
        assertEquals(listOf(PortfolioAssetKey(1, "USDC"), PortfolioAssetKey(8453, "EURC"), PortfolioAssetKey(1, "ETH")), result.visible.map { it.position.key })
        assertEquals(listOf(BigInteger("1000000"), BigInteger("1000000"), BigInteger("100000000000000")), result.visible.map { it.units })
        assertEquals(3, result.hiddenSmallCount)
        assertTrue(result.unknown.isEmpty())
        assertEquals(PortfolioEmptyState.NONE, result.emptyState)
        val above = holdings(base = balance("1000001", "1000001", "100000000000001"), hideSmall = true)
        assertEquals(3, above.visible.size)
        assertEquals(0, above.hiddenSmallCount)
        assertTrue(WalletPortfolio.SMALL_BALANCE_DESCRIPTION.contains("USD threshold"))
        assertTrue(WalletPortfolio.SMALL_BALANCE_DESCRIPTION.contains("Coinbase"))
    }

    @Test fun zeroSmallHiddenAndIncompleteCoverageHaveDifferentEmptyStates() {
        val empty = holdings()
        assertEquals(PortfolioEmptyState.ZERO, empty.emptyState)
        assertTrue(empty.visible.isEmpty())
        assertTrue(empty.unknown.isEmpty())
        assertEquals(0, empty.hiddenSmallCount)

        val small = holdings(base = balance(usdc = "10000"), hideSmall = true)
        assertEquals(PortfolioEmptyState.SMALL_BALANCES_HIDDEN, small.emptyState)
        assertTrue(small.visible.isEmpty())
        assertEquals(1, small.hiddenSmallCount)
        assertTrue(small.unknown.isEmpty())

        val partialSmall = holdings(base = balance(usdc = "10000"), ethereum = null, hideSmall = true)
        assertEquals(PortfolioEmptyState.SMALL_BALANCES_HIDDEN, partialSmall.emptyState)
        assertEquals(TestNetworks.ethereum.assets.size, partialSmall.unknown.size)
        assertTrue(partialSmall.unknown.all { it.chainId == 1L })

        for (partial in listOf(holdings(base = null, ethereum = null), holdings(ethereum = null),
            holdings(base = balance().copy(balances = mapOf("USDC" to BigInteger.ZERO))))) {
            assertEquals(PortfolioEmptyState.UNKNOWN, partial.emptyState)
            assertTrue(partial.unknown.isNotEmpty())
            assertTrue(partial.visible.isEmpty())
        }
    }

    @Test fun stalePositiveHoldingsStayVisibleAndCachedZerosVanishWithoutClaimingCurrentZero() {
        val stale = holdings(base = balance(usdc = "10000", observedAt = 0),
            states = mapOf(8453L to "unavailable", 1L to "live"))
        assertEquals(BigInteger("10000"), stale.visible.single().units)
        assertTrue(stale.visible.single().stale)
        assertTrue(stale.stale)
        assertTrue(stale.unknown.isEmpty())
        assertEquals(PortfolioEmptyState.NONE, stale.emptyState)

        val oldZero = holdings(base = balance(observedAt = 0), states = mapOf(8453L to "stale", 1L to "live"))
        assertTrue(oldZero.visible.isEmpty())
        assertTrue(oldZero.stale)
        assertEquals(PortfolioEmptyState.ZERO, oldZero.emptyState)

        val partial = holdings(base = balance(usdc = "10000", observedAt = 0), ethereum = null)
        assertEquals(PortfolioEmptyState.NONE, partial.emptyState)
        assertEquals(TestNetworks.ethereum.assets.size, partial.unknown.size)
        assertTrue(partial.stale)
    }

    @Test fun invalidQuantitiesAreUnknownRatherThanZeroAndMaximumValidUnitsStayExact() {
        val tooLarge = BigInteger.ONE.shiftLeft(256)
        val invalid = holdings(base = balance(usdc = "-1", eurc = tooLarge.toString()))
        assertEquals(setOf(PortfolioAssetKey(8453, "USDC"), PortfolioAssetKey(8453, "EURC")), invalid.unknown)
        assertEquals(PortfolioEmptyState.UNKNOWN, invalid.emptyState)
        assertTrue(invalid.visible.isEmpty())
        assertNull(WalletPortfolio.totalUsdc(mapOf(8453L to balance(usdc = tooLarge.toString()), 1L to balance()), live).usdcUnits)

        val maximum = tooLarge.subtract(BigInteger.ONE)
        val large = holdings(base = balance(usdc = maximum.toString()), hideSmall = true)
        assertEquals(maximum, large.visible.single().units)
        assertEquals(0, large.hiddenSmallCount)
        assertTrue(large.unknown.isEmpty())
    }

    @Test fun filteringDoesNotModifyBalancesTotalsDraftsOrSupportedDepositAndSendChoices() {
        val snapshots = rollupZeros + mapOf(8453L to balance(usdc = "10000"), 1L to balance())
        val before = snapshots.toString()
        val choices = WalletPortfolio.assets(networks)
        val drafts = TransferDrafts()
        val address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
        val key = PortfolioAssetKey(8453, "USDC")
        drafts.save(address, key, TransferDraft("0x1111111111111111111111111111111111111111", "0.01"))
        assertTrue(WalletPortfolio.holdings(networks, snapshots, live, true, quotes = quotes, nowMillis = 1000L).visible.isEmpty())
        assertEquals(before, snapshots.toString())
        assertEquals(BigInteger("10000"), WalletPortfolio.totalUsdc(snapshots, live).usdcUnits)
        assertEquals(choices, WalletPortfolio.assets(networks))
        assertEquals(TestNetworks.all.sumOf { it.assets.size }, WalletPortfolio.assets(networks).size)
        assertEquals("0.01", drafts.get(address, key).amount)
        assertEquals(PortfolioAssetKey(8453, "USDC"), WalletPortfolio.holdings(networks, snapshots, live).visible.single().position.key)
    }
}
