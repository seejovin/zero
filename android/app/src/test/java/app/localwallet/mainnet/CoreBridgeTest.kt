package app.localwallet.mainnet

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import uniffi.wallet_core.WalletJournal
import uniffi.wallet_core.assetRegistry
import uniffi.wallet_core.inspectSigned
import uniffi.wallet_core.preparePayment
import uniffi.wallet_core.restoreWallet
import uniffi.wallet_core.signPayment
import uniffi.wallet_core.walletAddress
import uniffi.wallet_core.validateActivity
import java.nio.file.Files
import java.util.UUID

/** Public Anvil fixture only. No network calls and no funded recovery words. */
class CoreBridgeTest {
    private val fixture = "test test test test test test test test test test test junk"
    private val account = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"

    @Test fun generatedBridgeVerifiesExactSmallUsdcDepositAndRejectsAnotherRecipient() {
        val hash="0x"+"a".repeat(64)
        val blockHash="0x"+"b".repeat(64)
        val sender="0x1111111111111111111111111111111111111111"
        val contract=TestNetworks.base.asset("USDC").contract!!
        val transaction=JSONObject().put("hash",hash).put("blockHash",blockHash).put("blockNumber","0x64")
            .put("from",sender).put("to",contract).put("value","0x0").put("chainId","0x2105")
        val log=JSONObject().put("address",contract).put("transactionHash",hash).put("blockHash",blockHash)
            .put("blockNumber","0x64").put("logIndex","0x0").put("removed",false)
            .put("topics",org.json.JSONArray().put(NetworkClient.TRANSFER_TOPIC).put("0x"+sender.substring(2).padStart(64,'0')).put("0x"+account.substring(2).lowercase().padStart(64,'0')))
            .put("data","0x"+Amounts.word(java.math.BigInteger("10000")))
        val receipt=JSONObject().put("transactionHash",hash).put("blockHash",blockHash).put("blockNumber","0x64")
            .put("from",sender).put("to",contract).put("status","0x1").put("logs",org.json.JSONArray().put(log))
        val block=JSONObject().put("hash",blockHash).put("number","0x64").put("timestamp","0x65000000").put("transactions",org.json.JSONArray().put(hash))
        val result=JSONObject(validateActivity(8453uL,account,transaction.toString(),receipt.toString(),block.toString(),100uL)).getJSONArray("activities")
        assertEquals(1,result.length())
        assertEquals("0.01",result.getJSONObject(0).getString("amount"))
        assertEquals("received",result.getJSONObject(0).getString("direction"))
        assertEquals("finalized",result.getJSONObject(0).getString("inclusion"))
        log.getJSONArray("topics").put(2,"0x"+"2".repeat(40).padStart(64,'0'))
        assertEquals(0,JSONObject(validateActivity(8453uL,account,transaction.toString(),receipt.toString(),block.toString(),100uL)).getJSONArray("activities").length())
    }

    @Test fun actualGeneratedKotlinBridgeRestoresOneWalletAndSignsAllReviewedEvmMainnets() {
        val key = ByteArray(32) { (it + 1).toByte() }
        try {
            val vault = JSONObject(restoreWallet(fixture, key))
            assertEquals(account, vault.getString("address"))
            assertEquals(account, walletAddress(vault.getString("vault_hex"), key))
            val networks = JSONObject(assetRegistry()).getJSONArray("networks").objects()
            assertEquals(setOf(1L, 8453L, 10L, 42161L, 43114L, 137L, 56L), networks.map { it.getLong("chain_id") }.toSet())
            for (chain in listOf(1L, 8453L, 10L, 42161L, 43114L, 137L)) {
                val prepared = preparePayment(request(chain).toString())
                val signed = JSONObject(signPayment(vault.getString("vault_hex"), key, prepared, JSONObject(prepared).getString("approval_digest")))
                val inspected = JSONObject(inspectSigned(signed.getString("raw_transaction")))
                assertEquals(account, inspected.getString("from"))
                assertEquals(chain, inspected.getLong("chain_id"))
                assertEquals("1000001", inspected.getString("amount_units"))
                assertEquals(signed.getString("hash"), inspected.getString("hash"))
                val contract = TestNetworks.all.first { it.chainId == chain }.asset("USDC").contract!!
                assertEquals(contract.lowercase(), inspected.getString("transaction_to").lowercase())
            }
        } finally { key.fill(0) }
    }

    @Test fun generatedJournalBridgeCommitsSignedBytesAndPreservesUncertainOutcome() {
        val key = ByteArray(32) { 19 }
        val directory = Files.createTempDirectory("wallet-kotlin-fixture-")
        try {
            val vault = JSONObject(restoreWallet(fixture, key))
            val prepared = preparePayment(request(8453).toString())
            val signed = signPayment(vault.getString("vault_hex"), key, prepared, JSONObject(prepared).getString("approval_digest"))
            val id = JSONObject(prepared).getJSONObject("request").getString("intent_id")
            WalletJournal(directory.resolve("journal.sqlite3").toString()).use { journal ->
                journal.recordPrepared(prepared)
                val committed = JSONObject(journal.recordSigned(signed))
                assertEquals("signed", committed.getString("phase"))
                assertEquals(JSONObject(signed).getString("raw_transaction"), committed.getString("raw_transaction"))
                assertEquals(committed.toString(), JSONObject(journal.recordSigned(signed)).toString())
                val unknown = JSONObject(journal.markSubmission(id, "unknown"))
                assertEquals("unknown", unknown.getString("phase"))
                assertEquals("unknown", unknown.getString("execution"))
            }
            WalletJournal(directory.resolve("journal.sqlite3").toString()).use { reopened ->
                assertEquals(JSONObject(signed).getString("hash"), JSONObject(reopened.getPayment(id)).getString("hash"))
            }
        } finally {
            key.fill(0)
            directory.toFile().deleteRecursively()
        }
    }

    private fun request(chain: Long): JSONObject = JSONObject()
        .put("intent_id", UUID.randomUUID().toString()).put("chain_id", chain)
        .put("from", account).put("to", "0x1234567890123456789012345678901234567890")
        .put("asset", "USDC").put("amount", "1.000001").put("nonce", 0).put("gas_limit", 100000)
        .put("max_fee_per_gas", "1000000000").put("max_priority_fee_per_gas", "100000000")
        .put("l1_fee_estimate_wei", if (chain == 8453L || chain == 10L) "1234" else "0")
        .put("operator_fee_estimate_wei", if (chain == 8453L || chain == 10L) "5678" else "0")
}
