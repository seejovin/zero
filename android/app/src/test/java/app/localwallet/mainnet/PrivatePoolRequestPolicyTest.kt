package app.localwallet.mainnet

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class PrivatePoolRequestPolicyTest {
    private fun request(kind: String) = JSONObject().put("schema_version", 1).put("chain_id", 42161).put("kind", kind)
    private fun hash(value: Int) = "0x" + value.toString(16).padStart(64, '0')
    private fun nullifiers() = JSONArray().apply { (1..16).forEach { put(hash(it)) } }

    @Test fun noWalletSpecificFiltersOrArbitraryRpcCanCrossThePort() {
        listOf("address", "recipient", "amount", "topics", "url", "method").forEach { field ->
            assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkObject(request("head").put(field, "fixture")) }
        }
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkObject(request("eth_call")) }
    }

    @Test fun onlyPinnedNetworkAndVersionAreAccepted() {
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkObject(request("head").put("chain_id", 1)) }
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkObject(request("head").put("schema_version", 2)) }
        assertEquals("tip", PrivatePoolRequestPolicy.checkObject(request("tip")).getString("kind"))
    }

    @Test fun publicLogWindowsHaveFixedBoundsAndAlignment() {
        assertEquals(2047L, PrivatePoolRequestPolicy.checkObject(request("logs").put("from_block", 0).put("to_block", 2047)).getLong("to_block"))
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkObject(request("logs").put("from_block", 0).put("to_block", 2048)) }
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkObject(request("logs").put("from_block", 1).put("to_block", 2048)) }
        // A short final window is allowed by this local wire rule. The authenticated gateway
        // must additionally prove that it ends at its independently agreed finalized tip.
        PrivatePoolRequestPolicy.checkObject(request("logs").put("from_block", 2048).put("to_block", 2050))
    }

    @Test fun nullifierQueriesRequireExactlySixteenUniquePaddedSlots() {
        val value = request("nullifiers").put("tree_number", 0).put("nullifiers", nullifiers())
        PrivatePoolRequestPolicy.checkObject(value)
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkObject(value.put("nullifiers", JSONArray().put(hash(1)))) }
        val duplicate = nullifiers().put(15, hash(1))
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkObject(value.put("nullifiers", duplicate)) }
    }

    @Test fun nullifierResponseMustMatchExactRequestOrderAndBooleanPositions() {
        val value = request("nullifiers").put("tree_number", 9).put("nullifiers", nullifiers())
        val response = request("nullifiers").put("tree_number", 9).put("nullifiers", nullifiers())
            .put("spent", JSONArray().apply { repeat(16) { put(false) } }).put("block", JSONObject())
        PrivatePoolRequestPolicy.checkResponse(value, response)
        response.getJSONArray("spent").put(3, "false")
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkResponse(value, response) }
    }

    @Test fun responseKindNetworkAndExtraFieldsCannotBeConfusedWithReceiptEvidence() {
        val value = request("head")
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkResponse(value, request("receipt").put("block", JSONObject())) }
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkResponse(value, request("head").put("block", JSONObject()).put("confirmed", true)) }
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkResponse(value, request("head").put("chain_id", 1).put("block", JSONObject())) }
    }

    @Test fun checkpointMustBindTheExactFinalizedBlockTreeAndCount() {
        val value = request("checkpoint").put("number", 1024)
        PrivatePoolRequestPolicy.checkObject(value)
        val response = request("checkpoint").put("block", JSONObject().put("number", 1024))
            .put("tree_number", 9).put("next_leaf_index", 65_536).put("merkle_root", hash(50))
        PrivatePoolRequestPolicy.checkResponse(value, response)
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkResponse(value, response.put("next_leaf_index", 65_537)) }
        response.put("next_leaf_index", 10).getJSONObject("block").put("number", 1023)
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkResponse(value, response) }
    }

    private fun blockLog(index: Int = 0) = JSONObject().put("address", "0xfa7093cdd9ee6932b4eb2c9e1cde7ce00b1fa4b9")
        .put("topics", JSONArray().put(hash(50))).put("data", "0x")
        .put("blockHash", hash(40)).put("blockNumber", "0x400").put("transactionHash", hash(60))
        .put("transactionIndex", "0x0").put("logIndex", "0x${index.toString(16)}").put("removed", false)

    private fun blockLogs() = request("block_logs")
        .put("block", JSONObject().put("number", 1024).put("hash", hash(40)).put("parent_hash", hash(39)).put("timestamp", 1_700_000_000))
        .put("logs", JSONArray().put(blockLog()))

    @Test fun fullBlockLogRequestsAcceptOnlyOneExplicitFinalizedBlockAndNoFilters() {
        val value = request("block_logs").put("number", 1024)
        PrivatePoolRequestPolicy.checkObject(value)
        listOf("address", "recipient", "amount", "topics", "url", "method", "from_block").forEach { field ->
            assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkObject(request("block_logs").put("number", 1024).put(field, "fixture")) }
        }
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkObject(request("block_logs").put("number", 0)) }
    }

    @Test fun fullBlockLogResponseBindsEveryLogToTheRequestedCanonicalBlock() {
        val value = request("block_logs").put("number", 1024)
        PrivatePoolRequestPolicy.checkResponse(value, blockLogs())
        val wrongNumber = blockLogs().apply { getJSONObject("block").put("number", 1023) }
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkResponse(value, wrongNumber) }
        listOf("blockHash" to hash(41), "blockNumber" to "0x401", "address" to "0x" + "1".repeat(40)).forEach { (field, content) ->
            val wrong = blockLogs().apply { getJSONArray("logs").getJSONObject(0).put(field, content) }
            assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkResponse(value, wrong) }
        }
        val removed = blockLogs().apply { getJSONArray("logs").getJSONObject(0).put("removed", true) }
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkResponse(value, removed) }
    }

    @Test fun duplicateReorderedOrMalformedFullBlockLogsAreRejected() {
        val value = request("block_logs").put("number", 1024)
        val duplicate = blockLogs().put("logs", JSONArray().put(blockLog()).put(blockLog()))
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkResponse(value, duplicate) }
        val reversed = blockLogs().put("logs", JSONArray().put(blockLog(1)).put(blockLog(0)))
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkResponse(value, reversed) }
        val relabelled = blockLogs().put("logs", JSONArray().put(blockLog()).put(blockLog(1).put("transactionHash", hash(61))))
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkResponse(value, relabelled) }
        listOf("0x00", "0x20000000000000", "1").forEach { bad ->
            val invalid = blockLogs().apply { getJSONArray("logs").getJSONObject(0).put("logIndex", bad) }
            assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkResponse(value, invalid) }
        }
        val extra = blockLogs().apply { getJSONArray("logs").getJSONObject(0).put("recipient", "fixture") }
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkResponse(value, extra) }
        val invalidData = blockLogs().apply { getJSONArray("logs").getJSONObject(0).put("data", "0x0") }
        assertThrows(Exception::class.java) { PrivatePoolRequestPolicy.checkResponse(value, invalidData) }
    }

}
