package app.localwallet.mainnet

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.math.BigInteger
import java.net.SocketTimeoutException
import uniffi.wallet_core.restoreWallet
import uniffi.wallet_core.solanaSign
import uniffi.wallet_core.solanaTokenAccount

/** Public BIP39 vector only. Every network response is mocked; no transaction is sent. */
class SolanaNetworkAdapterTest {
    private val owner = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk"
    private val recipient = "GmaDrppBC7P5ARKV8g3djiwP89vz1jLK23V2GBjuAEGB"
    private val phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    private val network = WalletNetwork(-101L, "Solana", "https://api.mainnet-beta.solana.com",
        "https://api.mainnet-beta.solana.com", "https://solscan.io", listOf(AssetInfo("SOL", 9, null)) +
            SolanaNetworkAdapter.MINTS.map { (symbol, mint) -> AssetInfo(symbol, 6, mint) })
    private val zero = BigInteger.ZERO

    private inner class Fake : RpcTransport {
        val methods = mutableListOf<String>()
        var genesis = SolanaNetworkAdapter.GENESIS_HASH
        var nativeBalance = BigInteger("1000000001")
        var tokenUnits = "10001"
        var simulationError = false
        var wrongDecimals = false
        var broadcastTimeout = false
        var observed = false
        var hashValid = true
        var finalizedWasUsed = false
        var height = 250_000_000L
        var fee = 5000L
        val slot = 300_000_000L
        private fun context(value: Any) = JSONObject().put("context", JSONObject().put("slot", slot)).put("value", value)
        private fun native() = JSONObject().put("owner", SolanaNetworkAdapter.SYSTEM_PROGRAM).put("executable", false)
            .put("lamports", nativeBalance).put("data", JSONArray().put("").put("base64"))
        private fun mint() = JSONObject().put("owner", SolanaNetworkAdapter.TOKEN_PROGRAM).put("executable", false)
            .put("data", JSONObject().put("program", "spl-token").put("parsed", JSONObject().put("type", "mint")
                .put("info", JSONObject().put("isInitialized", true).put("decimals", if (wrongDecimals) 9 else 6))))
        private fun token(symbol: String) = JSONObject().put("owner", SolanaNetworkAdapter.TOKEN_PROGRAM).put("executable", false)
            .put("lamports", 2039280).put("data", JSONObject().put("program", "spl-token")
                .put("parsed", JSONObject().put("type", "account").put("info", JSONObject().put("mint", SolanaNetworkAdapter.MINTS[symbol])
                    .put("owner", owner).put("state", "initialized")
                    .put("tokenAmount", JSONObject().put("amount", tokenUnits).put("decimals", 6)))))
        override fun request(endpoint: String, method: String, params: JSONArray): Any {
            methods += method
            return when (method) {
                "getGenesisHash" -> genesis
                "getTokenAccountsByOwner" -> {
                    val mint = params.getJSONObject(1).getString("mint")
                    val symbol = SolanaNetworkAdapter.MINTS.entries.first { it.value == mint }.key
                    context(JSONArray().put(JSONObject().put("pubkey", solanaTokenAccount(owner, symbol)).put("account", token(symbol))))
                }
                "getMultipleAccounts" -> context(JSONArray().also { values ->
                    for (index in 0 until params.getJSONArray(0).length()) {
                        val key = params.getJSONArray(0).getString(index)
                        values.put(when {
                            key == owner -> native()
                            key in SolanaNetworkAdapter.MINTS.values -> mint()
                            else -> token(SolanaNetworkAdapter.MINTS.keys.first { solanaTokenAccount(owner, it) == key })
                        })
                    }
                })
                "getBlock" -> JSONObject().put("blockhash", "4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi")
                "getAccountInfo" -> context(JSONObject.NULL) // a new ordinary recipient and new ATA.
                "getMinimumBalanceForRentExemption" -> BigInteger("2039280")
                "getLatestBlockhash" -> context(JSONObject().put("blockhash", "4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi")
                    .put("lastValidBlockHeight", 250_000_150L))
                "getRecentPrioritizationFees" -> JSONArray().put(JSONObject().put("slot", slot).put("prioritizationFee", 0))
                "getFeeForMessage" -> context(fee)
                "simulateTransaction" -> {
                    assertFalse(params.getJSONObject(1).getBoolean("sigVerify"))
                    assertFalse(params.getJSONObject(1).getBoolean("replaceRecentBlockhash"))
                    context(JSONObject().put("err", if (simulationError) JSONObject().put("InstructionError", JSONArray().put(2).put("Custom")) else JSONObject.NULL)
                        .put("unitsConsumed", 1000))
                }
                "isBlockhashValid" -> {
                    if (params.getJSONObject(1).getString("commitment") == "finalized") finalizedWasUsed = true
                    context(hashValid)
                }
                "getBlockHeight" -> height
                "getSignatureStatuses" -> context(JSONArray().put(if (observed) JSONObject().put("slot", slot) else JSONObject.NULL))
                "sendTransaction" -> {
                    assertFalse(params.getJSONObject(1).getBoolean("skipPreflight"))
                    assertEquals(0, params.getJSONObject(1).getInt("maxRetries"))
                    if (broadcastTimeout) throw SocketTimeoutException("Public test fixture")
                    throw AssertionError("This suite never returns a fabricated accepted transaction")
                }
                "getSignaturesForAddress" -> JSONArray()
                else -> throw AssertionError("Unexpected Solana RPC: $method")
            }
        }
    }

    @Test fun exactMaximumKeepsEveryLamportAndTokenUnit() {
        val rpc = Fake()
        val adapter = SolanaNetworkAdapter(network, owner, rpc)
        val native = adapter.prepare("SOL", recipient, "", true)
        assertEquals(BigInteger("999995001"), native.amountUnits)
        assertEquals("0.999995001", native.request.getString("amount"))
        val token = adapter.prepare("USDC", recipient, "", true)
        assertEquals(BigInteger("10001"), token.amountUnits)
        assertEquals("0.010001", token.request.getString("amount"))
        assertEquals(BigInteger("2044280"), token.feeReserve)
        assertEquals(BigInteger("5000"), token.estimatedFee)
        assertTrue(token.request.getBoolean("create_destination_ata"))
        assertFalse(rpc.methods.contains("sendTransaction"))
    }

    @Test fun insufficientSolStillDisplaysRealTokenFeeAndRent() {
        val rpc = Fake().also { it.nativeBalance = zero }
        val quote = SolanaNetworkAdapter(network, owner, rpc).prepare("USDC", recipient, "0.01", false)
        assertEquals(BigInteger("5000"), quote.estimatedFee)
        assertEquals(BigInteger("2044280"), quote.feeReserve)
        assertTrue(quote.note.contains("Add"))
        assertFalse(rpc.methods.contains("simulateTransaction"))
        try { SolanaNetworkAdapter(network, owner, rpc).revalidate(quote.json); fail("Allowed signing without SOL") } catch (_: WalletFailure) { }
    }

    @Test fun balanceReadsCanonicalAssetsAtOneBankContext() {
        val rpc = Fake()
        val balances = SolanaNetworkAdapter(network, owner, rpc).balance()
        assertEquals(rpc.slot, balances.height)
        assertEquals(BigInteger("1000000001"), balances.values["SOL"])
        assertEquals(BigInteger("10001"), balances.values["USDC"])
        assertEquals(BigInteger("10001"), balances.values["EURC"])
        assertEquals(BigInteger("10001"), balances.values["USDT"])
        assertEquals(1, rpc.methods.count { it == "getMultipleAccounts" })
    }

    @Test fun wrongGenesisOrDecimalsNeverBecomeZeroBalances() {
        val wrongChain = Fake().also { it.genesis = "devnet" }
        try { SolanaNetworkAdapter(network, owner, wrongChain).balance(); fail("Accepted wrong mainnet identity") } catch (_: WalletFailure) { }
        assertFalse(wrongChain.methods.contains("getTokenAccountsByOwner"))
        val wrongDecimals = Fake().also { it.wrongDecimals = true }
        try { SolanaNetworkAdapter(network, owner, wrongDecimals).balance(); fail("Accepted wrong mint decimals") } catch (_: WalletFailure) { }
    }

    @Test fun failedSimulationAndChangedFeeNeverReachSigning() {
        val rejected = Fake().also { it.simulationError = true }
        try { SolanaNetworkAdapter(network, owner, rejected).prepare("SOL", recipient, "0.01", false); fail("Accepted rejected simulation") } catch (_: WalletFailure) { }
        val rpc = Fake(); val adapter = SolanaNetworkAdapter(network, owner, rpc)
        val prepared = adapter.prepare("USDC", recipient, "0.01", false)
        rpc.fee = 7000
        try { adapter.revalidate(prepared.json); fail("Accepted changed network fee") } catch (_: WalletFailure) { }
        assertFalse(rpc.methods.contains("sendTransaction"))
    }

    @Test fun freshConfirmedBlockhashDoesNotWaitForFinalizedBank() {
        val rpc = Fake(); val adapter = SolanaNetworkAdapter(network, owner, rpc)
        val prepared = adapter.prepare("SOL", recipient, "0.01", false)
        adapter.requireRebroadcastable(prepared.json, JSONObject().put("hash", "2".repeat(88)).toString())
        assertFalse(rpc.finalizedWasUsed)
        rpc.observed = true
        try { adapter.requireRebroadcastable(prepared.json, JSONObject().put("hash", "2".repeat(88)).toString()); fail("Rebroadcasted observed signature") } catch (_: WalletFailure) { }
    }

    @Test fun timedOutBroadcastIsAttemptedExactlyOnce() {
        val rpc = Fake().also { it.broadcastTimeout = true }
        val adapter = SolanaNetworkAdapter(network, owner, rpc)
        val prepared = adapter.prepare("SOL", recipient, "0.000000001", false)
        val key = ByteArray(32) { 0x42 }
        try {
            val vault = JSONObject(restoreWallet(phrase, key))
            val signed = solanaSign(vault.getString("vault_hex"), key, prepared.json, prepared.objectValue.getString("approval_digest"))
            try { adapter.broadcast(signed); fail("Silenced unknown submission") } catch (_: SocketTimeoutException) { }
            assertEquals(1, rpc.methods.count { it == "sendTransaction" })
        } finally { key.fill(0) }
    }

    @Test fun countersFitSignedLongWithoutWrappingUnsignedAmounts() {
        assertEquals(0L, SolanaNetworkAdapter.exactLong("0"))
        assertEquals(Long.MAX_VALUE, SolanaNetworkAdapter.exactLong(Long.MAX_VALUE.toString()))
        for (invalid in listOf("9223372036854775808", "18446744073709551615", "-1", "1.5")) {
            assertThrows(WalletFailure::class.java) { SolanaNetworkAdapter.exactLong(invalid) }
        }
        // Token amounts retain the wider unsigned domain; only counters narrow to Long.
        assertEquals(BigInteger("18446744073709551615"), SolanaNetworkAdapter.exactInteger("18446744073709551615"))
    }

    @Test fun allocationNeverDropsDustOrTreatsFrozenFundsAsSpendable() {
        val accounts = listOf(SolanaTokenAccount("first", BigInteger("1000000"), false), SolanaTokenAccount("second", BigInteger("1"), false))
        assertEquals(listOf("first" to BigInteger("1000000"), "second" to BigInteger.ONE), SolanaNetworkAdapter.allocate(BigInteger("1000001"), accounts))
        try { SolanaNetworkAdapter.allocate(BigInteger("1000001"), accounts.map { it.copy(frozen = it.address == "second") }); fail("Truncated frozen Max") } catch (_: WalletFailure) { }
        try { SolanaNetworkAdapter.allocate(BigInteger.TEN, (1..10).map { SolanaTokenAccount(it.toString(), BigInteger.ONE, false) }); fail("Truncated more than eight source accounts") } catch (_: WalletFailure) { }
        assertEquals(BigInteger("18446744073709551615"), SolanaNetworkAdapter.exactInteger("18446744073709551615"))
        for (invalid in listOf("1.5", "-1", "18446744073709551616")) {
            try { SolanaNetworkAdapter.exactInteger(invalid); fail("Accepted invalid integer: $invalid") } catch (_: WalletFailure) { }
        }
    }
}
