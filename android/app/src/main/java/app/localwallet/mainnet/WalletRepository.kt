package app.localwallet.mainnet

import android.os.SystemClock
import org.json.JSONArray
import org.json.JSONObject
import uniffi.wallet_core.WalletJournal
import uniffi.wallet_core.assetRegistry
import uniffi.wallet_core.preparePayment
import uniffi.wallet_core.signPayment
import uniffi.wallet_core.validatePreparedJson
import uniffi.wallet_core.validateActivity
import java.math.BigInteger
import java.util.UUID

/**
 * Durable reads/writes, preparation, cancellation, signing and reconciliation run on the
 * Activity's single work executor; nonce allocation never overlaps. Only preview() and
 * maxTransfer() may run on its dedicated public-read executor: both use an isolated RPC
 * client, immutable manifest inputs and no vault, journal or incoming-storage handles.
 */
class WalletRepository(private val vault: SecureVault, val manifest: VaultManifest,
                       private val rpcFactory: (WalletNetwork) -> NetworkClient = { NetworkClient(it) },
                       private val elapsedMillis: () -> Long = { SystemClock.elapsedRealtime() }) : AutoCloseable {
    val networks: List<WalletNetwork> = JSONObject(assetRegistry()).getJSONArray("networks").objects().map { network ->
        WalletNetwork(network.getLong("chain_id"), network.getString("network"), network.getString("rpc_url"),
            network.getString("broadcast_url"), network.getString("explorer_url"),
            network.getJSONArray("assets").objects().map { AssetInfo(it.getString("symbol"), it.getInt("decimals"), it.optionalString("contract")) })
    }.also {
        if (it.map { n -> n.chainId }.toSet() != NetworkPolicy.evmChainOrder.toSet() || it.size != NetworkPolicy.evmChainOrder.size) {
            throw WalletFailure("Unexpected network registry.")
        }
        AssetSelection.assets(it)
    }
    private val journals = HashMap<Long, WalletJournal>()
    private var incomingStorage: ActivityStore? = null
    private fun activityStore(): ActivityStore = incomingStorage ?: ActivityStore(vault.directory, manifest.address).also { incomingStorage = it }

    private fun journal(network: WalletNetwork): WalletJournal = journals.getOrPut(network.chainId) {
        localRecord(LocalRecordStage.JOURNAL_OPEN) {
            WalletJournal(journalFile(vault.directory, network.chainId).absolutePath)
        }
    }
    private fun identity(record: JSONObject, network: WalletNetwork, addressKey: String = "from") {
        if (record.getLong("chain_id") != network.chainId || !record.getString(addressKey).equals(manifest.address, true)) {
            throw WalletFailure("Stored record belongs to a different account or network. Operation blocked.")
        }
    }
    fun history(network: WalletNetwork): List<JSONObject> = localRecord(LocalRecordStage.HISTORY_READ) {
        JSONArray(journal(network).listPayments()).objects().onEach { identity(it, network) }
    }
    fun events(network: WalletNetwork, id: String): String {
        identity(JSONObject(journal(network).getPayment(id)), network)
        return journal(network).listEvents(id)
    }
    fun cancelPrepared(network: WalletNetwork, id: String) {
        identity(JSONObject(journal(network).getPayment(id)), network)
        journal(network).cancelPrepared(id)
    }
    fun cachedBalance(network: WalletNetwork): BalanceView? = localRecord(LocalRecordStage.BALANCE_READ) {
        val text = journal(network).latestBalance()
        if (text == "null") return@localRecord null
        val snapshot = JSONObject(text)
        identity(snapshot, network, "address")
        balanceView(network, snapshot, 0)
    }
    private fun balanceView(network: WalletNetwork, snapshot: JSONObject, timestamp: Long): BalanceView {
        val values = snapshot.getJSONObject("balances")
        val symbols = network.assets.map { it.symbol }
        val stored = values.keys().asSequence().toSet()
        // Pre-expansion Ethereum records do not have USDT. Keep that asset unknown,
        // preserve the old ETH/USDC/EURC observation, and never synthesize a zero.
        val legacyEthereum = network.chainId == 1L && stored == symbols.toSet() - "USDT"
        if (stored != symbols.toSet() && !legacyEthereum) throw WalletFailure("Stored balance assets do not match the network.")
        return BalanceView(snapshot.getLong("block_number"), snapshot.getString("block_hash"),
            symbols.filter { it in stored }.associateWith { BigInteger(values.getString(it)) }, timestamp)
    }

    private fun latest(rpc: NetworkClient): JSONObject {
        val block = rpc.block("latest")
        val timestamp = Amounts.longQuantity(block.getString("timestamp"))
        val now = System.currentTimeMillis() / 1000
        if (timestamp > now + 120 || timestamp < now - 300) throw WalletFailure("Latest network block is stale or the device clock is incorrect.")
        Amounts.hash(block.getString("hash"))
        Amounts.longQuantity(block.getString("number"))
        return block
    }
    private fun assertCanonical(rpc: NetworkClient, block: JSONObject) {
        val current = rpc.block(block.getString("number"))
        if (!Amounts.hash(current.getString("hash")).equals(Amounts.hash(block.getString("hash")))) {
            throw WalletFailure("Network reorganized during this request. Refresh before continuing.")
        }
    }
    private fun balances(network: WalletNetwork, rpc: NetworkClient, block: JSONObject): Map<String, BigInteger> {
        val at = rpc.reference(block)
        val values = HashMap<String, BigInteger>()
        values[network.nativeSymbol] = Amounts.quantity(rpc.string("eth_getBalance", JSONArray().put(manifest.address).put(at)))
        for (asset in network.assets.filter { it.contract != null }) {
            val code = rpc.string("eth_getCode", JSONArray().put(asset.contract).put(at))
            if (!Regex("0x(?:[0-9a-fA-F]{2})+").matches(code)) throw WalletFailure("The selected token contract is missing.")
            values[asset.symbol] = Amounts.abiInteger(rpc.contractCall(asset.contract!!, Amounts.balanceData(manifest.address), at))
        }
        return values
    }
    fun refreshBalance(network: WalletNetwork): BalanceView {
        val rpc = NetworkClient(network)
        rpc.assertChain()
        val block = latest(rpc)
        val values = balances(network, rpc, block)
        assertCanonical(rpc, block)
        val snapshot = JSONObject().put("chain_id", network.chainId).put("address", manifest.address)
            .put("block_number", Amounts.longQuantity(block.getString("number"))).put("block_hash", Amounts.hash(block.getString("hash")))
            .put("balances", JSONObject(values.mapValues { it.value.toString() }))
        return saveBalanceObservation(network, snapshot, System.currentTimeMillis())
    }
    internal fun saveBalanceObservation(network: WalletNetwork, snapshot: JSONObject, observedAt: Long): BalanceView = localRecord(LocalRecordStage.BALANCE_SAVE) {
        identity(snapshot, network, "address")
        journal(network).recordBalance(snapshot.toString())
        val saved = JSONObject(journal(network).latestBalance())
        identity(saved, network, "address")
        if (saved.getString("block_hash") != snapshot.getString("block_hash") || saved.getLong("block_number") != snapshot.getLong("block_number")) {
            throw WalletFailure("Balance observation did not become the latest durable record.")
        }
        balanceView(network, saved, observedAt)
    }

    private fun assertNoOtherActive(network: WalletNetwork, exceptId: String? = null) {
        if (history(network).any { it.getString("intent_id") != exceptId && it.getString("phase") !in setOf("cancelled", "finalized") }) {
            throw WalletFailure("Resolve or cancel the existing payment on this network before creating another.")
        }
    }
    private fun checkedNonce(rpc: NetworkClient, block: JSONObject): Long {
        val code = rpc.string("eth_getCode", JSONArray().put(manifest.address).put(rpc.reference(block)))
        if (code != "0x") throw WalletFailure("Smart contract or delegated accounts are not supported by this wallet.")
        val pendingCode = rpc.string("eth_getCode", JSONArray().put(manifest.address).put("pending"))
        if (pendingCode != "0x") throw WalletFailure("A pending account delegation is present. Sending is blocked.")
        val latestNonce = Amounts.longQuantity(rpc.string("eth_getTransactionCount", JSONArray().put(manifest.address).put(rpc.reference(block))))
        val pendingNonce = Amounts.longQuantity(rpc.string("eth_getTransactionCount", JSONArray().put(manifest.address).put("pending")))
        if (latestNonce != pendingNonce) throw WalletFailure("This address has a pending transaction. Wait for it to resolve before sending.")
        return latestNonce
    }
    private fun transaction(network: WalletNetwork, request: JSONObject): JSONObject {
        val asset = network.asset(request.getString("asset"))
        val amount = Amounts.parse(request.getString("amount"), asset.decimals)
        return JSONObject().put("from", manifest.address).put("to", asset.contract ?: request.getString("to"))
            .put("value", if (asset.contract == null) Amounts.hex(amount) else "0x0")
            .put("data", if (asset.contract == null) "0x" else Amounts.transferData(request.getString("to"), amount))
            .put("nonce", Amounts.hex(BigInteger.valueOf(request.getLong("nonce"))))
            .put("maxFeePerGas", Amounts.hex(BigInteger(request.getString("max_fee_per_gas"))))
            .put("maxPriorityFeePerGas", Amounts.hex(BigInteger(request.getString("max_priority_fee_per_gas"))))
    }
    private fun simulation(network: WalletNetwork, rpc: NetworkClient, transaction: JSONObject, asset: AssetInfo, at: Any): Long {
        val result = rpc.string("eth_call", JSONArray().put(transaction).put(at))
        FeeEstimator.validateTokenResult(network, asset, result)
        val estimate = Amounts.longQuantity(rpc.string("eth_estimateGas", JSONArray().put(transaction).put("pending")))
        if (estimate !in 21_000L..300_000L) throw WalletFailure("Estimated gas is outside this wallet's transfer policy.")
        return estimate
    }
    private fun assertFunds(request: JSONObject, feeReserve: BigInteger, values: Map<String, BigInteger>, network: WalletNetwork) {
        val asset = network.asset(request.getString("asset"))
        val units = Amounts.parse(request.getString("amount"), asset.decimals)
        if (values.getValue(asset.symbol) < units) throw WalletFailure("Insufficient ${asset.symbol} on ${network.name}.")
        val requiredEth = feeReserve.add(if (asset.contract == null) units else BigInteger.ZERO)
        if (values.getValue(network.nativeSymbol) < requiredEth) throw WalletFailure("Insufficient ${network.nativeSymbol} on ${network.name} for the reviewed fee reserve.")
    }
    /** Read-only quote: no intent reservation, signature, or broadcast, even when ETH is missing. */
    fun preview(network: WalletNetwork, symbol: String, recipient: String, amount: String): FeePreview {
        val publicRecipient = PrivateAddressRules.requirePublicEvmRecipient(recipient)
        val rpc = rpcFactory(network)
        rpc.assertChain()
        val block = latest(rpc)
        val values = balances(network, rpc, block)
        val quote = FeeEstimator.estimate(network, manifest.address, symbol, publicRecipient, amount, block, values, rpc)
        assertCanonical(rpc, block)
        return quote
    }
    /** Max never opens a journal, reserves a nonce, accesses keys or submits a transaction. */
    fun maxTransfer(network: WalletNetwork, symbol: String, recipient: String): MaxTransferQuote {
        val started = elapsedMillis()
        val publicRecipient = PrivateAddressRules.requirePublicEvmRecipient(recipient)
        val rpc = rpcFactory(network)
        rpc.assertChain()
        val block = latest(rpc)
        MaxTransferCalculator.requireFreshBlock(block, System.currentTimeMillis())
        val calculation = MaxTransferCalculator.calculate(network, manifest.address, symbol, publicRecipient,
            block, balances(network, rpc, block), rpc)
        assertCanonical(rpc, block)
        MaxTransferCalculator.requireFreshBlock(block, System.currentTimeMillis())
        return MaxTransferQuote(manifest.address, network, symbol, publicRecipient, calculation.amount, calculation.units,
            calculation.balanceUnits, calculation.fee, Amounts.longQuantity(block.getString("number")),
            Amounts.hash(block.getString("hash")), started).also {
                it.requireCurrent(manifest.address, network, symbol, publicRecipient, it.amount, elapsedMillis())
            }
    }
    fun prepare(network: WalletNetwork, symbol: String, recipient: String, amount: String,
                maxQuote: MaxTransferQuote? = null): ReviewedPayment {
        val started = elapsedMillis()
        val publicRecipient = PrivateAddressRules.requirePublicEvmRecipient(recipient)
        maxQuote?.requireCurrent(manifest.address, network, symbol, publicRecipient, amount, started)
        val asset = network.asset(symbol)
        Amounts.parse(amount, asset.decimals)
        assertNoOtherActive(network)
        val rpc = rpcFactory(network)
        rpc.assertChain()
        val block = latest(rpc)
        val nonce = checkedNonce(rpc, block)
        val priority = FeeEstimator.priorityFee(network, rpc)
        val base = FeeEstimator.baseFee(network, block)
        val maximum = base.multiply(BigInteger.valueOf(2)).add(priority)
        val refreshedMax = maxQuote?.let {
            MaxTransferCalculator.requireFreshBlock(block, System.currentTimeMillis())
            MaxTransferCalculator.calculate(network, manifest.address, symbol, publicRecipient,
                block, balances(network, rpc, block), rpc, priorityFeePerGas = priority, nonce = nonce)
        }
        val request = JSONObject().put("intent_id", UUID.randomUUID().toString()).put("chain_id", network.chainId)
            .put("from", manifest.address).put("to", publicRecipient).put("asset", symbol).put("amount", refreshedMax?.amount ?: amount)
            .put("nonce", nonce).put("max_fee_per_gas", maximum.toString()).put("max_priority_fee_per_gas", priority.toString())
        val gas = refreshedMax?.fee?.gasLimit ?: run {
            val estimate = simulation(network, rpc, transaction(network, request), asset, rpc.reference(block))
            Math.addExact(Math.multiplyExact(estimate, 120), 99) / 100
        }
        if (gas > 300_000) throw WalletFailure("Gas estimate plus safety margin exceeds this wallet's policy.")
        request.put("gas_limit", gas)
            .put("l1_fee_estimate_wei", (refreshedMax?.fee?.l1Estimate ?: rpc.l1Estimate(rpc.reference(block))).toString())
            .put("operator_fee_estimate_wei", (refreshedMax?.fee?.operatorEstimate ?: rpc.operatorEstimate(gas, rpc.reference(block))).toString())
        val prepared = preparePayment(request.toString())
        val review = JSONObject(prepared)
        val feeReserve = BigInteger(review.getString("fee_reserve_wei"))
        val verifiedBalances = balances(network, rpc, block)
        assertFunds(review.getJSONObject("request"), feeReserve, verifiedBalances, network)
        assertCanonical(rpc, block)
        if (maxQuote != null && refreshedMax != null) {
            maxQuote.requireCurrent(manifest.address, network, symbol, publicRecipient, amount, elapsedMillis())
            MaxTransferCalculator.requireFreshBlock(block, System.currentTimeMillis())
            if (elapsedMillis() - started !in 0 until 30_000L ||
                verifiedBalances[symbol] != refreshedMax.balanceUnits || verifiedBalances[network.nativeSymbol] != refreshedMax.fee.nativeAvailable ||
                feeReserve != refreshedMax.fee.reserve) {
                throw WalletFailure("Max could not be verified against the current balance and fee reserve. Refresh and try again.")
            }
        }
        // Durably reserve this exact intent and nonce; no signing or network submission yet.
        journal(network).recordPrepared(prepared)
        return ReviewedPayment(network, prepared, elapsedMillis())
    }

    fun revalidate(review: ReviewedPayment): ReviewedPayment {
        val prepared = JSONObject(validatePreparedJson(review.json))
        val request = prepared.getJSONObject("request")
        identity(request, review.network)
        val now = System.currentTimeMillis() / 1000
        if (now >= prepared.getLong("expires_at")) throw WalletFailure("Review expired. Cancel it and prepare a fresh payment.")
        assertNoOtherActive(review.network, review.id)
        val record = JSONObject(journal(review.network).getPayment(review.id))
        if (record.getString("phase") != "prepared") throw WalletFailure("This payment has already been signed or cancelled.")
        val rpc = NetworkClient(review.network)
        rpc.assertChain()
        val block = latest(rpc)
        if (checkedNonce(rpc, block) != request.getLong("nonce")) throw WalletFailure("Account nonce changed. Cancel this review and prepare again.")
        val maximum = BigInteger(request.getString("max_fee_per_gas"))
        val base = FeeEstimator.baseFee(review.network, block)
        if (base.add(BigInteger(request.getString("max_priority_fee_per_gas"))) > maximum) {
            throw WalletFailure("Network fees have increased beyond this review. Cancel and prepare again.")
        }
        if (review.network.feeProfile == NetworkFeeProfile.BNB && FeeEstimator.priorityFee(review.network, rpc) > maximum) {
            throw WalletFailure("BNB gas price increased beyond this review. Cancel and prepare again.")
        }
        val tx = transaction(review.network, request).put("gas", Amounts.hex(BigInteger.valueOf(request.getLong("gas_limit"))))
        val estimate = simulation(review.network, rpc, tx, review.network.asset(request.getString("asset")), rpc.reference(block))
        if (estimate > request.getLong("gas_limit")) throw WalletFailure("Gas requirement changed. Cancel and prepare again.")
        val currentAdditional = rpc.l1Estimate(rpc.reference(block)).add(rpc.operatorEstimate(request.getLong("gas_limit"), rpc.reference(block)))
        val reservedAdditional = BigInteger(prepared.getString("fee_reserve_wei")).subtract(BigInteger(prepared.getString("max_fee_wei")))
        if (currentAdditional > reservedAdditional) throw WalletFailure("${review.network.name} fees exceed the reviewed reserve. Cancel and prepare again.")
        assertFunds(request, BigInteger(prepared.getString("fee_reserve_wei")), balances(review.network, rpc, block), review.network)
        assertCanonical(rpc, block)
        return review.copy(preflightAt = SystemClock.elapsedRealtime())
    }

    fun signAndBroadcast(review: ReviewedPayment, key: ByteArray, stillAuthorized: () -> Boolean): JSONObject {
        if (!stillAuthorized()) throw WalletFailure("Authentication was interrupted before signing.")
        if (SystemClock.elapsedRealtime() - review.preflightAt !in 0..30_000) {
            throw WalletFailure("Network checks are no longer fresh. Review and authenticate again.")
        }
        val prepared = JSONObject(validatePreparedJson(review.json))
        identity(prepared.getJSONObject("request"), review.network)
        if (!stillAuthorized()) throw WalletFailure("Authentication was interrupted before signing.")
        val signed = try {
            signPayment(manifest.vaultHex, key, review.json, prepared.getString("approval_digest"))
        } finally {
            // No decrypted vault key is retained while storage or network submission runs.
            key.fill(0)
        }
        val maySubmit = stillAuthorized()
        // The only submission path is from the committed journal return value.
        // If locking races with signing, preserve the exact signature, but do not submit it.
        val durable = JSONObject(journal(review.network).recordSigned(signed))
        if (!maySubmit || !stillAuthorized()) return durable
        return broadcastDurable(review.network, durable, stillAuthorized)
    }

    fun rebroadcast(network: WalletNetwork, id: String, stillAuthorized: () -> Boolean): JSONObject {
        if (!stillAuthorized()) throw WalletFailure("Rebroadcast was interrupted.")
        reconcile(network)
        val durable = JSONObject(journal(network).getPayment(id))
        identity(durable, network)
        if (durable.getString("phase") !in setOf("signed", "unknown", "pending")) {
            throw WalletFailure("Only an unresolved signed transaction can be rebroadcast.")
        }
        // Never create a new transaction, nonce, or signature for a retry.
        return broadcastDurable(network, durable, stillAuthorized)
    }
    private fun broadcastDurable(network: WalletNetwork, record: JSONObject, stillAuthorized: () -> Boolean): JSONObject {
        identity(record, network)
        val raw = record.optionalString("raw_transaction") ?: throw WalletFailure("No durable signed transaction exists.")
        val expected = record.optionalString("hash") ?: throw WalletFailure("No durable transaction hash exists.")
        val rpc = NetworkClient(network)
        var outcome = "unknown"
        var submissionStarted = false
        try {
            if (!stillAuthorized()) return record
            rpc.assertChain()
            if (!stillAuthorized()) return record
            submissionStarted = true
            val reply = rpc.call("eth_sendRawTransaction", JSONArray().put(raw), broadcast = true) as? String
            if (reply != null && Amounts.hash(reply).equals(expected, true)) outcome = "pending"
        } catch (_: Exception) {
            // A timeout, transport rejection, or malformed response does not prove failure.
            if (!submissionStarted) return record
        }
        return JSONObject(journal(network).markSubmission(record.getString("intent_id"), outcome))
    }

    fun reconcile(network: WalletNetwork): List<JSONObject> {
        val rpc = NetworkClient(network)
        rpc.assertChain()
        val payments = history(network).filter { it.optionalString("hash") != null }
        for (payment in payments) {
            if (payment.getString("phase") == "unresolved" && payment.getString("inclusion") == "unseen") {
                throw WalletFailure("A prior finality claim was contradicted. Automatic reconciliation is paused for investigation.")
            }
            val hash = payment.getString("hash")
            val priorHash = payment.optionalString("block_hash")
            var priorStillCanonical = false
            if (priorHash != null) {
                val priorCanonical = rpc.block(Amounts.hex(BigInteger.valueOf(payment.getLong("block_number"))))
                priorStillCanonical = priorCanonical.getString("hash").equals(priorHash, true)
                if (!priorStillCanonical) {
                    val correction = JSONObject(journal(network).markReorg(payment.getString("intent_id"), priorHash))
                    if (correction.getString("phase") == "unresolved") {
                        throw WalletFailure("A prior finalized inclusion was contradicted. Automatic reconciliation is paused for investigation.")
                    }
                }
            }
            val result = rpc.call("eth_getTransactionReceipt", JSONArray().put(hash))
            if (result == JSONObject.NULL) {
                continue
            }
            val receipt = result as? JSONObject ?: throw WalletFailure("Invalid receipt response.")
            val height = Amounts.longQuantity(receipt.getString("blockNumber"))
            val canonical = rpc.block(Amounts.hex(BigInteger.valueOf(height)))
            if (!canonical.getString("hash").equals(receipt.getString("blockHash"), true)) {
                throw WalletFailure("Receipt is not in the canonical chain. Payment remains unresolved.")
            }
            if (priorStillCanonical && !receipt.getString("blockHash").equals(priorHash, true)) {
                throw WalletFailure("RPC receipt conflicts with the stored canonical inclusion. Existing evidence is preserved.")
            }
            val normalized = rpc.normalizeReceipt(receipt)
            val finalized = rpc.block("finalized")
            val finalizedHeight = Amounts.longQuantity(finalized.getString("number"))
            // Recheck after the finality query, not before it alone.
            assertCanonical(rpc, canonical)
            journal(network).observeReceipt(payment.getString("intent_id"), normalized.toString(),
                Amounts.hash(canonical.getString("hash")), finalizedHeight.toULong())
        }
        return history(network)
    }

    fun activitySnapshot(network: WalletNetwork): ActivitySnapshot = localRecord(LocalRecordStage.ACTIVITY_READ) {
        activityStore().snapshot(network.chainId)
    }

    /** User-supplied hashes are public lookup hints. Only shared Rust receipt validation creates activity. */
    fun trackTransaction(network: WalletNetwork, suppliedHash: String): ActivitySnapshot {
        val hash = Amounts.hash(suppliedHash.trim())
        activityStore().track(network.chainId, hash)
        val rpc = NetworkClient(network)
        try {
            rpc.assertChain()
            verifyActivityHash(network, rpc, hash)
        } catch (failure: Exception) {
            activityStore().trackingStatus(network.chainId, hash, "unavailable")
            throw failure
        }
        return activityStore().snapshot(network.chainId)
    }

    private fun verifyActivityHash(network: WalletNetwork, rpc: NetworkClient, hash: String): String {
        val store = activityStore()
        val prior = store.latestForHash(network.chainId, hash)
        if (prior.any { it.getString("verification_state") == "conflict" }) {
            store.trackingStatus(network.chainId, hash, "conflict")
            return "conflict"
        }
        var priorCanonicalHash: String? = null
        for ((height, group) in prior.filter { it.getString("verification_state") == "observed" }.groupBy { it.getLong("block_number") }) {
            val canonical = rpc.block(Amounts.hex(BigInteger.valueOf(height)))
            if (!canonical.getString("hash").equals(group.first().getString("block_hash"), true)) {
                if (store.markReorgGroup(network.chainId, group, canonical)) {
                    store.trackingStatus(network.chainId, hash, "conflict")
                    return "conflict"
                }
            } else priorCanonicalHash = canonical.getString("hash")
        }
        val transaction = rpc.call("eth_getTransactionByHash", JSONArray().put(hash))
        if (transaction != JSONObject.NULL) {
            val returned = transaction as? JSONObject ?: throw WalletFailure("Malformed transaction lookup response.")
            if (Amounts.hash(returned.getString("hash")) != hash) throw WalletFailure("Transaction lookup returned a different hash.")
        }
        val receiptResult = rpc.call("eth_getTransactionReceipt", JSONArray().put(hash))
        if (receiptResult == JSONObject.NULL) {
            val state = if (priorCanonicalHash != null) "unavailable" else if (transaction == JSONObject.NULL) "not_found" else "pending"
            store.trackingStatus(network.chainId, hash, state)
            return state
        }
        val tx = transaction as? JSONObject ?: throw WalletFailure("Receipt details are not fully available yet. Try tracking the hash again.")
        val receipt = receiptResult as? JSONObject ?: throw WalletFailure("Malformed transfer receipt.")
        if (Amounts.hash(tx.getString("hash")) != hash || Amounts.hash(receipt.getString("transactionHash")) != hash) throw WalletFailure("The network returned a different transaction.")
        val canonical = rpc.block(receipt.getString("blockNumber"))
        if (priorCanonicalHash != null && !priorCanonicalHash.equals(receipt.getString("blockHash"),true)) throw WalletFailure("Receipt conflicts with a prior canonical inclusion. Existing evidence is preserved.")
        if (!canonical.getString("hash").equals(receipt.getString("blockHash"), true)) throw WalletFailure("Transfer receipt is not currently canonical. Try again after the network settles.")
        val finalized = rpc.block("finalized")
        val finalizedHeight = Amounts.longQuantity(finalized.getString("number"))
        val verified = JSONObject(validateActivity(network.chainId.toULong(), manifest.address, tx.toString(), receipt.toString(), canonical.toString(), finalizedHeight.toULong()))
        assertCanonical(rpc, canonical)
        if (verified.getJSONArray("activities").length() == 0) {
            if (priorCanonicalHash != null) throw WalletFailure("Receipt conflicts with an earlier verified transfer in the same block.")
            val state = if (Amounts.longQuantity(receipt.getString("status")) == 0L) "failed" else "not_recipient"
            store.trackingStatus(network.chainId, hash, state)
            return state
        } else {
            store.observe(network.chainId, verified, JSONObject().put("transaction", tx).put("receipt", receipt).put("canonical_block", canonical)
                .put("finalized_height", finalizedHeight).put("provider", rpc.lastEndpoint ?: "unknown"))
            store.trackingStatus(network.chainId, hash, "verified")
            return "verified"
        }
    }

    fun discoverActivity(network: WalletNetwork, stillInterested: () -> Boolean = { true }): ActivitySnapshot {
        val store = activityStore()
        val rpc = NetworkClient(network)
        rpc.assertChain()
        // Recheck known recent inclusions and explicit tracking requests before looking for new hints.
        val existing = (store.tracked(network.chainId).map { it.getString("hash") } +
            store.latest(network.chainId, 30).map { it.getString("transaction_hash") }).distinct().take(20)
        var allResolved = true
        for (hash in existing) {
            if (!stillInterested()) return store.snapshot(network.chainId)
            if (verifyActivityHash(network, rpc, hash) !in setOf("verified","failed","not_recipient")) allResolved = false
        }
        val head = latest(rpc)
        val end = Amounts.longQuantity(head.getString("number"))
        val priorEnd = store.coverage(network.chainId)?.second?.takeIf { it <= end }
        val start = maxOf(0L, end - 7_999L, (priorEnd?.minus(63L) ?: 0L))
        val contracts = JSONArray(network.assets.mapNotNull { it.contract })
        val recipientTopic = "0x" + manifest.address.substring(2).lowercase().padStart(64, '0')
        val hashes = LinkedHashSet<String>()
        var at = start
        while (at <= end) {
            if (!stillInterested()) return store.snapshot(network.chainId)
            val last = minOf(at + 1_999L, end)
            val filter = JSONObject().put("fromBlock", Amounts.hex(BigInteger.valueOf(at))).put("toBlock", Amounts.hex(BigInteger.valueOf(last)))
                .put("address", contracts).put("topics", JSONArray().put(NetworkClient.TRANSFER_TOPIC).put(JSONObject.NULL).put(recipientTopic))
            val logs = rpc.call("eth_getLogs", JSONArray().put(filter)) as? JSONArray ?: throw WalletFailure("Malformed transfer discovery response.")
            if (logs.length() > 2_000) throw WalletFailure("Recent transfer scan is too large. Track a specific transaction hash instead.")
            for (log in logs.objects()) {
                val topics = log.getJSONArray("topics")
                if (topics.length() != 3 || !topics.getString(0).equals(NetworkClient.TRANSFER_TOPIC, true) || !topics.getString(2).equals(recipientTopic, true)) continue
                if (network.assets.none { it.contract?.equals(log.getString("address"), true) == true }) continue
                val height = Amounts.longQuantity(log.getString("blockNumber"))
                if (height !in at..last || log.optBoolean("removed", false)) continue
                hashes.add(Amounts.hash(log.getString("transactionHash")))
            }
            at = last + 1
        }
        assertCanonical(rpc, head)
        val unknown = hashes.toList().reversed().filter { it !in store.knownHashes(network.chainId) }
        for (hash in unknown.take(20)) {
            if (!stillInterested()) return store.snapshot(network.chainId)
            store.track(network.chainId, hash)
            if (verifyActivityHash(network, rpc, hash) !in setOf("verified","failed","not_recipient")) allResolved = false
        }
        val note = if (unknown.size > 20) "More recent transfers are waiting for verification. Refresh again or track a specific hash." else if (!allResolved) "Some tracked transfers are still waiting for verification. Their status is shown below." else null
        if (unknown.size <= 20 && allResolved) store.scanned(network.chainId, start, end, note)
        return store.snapshot(network.chainId, note)
    }

    fun auditExport(): String {
        val result = JSONObject().put("format", "local-wallet-public-audit-v1").put("address", manifest.address)
            .put("exported_at", System.currentTimeMillis() / 1000)
        val chainRecords = JSONArray()
        for (network in networks) {
            val payments = JSONArray()
            for (record in history(network)) {
                val publicRecord = JSONObject(record.toString()).apply { remove("raw_transaction") }
                payments.put(JSONObject().put("record", publicRecord).put("events", JSONArray(events(network, record.getString("intent_id")))))
            }
            chainRecords.put(JSONObject().put("chain_id", network.chainId).put("network", network.name).put("payments", payments)
                .put("activity_observations", activityStore().audit(network.chainId)))
        }
        fun redact(value: Any): Any = when (value) {
            is JSONObject -> JSONObject().also { clean ->
                value.keys().forEach { key -> if (key != "raw_transaction") clean.put(key, redact(value.get(key))) }
            }
            is JSONArray -> JSONArray().also { clean -> (0 until value.length()).forEach { clean.put(redact(value.get(it))) } }
            else -> value
        }
        return (redact(result.put("networks", chainRecords)) as JSONObject).toString(2)
    }
    override fun close() { journals.values.forEach { it.close() }; journals.clear(); incomingStorage?.close(); incomingStorage = null }
}
