package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test
import java.math.BigDecimal
import java.math.BigInteger

/** Local IDs and asset groups must never erase protocol or token identity. */
class FamilyNetworkPolicyTest {
    @Test fun allNineNetworksShareOneAssetPickerWithExplicitNetworkSelection() {
        val choices = AssetSelection.assets(TestNetworks.all)
        assertEquals(9, TestNetworks.all.size)
        assertEquals(9, choices.size)
        assertEquals(24, choices.sumOf { it.positions.size })
        assertEquals(7, AssetSelection.networksFor(TestNetworks.all, "USDC").size)
        assertEquals(4, AssetSelection.networksFor(TestNetworks.all, "EURC").size)
        assertEquals(4, AssetSelection.networksFor(TestNetworks.all, "USDT").size)
        for (choice in choices) assertNull(AssetSelection.resolve(TestNetworks.all, choice.symbol, null))
    }
    @Test fun protocolFamilyKeysCannotEnterEvmFeeOrRpcPolicies() {
        for (network in FamilyNetworks.all) {
            assertTrue(network.chainId < 0)
            assertFalse(network.chainId in NetworkPolicy.evmChainOrder)
            assertThrows(WalletFailure::class.java) { NetworkPolicy.feeProfile(network.chainId) }
            assertThrows(WalletFailure::class.java) { NetworkClient.endpoints(network.chainId) }
        }
    }
    @Test fun base58TokenIdentitiesAreCaseSensitiveAndCannotCrossNetworks() {
        val solana = FamilyNetworks.all.first { it.chainId == FamilyNetworks.SOLANA }
        val tron = FamilyNetworks.all.first { it.chainId == FamilyNetworks.TRON }
        val usdc = solana.asset("USDC")
        assertThrows(WalletFailure::class.java) { AssetSelection.assets(listOf(solana.copy(assets = listOf(usdc.copy(contract = usdc.contract!!.lowercase()))))) }
        assertThrows(WalletFailure::class.java) { AssetSelection.assets(listOf(tron.copy(assets = listOf(solana.asset("USDT"))))) }
        assertFalse(NetworkPolicy.supports(FamilyNetworks.TRON, "USDC"))
        assertFalse(NetworkPolicy.supports(56, "USDC"))
        assertFalse(NetworkPolicy.supports(56, "USDT"))
    }
    @Test fun nativePrecisionIsIndependentFromEvmDecimals() {
        assertEquals(9, NetworkPolicy.decimals(FamilyNetworks.SOLANA, "SOL"))
        assertEquals(6, NetworkPolicy.decimals(FamilyNetworks.TRON, "TRX"))
        assertEquals(BigInteger("1234567891"), Amounts.parse("1.234567891", 9))
        assertEquals(BigInteger("1234567"), Amounts.parse("1.234567", 6))
        assertThrows(WalletFailure::class.java) { Amounts.parse("1.2345678", 6) }
    }
    @Test fun unsupportedUsdPriceDoesNotHideOrValuePositiveNativeBalanceAtZero() {
        val network = FamilyNetworks.all.first { it.chainId == FamilyNetworks.TRON }
        val balances = TestNetworks.all.associate { it.chainId to BalanceView(100,"fixture",it.assets.associate { a -> a.symbol to BigInteger.ZERO },1000) }.toMutableMap()
        balances[network.chainId] = balances.getValue(network.chainId).copy(balances = mapOf("TRX" to BigInteger.ONE,"USDT" to BigInteger.ZERO))
        val total = WalletPortfolio.totalUsd(TestNetworks.all, balances, TestNetworks.live, emptyMap(),1000)
        assertNull(total.amount)
        assertEquals(setOf(PortfolioAssetKey(FamilyNetworks.TRON,"TRX")),total.missingPrices)
        val visible = WalletPortfolio.holdings(TestNetworks.all,balances,TestNetworks.live,true,BigDecimal.ONE,emptyMap(),1000)
        assertEquals("TRX",visible.visible.single().position.asset.symbol)
    }
    @Test fun publicResumeRoutesPreserveFamilyAndAssetWithoutRestoringApproval() {
        val identity = "0x1111111111111111111111111111111111111111"
        for (network in FamilyNetworks.all) {
            val route = WalletNavigation.capture(identity,"send",network.chainId,network.nativeSymbol,"send")!!
            assertEquals(route,WalletNavigation.decode(WalletNavigation.encode(route)))
            assertEquals(WalletDestination.HISTORY,WalletNavigation.capture(identity,"review",network.chainId,network.nativeSymbol,"send")!!.destination)
        }
    }
}
