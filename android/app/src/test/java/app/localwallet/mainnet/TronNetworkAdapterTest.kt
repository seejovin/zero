package app.localwallet.mainnet

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.math.BigInteger
import java.net.SocketTimeoutException
import uniffi.wallet_core.restoreWallet
import uniffi.wallet_core.tronAddressHex
import uniffi.wallet_core.tronSign

/** Public Ledger mnemonic fixtures only; all transport operations are mocked. */
class TronNetworkAdapterTest {
    private val owner = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH"
    private val recipient = "TDp4x8kCJ6aHz8wnqZgkaVQAm2ajH1og3L"
    private val fixture = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    private val network = WalletNetwork(-195L, "TRON", TronNetworkAdapter.ORIGIN, TronNetworkAdapter.ORIGIN,
        "https://tronscan.org/#", listOf(AssetInfo("TRX", 6, null), AssetInfo("USDT", 6, TronNetworkAdapter.USDT)))
    private inner class Fake : TronTransport {
        val paths = mutableListOf<String>()
        var chain = TronNetworkAdapter.MAINNET_CHAIN_ID
        var active = true
        var simulate = true
        var omitFees = false
        var broadcastTimeout = false
        var changeHead = false
        private var headReads = 0
        private val height = 80_000_000L
        private val blockId = height.toString(16).padStart(16, '0') + "12".repeat(24)
        private val params = mapOf("getTransactionFee" to 1000L, "getEnergyFee" to 100L,
            "getCreateNewAccountFeeInSystemContract" to 1_000_000L, "getCreateAccountFee" to 100_000L,
            "getCreateNewAccountBandwidthRate" to 1L, "getMaxFeeLimit" to 15_000_000_000L, "getDynamicEnergyMaxFactor" to 34000L)
        override fun post(path: String, body: JSONObject): JSONObject {
            paths.add(path)
            return when (path) {
                "jsonrpc" -> JSONObject().put("jsonrpc", "2.0").put("id", body.get("id")).put("result", chain)
                "wallet/getnowblock", "wallet/getblockbynum" -> {
                    if (path == "wallet/getnowblock") headReads++
                    val currentHeight = if (changeHead && headReads >= 2) height + 1 else height
                    JSONObject().put("blockID", if (currentHeight == height) blockId else currentHeight.toString(16).padStart(16, '0') + "12".repeat(24))
                        .put("block_header", JSONObject().put("raw_data", JSONObject().put("number", currentHeight).put("timestamp", System.currentTimeMillis() - 1000)))
                }
                "wallet/getaccount" -> if (!active && body.getString("address") == tronAddressHex(owner)) JSONObject() else
                    JSONObject().put("address", body.getString("address")).put("balance", 1_000_000_000L)
                "wallet/getchainparameters" -> JSONObject().put("chainParameter", JSONArray().also { out ->
                    params.filterKeys { !(omitFees && it == "getEnergyFee") }.forEach { (key, value) -> out.put(JSONObject().put("key", key).put("value", value)) }
                })
                "wallet/getaccountresource" -> JSONObject().put("TotalNetLimit", 43_200_000_000L).put("TotalEnergyLimit", 180_000_000_000L)
                "wallet/triggerconstantcontract" -> if (body.getString("function_selector") == "balanceOf(address)")
                    JSONObject().put("result", JSONObject().put("result", true)).put("constant_result", JSONArray().put(Amounts.word(BigInteger("10001"))))
                else JSONObject().put("result", JSONObject().put("result", simulate)).put("constant_result", JSONArray().put(Amounts.word(BigInteger.ONE)))
                    .put("energy_used", 65000).put("energy_penalty", 35000)
                "wallet/broadcasttransaction" -> if (broadcastTimeout) throw SocketTimeoutException("public test fixture") else JSONObject().put("result", true).put("txid", body.getString("txID"))
                else -> throw AssertionError("Unexpected TRON operation: $path")
            }
        }
        override fun get(path: String): JSONObject {
            paths.add(path)
            val id = if (path.contains("/trc20?")) "transaction_id" else "txID"
            return JSONObject().put("success", true).put("data", JSONArray().also { rows ->
                for (i in 1..20) rows.put(JSONObject().put(id, i.toString(16).padStart(64, '0')).put("block_timestamp", i * 1000L))
            })
        }
    }
    @Test fun balanceReadsNativeAndOfficialTokenWithoutInventingZeros() {
        val rpc = Fake()
        val result = TronNetworkAdapter(network, owner, rpc).balance()
        assertEquals(mapOf("TRX" to BigInteger("1000000000"), "USDT" to BigInteger("10001")), result.values)
        assertEquals(80_000_000L, result.height)
    }
    @Test fun testnetIdentityStopsAllAccountReads() {
        val rpc = Fake().also { it.chain = "0x94a9059e" }
        try { TronNetworkAdapter(network, owner, rpc).balance(); fail("Accepted Shasta") } catch (_: WalletFailure) { }
        assertEquals(listOf("jsonrpc"), rpc.paths)
    }
    @Test fun changedHeadRejectsTheEntireBalanceObservation() {
        val rpc = Fake().also { it.changeHead = true }
        try { TronNetworkAdapter(network, owner, rpc).balance(); fail("Accepted balances observed across different heads") }
        catch (failure: WalletFailure) { assertTrue(failure.message!!.contains("head changed")) }
        assertEquals(2, rpc.paths.count { it == "wallet/getnowblock" })
    }
    @Test fun maximumTokenTransferKeepsEveryUnitAndPricesEnergy() {
        val prepared = TronNetworkAdapter(network, owner, Fake()).prepare("USDT", recipient, "", true)
        assertEquals(BigInteger("10001"), prepared.amountUnits)
        assertTrue(prepared.feeReserve > prepared.estimatedFee)
        assertEquals(BigInteger("10001"), prepared.assetAvailable)
        assertEquals("15840000", JSONObject(prepared.json).getString("fee_limit_sun"))
    }
    @Test fun inactiveSenderExplainsActivationBeforeAnySigning() {
        val rpc = Fake().also { it.active = false }
        try { TronNetworkAdapter(network, owner, rpc).prepare("USDT", recipient, "0.01", false); fail("Accepted inactive sender") }
        catch (failure: WalletFailure) { assertTrue(failure.message!!.contains("activate")) }
        assertFalse(rpc.paths.contains("wallet/triggerconstantcontract"))
    }
    @Test fun rejectedSimulationAndMissingFeeParametersFailClosed() {
        for (rpc in listOf(Fake().also { it.simulate = false }, Fake().also { it.omitFees = true })) {
            try { TronNetworkAdapter(network, owner, rpc).prepare("USDT", recipient, "0.01", false); fail("Invented a safe fee") }
            catch (_: WalletFailure) { }
            assertFalse(rpc.paths.contains("wallet/broadcasttransaction"))
        }
    }
    @Test fun uncertainBroadcastIsAttemptedExactlyOnce() {
        val rpc = Fake().also { it.broadcastTimeout = true }
        val adapter = TronNetworkAdapter(network, owner, rpc)
        val prepared = adapter.prepare("TRX", recipient, "0.000001", false)
        val key = ByteArray(32) { 0x42 }
        try {
            val vault = JSONObject(restoreWallet(fixture, key))
            val signed = tronSign(vault.getString("vault_hex"), key, prepared.json, prepared.objectValue.getString("approval_digest"))
            try { adapter.broadcast(signed); fail("Silenced uncertain network submission") } catch (_: SocketTimeoutException) { }
            assertEquals(1, rpc.paths.count { it == "wallet/broadcasttransaction" })
        } finally { key.fill(0) }
    }
    @Test fun discoveryIdsAreBoundedAndDeduplicatedWithoutTrustingAmounts() {
        val rpc = Fake()
        val ids = TronNetworkAdapter(network, owner, rpc).recentTransactionIds()
        assertEquals(20, ids.size)
        assertEquals(ids.size, ids.distinct().size)
        assertEquals(20.toString(16).padStart(64, '0'), ids.first())
        assertEquals(2, rpc.paths.count { it.startsWith("v1/accounts/") })
    }
    @Test fun malformedIntegersCannotBeCoercedFromFloatsOrSigns() {
        for (value in listOf<Any>(1.5, -1, "1e6", "+1", "01", "", true)) {
            try { TronNetworkAdapter.integer(value, "fee"); fail("Accepted $value") } catch (_: WalletFailure) { }
        }
    }
}
