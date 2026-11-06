package app.localwallet.mainnet

import org.json.JSONArray
import org.json.JSONObject
import java.math.BigDecimal
import java.math.BigInteger
import java.util.Base64
import java.util.UUID
import uniffi.wallet_core.solanaCheckExpiry
import uniffi.wallet_core.solanaPrepare
import uniffi.wallet_core.solanaTokenAccount
import uniffi.wallet_core.solanaValidateAddress
import uniffi.wallet_core.solanaValidatePrepared

/**
 * Public Solana data only. No seed, vault, signature creation, account mutation or
 * automatic replacement lives here. Every submission is one identical signed
 * wire transaction. The Rust journal has committed those bytes before this call.
 */
class SolanaNetworkAdapter(
    override val network: WalletNetwork,
    override val address: String,
    private val transport: RpcTransport = HttpRpcTransport()
) : FamilyNetworkAdapter {
    companion object {
        const val LOCAL_NETWORK_KEY = -101L
        const val NETWORK = "solana-mainnet"
        const val GENESIS_HASH = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"
        const val SYSTEM_PROGRAM = "11111111111111111111111111111111"
        const val TOKEN_PROGRAM = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
        val MINTS: Map<String, String> = linkedMapOf(
            "USDC" to "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "EURC" to "HzwqbKZw8HxMN6bF2yFZNrht3c2iXXzpKcFu7uBEDKtr",
            "USDT" to "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB"
        )
        private val endpoints = listOf("https://api.mainnet-beta.solana.com", "https://solana-rpc.publicnode.com")
        private val readMethods = setOf("getGenesisHash", "getSlot", "getBlockHeight", "getBlock", "getBalance",
            "getAccountInfo", "getMultipleAccounts", "getTokenAccountsByOwner", "getLatestBlockhash",
            "getFeeForMessage", "getMinimumBalanceForRentExemption", "getRecentPrioritizationFees",
            "simulateTransaction", "getSignatureStatuses", "getTransaction", "isBlockhashValid", "getSignaturesForAddress")
        private val ZERO = BigInteger.ZERO
        private val ONE = BigInteger.ONE
        private val FEE_CEILING = BigInteger("50000000")
        private val MAX_U64 = BigInteger.ONE.shiftLeft(64) - ONE

        internal fun exactInteger(value: Any): BigInteger {
            if (value is String && !Regex("(?:0|[1-9][0-9]{0,19})").matches(value)) {
                throw WalletFailure("Solana returned a noncanonical integer amount.")
            }
            val result = try { BigDecimal(value.toString()).toBigIntegerExact() } catch (_: Exception) {
                throw WalletFailure("Solana returned a malformed integer.")
            }
            if (result < ZERO || result > MAX_U64) throw WalletFailure("Solana returned an out-of-range integer.")
            return result
        }
        internal fun exactLong(value: Any): Long {
            val result = exactInteger(value)
            if (result > BigInteger.valueOf(Long.MAX_VALUE)) throw WalletFailure("Solana returned an out-of-range counter.")
            // BigInteger.longValueExact is unavailable on Android 11 (API 30).
            // exactInteger already rejects negatives; the upper bound prevents truncation.
            return result.toLong()
        }
        internal fun allocate(amount: BigInteger, accounts: List<SolanaTokenAccount>): List<Pair<String, BigInteger>> {
            if (amount <= ZERO) throw WalletFailure("Enter an amount greater than zero.")
            var remaining = amount
            val allocations = mutableListOf<Pair<String, BigInteger>>()
            // Larger accounts first minimizes instruction count without truncating
            // any decimals. Equal balances use a deterministic address ordering.
            for (account in accounts.sortedWith(compareByDescending<SolanaTokenAccount> { it.amount }.thenBy { it.address })) {
                if (remaining == ZERO) break
                if (account.frozen) continue
                val take = remaining.min(account.amount)
                if (take > ZERO) { allocations += account.address to take; remaining -= take }
            }
            if (remaining != ZERO) throw WalletFailure("The selected asset has insufficient spendable funds or a frozen token account.")
            if (allocations.size > 8) throw WalletFailure("This balance uses more than eight token accounts. A single transfer cannot safely include all of them.")
            return allocations
        }
        internal fun maximumNative(balance: BigInteger, fee: BigInteger): BigInteger {
            val amount = balance - fee
            if (fee <= ZERO || amount <= ZERO) throw WalletFailure("Add SOL to cover the network fee before sending.")
            return amount
        }
    }

    private val checked = HashSet<String>()
    private var preferred: String = endpoints.first()

    init {
        if (network.chainId != LOCAL_NETWORK_KEY) throw WalletFailure("Wrong network for the Solana adapter.")
        solanaValidateAddress(address)
    }

    private fun checkEndpoint(endpoint: String, force: Boolean = false) {
        if (force || endpoint !in checked) {
            if (transport.request(endpoint, "getGenesisHash", JSONArray()) != GENESIS_HASH) {
                checked.remove(endpoint)
                throw WalletFailure("The Solana connection returned the wrong mainnet identity.")
            }
            checked += endpoint
        }
    }

    private fun call(method: String, params: JSONArray = JSONArray()): Any {
        if (method !in readMethods) throw WalletFailure("Unsupported Solana read.")
        for (endpoint in endpoints.sortedBy { if (it == preferred) 0 else 1 }) {
            try {
                checkEndpoint(endpoint)
                val value = if (method == "getGenesisHash") GENESIS_HASH else transport.request(endpoint, method, params)
                preferred = endpoint
                return value
            } catch (_: Exception) { checked.remove(endpoint) }
        }
        throw NetworkReadUnavailable("Solana")
    }
    private fun objectCall(method: String, params: JSONArray = JSONArray()): JSONObject =
        call(method, params) as? JSONObject ?: throw WalletFailure("Solana returned a malformed response.")
    private fun commitment(minSlot: Long = 0, encoding: Boolean = false, finalized: Boolean = false): JSONObject =
        JSONObject().put("commitment", if (finalized) "finalized" else "confirmed").apply {
            if (minSlot > 0) put("minContextSlot", minSlot)
            if (encoding) put("encoding", "jsonParsed")
        }
    private fun context(result: JSONObject, minimum: Long = 0): Long {
        val slot = exactLong(result.getJSONObject("context").get("slot"))
        if (slot <= 0 || slot < minimum) throw WalletFailure("Solana returned data from an older slot.")
        return slot
    }
    private fun nativeAccount(account: JSONObject?, minimumBalance: BigInteger = ZERO): BigInteger {
        if (account == null) {
            if (minimumBalance > ZERO) throw WalletFailure("The Solana sender account is unavailable.")
            return ZERO
        }
        if (account.getString("owner") != SYSTEM_PROGRAM || account.getBoolean("executable")) {
            throw WalletFailure("Use an ordinary Solana wallet address, not a program or token account.")
        }
        val data = account.optJSONArray("data")
            ?: throw WalletFailure("The Solana wallet account has unsupported data.")
        if (data.length() != 2 || data.getString(0).isNotEmpty() || data.getString(1) != "base64") {
            throw WalletFailure("Solana nonce and program accounts are not supported as wallet recipients.")
        }
        val amount = exactInteger(account.get("lamports"))
        if (amount < minimumBalance) throw WalletFailure("The SOL balance changed. Review a new transfer.")
        return amount
    }
    private fun parsedInfo(account: JSONObject, type: String): JSONObject {
        if (account.getString("owner") != TOKEN_PROGRAM || account.getBoolean("executable")) {
            throw WalletFailure("Solana returned an unsupported token program.")
        }
        val data = account.getJSONObject("data")
        if (data.getString("program") != "spl-token") throw WalletFailure("Solana token parser mismatch.")
        val parsed = data.getJSONObject("parsed")
        if (parsed.getString("type") != type) throw WalletFailure("Solana account type mismatch.")
        return parsed.getJSONObject("info")
    }
    private fun validateMint(account: JSONObject, symbol: String) {
        val info = parsedInfo(account, "mint")
        if (!info.getBoolean("isInitialized") || exactInteger(info.get("decimals")) != BigInteger.valueOf(6)) {
            throw WalletFailure("$symbol mint metadata does not match the fixed Solana registry.")
        }
    }
    private fun tokenAccount(publicKey: String, account: JSONObject, symbol: String, owner: String): SolanaTokenAccount {
        val info = parsedInfo(account, "account")
        if (info.getString("mint") != MINTS[symbol] || info.getString("owner") != owner) {
            throw WalletFailure("Solana token account owner or mint does not match.")
        }
        val state = info.getString("state")
        if (state != "initialized" && state != "frozen") throw WalletFailure("Solana token account is not initialized.")
        val amount = info.getJSONObject("tokenAmount")
        if (exactInteger(amount.get("decimals")) != BigInteger.valueOf(6)) throw WalletFailure("Solana token decimals do not match.")
        return SolanaTokenAccount(publicKey, exactInteger(amount.get("amount")), state == "frozen")
    }

    private data class Snapshot(val slot: Long, val native: BigInteger, val tokens: Map<String, List<SolanaTokenAccount>>) {
        fun values(): Map<String, BigInteger> = linkedMapOf("SOL" to native).apply {
            for ((symbol, accounts) in tokens) {
                val total = accounts.fold(ZERO) { sum, account -> sum + account.amount }
                if (total > MAX_U64) throw WalletFailure("Solana token holdings exceed the mint's integer range.")
                put(symbol, total)
            }
        }
    }

    /** Discover accounts, then read sender, fixed mints and every discovered
     * token account in ONE bank context. Missing accounts are not turned into 0.
     * New accounts appearing after discovery are picked up on the next refresh.
     */
    private fun snapshot(minSlot: Long = 0): Snapshot {
        var minimum = minSlot
        val discovered = linkedMapOf<String, List<String>>()
        for ((symbol, mint) in MINTS) {
            val result = objectCall("getTokenAccountsByOwner", JSONArray().put(address).put(JSONObject().put("mint", mint)).put(commitment(minimum, encoding = true)))
            minimum = maxOf(minimum, context(result, minimum))
            val entries = result.getJSONArray("value")
            if (entries.length() > 64) throw WalletFailure("Solana returned too many token accounts for a single safe snapshot.")
            val accounts = mutableListOf<String>()
            for (entry in entries.objects()) {
                val key = entry.getString("pubkey")
                tokenAccount(key, entry.getJSONObject("account"), symbol, address)
                if (key in accounts) throw WalletFailure("Solana returned a duplicate token account.")
                accounts += key
            }
            discovered[symbol] = accounts
        }
        val addresses = listOf(address) + MINTS.values + discovered.values.flatten()
        if (addresses.size > 100 || addresses.distinct().size != addresses.size) throw WalletFailure("Solana token account inventory exceeds the supported snapshot limit.")
        val result = objectCall("getMultipleAccounts", JSONArray().put(JSONArray(addresses)).put(commitment(minimum, encoding = true)))
        val slot = context(result, minimum)
        val values = result.getJSONArray("value")
        if (values.length() != addresses.size) throw WalletFailure("Solana account snapshot is incomplete.")
        val native = nativeAccount(values.optJSONObject(0))
        for ((index, symbol) in MINTS.keys.withIndex()) validateMint(values.getJSONObject(index + 1), symbol)
        var index = 1 + MINTS.size
        val tokens = linkedMapOf<String, List<SolanaTokenAccount>>()
        for ((symbol, keys) in discovered) {
            tokens[symbol] = keys.map { key ->
                val value = values.optJSONObject(index++) ?: throw WalletFailure("A Solana token account changed during the read. Refresh again.")
                tokenAccount(key, value, symbol, address)
            }
        }
        return Snapshot(slot, native, tokens)
    }

    override fun balance(): FamilyBalance {
        val snapshot = snapshot()
        val block = objectCall("getBlock", JSONArray().put(snapshot.slot).put(JSONObject().put("commitment", "confirmed")
            .put("transactionDetails", "none").put("rewards", false).put("maxSupportedTransactionVersion", 0)))
        return FamilyBalance(snapshot.slot, block.getString("blockhash"), snapshot.values(), System.currentTimeMillis())
    }

    private fun recipientInfo(recipient: String, symbol: String, minSlot: Long): Pair<Boolean, BigInteger> {
        val result = objectCall("getAccountInfo", JSONArray().put(recipient).put(commitment(minSlot, encoding = true)))
        context(result, minSlot)
        nativeAccount(result.optJSONObject("value"))
        if (symbol == "SOL") return false to ZERO
        val ata = solanaTokenAccount(recipient, symbol)
        val token = objectCall("getAccountInfo", JSONArray().put(ata).put(commitment(minSlot, encoding = true)))
        context(token, minSlot)
        val account = token.optJSONObject("value")
        if (account != null) {
            if (tokenAccount(ata, account, symbol, recipient).frozen) throw WalletFailure("The recipient's token account is frozen.")
            return false to ZERO
        }
        val rent = exactInteger(call("getMinimumBalanceForRentExemption", JSONArray().put(165).put(JSONObject().put("commitment", "confirmed"))))
        if (rent <= ZERO || rent > BigInteger("10000000")) throw WalletFailure("Solana associated-account rent exceeds the configured safety limit.")
        return true to rent
    }

    private fun latestBlockhash(minSlot: Long): JSONObject {
        val result = objectCall("getLatestBlockhash", JSONArray().put(commitment(minSlot)))
        context(result, minSlot)
        return JSONObject(result.getJSONObject("value").toString()).put("context_slot", context(result, minSlot))
    }

    private fun priorityPrice(): BigInteger {
        val rows = call("getRecentPrioritizationFees", JSONArray()) as? JSONArray
            ?: throw WalletFailure("Solana priority fee observation is unavailable.")
        if (rows.length() == 0 || rows.length() > 200) throw WalletFailure("Solana priority fee observation is incomplete.")
        val values = rows.objects().sortedByDescending { exactInteger(it.get("slot")) }.take(20)
            .map { exactInteger(it.get("prioritizationFee")) }.sorted()
        return values[(values.size * 3 / 4).coerceAtMost(values.lastIndex)]
    }

    private fun fee(message: String, minSlot: Long): BigInteger {
        val response = objectCall("getFeeForMessage", JSONArray().put(message).put(commitment(minSlot)))
        context(response, minSlot)
        if (response.isNull("value")) throw WalletFailure("The Solana blockhash expired. Review a fresh transfer.")
        val fee = exactInteger(response.get("value"))
        if (fee <= ZERO || fee > FEE_CEILING) throw WalletFailure("Solana network fee exceeds the configured safety limit.")
        return fee
    }

    private fun makeRequest(symbol: String, recipient: String, amount: BigInteger, snapshot: Snapshot,
                            hash: JSONObject, create: Boolean, rent: BigInteger, price: BigInteger,
                            computeLimit: Int, fee: BigInteger, intent: String): JSONObject {
        val sources = if (symbol == "SOL") emptyList() else allocate(amount, snapshot.tokens[symbol].orEmpty())
        return JSONObject().put("intent_id", intent).put("network", NETWORK).put("genesis_hash", GENESIS_HASH)
            .put("from", address).put("to", recipient).put("asset", symbol)
            .put("amount", Amounts.display(amount, if (symbol == "SOL") 9 else 6))
            .put("recent_blockhash", hash.getString("blockhash"))
            .put("last_valid_block_height", exactInteger(hash.get("lastValidBlockHeight")))
            .put("context_slot", hash.getLong("context_slot")).put("fee_lamports", fee.toString())
            .put("rent_lamports", rent.toString()).put("create_destination_ata", create)
            .put("source_accounts", JSONArray(sources.map { (key, units) -> JSONObject().put("address", key).put("amount_units", units.toString()) }))
            .put("compute_unit_limit", computeLimit).put("compute_unit_price_micro_lamports", price.toString())
    }

    private fun simulate(prepared: JSONObject): Long {
        val request = prepared.getJSONObject("request")
        val message = Base64.getDecoder().decode(prepared.getString("unsigned_transaction"))
        val wire = ByteArray(65 + message.size)
        wire[0] = 1 // one zero signature; signature verification is explicitly off for simulation.
        message.copyInto(wire, 65)
        val params = commitment(request.getLong("context_slot")).put("encoding", "base64")
            .put("sigVerify", false).put("replaceRecentBlockhash", false)
        val result = objectCall("simulateTransaction", JSONArray().put(Base64.getEncoder().encodeToString(wire)).put(params))
        context(result, request.getLong("context_slot"))
        val value = result.getJSONObject("value")
        if (!value.isNull("err")) throw WalletFailure("Solana rejected this transfer simulation. Nothing was signed.")
        val consumed = exactLong(value.get("unitsConsumed"))
        if (consumed <= 0 || consumed > request.getLong("compute_unit_limit")) throw WalletFailure("Solana simulation returned an invalid compute cost.")
        return consumed
    }

    override fun prepare(symbol: String, recipient: String, amount: String, maximum: Boolean): FamilyPrepared {
        if (symbol != "SOL" && symbol !in MINTS) throw WalletFailure("Unsupported Solana asset.")
        solanaValidateAddress(recipient)
        if (recipient == address) throw WalletFailure("Choose a recipient different from this wallet.")
        val snapshot = snapshot()
        val available = snapshot.values()[symbol] ?: throw WalletFailure("Asset balance is unavailable.")
        val (create, rent) = recipientInfo(recipient, symbol, snapshot.slot)
        val blockhash = latestBlockhash(snapshot.slot)
        val slot = blockhash.getLong("context_slot")
        val price = priorityPrice()
        val intent = UUID.randomUUID().toString()
        var limit = if (symbol == "SOL") 10_000 else 200_000
        var transfer = if (maximum) { if (symbol == "SOL") ONE else available } else Amounts.parse(amount, if (symbol == "SOL") 9 else 6)
        if (transfer <= ZERO || transfer > available) throw WalletFailure("The selected asset has insufficient funds.")
        // The provisional ceiling is only used to construct bytes for the fee
        // oracle. It is never shown, signed or used as a fabricated network fee.
        var request = makeRequest(symbol, recipient, transfer, snapshot, blockhash, create, rent, price, limit, FEE_CEILING, intent)
        var prepared = JSONObject(solanaPrepare(request.toString()))
        var actualFee = fee(prepared.getString("unsigned_transaction"), slot)
        if (maximum && symbol == "SOL") transfer = maximumNative(snapshot.native, actualFee)
        request = makeRequest(symbol, recipient, transfer, snapshot, blockhash, create, rent, price, limit, actualFee, intent)
        prepared = JSONObject(solanaPrepare(request.toString()))
        val preliminaryNeeded = actualFee + rent + if (symbol == "SOL") transfer else ZERO
        var simulated = false
        if (snapshot.native >= preliminaryNeeded) {
            val consumed = simulate(prepared)
            limit = ((consumed * 120 + 99) / 100 + 1_000).coerceIn(5_000, 1_400_000).toInt()
            // A different compute limit changes the priority fee. Re-quote and
            // recalculate native Max, then simulate those final exact bytes.
            request = makeRequest(symbol, recipient, transfer, snapshot, blockhash, create, rent, price, limit, FEE_CEILING, intent)
            val revised = JSONObject(solanaPrepare(request.toString()))
            actualFee = fee(revised.getString("unsigned_transaction"), slot)
            if (maximum && symbol == "SOL") transfer = maximumNative(snapshot.native, actualFee)
            request = makeRequest(symbol, recipient, transfer, snapshot, blockhash, create, rent, price, limit, actualFee, intent)
            prepared = JSONObject(solanaPrepare(request.toString()))
            val needed = actualFee + rent + if (symbol == "SOL") transfer else ZERO
            if (snapshot.native >= needed) { simulate(prepared); simulated = true }
        }
        val reserve = actualFee + rent
        val shortfall = (reserve + (if (symbol == "SOL") transfer else ZERO) - snapshot.native).max(ZERO)
        val note = when {
            shortfall > ZERO -> "Add ${Amounts.display(shortfall, 9)} SOL on Solana for the network fee${if (create) " and the recipient's token account" else ""}."
            create -> "Includes ${Amounts.display(actualFee, 9)} SOL network fee and up to ${Amounts.display(rent, 9)} SOL to create the recipient's token account."
            simulated -> "Live Solana fee. The exact transfer was simulated successfully."
            else -> "Live Solana fee. A successful fresh simulation is required before signing."
        }
        return FamilyPrepared(prepared.toString(), actualFee, reserve, snapshot.native, available, note,
            Math.multiplyExact(prepared.getLong("expires_at"), 1_000L))
    }

    private fun liveBlockhash(request: JSONObject, finalized: Boolean = false): Boolean {
        val result = objectCall("isBlockhashValid", JSONArray().put(request.getString("recent_blockhash"))
            .put(commitment(request.getLong("context_slot"), finalized = finalized)))
        context(result, request.getLong("context_slot"))
        return result.getBoolean("value")
    }

    override fun revalidate(preparedJson: String) {
        val prepared = JSONObject(solanaValidatePrepared(preparedJson))
        val request = prepared.getJSONObject("request")
        if (request.getString("from") != address || request.getString("network") != NETWORK) throw WalletFailure("Solana transfer belongs to a different wallet.")
        if (System.currentTimeMillis() >= Math.multiplyExact(prepared.getLong("expires_at"), 1_000L)) throw WalletFailure("Solana review expired. Review a new transfer.")
        if (!liveBlockhash(request)) throw WalletFailure("Solana blockhash expired. Review a new transfer.")
        val height = exactInteger(call("getBlockHeight", JSONArray().put(commitment(request.getLong("context_slot")))))
        if (height > exactInteger(request.get("last_valid_block_height"))) throw WalletFailure("Solana transfer lifetime expired.")
        val current = snapshot(request.getLong("context_slot"))
        val symbol = request.getString("asset")
        val amount = exactInteger(prepared.get("amount_units"))
        val reserve = exactInteger(prepared.get("fee_reserve_units"))
        if (current.native < reserve + if (symbol == "SOL") amount else ZERO) throw WalletFailure("Add SOL to cover this transfer and its fee.")
        val (needsCreation, rent) = recipientInfo(request.getString("to"), symbol, current.slot)
        if (needsCreation && !request.getBoolean("create_destination_ata")) throw WalletFailure("The recipient token account changed. Review a new transfer.")
        if (rent > exactInteger(request.get("rent_lamports"))) throw WalletFailure("The recipient's account cost changed. Review a new transfer.")
        if (symbol != "SOL") {
            val sources = current.tokens[symbol].orEmpty().associateBy { it.address }
            for (allocation in request.getJSONArray("source_accounts").objects()) {
                val account = sources[allocation.getString("address")] ?: throw WalletFailure("A source token account changed. Review a new transfer.")
                if (account.frozen || account.amount < exactInteger(allocation.get("amount_units"))) throw WalletFailure("A source token account has insufficient spendable units.")
            }
        }
        if (fee(prepared.getString("unsigned_transaction"), current.slot) != exactInteger(request.get("fee_lamports"))) throw WalletFailure("Solana fee quote changed. Review a new transfer.")
        simulate(prepared)
    }

    override fun broadcast(signedJson: String): String {
        val signed = JSONObject(signedJson)
        if (signed.getString("network") != NETWORK || signed.getString("from") != address) throw WalletFailure("Signed Solana transfer belongs to another wallet.")
        val raw = signed.getString("raw_transaction")
        val bytes = Base64.getDecoder().decode(raw)
        if (bytes.size !in 66..1232 || Base64.getEncoder().encodeToString(bytes) != raw) throw WalletFailure("Malformed signed Solana transaction.")
        val endpoint = preferred
        checkEndpoint(endpoint, force = true)
        // Never fall back or retry after this call. Any exception is an uncertain
        // submission; the committed signature is looked up before another action.
        val result = transport.request(endpoint, "sendTransaction", JSONArray().put(raw).put(JSONObject()
            .put("encoding", "base64").put("skipPreflight", false).put("preflightCommitment", "confirmed").put("maxRetries", 0)))
        val identifier = result as? String ?: throw WalletFailure("Solana submission returned an unknown result. Check the saved transaction.")
        if (identifier != signed.getString("hash")) throw WalletFailure("Solana returned a different transaction signature. Check the saved transaction.")
        return identifier
    }

    private fun signatureStatus(identifier: String): JSONObject? {
        if (!Regex("[1-9A-HJ-NP-Za-km-z]{64,88}").matches(identifier)) throw WalletFailure("Enter a complete Solana transaction signature.")
        val response = objectCall("getSignatureStatuses", JSONArray().put(JSONArray().put(identifier))
            .put(JSONObject().put("searchTransactionHistory", true)))
        val values = response.getJSONArray("value")
        if (values.length() != 1) throw WalletFailure("Solana returned an ambiguous transaction status.")
        return values.optJSONObject(0)
    }
    private fun ledgerEvidence(identifier: String): String? {
        val status = signatureStatus(identifier) ?: return null
        val confirmed = status.optString("confirmationStatus")
        if (confirmed != "confirmed" && confirmed != "finalized") return null
        val result = call("getTransaction", JSONArray().put(identifier).put(JSONObject().put("encoding", "base64")
            .put("commitment", "confirmed").put("maxSupportedTransactionVersion", 0)))
        if (result == JSONObject.NULL) return null
        val transaction = result as? JSONObject ?: throw WalletFailure("Solana receipt is malformed.")
        val slot = exactLong(transaction.get("slot"))
        if (slot != exactLong(status.get("slot"))) throw WalletFailure("Solana transaction status changed. Refresh again.")
        val block = objectCall("getBlock", JSONArray().put(slot).put(JSONObject().put("commitment", "confirmed")
            .put("transactionDetails", "signatures").put("rewards", false).put("maxSupportedTransactionVersion", 0)))
        val finalizedSlot = exactInteger(call("getSlot", JSONArray().put(commitment(finalized = true))))
        return JSONObject().put("network", NETWORK).put("genesis_hash", GENESIS_HASH).put("signature_status", status)
            .put("requested_signature", identifier).put("canonical_block_slot", slot)
            .put("transaction", transaction).put("canonical_block", block).put("finalized_slot", finalizedSlot).toString()
    }
    override fun receiptEvidence(preparedJson: String, signedJson: String): String? = ledgerEvidence(JSONObject(signedJson).getString("hash"))
    override fun activityEvidence(transactionId: String): String? = ledgerEvidence(transactionId)

    override fun requireRebroadcastable(preparedJson: String, signedJson: String) {
        val prepared = JSONObject(solanaValidatePrepared(preparedJson))
        val request = prepared.getJSONObject("request")
        val identifier = JSONObject(signedJson).getString("hash")
        if (signatureStatus(identifier) != null) throw WalletFailure("Solana has already observed this transaction. Refresh its status.")
        // A newly prepared confirmed blockhash is normally ahead of the finalized
        // bank. Requiring finalized minContextSlot here would block every first
        // submission. Confirmed validity authorizes only these identical bytes;
        // finalized observations are reserved for the expired/unresolved branch.
        if (liveBlockhash(request)) {
            val liveHeight = exactInteger(call("getBlockHeight", JSONArray().put(commitment(request.getLong("context_slot")))))
            if (liveHeight <= exactInteger(request.get("last_valid_block_height"))) return
        }
        val valid = objectCall("isBlockhashValid", JSONArray().put(request.getString("recent_blockhash"))
            .put(commitment(request.getLong("context_slot"), finalized = true)))
        val finalSlot = context(valid, request.getLong("context_slot"))
        val height = exactInteger(call("getBlockHeight", JSONArray().put(commitment(finalSlot, finalized = true))))
        val observation = JSONObject(solanaCheckExpiry(JSONObject().put("network", NETWORK).put("genesis_hash", GENESIS_HASH)
            .put("recent_blockhash", request.getString("recent_blockhash")).put("last_valid_block_height", request.get("last_valid_block_height"))
            .put("context_slot", request.get("context_slot")).put("finalized_context_slot", finalSlot)
            .put("current_finalized_block_height", height).put("history_search_enabled", true)
            .put("signature_found", false).put("blockhash_valid", valid.getBoolean("value")).toString()))
        if (observation.getBoolean("expired")) throw WalletFailure("This Solana blockhash expired. The transaction outcome still needs reconciliation; its saved record remains reserved.")
        if (!observation.getBoolean("rebroadcastable")) throw WalletFailure("Solana transaction status is unresolved. Refresh before retrying.")
    }

    override fun recentTransactionIds(): List<String> {
        val snapshot = snapshot()
        val addresses = (listOf(address) + MINTS.keys.map { solanaTokenAccount(address, it) } +
            snapshot.tokens.values.flatten().map { it.address }).distinct()
        val found = linkedMapOf<String, BigInteger>()
        for (account in addresses) {
            val rows = call("getSignaturesForAddress", JSONArray().put(account).put(JSONObject()
                .put("commitment", "confirmed").put("limit", 8).put("minContextSlot", snapshot.slot))) as? JSONArray
                ?: throw WalletFailure("Solana recent activity is temporarily unavailable.")
            if (rows.length() > 8) throw WalletFailure("Solana activity response exceeds the requested limit.")
            for (row in rows.objects()) {
                val signature = row.getString("signature")
                if (!Regex("[1-9A-HJ-NP-Za-km-z]{64,88}").matches(signature)) throw WalletFailure("Solana returned an invalid activity signature.")
                found[signature] = exactInteger(row.get("slot"))
            }
        }
        return found.entries.sortedByDescending { it.value }.take(20).map { it.key }
    }
}

/** Public parsed balance, including frozen funds for honest holdings display. */
data class SolanaTokenAccount(val address: String, val amount: BigInteger, val frozen: Boolean)
