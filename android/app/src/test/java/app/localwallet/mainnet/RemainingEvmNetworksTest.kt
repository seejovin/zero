package app.localwallet.mainnet

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import java.io.IOException
import java.math.BigDecimal
import java.math.BigInteger
import java.net.SocketTimeoutException

/** Offline synthetic public observations. No real mnemonic, RPC call or submitted payment. */
class RemainingEvmNetworksTest {
    private val sender = "0x1111111111111111111111111111111111111111"
    private val recipient = "0x2222222222222222222222222222222222222222"
    private val hash = "0x" + "a".repeat(64)
    private val newNetworks = listOf(TestNetworks.avalanche, TestNetworks.polygon, TestNetworks.bnb)
    private val balance = BigInteger("1000000000000000001")

    @Before fun reset() = NetworkClient.clearPreferenceForTests()

    private fun block(network: WalletNetwork) = JSONObject().put("number", "0x64").put("hash", hash)
        .put("baseFeePerGas", if (network.chainId == 56L) "0x0" else "0x4c4b40")

    private fun values(network: WalletNetwork, native: BigInteger = balance): Map<String, BigInteger> =
        network.assets.associate { it.symbol to if (it.contract == null) native else BigInteger("123456789") }

    private fun rpc(network: WalletNetwork, methods: MutableList<String> = mutableListOf(), tokenResult: String? = null,
                    gasPrice: String = "0x3b9aca00", estimate: Long = 21_000L): NetworkClient =
        NetworkClient(network, RpcTransport { _, method, params ->
            methods.add(method)
            when (method) {
                "eth_chainId" -> Amounts.hex(BigInteger.valueOf(network.chainId))
                "eth_gasPrice" -> { assertEquals(56L, network.chainId); gasPrice }
                "eth_maxPriorityFeePerGas" -> { assertNotEquals(56L, network.chainId); "0xf4240" }
                "eth_estimateGas" -> Amounts.hex(BigInteger.valueOf(estimate))
                "eth_call" -> {
                    val tx = params.getJSONObject(0)
                    assertFalse(tx.getString("to").equals(NetworkClient.ORACLE, true))
                    if (tx.getString("data") == "0x") "0x" else tokenResult ?: ("0x" + Amounts.word(BigInteger.ONE))
                }
                else -> throw AssertionError("Unexpected method $method")
            }
        })

    @Test fun nativeCurrencyIdentityIsChainSpecificAndBnbDoesNotInventPeggedStablecoins() {
        for ((network, symbol) in listOf(TestNetworks.avalanche to "AVAX", TestNetworks.polygon to "POL", TestNetworks.bnb to "BNB")) {
            assertEquals(symbol, network.nativeSymbol)
            assertEquals(AssetInfo(symbol, 18, null), network.nativeAsset)
            assertThrows(WalletFailure::class.java) { network.asset("ETH") }
            assertEquals(symbol, AssetSelection.assets(listOf(network)).single { it.positions.single().asset.contract == null }.symbol)
        }
        assertEquals(setOf("BNB"), TestNetworks.bnb.assets.map { it.symbol }.toSet())
        for (symbol in listOf("USDC", "EURC", "USDT")) {
            assertThrows(WalletFailure::class.java) { TestNetworks.bnb.asset(symbol) }
            assertThrows(WalletFailure::class.java) { AssetSelection.resolve(TestNetworks.evm, symbol, 56L) }
        }
        assertEquals(listOf(1L, 43114L), AssetSelection.networksFor(TestNetworks.evm, "USDT").map { it.chainId })
        assertThrows(WalletFailure::class.java) { AssetSelection.resolve(TestNetworks.evm, "EURC", 137L) }
        assertThrows(WalletFailure::class.java) { AssetSelection.assets(listOf(TestNetworks.polygon.copy(assets = listOf(AssetInfo("POL", 6, null))))) }
    }

    @Test fun nativeMaxUsesEveryAtomicUnitAfterTheCorrectGasCurrencyReserve() {
        for (network in newNetworks) {
            val methods = mutableListOf<String>()
            val result = MaxTransferCalculator.calculate(network, sender, network.nativeSymbol, recipient,
                block(network), values(network), rpc(network, methods))
            assertEquals(balance, result.units + result.fee.reserve)
            assertEquals(result.units, Amounts.parse(result.amount, 18))
            assertEquals(18, result.amount.substringAfter('.').length)
            assertEquals(balance, result.fee.nativeAvailable)
            assertEquals(balance, result.fee.nativeNeeded)
            assertTrue(result.fee.note.contains(network.nativeSymbol))
            assertEquals(BigInteger.ZERO, result.fee.l1Estimate)
            assertEquals(BigInteger.ZERO, result.fee.operatorEstimate)
            assertFalse(methods.any { it.contains("send", true) || it.contains("sign", true) })
        }
    }

    @Test fun tokenMaxPreservesAllSixDecimalsAndRequiresNativeGasNotAnotherNetworksEth() {
        for ((network, symbol) in listOf(TestNetworks.avalanche to "USDT", TestNetworks.avalanche to "USDC", TestNetworks.polygon to "USDC")) {
            val funds = values(network, BigInteger.ZERO) + ("ETH" to balance)
            val result = MaxTransferCalculator.calculate(network, sender, symbol, recipient, block(network), funds, rpc(network))
            assertEquals("123.456789", result.amount)
            assertEquals(BigInteger("123456789"), result.units)
            assertEquals(BigInteger.ZERO, result.fee.nativeAvailable)
            assertEquals(result.fee.reserve, result.fee.shortfall)
            assertTrue(result.fee.transferAvailable)
        }
    }

    @Test fun bnbTypeTwoQuoteUsesGasPriceAndRejectsUnreviewedBaseFeeChanges() {
        val network = TestNetworks.bnb
        val methods = mutableListOf<String>()
        val fee = FeeEstimator.estimate(network, sender, "BNB", recipient, "0.1", block(network), values(network), rpc(network, methods), true)
        assertTrue("eth_gasPrice" in methods)
        assertFalse("eth_maxPriorityFeePerGas" in methods)
        assertEquals(BigInteger("25200000000000"), fee.reserve)
        assertEquals(BigInteger("21000000000000"), fee.totalEstimate)
        assertThrows(WalletFailure::class.java) {
            FeeEstimator.estimate(network, sender, "BNB", recipient, "0.1", block(network).put("baseFeePerGas", "0x1"), values(network), rpc(network), true)
        }
        assertThrows(WalletFailure::class.java) {
            FeeEstimator.estimate(network, sender, "BNB", recipient, "0.1", block(network), values(network), rpc(network, gasPrice = "0x2540be401"), true)
        }
    }

    @Test fun onlyCanonicalEthereumUsdtMayHaveAnEmptyTransferResult() {
        FeeEstimator.validateTokenResult(TestNetworks.ethereum, TestNetworks.ethereum.asset("USDT"), "0x")
        FeeEstimator.validateTokenResult(TestNetworks.ethereum, TestNetworks.ethereum.asset("USDT"), "0x" + Amounts.word(BigInteger.ONE))
        for ((network, symbol) in listOf(TestNetworks.ethereum to "USDC", TestNetworks.avalanche to "USDT")) {
            assertThrows(WalletFailure::class.java) { FeeEstimator.validateTokenResult(network, network.asset(symbol), "0x") }
        }
        for (result in listOf("0x" + Amounts.word(BigInteger.ZERO), "0x01", "true")) {
            assertThrows(WalletFailure::class.java) { FeeEstimator.validateTokenResult(TestNetworks.ethereum, TestNetworks.ethereum.asset("USDT"), result) }
        }
        val forged = TestNetworks.ethereum.asset("USDT").copy(contract = "0x" + "1".repeat(40))
        assertThrows(WalletFailure::class.java) { FeeEstimator.validateTokenResult(TestNetworks.ethereum, forged, "0x") }
    }

    @Test fun gasValueUsesNativeUsdQuoteWithExactEighteenDecimalPrecision() {
        for (network in newNetworks) {
            val position = PortfolioAsset(network, network.nativeAsset)
            val price = UsdPrices.quote(network.nativeSymbol, "2.3456789", 1000L)
            val usd = UsdPrices.value(position, BigInteger.ONE, mapOf(network.nativeSymbol to price), 1000L)
            assertEquals(0, BigDecimal("0.0000000000000000023456789").compareTo(usd.amount))
            assertNull(UsdPrices.value(position, BigInteger.ONE, mapOf("ETH" to UsdPrices.quote("ETH", "4000", 1000L)), 1000L).amount)
        }
    }

    @Test fun noFinalityEvidenceNeverFallsBackToLatestAndBroadcastNeverRetries() {
        for (network in newNetworks) {
            val tags = mutableListOf<String>()
            val reads = NetworkClient(network, RpcTransport { _, method, params ->
                if (method == "eth_chainId") Amounts.hex(BigInteger.valueOf(network.chainId)) else {
                    tags.add(params.getString(0)); throw IOException("Offline finalized fixture")
                }
            })
            assertThrows(NetworkReadUnavailable::class.java) { reads.block("finalized") }
            assertEquals(NetworkClient.endpoints(network.chainId).size, tags.size)
            assertTrue(tags.all { it == "finalized" })
            var submitted = 0
            val writes = NetworkClient(network, RpcTransport { _, method, params ->
                assertEquals("eth_sendRawTransaction", method)
                assertEquals("0x02aabb", params.getString(0))
                submitted++; throw SocketTimeoutException("Offline uncertain submission")
            })
            assertThrows(SocketTimeoutException::class.java) { writes.call("eth_sendRawTransaction", JSONArray().put("0x02aabb"), true) }
            assertEquals(1, submitted)
        }
    }
}
