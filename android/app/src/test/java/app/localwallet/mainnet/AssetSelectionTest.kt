package app.localwallet.mainnet

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import uniffi.wallet_core.assetRegistry
import java.math.BigDecimal
import java.math.BigInteger

class AssetSelectionTest {
    private val networks = TestNetworks.evm
    private val expectedChains = mapOf("USDC" to listOf(8453L, 1L, 10L, 42161L, 43114L, 137L),
        "EURC" to listOf(8453L, 1L, 43114L), "USDT" to listOf(1L, 43114L),
        "ETH" to listOf(8453L, 1L, 10L, 42161L), "AVAX" to listOf(43114L), "POL" to listOf(137L), "BNB" to listOf(56L))

    @Test fun actualCoreRegistryProducesSevenCanonicalChoicesAndEighteenEvmPositions() {
        val nativeRegistry = JSONObject(assetRegistry()).getJSONArray("networks").objects().map { network ->
            WalletNetwork(network.getLong("chain_id"), network.getString("network"), network.getString("rpc_url"),
                network.getString("broadcast_url"), network.getString("explorer_url"),
                network.getJSONArray("assets").objects().map { AssetInfo(it.getString("symbol"), it.getInt("decimals"), it.optionalString("contract")) })
        }
        val choices = AssetSelection.assets(nativeRegistry)
        assertEquals(listOf("USDC", "EURC", "USDT", "ETH", "AVAX", "POL", "BNB"), choices.map { it.symbol })
        assertEquals(18, choices.flatMap { it.positions }.map { it.key }.toSet().size)
        for (choice in choices) {
            assertEquals(expectedChains.getValue(choice.symbol), choice.positions.map { it.network.chainId })
            assertTrue(choice.positions.all { it.asset.symbol == choice.symbol })
        }
        assertNotEquals(choices[0].positions[0].asset.contract!!.lowercase(), choices[0].positions[1].asset.contract!!.lowercase())
        assertNotEquals(choices[1].positions[0].asset.contract!!.lowercase(), choices[1].positions[1].asset.contract!!.lowercase())
        assertTrue(choices.first { it.symbol == "ETH" }.positions.all { it.asset.contract == null && it.asset.decimals == 18 })
    }

    @Test fun assetAndNetworkOrderingRemainStableWhenRegistryEntriesAreReordered() {
        val shuffled = networks.reversed().map { it.copy(assets = it.assets.reversed()) }
        val original = AssetSelection.assets(networks)
        val reordered = AssetSelection.assets(shuffled)
        assertEquals(original.map { it.symbol }, reordered.map { it.symbol })
        assertEquals(original.flatMap { it.positions }.map { it.key }, reordered.flatMap { it.positions }.map { it.key })
        for (symbol in listOf("USDC", "EURC", "ETH")) {
            assertEquals(expectedChains.getValue(symbol), AssetSelection.networksFor(shuffled, symbol).map { it.chainId })
        }
    }

    @Test fun supportedNetworkChoicesAreDerivedPerCanonicalAssetWithoutInventingMissingPositions() {
        val subset = listOf(
            TestNetworks.base.copy(assets = TestNetworks.base.assets.filter { it.symbol != "EURC" }),
            TestNetworks.ethereum.copy(assets = TestNetworks.ethereum.assets.filter { it.symbol != "USDC" })
        )
        assertEquals(listOf("USDC", "EURC", "USDT", "ETH"), AssetSelection.assets(subset).map { it.symbol })
        assertEquals(listOf(8453L), AssetSelection.networksFor(subset, "USDC").map { it.chainId })
        assertEquals(listOf(1L), AssetSelection.networksFor(subset, "EURC").map { it.chainId })
        assertEquals(listOf(8453L, 1L), AssetSelection.networksFor(subset, "ETH").map { it.chainId })
        assertThrows(WalletFailure::class.java) { AssetSelection.resolve(subset, "USDC", 1L) }
        assertThrows(WalletFailure::class.java) { AssetSelection.resolve(subset, "EURC", 8453L) }
        val oneNetwork = listOf(TestNetworks.base.copy(assets = listOf(TestNetworks.base.asset("ETH"))))
        assertEquals(listOf("ETH"), AssetSelection.assets(oneNetwork).map { it.symbol })
        assertEquals(listOf(8453L), AssetSelection.networksFor(oneNetwork, "ETH").map { it.chainId })
        assertThrows(WalletFailure::class.java) { AssetSelection.networksFor(oneNetwork, "USDC") }
    }

    @Test fun genericSelectionMustRemainUnchosenUntilAnExplicitSupportedNetworkIsProvided() {
        for (symbol in listOf("USDC", "EURC", "ETH")) {
            assertNull(AssetSelection.resolve(networks, symbol, null))
            val failure = assertThrows(WalletFailure::class.java) { AssetSelection.requireSelected(networks, symbol, null) }
            assertEquals("Choose a network to continue.", failure.message)
            for (chain in listOf(0L, -1L, 84532L, 80002L)) {
                assertThrows(WalletFailure::class.java) { AssetSelection.resolve(networks, symbol, chain) }
            }
        }
        for (symbol in listOf("", "DAI", "USDC.e", "usdc", "USD\u0421")) {
            assertThrows(WalletFailure::class.java) { AssetSelection.resolve(networks, symbol, null) }
            assertThrows(WalletFailure::class.java) { AssetSelection.resolve(networks, symbol, 8453L) }
        }
    }

    @Test fun priorExplicitSelectionCannotLeakIntoAClearedOrChangedAssetAndCrossChainContractsStayBound() {
        val base = AssetSelection.requireSelected(networks, "USDC", 8453L)
        assertEquals(PortfolioAssetKey(8453L, "USDC"), base.key)
        assertNull(AssetSelection.resolve(networks, "USDC", null))
        assertNull(AssetSelection.resolve(networks, "ETH", null))
        val ethereum = AssetSelection.requireSelected(networks, "USDC", 1L)
        assertEquals(PortfolioAssetKey(1L, "USDC"), ethereum.key)
        assertNotEquals(base.asset.contract!!.lowercase(), ethereum.asset.contract!!.lowercase())
        assertEquals(base, AssetSelection.requireSelected(networks, "USDC", 8453L))
        assertEquals(PortfolioAssetKey(8453L, "EURC"), AssetSelection.requireSelected(networks, "EURC", 8453L).key)
        assertNull(AssetSelection.resolve(networks, "USDC", null))
    }

    @Test fun tickerSpoofsWrongChainContractsAndIncorrectNativeOrDecimalIdentitiesAreRejected() {
        val valid = TestNetworks.base.asset("USDC")
        val wrongAssets = listOf(
            valid.copy(contract = TestNetworks.ethereum.asset("USDC").contract),
            valid.copy(contract = TestNetworks.base.asset("EURC").contract),
            valid.copy(contract = "0x" + "1".repeat(40)),
            valid.copy(contract = null), valid.copy(decimals = 18), valid.copy(symbol = "USDC.e"),
            valid.copy(contract = valid.contract + " "),
            AssetInfo("ETH", 18, valid.contract), AssetInfo("ETH", 6, null), AssetInfo("EURC", 18, TestNetworks.base.asset("EURC").contract)
        )
        for (wrong in wrongAssets) {
            val registry = listOf(TestNetworks.base.copy(assets = listOf(wrong)), TestNetworks.ethereum)
            assertThrows(WalletFailure::class.java) { AssetSelection.assets(registry) }
            assertThrows(WalletFailure::class.java) { AssetSelection.resolve(registry, "USDC", null) }
        }
        val sameAddressBytes = TestNetworks.base.copy(assets = listOf(valid.copy(contract = "0x" + valid.contract!!.substring(2).uppercase())))
        assertEquals(listOf("USDC"), AssetSelection.assets(listOf(sameAddressBytes)).map { it.symbol })
    }

    @Test fun duplicateOrUnsupportedRegistryEntriesCannotChooseTheFirstEntrySilently() {
        val duplicateAsset = TestNetworks.base.copy(assets = listOf(TestNetworks.base.asset("USDC"), TestNetworks.base.asset("USDC"), TestNetworks.base.asset("ETH")))
        val invalid = listOf(
            emptyList(), listOf(TestNetworks.base, TestNetworks.base), listOf(TestNetworks.base, TestNetworks.ethereum, TestNetworks.base),
            listOf(TestNetworks.base.copy(chainId = 84532L)), listOf(TestNetworks.base.copy(name = "Ethereum")),
            listOf(TestNetworks.base.copy(assets = emptyList())), listOf(duplicateAsset),
            listOf(TestNetworks.base.copy(assets = listOf(AssetInfo("DAI", 18, "0x" + "1".repeat(40)))))
        )
        for (registry in invalid) assertThrows(WalletFailure::class.java) { AssetSelection.assets(registry) }
    }

    @Test fun emptyOrFilteredHoldingsNeverRemoveDepositOrSendAssetAndNetworkChoices() {
        val supported = AssetSelection.assets(networks)
        fun balance(units: BigInteger) = BalanceView(100, "0x" + "a".repeat(64),
            listOf("USDC", "EURC", "ETH").associateWith { units }, 1000L)
        val live = TestNetworks.live
        val quotes = mapOf("USDC" to UsdPrices.quote("USDC", "1", 1000L),
            "EURC" to UsdPrices.quote("EURC", "1.12", 1000L), "ETH" to UsdPrices.quote("ETH", "2000", 1000L))
        for (units in listOf(BigInteger.ZERO, BigInteger.ONE)) {
            val balances = TestNetworks.rollupZeros() + mapOf(8453L to balance(units), 1L to balance(units))
            assertTrue(WalletPortfolio.holdings(TestNetworks.all, balances, live, true, BigDecimal.ONE, quotes, 1000L).visible.isEmpty())
            assertEquals(supported, AssetSelection.assets(networks))
            for (choice in supported) assertEquals(expectedChains.getValue(choice.symbol).size, AssetSelection.networksFor(networks, choice.symbol).size)
        }
    }
}
