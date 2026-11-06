package app.localwallet.mainnet

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import java.io.IOException
import java.net.SocketTimeoutException

class NetworkClientTest {
    @Before fun reset() = NetworkClient.clearPreferenceForTests()
    @Test fun readFallsBackAfterTransportFailureAndChecksReplacementChain() {
        val calls=mutableListOf<Pair<String,String>>()
        val rpc=NetworkClient(TestNetworks.ethereum,RpcTransport { endpoint,method,_ ->
            calls.add(endpoint to method)
            if(endpoint.contains("publicnode")) throw IOException("fixture outage")
            if(method=="eth_chainId") "0x1" else "0x10"
        })
        assertEquals("0x10",rpc.string("eth_getBalance",JSONArray().put("0x"+"1".repeat(40)).put("latest")))
        assertEquals(listOf("eth_chainId","eth_chainId","eth_getBalance"),calls.map { it.second })
        assertEquals("https://eth-mainnet.g.alchemy.com/public",rpc.lastEndpoint)
    }
    @Test fun wrongChainDataIsNeverUsed() {
        var wrongChainRead=false
        val rpc=NetworkClient(TestNetworks.ethereum,RpcTransport { endpoint,method,_ ->
            if(endpoint.contains("publicnode")) {
                if(method!="eth_chainId") wrongChainRead=true
                "0x2105"
            } else if(method=="eth_chainId") "0x1" else "0x0"
        })
        assertEquals("0x0",rpc.string("eth_getBalance",JSONArray()))
        assertFalse(wrongChainRead)
    }
    @Test fun allReadProvidersUnavailableProducesAReadFailure() {
        val rpc=NetworkClient(TestNetworks.base,RpcTransport { _,_,_ -> throw IOException("fixture outage") })
        try { rpc.assertChain(); fail("Accepted unreachable chain") } catch (_: NetworkReadUnavailable) { }
    }
    @Test fun simulationRevertIsNotRetriedOnAnotherProvider() {
        val endpoints=mutableSetOf<String>()
        val rpc=NetworkClient(TestNetworks.ethereum,RpcTransport { endpoint,method,_ ->
            endpoints.add(endpoint)
            if(method=="eth_chainId") "0x1" else throw RpcRejected(RpcRejectionKind.REVERTED,3)
        })
        try { rpc.call("eth_call",JSONArray()); fail("Ignored simulation rejection") } catch (failure:RpcRejected) { assertEquals(RpcRejectionKind.REVERTED,failure.kind) }
        assertEquals(1,endpoints.size)
    }
    @Test fun uncertainBroadcastIsAttemptedOnlyOnce() {
        val calls=mutableListOf<Pair<String,String>>()
        val rpc=NetworkClient(TestNetworks.base,RpcTransport { endpoint,method,_ -> calls.add(endpoint to method); throw SocketTimeoutException("fixture timeout after write") })
        try { rpc.call("eth_sendRawTransaction",JSONArray().put("0x02"),broadcast=true); fail("Suppressed uncertainty") } catch (_:SocketTimeoutException) { }
        assertEquals(listOf("https://mainnet-sequencer.base.org" to "eth_sendRawTransaction"),calls)
    }
    @Test fun blockNumberCompatibilityReadChecksCanonicalHashTwice() {
        val hash="0x"+"a".repeat(64)
        val block=JSONObject().put("number","0x64").put("hash",hash)
        var canonicalChecks=0
        val rpc=NetworkClient(TestNetworks.ethereum,RpcTransport { _,method,params -> when(method) {
            "eth_chainId" -> "0x1"
            "eth_getBlockByNumber" -> { canonicalChecks++; block }
            "eth_getBalance" -> if(params.opt(1) is JSONObject) throw RpcRejected(RpcRejectionKind.UNSUPPORTED,-32602) else "0x5"
            else -> throw AssertionError("Unexpected method")
        } })
        assertEquals("0x5",rpc.string("eth_getBalance",JSONArray().put("0x"+"1".repeat(40)).put(rpc.reference(block))))
        assertEquals(2,canonicalChecks)
    }
    @Test fun reorganizationDuringCompatibleReadDoesNotReturnTheBalance() {
        val block=JSONObject().put("number","0x64").put("hash","0x"+"a".repeat(64))
        var read=false
        val rpc=NetworkClient(TestNetworks.ethereum,RpcTransport { endpoint,method,params ->
            if(endpoint.contains("alchemy")) throw IOException("fixture unavailable")
            when(method) {
                "eth_chainId" -> "0x1"
                "eth_getBlockByNumber" -> if(read) JSONObject(block.toString()).put("hash","0x"+"b".repeat(64)) else block
                "eth_getBalance" -> if(params.opt(1) is JSONObject) throw RpcRejected(RpcRejectionKind.UNSUPPORTED,-32602) else { read=true; "0x5" }
                else -> throw AssertionError("Unexpected method")
            }
        })
        try { rpc.string("eth_getBalance",JSONArray().put("0x"+"1".repeat(40)).put(rpc.reference(block))); fail("Returned a stale-fork balance") } catch (_:NetworkReadUnavailable) { }
    }
}
