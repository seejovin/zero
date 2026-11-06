package app.localwallet.mainnet

import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.math.BigInteger
import java.net.URL
import java.nio.charset.StandardCharsets
import java.util.Locale
import java.util.UUID
import java.util.concurrent.atomic.AtomicLong
import javax.net.ssl.HttpsURLConnection
import uniffi.wallet_core.tronAddressHex
import uniffi.wallet_core.tronNormalizeAddress
import uniffi.wallet_core.tronPrepare
import uniffi.wallet_core.tronValidatePrepared
import uniffi.wallet_core.tronValidateSigned

/** All endpoints are fixed; no provider can supply a transaction for local signing. */
fun interface TronTransport {
    fun post(path: String, body: JSONObject): JSONObject
    fun get(path: String): JSONObject = throw NetworkReadUnavailable("TRON history")
}

class HttpsTronTransport(private val apiKey: String? = null) : TronTransport {
    init {
        if (apiKey != null && (apiKey.length !in 8..256 || !apiKey.all { it.code in 33..126 })) {
            throw WalletFailure("The TronGrid API key has an invalid format.")
        }
    }
    override fun post(path: String, body: JSONObject): JSONObject {
        if (path !in TronNetworkAdapter.ALLOWED_PATHS) throw WalletFailure("Unsupported TRON operation.")
        return exchange(path, body)
    }
    override fun get(path: String): JSONObject {
        // Addresses and fixed parameters are the only path inputs. Discovery is
        // never permitted to redirect or select a different API host.
        val address = "T[1-9A-HJ-NP-Za-km-z]{33}"
        val native = Regex("v1/accounts/$address/transactions\\?limit=20&only_confirmed=true&only_to=true")
        val token = Regex("v1/accounts/$address/transactions/trc20\\?limit=20&only_confirmed=true&contract_address=${TronNetworkAdapter.USDT}")
        if (!native.matches(path) && !token.matches(path)) throw WalletFailure("Unsupported TRON discovery path.")
        return exchange(path, null)
    }
    private fun exchange(path: String, body: JSONObject?): JSONObject {
        val connection = URL("${TronNetworkAdapter.ORIGIN}/$path").openConnection() as HttpsURLConnection
        try {
            connection.requestMethod = if (body == null) "GET" else "POST"
            connection.connectTimeout = 7_000
            connection.readTimeout = 12_000
            connection.instanceFollowRedirects = false
            connection.useCaches = false
            connection.setRequestProperty("Content-Type", "application/json")
            connection.setRequestProperty("Accept", "application/json")
            apiKey?.let { connection.setRequestProperty("TRON-PRO-API-KEY", it) }
            if (body != null) {
                connection.doOutput = true
                val bytes = body.toString().toByteArray(StandardCharsets.UTF_8)
                connection.setFixedLengthStreamingMode(bytes.size)
                connection.outputStream.use { it.write(bytes) }
            }
            if (connection.responseCode in setOf(401, 403, 429)) throw WalletFailure("TRON provider access or quota is unavailable. A configured TronGrid API key is required for production use.")
            if (connection.responseCode != 200) throw NetworkReadUnavailable("TRON")
            val output = ByteArrayOutputStream()
            connection.inputStream.use { stream ->
                val buffer = ByteArray(8192)
                while (true) {
                    val count = stream.read(buffer)
                    if (count < 0) break
                    if (output.size() + count > 4 * 1024 * 1024) throw WalletFailure("TRON response exceeds the size limit.")
                    output.write(buffer, 0, count)
                }
            }
            // RpcJson rejects duplicate object keys and excessive nesting.
            return RpcJson.parse(output.toString(StandardCharsets.UTF_8.name()), maxChars = 4 * 1024 * 1024, maxNodes = 200_000)
        } finally { connection.disconnect() }
    }
}

/** Public network access only: this object cannot retrieve keys or sign. */
class TronNetworkAdapter(
    override val network: WalletNetwork,
    override val address: String,
    private val transport: TronTransport = HttpsTronTransport(),
    private val clock: () -> Long = { System.currentTimeMillis() }
) : FamilyNetworkAdapter {
    companion object {
        const val ORIGIN = "https://api.trongrid.io"
        const val USDT = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"
        const val NETWORK = "tron-mainnet"
        const val MAINNET_CHAIN_ID = "0x2b6653dc"
        private val ids = AtomicLong()
        internal val ALLOWED_PATHS = setOf(
            "jsonrpc", "wallet/getnowblock", "wallet/getblockbynum", "wallet/getaccount",
            "wallet/getaccountresource", "wallet/getchainparameters", "wallet/triggerconstantcontract",
            "wallet/broadcasttransaction", "walletsolidity/getnowblock", "walletsolidity/getblockbynum",
            "walletsolidity/gettransactionbyid", "walletsolidity/gettransactioninfobyid"
        )
        private val PARAMS = mapOf(
            "bandwidth_price_sun" to "getTransactionFee", "energy_price_sun" to "getEnergyFee",
            "creation_fee_sun" to "getCreateNewAccountFeeInSystemContract",
            "creation_bandwidth_fee_sun" to "getCreateAccountFee",
            "creation_bandwidth_rate" to "getCreateNewAccountBandwidthRate",
            "max_fee_limit_sun" to "getMaxFeeLimit", "max_energy_factor" to "getDynamicEnergyMaxFactor"
        )
        private val NUMERIC_PARAMS = setOf("creation_bandwidth_rate", "max_energy_factor")
        internal fun integer(value: Any?, field: String): BigInteger {
            if (value !is Number && value !is String) throw WalletFailure("TRON returned a missing or invalid $field.")
            val text = value.toString()
            if (!Regex("(?:0|[1-9][0-9]{0,77})").matches(text)) throw WalletFailure("TRON returned a malformed integer.")
            return BigInteger(text)
        }
        private fun number(json: JSONObject, field: String, defaultZero: Boolean = false): Long {
            if (!json.has(field) && defaultZero) return 0
            val result = integer(json.opt(field), field)
            if (result.bitLength() > 63) throw WalletFailure("TRON integer exceeds the supported range.")
            return result.toLong()
        }
        private fun hash(input: String): String {
            if (!Regex("[0-9a-fA-F]{64}").matches(input)) throw WalletFailure("TRON returned a malformed transaction or block identifier.")
            return input.lowercase(Locale.ROOT)
        }
        private fun abi(input: String): BigInteger = BigInteger(hash(input), 16)
    }
    init {
        if (network.chainId != -195L || network.rpcUrl != ORIGIN || network.broadcastUrl != ORIGIN || tronNormalizeAddress(address) != address) {
            throw WalletFailure("Unsupported TRON network configuration.")
        }
    }
    private val ownerHex: String get() = tronAddressHex(address)
    private val tokenHex: String get() = tronAddressHex(USDT)
    private fun read(path: String, body: JSONObject = JSONObject()): JSONObject {
        if (path == "wallet/broadcasttransaction") throw WalletFailure("A submission cannot be performed as a read.")
        val result = transport.post(path, body)
        if (result.has("Error") || result.has("error") || result.has("ErrorCode")) throw NetworkReadUnavailable("TRON")
        return result
    }
    private fun assertMainnet() {
        val id = ids.incrementAndGet()
        val response = read("jsonrpc", JSONObject().put("jsonrpc", "2.0").put("id", id).put("method", "eth_chainId").put("params", JSONArray()))
        if (response.optString("jsonrpc") != "2.0" || response.opt("id")?.toString() != id.toString() || response.optString("result") != MAINNET_CHAIN_ID) {
            throw WalletFailure("The connection did not identify TRON mainnet.")
        }
    }
    private data class Head(val number: Long, val id: String, val timestamp: Long)
    private fun parseBlock(block: JSONObject, fresh: Boolean = false): Head {
        val raw = block.getJSONObject("block_header").getJSONObject("raw_data")
        val height = number(raw, "number")
        val timestamp = number(raw, "timestamp")
        val id = hash(block.getString("blockID"))
        if (height <= 0 || id.take(16) != height.toString(16).padStart(16, '0')) throw WalletFailure("TRON returned an inconsistent block identifier.")
        if (fresh && (timestamp > clock() + 5_000 || clock() - timestamp > 60_000)) throw WalletFailure("The TRON connection is behind the network. Try again later.")
        return Head(height, id, timestamp)
    }
    private data class Account(val active: Boolean, val balance: BigInteger, val contract: Boolean, val objectValue: JSONObject)
    private fun account(target: String): Account {
        val hex = tronAddressHex(target)
        val result = read("wallet/getaccount", JSONObject().put("address", hex).put("visible", false))
        if (result.length() == 0) return Account(false, BigInteger.ZERO, false, result)
        if (result.optString("address").lowercase(Locale.ROOT) != hex) throw WalletFailure("TRON account identity did not match the requested address.")
        val balance = BigInteger.valueOf(number(result, "balance", true))
        val type = result.opt("type")?.toString()
        if (type != null && type !in setOf("Normal", "0", "Contract", "2")) throw WalletFailure("This TRON account type is unsupported.")
        return Account(true, balance, type == "Contract" || type == "2", result)
    }
    private fun assertOwner(account: Account) {
        if (!account.active) throw WalletFailure("Deposit TRX first to activate this TRON account and pay network fees.")
        if (account.contract) throw WalletFailure("Contract accounts cannot be controlled by this wallet.")
        val permission = account.objectValue.optJSONObject("owner_permission") ?: return // Protocol default: the account's own key.
        val keys = permission.optJSONArray("keys") ?: throw WalletFailure("TRON account ownership is unsupported.")
        if (number(permission, "id", true) != 0L || number(permission, "threshold") != 1L || keys.length() != 1) throw WalletFailure("This wallet does not support changed or multisignature TRON ownership.")
        val key = keys.getJSONObject(0)
        if (key.optString("address").lowercase(Locale.ROOT) != ownerHex || number(key, "weight") != 1L) throw WalletFailure("The TRON ownership permission does not match this wallet.")
    }
    private fun constant(selector: String, parameter: String, tokenOwner: String = ownerHex): JSONObject {
        val result = read("wallet/triggerconstantcontract", JSONObject().put("owner_address", tokenOwner).put("contract_address", tokenHex)
            .put("function_selector", selector).put("parameter", parameter).put("visible", false))
        if (result.optJSONObject("result")?.opt("result") != true) throw WalletFailure("The USDT contract read or transfer simulation was rejected. Nothing was signed.")
        return result
    }
    private fun tokenBalance(): BigInteger {
        val results = constant("balanceOf(address)", ownerHex.drop(2).padStart(64, '0')).getJSONArray("constant_result")
        if (results.length() != 1) throw WalletFailure("TRON returned an ambiguous USDT balance.")
        return abi(results.getString(0))
    }
    override fun balance(): FamilyBalance {
        assertMainnet()
        val head = parseBlock(read("wallet/getnowblock"), true)
        val owner = account(address)
        val tokens = tokenBalance()
        // REST latest-state reads cannot be pinned to an historical block.
        // Accept only a stable head around both reads. This is optimistic RPC
        // consistency, not a Merkle proof or a claim of consensus finality.
        val unchanged = parseBlock(read("wallet/getnowblock"), true)
        if (unchanged.number != head.number || unchanged.id != head.id) {
            throw WalletFailure("The TRON network head changed while balances were being read. Refresh to get a consistent observation.")
        }
        return FamilyBalance(head.number, head.id, mapOf("TRX" to owner.balance, "USDT" to tokens), clock())
    }
    private fun parameters(): Map<String, Long> {
        val items = read("wallet/getchainparameters").getJSONArray("chainParameter")
        val result = HashMap<String, Long>()
        for (i in 0 until items.length()) {
            val entry = items.getJSONObject(i)
            val key = entry.getString("key")
            if (key in PARAMS.values) {
                if (result.containsKey(key)) throw WalletFailure("TRON returned duplicate fee parameters.")
                result[key] = number(entry, "value", true)
            }
        }
        if (!result.keys.containsAll(PARAMS.values)) throw WalletFailure("Current TRON network fee parameters are unavailable.")
        return result
    }
    private fun remaining(resource: JSONObject, limit: String, used: String): Long =
        (number(resource, limit, true) - number(resource, used, true)).coerceAtLeast(0)

    private fun request(symbol: String, recipient: String, amount: String, maximum: Boolean, id: String): JSONObject {
        val startedAt = clock()
        if (symbol !in setOf("TRX", "USDT")) throw WalletFailure("TRON supports TRX and official USDT only.")
        tronNormalizeAddress(recipient)
        assertMainnet()
        val owner = account(address)
        assertOwner(owner)
        val recipientAccount = account(recipient)
        if (recipientAccount.contract) throw WalletFailure("Use a personal TRON receiving address. Sending to contract addresses is not supported.")
        val balance = if (symbol == "USDT") tokenBalance() else BigInteger.ZERO
        val units = if (maximum && symbol == "USDT") balance else if (maximum) BigInteger.ONE else Amounts.parse(amount, 6)
        if (units <= BigInteger.ZERO) throw WalletFailure("There is no balance available to send.")
        val params = parameters()
        val resources = read("wallet/getaccountresource", JSONObject().put("address", ownerHex).put("visible", false))
        // A wholly empty resource reply for an existing account is not evidence
        // of zero usage/limits. Normal replies contain these global totals.
        if (!resources.has("TotalNetLimit") || !resources.has("TotalEnergyLimit")) throw WalletFailure("Current TRON resource information is unavailable.")
        var energy = 0L
        var penalty = 0L
        if (symbol == "USDT") {
            val simulation = constant("transfer(address,uint256)", tronAddressHex(recipient).drop(2).padStart(64, '0') + Amounts.word(units))
            val results = simulation.getJSONArray("constant_result")
            if (results.length() != 1 || abi(results.getString(0)) != BigInteger.ONE) throw WalletFailure("The exact USDT transfer did not simulate successfully.")
            energy = number(simulation, "energy_used")
            penalty = number(simulation, "energy_penalty", true)
        }
        val head = parseBlock(read("wallet/getnowblock"), true)
        if (clock() - startedAt > 30_000) throw WalletFailure("The TRON fee read took too long. Refresh to get a current estimate.")
        val request = JSONObject().put("intent_id", id).put("network", NETWORK).put("from", address).put("to", recipient)
            .put("asset", symbol).put("amount", if (maximum) "0" else amount).put("maximum", maximum)
            .put("balance_sun", owner.balance.toString()).put("token_balance_units", balance.toString())
            .put("ref_block_id", head.id).put("ref_block_number", head.number).put("ref_block_timestamp_ms", head.timestamp)
            .put("observed_at_ms", clock()).put("recipient_active", recipientAccount.active)
            .put("staked_bandwidth", remaining(resources, "NetLimit", "NetUsed"))
            .put("free_bandwidth", remaining(resources, "freeNetLimit", "freeNetUsed"))
            .put("available_energy", remaining(resources, "EnergyLimit", "EnergyUsed"))
            .put("energy_used", energy).put("energy_penalty", penalty)
        PARAMS.forEach { (field, key) ->
            val value = params.getValue(key)
            request.put(field, if (field in NUMERIC_PARAMS) value else value.toString())
        }
        return request
    }
    override fun prepare(symbol: String, recipient: String, amount: String, maximum: Boolean): FamilyPrepared {
        val input = request(symbol, recipient, amount, maximum, UUID.randomUUID().toString())
        val json = tronPrepare(input.toString())
        val prepared = JSONObject(json)
        return FamilyPrepared(json, BigInteger(prepared.getString("estimated_fee_units")), BigInteger(prepared.getString("fee_reserve_units")),
            BigInteger(input.getString("balance_sun")), BigInteger(input.getString(if (symbol == "TRX") "balance_sun" else "token_balance_units")),
            if (symbol == "USDT") "Paid in TRX. The reserve covers bandwidth and a conservative energy limit. Actual network fees can be lower."
            else if (!input.getBoolean("recipient_active")) "Includes the receiving account's activation fee. Max leaves the reviewed network fee reserve."
            else "Paid in TRX. Max leaves the reviewed bandwidth fee reserve; available resources may reduce the final fee.",
            minOf(prepared.getLong("expires_at") - 15_000, prepared.getLong("created_at") + 120_000))
    }
    private fun bound(preparedJson: String): JSONObject {
        val prepared = JSONObject(tronValidatePrepared(preparedJson))
        if (prepared.getJSONObject("request").getString("from") != address) throw WalletFailure("This TRON review belongs to another wallet.")
        return prepared
    }
    override fun revalidate(preparedJson: String) {
        val old = bound(preparedJson)
        val oldRequest = old.getJSONObject("request")
        if (clock() > old.getLong("created_at") + 120_000 || clock() + 15_000 >= old.getLong("expires_at")) throw WalletFailure("This transfer review expired. Review the amount and fees again.")
        val exactAmount = Amounts.display(BigInteger(old.getString("amount_units")), 6)
        val freshInput = request(oldRequest.getString("asset"), oldRequest.getString("to"), exactAmount, false, oldRequest.getString("intent_id"))
        val fresh = JSONObject(tronPrepare(freshInput.toString()))
        val reference = parseBlock(read("wallet/getblockbynum", JSONObject().put("num", oldRequest.getLong("ref_block_number"))))
        if (reference.id != oldRequest.getString("ref_block_id")) throw WalletFailure("The TRON reference block changed. Review a new transfer.")
        if (fresh.getString("amount_units") != old.getString("amount_units") ||
            BigInteger(fresh.getString("fee_reserve_units")) > BigInteger(old.getString("fee_reserve_units")) ||
            BigInteger(fresh.getString("fee_limit_sun")) > BigInteger(old.getString("fee_limit_sun")) ||
            freshInput.getBoolean("recipient_active") != oldRequest.getBoolean("recipient_active")) {
            throw WalletFailure("TRON fee or recipient conditions changed. Review a new transfer before signing.")
        }
        val stillNeeded = BigInteger(old.getString("fee_reserve_units")) +
            if (oldRequest.getString("asset") == "TRX") BigInteger(old.getString("amount_units")) else BigInteger.ZERO
        if (BigInteger(freshInput.getString("balance_sun")) < stillNeeded) throw WalletFailure("The TRX balance no longer covers the reviewed fee reserve.")
        // The exact original prepared bytes remain untouched.
    }
    override fun broadcast(signedJson: String): String {
        val signed = JSONObject(tronValidateSigned(signedJson))
        if (signed.getString("from") != address) throw WalletFailure("The signed transfer belongs to another wallet.")
        requireRebroadcastable(signed.getString("prepared_json"), signedJson)
        // Only this single call may write to the network. A timeout is uncertain.
        val result = transport.post("wallet/broadcasttransaction", signed.getJSONObject("broadcast"))
        if (result.opt("result") != true) throw WalletFailure("TRON did not acknowledge this submission. Its final outcome must be checked by transaction ID before any new transfer.")
        val returned = result.optString("txid")
        if (returned.isNotEmpty() && hash(returned) != signed.getString("hash")) throw WalletFailure("TRON returned a different transaction ID. Check the original transaction's outcome.")
        return signed.getString("hash")
    }
    override fun requireRebroadcastable(preparedJson: String, signedJson: String) {
        val prepared = bound(preparedJson)
        val signed = JSONObject(tronValidateSigned(signedJson))
        if (signed.getString("approval_digest") != prepared.getString("approval_digest")) throw WalletFailure("Signed TRON bytes do not match this review.")
        assertMainnet()
        val head = parseBlock(read("wallet/getnowblock"), true)
        if (clock() + 15_000 >= prepared.getLong("expires_at") || head.timestamp + 15_000 >= prepared.getLong("expires_at")) {
            throw WalletFailure("This TRON transaction has expired for broadcasting. Its recorded outcome must still be checked; expiration alone does not prove it was never received.")
        }
        val input = prepared.getJSONObject("request")
        val reference = parseBlock(read("wallet/getblockbynum", JSONObject().put("num", input.getLong("ref_block_number"))))
        if (reference.id != input.getString("ref_block_id")) throw WalletFailure("The signed TRON reference block is no longer canonical.")
    }
    override fun receiptEvidence(preparedJson: String, signedJson: String): String? {
        val prepared = bound(preparedJson)
        val signed = JSONObject(tronValidateSigned(signedJson))
        if (signed.getString("approval_digest") != prepared.getString("approval_digest")) throw WalletFailure("Receipt lookup does not match the reviewed transfer.")
        return activityEvidence(signed.getString("hash"))
    }
    override fun activityEvidence(transactionId: String): String? {
        val id = hash(transactionId.removePrefix("0x"))
        assertMainnet()
        val body = JSONObject().put("value", id)
        val tx = read("walletsolidity/gettransactionbyid", body)
        val info = read("walletsolidity/gettransactioninfobyid", body)
        if (tx.length() == 0 || info.length() == 0) return null
        if (hash(tx.getString("txID")) != id || hash(info.getString("id")) != id) throw WalletFailure("TRON returned evidence for a different transaction.")
        val block = read("walletsolidity/getblockbynum", JSONObject().put("num", number(info, "blockNumber")))
        val included = parseBlock(block)
        val solid = parseBlock(read("walletsolidity/getnowblock"))
        if (solid.number < included.number) throw WalletFailure("TRON finality information is inconsistent.")
        // Rust checks raw_data hash, restricted contract, exact Transfer event,
        // receipt execution and matching transaction inclusion in this block.
        return JSONObject().put("network", NETWORK).put("chain_id", MAINNET_CHAIN_ID).put("finality_source", "walletsolidity")
            .put("transaction", tx).put("info", info).put("block", block).put("solid_height", solid.number).toString()
    }
    override fun recentTransactionIds(): List<String> {
        assertMainnet()
        val candidates = ArrayList<Pair<Long, String>>()
        var successfulSources = 0
        for ((path, idField) in listOf(
            "v1/accounts/$address/transactions?limit=20&only_confirmed=true&only_to=true" to "txID",
            "v1/accounts/$address/transactions/trc20?limit=20&only_confirmed=true&contract_address=$USDT" to "transaction_id"
        )) {
            try {
                val response = transport.get(path)
                if (response.opt("success") != true) throw NetworkReadUnavailable("TRON history")
                val data = response.getJSONArray("data")
                if (data.length() > 20) throw WalletFailure("TRON history exceeded its requested bound.")
                for (i in 0 until data.length()) {
                    val row = data.getJSONObject(i)
                    candidates.add(number(row, "block_timestamp") to hash(row.getString(idField)))
                }
                successfulSources++
            } catch (_: Exception) {
                // Discovery is a hint, not receipt authority; another fixed
                // history source may still provide verifiable candidates.
            }
        }
        if (successfulSources == 0) throw NetworkReadUnavailable("TRON history")
        return candidates.sortedByDescending { it.first }.map { it.second }.distinct().take(20)
    }
}
