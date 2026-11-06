package app.localwallet.mainnet

import org.json.JSONObject
import java.math.BigInteger

internal fun interface PrivatePoolReadTransport { fun read(request: JSONObject): JSONObject }

/** Only fixed pool operations. There is no address, amount, arbitrary call, topic filter or URL. */
internal object PrivatePoolRequestPolicy {
    private const val MAX_BLOCK = 9_007_199_254_740_991L
    fun check(input: JSONObject): JSONObject {
        val value = RpcJson.parse(input.toString(), maxChars = 8_192, maxNodes = 100)
        return checkObject(value)
    }

    internal fun checkObject(value: JSONObject): JSONObject {
        PrivateEngineWire.number(value, "schema_version", 1, 1)
        PrivateEngineWire.number(value, "chain_id", 42161, 42161)
        val common = arrayOf("schema_version", "chain_id", "kind")
        when (PrivateEngineWire.string(value, "kind")) {
            "head", "tip" -> PrivateEngineWire.fields(value, *common)
            "block", "checkpoint", "block_logs" -> {
                PrivateEngineWire.fields(value, *common, "number")
                PrivateEngineWire.number(value, "number", 1, MAX_BLOCK)
            }
            "logs" -> {
                PrivateEngineWire.fields(value, *common, "from_block", "to_block")
                val first = PrivateEngineWire.number(value, "from_block", 0, MAX_BLOCK)
                val last = PrivateEngineWire.number(value, "to_block", first, minOf(MAX_BLOCK, first + 2047))
                // Ordinary requests are aligned power-of-two windows. A short final window is
                // allowed only when the gateway verifies the independently agreed current tip.
                val length = last - first + 1
                val rounded = java.lang.Long.highestOneBit(length - 1).let { if (length == 1L) 1L else it shl 1 }
                if (first % rounded != 0L) throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
            }
            "root" -> {
                PrivateEngineWire.fields(value, *common, "tree_number", "root")
                PrivateEngineWire.number(value, "tree_number", 0, 65_535)
                PrivateIdentifiers.require(PrivateEngineWire.string(value, "root"))
            }
            "nullifiers" -> {
                PrivateEngineWire.fields(value, *common, "tree_number", "nullifiers")
                PrivateEngineWire.number(value, "tree_number", 0, 65_535)
                val list = value.getJSONArray("nullifiers")
                if (list.length() != 16) throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
                val values = (0 until 16).map { PrivateIdentifiers.require(list.get(it) as? String ?: throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)) }
                if (values.toSet().size != 16) throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
            }
            "receipt" -> {
                PrivateEngineWire.fields(value, *common, "transaction_hash")
                PrivateIdentifiers.require(PrivateEngineWire.string(value, "transaction_hash"))
            }
            else -> throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
        }
        return value
    }

    /** This is wire validation, not a claim of independent cryptographic chain verification. */
    fun checkResponse(request: JSONObject, result: JSONObject) {
        PrivateEngineWire.number(result, "schema_version", 1, 1)
        PrivateEngineWire.number(result, "chain_id", 42161, 42161)
        val kind = PrivateEngineWire.string(request, "kind")
        if (PrivateEngineWire.string(result, "kind") != kind) throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
        val common = arrayOf("schema_version", "chain_id", "kind")
        when (kind) {
            "head", "tip", "block" -> PrivateEngineWire.fields(result, *common, "block")
            "checkpoint" -> {
                PrivateEngineWire.fields(result, *common, "block", "tree_number", "next_leaf_index", "merkle_root")
                PrivateEngineWire.number(result, "tree_number", 0, 65_535)
                PrivateEngineWire.number(result, "next_leaf_index", 0, 65_536)
                PrivateIdentifiers.require(PrivateEngineWire.string(result, "merkle_root"))
                if (PrivateEngineWire.number(result.getJSONObject("block"), "number", 1, MAX_BLOCK) != request.getLong("number")) {
                    throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
                }
            }
            "logs" -> {
                PrivateEngineWire.fields(result, *common, "from_block", "to_block", "block_to_hash", "logs")
                if (result.getLong("from_block") != request.getLong("from_block") || result.getLong("to_block") != request.getLong("to_block")) {
                    throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
                }
                PrivateIdentifiers.require(PrivateEngineWire.string(result, "block_to_hash"))
                if (result.getJSONArray("logs").length() > 4096) throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
            }
            "block_logs" -> {
                PrivateEngineWire.fields(result, *common, "block", "logs")
                val block = result.getJSONObject("block")
                PrivateEngineWire.fields(block, "number", "hash", "parent_hash", "timestamp")
                val number = PrivateEngineWire.number(block, "number", 1, MAX_BLOCK)
                if (number != request.getLong("number")) throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
                val blockHash = PrivateIdentifiers.require(PrivateEngineWire.string(block, "hash"))
                PrivateIdentifiers.require(PrivateEngineWire.string(block, "parent_hash"))
                PrivateEngineWire.number(block, "timestamp", 1, MAX_BLOCK)
                val logs = result.getJSONArray("logs")
                if (logs.length() > 4096) throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
                var previousLogIndex = -1L
                var previousTransactionIndex = -1L
                var previousTransactionHash: String? = null
                val transactions = HashSet<String>()
                for (index in 0 until logs.length()) {
                    val log = logs.getJSONObject(index)
                    PrivateEngineWire.fields(log, "address", "topics", "data", "blockHash", "blockNumber", "transactionHash", "transactionIndex", "logIndex", "removed")
                    if (PrivateEngineWire.string(log, "address") != "0xfa7093cdd9ee6932b4eb2c9e1cde7ce00b1fa4b9" ||
                        PrivateEngineWire.string(log, "blockHash") != blockHash || quantity(log, "blockNumber") != number ||
                        PrivateEngineWire.boolean(log, "removed")) throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
                    val transactionHash = PrivateIdentifiers.require(PrivateEngineWire.string(log, "transactionHash"))
                    val transactionIndex = quantity(log, "transactionIndex")
                    val logIndex = quantity(log, "logIndex")
                    if (logIndex <= previousLogIndex || transactionIndex < previousTransactionIndex ||
                        (transactionIndex == previousTransactionIndex && transactionHash != previousTransactionHash) ||
                        (transactionIndex != previousTransactionIndex && !transactions.add(transactionHash))) {
                        throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
                    }
                    previousLogIndex = logIndex
                    previousTransactionIndex = transactionIndex
                    previousTransactionHash = transactionHash
                    val topics = log.getJSONArray("topics")
                    if (topics.length() != 1) throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
                    PrivateIdentifiers.require(topics.get(0) as? String ?: throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION))
                    val data = PrivateEngineWire.string(log, "data")
                    if (data.length !in 2..262_146 || !data.startsWith("0x") || data.length % 2 != 0 ||
                        data.substring(2).any { it !in '0'..'9' && it !in 'a'..'f' }) throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
                }
            }
            "root" -> {
                PrivateEngineWire.fields(result, *common, "tree_number", "root", "known", "block")
                if (result.getLong("tree_number") != request.getLong("tree_number") || result.getString("root") != request.getString("root")) {
                    throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
                }
                PrivateEngineWire.boolean(result, "known")
            }
            "nullifiers" -> {
                PrivateEngineWire.fields(result, *common, "tree_number", "nullifiers", "spent", "block")
                if (result.getLong("tree_number") != request.getLong("tree_number") ||
                    result.getJSONArray("nullifiers").toString() != request.getJSONArray("nullifiers").toString() ||
                    result.getJSONArray("spent").length() != 16 || (0 until 16).any { result.getJSONArray("spent").get(it) !is Boolean }) {
                    throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
                }
            }
            "receipt" -> PrivateEngineWire.fields(result, *common, "receipt", "finalized_block", "transaction_input")
        }
    }

    private fun quantity(value: JSONObject, field: String): Long {
        val text = PrivateEngineWire.string(value, field)
        if (text.length !in 3..16 || !Regex("0x(?:0|[1-9a-f][0-9a-f]*)").matches(text)) throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
        val number = BigInteger(text.substring(2), 16)
        if (number > BigInteger.valueOf(MAX_BLOCK)) throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
        return number.toLong()
    }
}

internal class PrivateEngineNativePorts(
    private val assets: PrivateEngineAssets,
    private val store: PrivateSdkSnapshotStore,
    private val pool: PrivatePoolReadTransport
) : PrivateEnginePorts, AutoCloseable {
    override fun request(port: String, payload: JSONObject): JSONObject = when (port) {
        "rpc" -> pool.read(PrivatePoolRequestPolicy.check(payload))
        "storage_read" -> store.read(payload)
        "storage_write" -> store.write(payload)
        "artifact_read" -> {
            PrivateEngineWire.fields(payload, "path", "offset", "length")
            assets.artifact(PrivateEngineWire.string(payload, "path"),
                PrivateEngineWire.number(payload, "offset", 0, 268_435_456),
                PrivateEngineWire.number(payload, "length", 1, 262_144).toInt())
        }
        else -> throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
    }
    override fun close() { store.close() }
}
