package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test
import java.math.BigInteger

class WalletPortfolioTest {
    private val address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
    private fun balance(usdc: String, observedAt: Long = 1_000) = BalanceView(100, "0x" + "a".repeat(64),
        mapOf("USDC" to BigInteger(usdc), "EURC" to BigInteger("999999999999"), "ETH" to BigInteger("999999999999999999999")), observedAt)
    private val live = TestNetworks.live
    private val rollupZeros = TestNetworks.rollupZeros()

    @Test fun allRowsHaveDistinctChainAssetIdentityAndCorrectContract() {
        val rows = WalletPortfolio.assets(TestNetworks.all)
        val expected = AssetSelection.assets(TestNetworks.all).flatMap { it.positions }.map { it.key }
        assertEquals(expected, rows.map { it.key })
        assertEquals(TestNetworks.all.sumOf { it.assets.size }, rows.map { it.key }.toSet().size)
        assertNotEquals(rows[0].asset.contract!!.lowercase(), rows[1].asset.contract!!.lowercase())
        assertEquals(8453L, rows[0].key.chainId)
        assertEquals(1L, rows[1].key.chainId)
        assertEquals(6, rows[0].asset.decimals)
        assertEquals(18, rows.last().asset.decimals)
    }

    @Test fun totalUsdcUsesExactIntegersAndNeverAddsEthOrEurc() {
        val sum = WalletPortfolio.totalUsdc(rollupZeros + mapOf(8453L to balance("9007199254740993"), 1L to balance("10001")), live)
        assertEquals(BigInteger("9007199254750994"), sum.usdcUnits)
        assertFalse(sum.stale)
        assertEquals("9007199254.750994", Amounts.display(sum.usdcUnits!!, 6))
    }

    @Test fun missingNetworkOrMissingUsdcNeverBecomesZero() {
        assertNull(WalletPortfolio.totalUsdc(emptyMap(), live).usdcUnits)
        assertNull(WalletPortfolio.totalUsdc(rollupZeros + mapOf(8453L to balance("10000")), live).usdcUnits)
        assertNull(WalletPortfolio.totalUsdc(rollupZeros + mapOf(1L to balance("10000")), live).usdcUnits)
        val missingToken = balance("0").copy(balances = mapOf("ETH" to BigInteger.ZERO, "EURC" to BigInteger.ZERO))
        assertNull(WalletPortfolio.totalUsdc(rollupZeros + mapOf(8453L to balance("10000"), 1L to missingToken), live).usdcUnits)
    }

    @Test fun observedZeroIsZeroAndStaleValuesStayExplicitlyStale() {
        assertEquals(BigInteger.ZERO, WalletPortfolio.totalUsdc(rollupZeros + mapOf(8453L to balance("0"), 1L to balance("0")), live).usdcUnits)
        val cached = WalletPortfolio.totalUsdc(rollupZeros + mapOf(8453L to balance("10000", 0), 1L to balance("0")), live)
        assertEquals(BigInteger("10000"), cached.usdcUnits)
        assertTrue(cached.stale)
        val offline = WalletPortfolio.totalUsdc(rollupZeros + mapOf(8453L to balance("10000"), 1L to balance("0")), live + (1L to "unavailable"))
        assertEquals(BigInteger("10000"), offline.usdcUnits)
        assertTrue(offline.stale)
        assertNull(WalletPortfolio.totalUsdc(rollupZeros + mapOf(8453L to balance("-1"), 1L to balance("0")), live).usdcUnits)
    }

    @Test fun lateReadsCannotUpdateAnotherAccountLockedSessionOrUnsupportedChain() {
        val context = PortfolioRefreshContext(address, 8)
        assertTrue(context.accepts(8453, address.lowercase(), 8, unlocked = true, resumed = true))
        assertTrue(context.accepts(1, address, 8, unlocked = true, resumed = true))
        assertFalse(context.accepts(8453, address, 9, unlocked = true, resumed = true))
        assertFalse(context.accepts(8453, address, 8, unlocked = false, resumed = true))
        assertFalse(context.accepts(8453, address, 8, unlocked = true, resumed = false))
        assertFalse(context.accepts(8453, null, 8, unlocked = true, resumed = true))
        assertFalse(context.accepts(8453, "0x1111111111111111111111111111111111111111", 8, unlocked = true, resumed = true))
        assertFalse(context.accepts(84532, address, 8, unlocked = true, resumed = true))
    }

    @Test fun transferDraftsStayBoundToAccountNetworkAndAsset() {
        val drafts = TransferDrafts()
        val base = PortfolioAssetKey(8453, "USDC")
        val ethereum = PortfolioAssetKey(1, "USDC")
        drafts.save(address, base, TransferDraft("0x1111111111111111111111111111111111111111", "0.01"))
        drafts.save(address, ethereum, TransferDraft("0x2222222222222222222222222222222222222222", "5"))
        assertEquals("0.01", drafts.get(address.lowercase(), base).amount)
        assertEquals("5", drafts.get(address, ethereum).amount)
        assertEquals(TransferDraft(), drafts.get(address, PortfolioAssetKey(8453, "ETH")))
        assertEquals(TransferDraft(), drafts.get("0x1111111111111111111111111111111111111111", base))
        // Quiet portfolio reads are independent of the form's selected draft.
        WalletPortfolio.totalUsdc(rollupZeros + mapOf(8453L to balance("20000"), 1L to balance("0")), live)
        assertEquals("0.01", drafts.get(address, base).amount)
        assertEquals("0x2222222222222222222222222222222222222222", drafts.get(address, ethereum).recipient)
    }

    @Test fun incompleteDuplicateOrUnsupportedNetworkRegistryDoesNotProduceChoices() {
        assertThrows(WalletFailure::class.java) { WalletPortfolio.assets(listOf(TestNetworks.base)) }
        assertThrows(WalletFailure::class.java) { WalletPortfolio.assets(listOf(TestNetworks.base, TestNetworks.base)) }
        assertThrows(WalletFailure::class.java) { WalletPortfolio.assets(listOf(TestNetworks.base, TestNetworks.ethereum.copy(chainId = 84532))) }
    }
}
