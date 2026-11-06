package app.localwallet.mainnet

import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.math.BigInteger
import java.net.URL
import java.nio.charset.StandardCharsets
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong
import javax.net.ssl.HttpsURLConnection

fun interface RpcTransport {
    fun request(endpoint: String, method: String, params: JSONArray): Any
}

/** Error text supplied by an RPC is never displayed verbatim. */
class HttpRpcTransport : RpcTransport {
    companion object { private val requestIds = AtomicLong(0) }
    override fun request(endpoint: String, method: String, params: JSONArray): Any {
        val id = requestIds.incrementAndGet()
        val body = JSONObject().put("jsonrpc", "2.0").put("id", id).put("method", method).put("params", params)
            .toString().toByteArray(StandardCharsets.UTF_8)
        val connection = URL(endpoint).openConnection() as HttpsURLConnection
        try {
            connection.requestMethod = "POST"
            connection.connectTimeout = 7_000
            connection.readTimeout = 10_000
            connection.instanceFollowRedirects = false
            connection.useCaches = false
            connection.setRequestProperty("Content-Type", "application/json")
            connection.setRequestProperty("Accept", "application/json")
            connection.setRequestProperty("Accept-Encoding", "identity")
            connection.setFixedLengthStreamingMode(body.size)
            connection.doOutput = true
            connection.outputStream.use { it.write(body) }
            if (connection.responseCode != 200) throw java.io.IOException("RPC HTTP unavailable")
            if (connection.contentLengthLong > 1_048_576) throw WalletFailure("RPC response exceeds the size limit.")
            val bytes = ByteArrayOutputStream()
            connection.inputStream.use { stream ->
                val buffer = ByteArray(8192)
                while (true) {
                    val count = stream.read(buffer)
                    if (count == -1) break
                    if (bytes.size() + count > 1_048_576) throw WalletFailure("RPC response exceeds the size limit.")
                    bytes.write(buffer, 0, count)
                }
            }
            val response = RpcJson.parse(bytes.toString(StandardCharsets.UTF_8.name()))
            if (response.optString("jsonrpc") != "2.0" || response.opt("id")?.toString() != id.toString()) {
                throw WalletFailure("RPC response identity did not match the request.")
            }
            if (response.has("error")) {
                val error = response.optJSONObject("error") ?: throw WalletFailure("Malformed RPC error.")
                val code = error.optInt("code", -1)
                val message = error.optString("message").take(512).lowercase(java.util.Locale.ROOT)
                val kind = when {
                    code == -32601 || code == -32602 -> RpcRejectionKind.UNSUPPORTED
                    "insufficient funds" in message || "exceeds balance" in message -> RpcRejectionKind.INSUFFICIENT_FUNDS
                    code == 3 || "execution reverted" in message -> RpcRejectionKind.REVERTED
                    else -> RpcRejectionKind.UNAVAILABLE
                }
                throw RpcRejected(kind, code)
            }
            if (!response.has("result")) throw WalletFailure("RPC response has no result.")
            return response.get("result")
        } finally { connection.disconnect() }
    }
}

/** Reads may fail over; a broadcast is attempted once per explicit submission. */
class NetworkClient(private val network: WalletNetwork, private val transport: RpcTransport = HttpRpcTransport()) {
    companion object {
        private val preferred = ConcurrentHashMap<Long, String>()
        private val reads = mapOf(
            1L to listOf("https://ethereum-rpc.publicnode.com", "https://eth-mainnet.g.alchemy.com/public"),
            8453L to listOf("https://base-rpc.publicnode.com", "https://mainnet.base.org"),
            10L to listOf("https://optimism-rpc.publicnode.com", "https://mainnet.optimism.io"),
            42161L to listOf("https://arbitrum-one-rpc.publicnode.com", "https://arb1.arbitrum.io/rpc"),
            43114L to listOf("https://api.avax.network/ext/bc/C/rpc", "https://avalanche-c-chain-rpc.publicnode.com"),
            137L to listOf("https://polygon-bor-rpc.publicnode.com"),
            56L to listOf("https://bsc-dataseed.bnbchain.org", "https://bsc-rpc.publicnode.com")
        )
        private val broadcasts = mapOf(
            1L to "https://ethereum-rpc.publicnode.com", 8453L to "https://mainnet-sequencer.base.org",
            10L to "https://mainnet-sequencer.optimism.io", 42161L to "https://arb1-sequencer.arbitrum.io/rpc",
            43114L to "https://api.avax.network/ext/bc/C/rpc", 137L to "https://polygon-bor-rpc.publicnode.com",
            56L to "https://bsc-dataseed.bnbchain.org")
        private val readMethods = setOf("eth_chainId", "eth_getBlockByNumber", "eth_getBlockByHash", "eth_getBalance", "eth_getCode",
            "eth_call", "eth_estimateGas", "eth_getTransactionCount", "eth_maxPriorityFeePerGas", "eth_getTransactionReceipt",
            "eth_getTransactionByHash", "eth_getLogs", "eth_gasPrice")
        const val ORACLE = "0x420000000000000000000000000000000000000F"
        const val L1_UPPER_BOUND_SELECTOR = "0xf1c7a58b"
        const val OPERATOR_FEE_SELECTOR = "0x275aedd2"
        const val TRANSFER_TOPIC = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
        fun endpoints(chainId: Long): List<String> = reads[chainId] ?: throw WalletFailure("Unsupported network.")
        internal fun clearPreferenceForTests() = preferred.clear()
    }
    private val checked = HashSet<String>()
    private val blockHeights = HashMap<String, String>()
    var lastEndpoint: String? = null
        private set

    fun call(method: String, params: JSONArray, broadcast: Boolean = false): Any {
        if (broadcast) {
            if (method != "eth_sendRawTransaction") throw WalletFailure("Invalid submission operation.")
            val endpoint = broadcasts[network.chainId] ?: throw WalletFailure("Unsupported network.")
            return transport.request(endpoint, method, params)
        }
        if (method !in readMethods) throw WalletFailure("Unsupported read operation.")
        val endpoints = endpoints(network.chainId).sortedBy { if (it == preferred[network.chainId]) 0 else 1 }
        for (endpoint in endpoints) {
            try {
                if (endpoint !in checked) {
                    val actual = transport.request(endpoint, "eth_chainId", JSONArray()) as? String
                        ?: throw WalletFailure("Malformed RPC chain identity.")
                    if (Amounts.longQuantity(actual) != network.chainId) throw WalletFailure("RPC network mismatch.")
                    checked.add(endpoint)
                }
                val result = if (method == "eth_chainId") Amounts.hex(BigInteger.valueOf(network.chainId)) else try {
                    transport.request(endpoint, method, params)
                } catch (failure: RpcRejected) {
                    if (failure.kind != RpcRejectionKind.UNSUPPORTED) throw failure
                    readAtNumberIfNeeded(endpoint, method, params, failure)
                }
                preferred[network.chainId] = endpoint
                lastEndpoint = endpoint
                return result
            } catch (failure: RpcRejected) {
                if (failure.kind == RpcRejectionKind.INSUFFICIENT_FUNDS || failure.kind == RpcRejectionKind.REVERTED) throw failure
            } catch (_: Exception) {
                // Another fixed, independently chain-checked read provider may be available.
            }
            checked.remove(endpoint)
        }
        throw NetworkReadUnavailable(network.name)
    }

    private fun readAtNumberIfNeeded(endpoint: String, method: String, params: JSONArray, original: RpcRejected): Any {
        val index = params.length() - 1
        val reference = if (index >= 0) params.optJSONObject(index) else null
        val hash = reference?.optString("blockHash")?.lowercase(java.util.Locale.ROOT) ?: throw original
        val number = blockHeights[hash] ?: throw original
        fun assertBlock() {
            val block = transport.request(endpoint, "eth_getBlockByNumber", JSONArray().put(number).put(false)) as? JSONObject
                ?: throw original
            if (Amounts.hash(block.getString("hash")) != hash || Amounts.longQuantity(block.getString("number")) != Amounts.longQuantity(number)) {
                throw WalletFailure("Network reorganized during a pinned read.")
            }
        }
        assertBlock()
        val compatible = JSONArray(params.toString()).put(index, number)
        val result = transport.request(endpoint, method, compatible)
        assertBlock()
        return result
    }

    fun assertChain() { call("eth_chainId", JSONArray()) }
    fun string(method: String, params: JSONArray): String = call(method, params) as? String
        ?: throw WalletFailure("The network returned an unexpected response type.")
    fun block(tag: String): JSONObject {
        val result = call("eth_getBlockByNumber", JSONArray().put(tag).put(false)) as? JSONObject
            ?: throw WalletFailure("The requested canonical block is unavailable.")
        val height = Amounts.longQuantity(result.getString("number"))
        if (height > 9_007_199_254_740_991L) throw WalletFailure("The network block height exceeds supported limits.")
        Amounts.hash(result.getString("hash"))
        if (tag.startsWith("0x") && height != Amounts.longQuantity(tag)) throw WalletFailure("RPC returned a different block height.")
        return result
    }
    fun reference(block: JSONObject): JSONObject {
        val hash = Amounts.hash(block.getString("hash"))
        blockHeights[hash] = block.getString("number")
        return JSONObject().put("blockHash", hash).put("requireCanonical", true)
    }
    fun contractCall(contract: String, data: String, at: Any): String = string("eth_call", JSONArray().put(JSONObject().put("to", contract).put("data", data)).put(at))
    fun oracleFee(selector: String, value: BigInteger, at: Any): BigInteger =
        Amounts.abiInteger(contractCall(ORACLE, selector + Amounts.word(value), at))
    // Arbitrum's L1 posting cost is already included in eth_estimateGas. Calling an OP
    // oracle or adding a separate L1 estimate there would double-count the fee.
    fun l1Estimate(at: Any): BigInteger = if (network.feeProfile == NetworkFeeProfile.OP_STACK)
        oracleFee(L1_UPPER_BOUND_SELECTOR, BigInteger.valueOf(1024), at) else BigInteger.ZERO
    fun operatorEstimate(gas: Long, at: Any): BigInteger = if (network.feeProfile == NetworkFeeProfile.OP_STACK)
        oracleFee(OPERATOR_FEE_SELECTOR, BigInteger.valueOf(gas), at) else BigInteger.ZERO
    fun normalizeReceipt(receipt: JSONObject): JSONObject {
        val copy = JSONObject(receipt.toString())
        if (network.feeProfile == NetworkFeeProfile.OP_STACK) {
            if (!copy.has("l1Fee")) throw WalletFailure("${network.name} receipt is missing its L1 fee. Reconciliation remains unresolved.")
            Amounts.quantity(copy.getString("l1Fee"))
            val gas = Amounts.quantity(copy.getString("gasUsed"))
            val canonical = block(copy.getString("blockNumber"))
            if (!canonical.getString("hash").equals(copy.getString("blockHash"), true)) throw WalletFailure("Receipt block is no longer canonical.")
            val operator = oracleFee(OPERATOR_FEE_SELECTOR, gas, reference(canonical))
            if (copy.has("operatorFee") && Amounts.quantity(copy.getString("operatorFee")) != operator) throw WalletFailure("${network.name} operator fee evidence is inconsistent.")
            copy.put("operatorFee", Amounts.hex(operator))
        }
        return copy
    }
}
