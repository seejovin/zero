package app.localwallet.mainnet

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import java.math.BigInteger

class FeeEstimatorTest {
    private val sender="0x1111111111111111111111111111111111111111"
    private val recipient="0x2222222222222222222222222222222222222222"
    private val block=JSONObject().put("number","0x64").put("hash","0x"+"a".repeat(64)).put("baseFeePerGas","0x4c4b40")
    @Before fun reset()=NetworkClient.clearPreferenceForTests()
    @Test fun zeroEthWalletStillGetsExactTokenFeeQuoteAndShortfallWithoutSigning() {
        val methods=mutableListOf<String>()
        val rpc=NetworkClient(TestNetworks.base,RpcTransport { _,method,params ->
            methods.add(method)
            when(method) {
                "eth_chainId" -> "0x2105"
                "eth_maxPriorityFeePerGas" -> "0xf4240"
                "eth_estimateGas" -> {
                    assertFalse(params.getJSONObject(0).has("maxFeePerGas")); "0xea60"
                }
                "eth_call" -> {
                    val call=params.getJSONObject(0)
                    assertFalse(call.has("maxFeePerGas"))
                    val value=when {
                        call.getString("data").startsWith(NetworkClient.L1_UPPER_BOUND_SELECTOR) -> BigInteger("123")
                        call.getString("data").startsWith(NetworkClient.OPERATOR_FEE_SELECTOR) -> BigInteger("10000")
                        else -> BigInteger.ONE
                    }
                    "0x"+Amounts.word(value)
                }
                else -> throw AssertionError("Quote attempted unexpected method: $method")
            }
        })
        val quote=FeeEstimator.estimate(TestNetworks.base,sender,"USDC",recipient,"0.01",block,mapOf("ETH" to BigInteger.ZERO,"USDC" to BigInteger("10000"),"EURC" to BigInteger.ZERO),rpc)
        assertTrue(quote.simulated)
        assertTrue(quote.transferAvailable)
        assertEquals(BigInteger("10000"), quote.assetAvailable)
        assertEquals(BigInteger("360000010123"),quote.totalEstimate)
        assertEquals(BigInteger("792000020246"),quote.shortfall)
        assertEquals(72_000L,quote.gasLimit)
        assertFalse(methods.any { it.contains("send",true) || it.contains("sign",true) || it=="eth_getTransactionCount" })
    }
    @Test fun unfundedNativeTransferShowsBudgetIncludingAmount() {
        val rpc=NetworkClient(TestNetworks.ethereum,RpcTransport { _,method,_ -> when(method) {
            "eth_chainId" -> "0x1"
            "eth_maxPriorityFeePerGas" -> "0xf4240"
            "eth_call" -> throw RpcRejected(RpcRejectionKind.INSUFFICIENT_FUNDS,-32000)
            "eth_getCode" -> "0x"
            else -> throw AssertionError("Unexpected method: $method")
        } })
        val quote=FeeEstimator.estimate(TestNetworks.ethereum,sender,"ETH",recipient,"0.01",block,mapOf("ETH" to BigInteger.ZERO,"USDC" to BigInteger.ZERO,"EURC" to BigInteger.ZERO),rpc)
        assertFalse(quote.simulated)
        assertFalse(quote.transferAvailable)
        assertEquals(BigInteger.ZERO, quote.assetAvailable)
        assertEquals(21_000L,quote.gasLimit)
        assertEquals(BigInteger("10000000000000000").add(quote.reserve),quote.shortfall)
    }
    @Test fun unavailableTokenSimulationReturnsClearlyConservativeBudget() {
        val rpc=NetworkClient(TestNetworks.ethereum,RpcTransport { _,method,_ -> when(method) {
            "eth_chainId" -> "0x1"
            "eth_maxPriorityFeePerGas" -> "0xf4240"
            "eth_call" -> throw RpcRejected(RpcRejectionKind.INSUFFICIENT_FUNDS,-32000)
            else -> throw AssertionError("Unexpected method: $method")
        } })
        val quote=FeeEstimator.estimate(TestNetworks.ethereum,sender,"USDC",recipient,"0.01",block,mapOf("ETH" to BigInteger.ZERO,"USDC" to BigInteger("10000"),"EURC" to BigInteger.ZERO),rpc)
        assertFalse(quote.simulated)
        assertEquals(300_000L,quote.gasLimit)
        assertTrue(quote.note.startsWith("Conservative budget"))
        assertTrue(quote.shortfall>BigInteger.ZERO)
    }
}
