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

/** Public, offline fixtures only. No funded wallet, network submission or external token metadata. */
class MultiNetworkTest {
    private val sender = "0x1111111111111111111111111111111111111111"
    private val recipient = "0x2222222222222222222222222222222222222222"
    private val hash = "0x" + "a".repeat(64)
    private val block = JSONObject().put("number", "0x64").put("hash", hash).put("baseFeePerGas", "0x4c4b40")
    private val funds = mapOf("ETH" to BigInteger("1000000000000000001"), "USDC" to BigInteger("123456789"))

    @Before fun reset() = NetworkClient.clearPreferenceForTests()

    @Test fun assetFirstSelectionKeepsFourUsdcNetworksButOnlyTwoEurcNetworks() {
        assertEquals(listOf("USDC", "EURC", "USDT", "ETH", "AVAX", "POL", "BNB"), AssetSelection.assets(TestNetworks.evm).map { it.symbol })
        assertEquals(listOf(8453L, 1L, 10L, 42161L, 43114L, 137L), AssetSelection.networksFor(TestNetworks.evm, "USDC").map { it.chainId })
        assertEquals(listOf(8453L, 1L, 43114L), AssetSelection.networksFor(TestNetworks.evm, "EURC").map { it.chainId })
        for (network in listOf(TestNetworks.optimism, TestNetworks.arbitrum)) {
            assertNull(AssetSelection.resolve(TestNetworks.all, "USDC", null))
            assertThrows(WalletFailure::class.java) { AssetSelection.resolve(TestNetworks.all, "EURC", network.chainId) }
            assertThrows(WalletFailure::class.java) { network.asset("EURC") }
            val forged = network.copy(assets = network.assets + TestNetworks.ethereum.asset("EURC"))
            assertThrows(WalletFailure::class.java) { AssetSelection.assets(listOf(forged)) }
            assertThrows(WalletFailure::class.java) { forged.asset("EURC") }
            for (other in TestNetworks.evm.filter { it.chainId != network.chainId && it.assets.any { asset -> asset.symbol == "USDC" } }) {
                val counterfeit = network.copy(assets = listOf(other.asset("USDC")))
                assertThrows(WalletFailure::class.java) { AssetSelection.assets(listOf(counterfeit)) }
            }
        }
    }

    @Test fun explicitFeeProfilesAndGlyphsDoNotClassifyEveryRollupAsBaseOrEthereum() {
        assertEquals(NetworkFeeProfile.ETHEREUM, TestNetworks.ethereum.feeProfile)
        assertEquals(NetworkFeeProfile.OP_STACK, TestNetworks.base.feeProfile)
        assertEquals(NetworkFeeProfile.OP_STACK, TestNetworks.optimism.feeProfile)
        assertEquals(NetworkFeeProfile.ARBITRUM, TestNetworks.arbitrum.feeProfile)
        assertEquals(7, TestNetworks.evm.map { it.glyph }.toSet().size)
        for (unsupported in listOf(0L, 97L, 80002L, 42170L, 43113L, 84532L, 11155420L)) {
            assertThrows(WalletFailure::class.java) { NetworkPolicy.feeProfile(unsupported) }
            assertThrows(WalletFailure::class.java) { NetworkClient.endpoints(unsupported) }
        }
    }

    @Test fun eachNewNetworkReadChecksBothProvidersAndRejectsWrongChainBeforeReading() {
        for (network in listOf(TestNetworks.optimism, TestNetworks.arbitrum)) {
            val calls = mutableListOf<Pair<String, String>>()
            val endpoints = NetworkClient.endpoints(network.chainId)
            val rpc = NetworkClient(network, RpcTransport { endpoint, method, _ ->
                calls.add(endpoint to method)
                if (endpoint == endpoints.first()) {
                    assertEquals("eth_chainId", method)
                    "0x2105" // Another supported mainnet is still the wrong chain.
                } else if (method == "eth_chainId") Amounts.hex(BigInteger.valueOf(network.chainId)) else "0x2710"
            })
            assertEquals("0x2710", rpc.string("eth_getBalance", JSONArray().put(sender).put("latest")))
            assertEquals(listOf("eth_chainId", "eth_chainId", "eth_getBalance"), calls.map { it.second })
            assertEquals(endpoints.last(), rpc.lastEndpoint)
        }
    }

    @Test fun newChainBroadcastTimeoutNeverFallsBackOrChangesTheSignedBytes() {
        val expected = mapOf(10L to "https://mainnet-sequencer.optimism.io", 42161L to "https://arb1-sequencer.arbitrum.io/rpc")
        for (network in listOf(TestNetworks.optimism, TestNetworks.arbitrum)) {
            val calls = mutableListOf<String>()
            val rpc = NetworkClient(network, RpcTransport { endpoint, method, params ->
                calls.add(endpoint)
                assertEquals("eth_sendRawTransaction", method)
                assertEquals("0x02aabb", params.getString(0))
                throw SocketTimeoutException("offline timeout fixture")
            })
            assertThrows(SocketTimeoutException::class.java) {
                rpc.call("eth_sendRawTransaction", JSONArray().put("0x02aabb"), broadcast = true)
            }
            assertEquals(listOf(expected.getValue(network.chainId)), calls)
        }
    }

    private fun feeRpc(network: WalletNetwork, oracleCalls: MutableList<String> = mutableListOf(),
                       estimate: Long = 60_000L, rejectSimulation: Boolean = false): NetworkClient =
        NetworkClient(network, RpcTransport { _, method, params -> when (method) {
            "eth_chainId" -> Amounts.hex(BigInteger.valueOf(network.chainId))
            "eth_maxPriorityFeePerGas" -> "0xf4240"
            "eth_estimateGas" -> Amounts.hex(BigInteger.valueOf(estimate))
            "eth_call" -> {
                val call = params.getJSONObject(0)
                val data = call.getString("data")
                if (call.getString("to").equals(NetworkClient.ORACLE, true)) {
                    oracleCalls.add(data)
                    assertEquals(NetworkFeeProfile.OP_STACK, network.feeProfile)
                    assertEquals(hash, params.getJSONObject(1).getString("blockHash"))
                    "0x" + Amounts.word(if (data.startsWith(NetworkClient.L1_UPPER_BOUND_SELECTOR)) BigInteger("123") else BigInteger("10000"))
                } else {
                    if (rejectSimulation) throw RpcRejected(RpcRejectionKind.INSUFFICIENT_FUNDS, -32000)
                    if (data == "0x") "0x" else "0x" + Amounts.word(BigInteger.ONE)
                }
            }
            else -> throw AssertionError("Unexpected operation: $method")
        } })

    @Test fun opFeeQuoteIncludesCanonicalBlockDataAndOperatorFeesExactlyOnce() {
        val oracleCalls = mutableListOf<String>()
        val quote = FeeEstimator.estimate(TestNetworks.optimism, sender, "USDC", recipient, "0.000001", block,
            funds, feeRpc(TestNetworks.optimism, oracleCalls))
        assertTrue(quote.simulated)
        assertEquals(72_000L, quote.gasLimit)
        assertEquals(BigInteger("360000010123"), quote.totalEstimate)
        assertEquals(BigInteger("792000020246"), quote.reserve)
        assertEquals(2, oracleCalls.size)
        assertTrue(oracleCalls[0].startsWith(NetworkClient.L1_UPPER_BOUND_SELECTOR))
        assertEquals(BigInteger("1024"), BigInteger(oracleCalls[0].substring(10), 16))
        assertTrue(oracleCalls[1].startsWith(NetworkClient.OPERATOR_FEE_SELECTOR))
        assertEquals(BigInteger("72000"), BigInteger(oracleCalls[1].substring(10), 16))
    }

    @Test fun arbitrumFeeQuoteUsesCombinedGasWithoutCallingOrAddingOpFees() {
        val oracleCalls = mutableListOf<String>()
        val quote = FeeEstimator.estimate(TestNetworks.arbitrum, sender, "USDC", recipient, "123.456789", block,
            funds, feeRpc(TestNetworks.arbitrum, oracleCalls, estimate = 100_000))
        assertTrue(quote.simulated)
        assertEquals(120_000L, quote.gasLimit)
        assertEquals(BigInteger("600000000000"), quote.totalEstimate)
        assertEquals(BigInteger("1320000000000"), quote.reserve)
        assertEquals(quote.executionLimit, quote.reserve)
        assertEquals(BigInteger.ZERO, quote.l1Estimate)
        assertEquals(BigInteger.ZERO, quote.operatorEstimate)
        assertTrue(quote.note.contains("L1 data posting"))
        assertTrue(oracleCalls.isEmpty())
    }

    @Test fun failedArbitrumNativeSimulationNeverInventsA21000GasTotal() {
        val quote = FeeEstimator.estimate(TestNetworks.arbitrum, sender, "ETH", recipient, "0.01", block,
            funds, feeRpc(TestNetworks.arbitrum, rejectSimulation = true))
        assertFalse(quote.simulated)
        assertEquals(300_000L, quote.gasLimit)
        assertTrue(quote.note.startsWith("Conservative budget"))
        assertEquals(BigInteger.ZERO, quote.l1Estimate)
        assertThrows(RpcRejected::class.java) {
            MaxTransferCalculator.calculate(TestNetworks.arbitrum, sender, "ETH", recipient, block, funds,
                feeRpc(TestNetworks.arbitrum, rejectSimulation = true))
        }
    }

    @Test fun newChainNativeMaxUsesEveryWeiAfterItsOwnFeeReserve() {
        for (network in listOf(TestNetworks.optimism, TestNetworks.arbitrum)) {
            val result = MaxTransferCalculator.calculate(network, sender, "ETH", recipient, block, funds, feeRpc(network))
            assertEquals(funds.getValue("ETH"), result.units + result.fee.reserve)
            assertEquals(result.units, Amounts.parse(result.amount, 18))
            assertEquals(18, result.amount.substringAfter('.').length)
            if (network.chainId == 42161L) assertEquals(result.fee.executionLimit, result.fee.reserve)
            else assertTrue(result.fee.reserve > result.fee.executionLimit)
        }
    }

    @Test fun opReceiptFeeNormalizationUsesItsInclusionBlockAndRejectsConflictingEvidence() {
        var oracleAt: JSONObject? = null
        val rpc = NetworkClient(TestNetworks.optimism, RpcTransport { _, method, params -> when (method) {
            "eth_chainId" -> "0xa"
            "eth_getBlockByNumber" -> { assertEquals("0x64", params.getString(0)); block }
            "eth_call" -> {
                assertEquals(NetworkClient.ORACLE, params.getJSONObject(0).getString("to"))
                assertEquals(NetworkClient.OPERATOR_FEE_SELECTOR + Amounts.word(BigInteger("21000")), params.getJSONObject(0).getString("data"))
                oracleAt = params.getJSONObject(1)
                "0x" + Amounts.word(BigInteger("1234"))
            }
            else -> throw AssertionError("Unexpected operation: $method")
        } })
        val receipt = JSONObject().put("blockNumber", "0x64").put("blockHash", hash).put("gasUsed", "0x5208").put("l1Fee", "0x7b")
        val normalized = rpc.normalizeReceipt(receipt)
        assertEquals("0x4d2", normalized.getString("operatorFee"))
        assertEquals(hash, oracleAt!!.getString("blockHash"))
        assertTrue(oracleAt!!.getBoolean("requireCanonical"))
        assertFalse(receipt.has("operatorFee"))
        assertThrows(WalletFailure::class.java) { rpc.normalizeReceipt(JSONObject(receipt.toString()).put("operatorFee", "0x1")) }
        assertThrows(WalletFailure::class.java) { rpc.normalizeReceipt(JSONObject(receipt.toString()).apply { remove("l1Fee") }) }
    }

    @Test fun arbitrumReceiptKeepsCombinedGasAndNeverInventsSeparateOpFees() {
        val rpc = NetworkClient(TestNetworks.arbitrum, RpcTransport { _, _, _ -> throw AssertionError("Arbitrum receipt must not call an OP oracle") })
        val receipt = JSONObject().put("gasUsed", "0x186a0").put("gasUsedForL1", "0x7530").put("effectiveGasPrice", "0x5b8d80")
        val normalized = rpc.normalizeReceipt(receipt)
        assertEquals(receipt.toString(), normalized.toString())
        assertFalse(normalized.has("l1Fee"))
        assertFalse(normalized.has("operatorFee"))
    }

    @Test fun missingFinalityEvidenceDoesNotSubstituteLatestOrClaimFinalization() {
        for (network in listOf(TestNetworks.optimism, TestNetworks.arbitrum)) {
            val requestedTags = mutableListOf<String>()
            val rpc = NetworkClient(network, RpcTransport { _, method, params ->
                if (method == "eth_chainId") Amounts.hex(BigInteger.valueOf(network.chainId)) else {
                    assertEquals("eth_getBlockByNumber", method)
                    requestedTags.add(params.getString(0))
                    throw IOException("offline finality fixture")
                }
            })
            assertThrows(NetworkReadUnavailable::class.java) { rpc.block("finalized") }
            assertEquals(listOf("finalized", "finalized"), requestedTags)
        }
    }

    @Test fun newNetworkRoutesKeepChainIdentityAndCannotRestoreUnsupportedEurc() {
        for (network in listOf(TestNetworks.optimism, TestNetworks.arbitrum)) {
            for (screen in listOf("send", "receive", "asset", "track")) {
                val route = WalletNavigation.capture(sender, screen, network.chainId, "USDC", "send")!!
                assertEquals(route, WalletNavigation.decode(WalletNavigation.encode(route)))
                assertNull(WalletNavigation.decode(WalletNavigation.encode(route) + ("wallet.route.asset" to "EURC")))
            }
        }
    }

    @Test fun newHoldingsUseUsdValueAndMissingRollupCoverageDoesNotEraseOtherBalances() {
        val balances = TestNetworks.all.associate { network -> network.chainId to BalanceView(100, hash,
            network.assets.associate { it.symbol to BigInteger.ZERO }, 1000L) }.toMutableMap()
        balances[10] = balances.getValue(10).copy(balances = mapOf("ETH" to BigInteger.ZERO, "USDC" to BigInteger("10000")))
        balances[42161] = balances.getValue(42161).copy(balances = mapOf("ETH" to BigInteger("1000000000000000"), "USDC" to BigInteger.ZERO))
        val quotes = mapOf("USDC" to UsdPrices.quote("USDC", "0.99", 1000L), "ETH" to UsdPrices.quote("ETH", "2500", 1000L))
        val total = WalletPortfolio.totalUsd(TestNetworks.all, balances, TestNetworks.live, quotes, 1000L)
        assertEquals(UsdCoverage.COMPLETE, total.coverage)
        assertEquals(0, BigDecimal("2.5099").compareTo(total.amount))
        val filtered = WalletPortfolio.holdings(TestNetworks.all, balances, TestNetworks.live, true, BigDecimal.ONE, quotes, 1000L)
        assertEquals(1, filtered.hiddenSmallCount)
        assertEquals(PortfolioAssetKey(42161, "ETH"), filtered.visible.single().position.key)
        balances.remove(42161)
        val partial = WalletPortfolio.totalUsd(TestNetworks.all, balances, TestNetworks.live, quotes, 1000L)
        assertEquals(UsdCoverage.PARTIAL, partial.coverage)
        assertNull(partial.amount)
        assertEquals(0, BigDecimal("0.0099").compareTo(partial.knownSubtotal))
        assertEquals(setOf(PortfolioAssetKey(42161, "ETH"), PortfolioAssetKey(42161, "USDC")), partial.missingBalances)
        assertTrue(partial.missingBalances.none { it.symbol == "EURC" })
    }
}
