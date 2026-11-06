package app.localwallet.mainnet

import android.os.SystemClock
import org.json.JSONArray
import org.json.JSONObject
import uniffi.wallet_core.FamilyJournal
import uniffi.wallet_core.solanaAddress
import uniffi.wallet_core.tronAddress
import uniffi.wallet_core.solanaSign
import uniffi.wallet_core.tronSign
import java.io.File
import java.math.BigInteger
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.attribute.BasicFileAttributes

data class FamilyReview(val network: WalletNetwork, val prepared: FamilyPrepared, val preflightAt: Long) {
    val id: String get() = prepared.id
}

/**
 * The Android facade dispatches to independently restricted protocol engines.
 * Public reads can use the quote executor; all journal/sign/submit methods use the
 * existing single wallet work executor. No network adapter receives a private key.
 */
class MultiNetworkRepository(private val vault: SecureVault, val manifest: VaultManifest, private val tronApiKey: () -> String? = { null }) : AutoCloseable {
    private val evm = WalletRepository(vault, manifest)
    val networks: List<WalletNetwork> = (evm.networks + FamilyNetworks.all).also {
        if (it.map { n -> n.chainId }.toSet() != NetworkPolicy.chainOrder.toSet()) throw WalletFailure("Invalid network registry.")
        AssetSelection.assets(it)
    }
    @Volatile private var familyAddresses: FamilyAddresses? = null
    private val journals = HashMap<Long, FamilyJournal>()

    fun initializeFamilyAddresses(key: ByteArray) {
        val addresses = FamilyAddresses(solanaAddress(manifest.vaultHex, key), tronAddress(manifest.vaultHex, key))
        familyAddresses?.let { if (it != addresses) throw WalletFailure("The vault's public addresses changed. Operation blocked.") }
        familyAddresses = addresses
    }
    fun retainAuthenticatedAddresses(previous: MultiNetworkRepository) {
        if (manifest.address != previous.manifest.address || manifest.vaultHex != previous.manifest.vaultHex) throw WalletFailure("Wallet identity changed.")
        familyAddresses = previous.familyAddresses
    }
    fun address(network: WalletNetwork): String = when (NetworkPolicy.family(network.chainId)) {
        ChainFamily.EVM -> manifest.address
        ChainFamily.SOLANA -> familyAddresses?.solana ?: throw WalletFailure("Reopen the wallet to verify its Solana address.")
        ChainFamily.TRON -> familyAddresses?.tron ?: throw WalletFailure("Reopen the wallet to verify its TRON address.")
    }
    private fun isEvm(network: WalletNetwork) = NetworkPolicy.family(network.chainId) == ChainFamily.EVM
    private fun adapter(network: WalletNetwork): FamilyNetworkAdapter = when (NetworkPolicy.family(network.chainId)) {
        ChainFamily.SOLANA -> SolanaNetworkAdapter(network, address(network))
        ChainFamily.TRON -> TronNetworkAdapter(network, address(network), HttpsTronTransport(tronApiKey()))
        else -> throw WalletFailure("This network uses the EVM transaction engine.")
    }
    private fun journal(network: WalletNetwork): FamilyJournal = journals.getOrPut(network.chainId) {
        localRecord(LocalRecordStage.JOURNAL_OPEN) {
            val attributes = Files.readAttributes(vault.directory.toPath(), BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS)
            if (!attributes.isDirectory || attributes.isSymbolicLink) throw LocalRecordFailure(LocalRecordStage.JOURNAL_PATH)
            val path = File(vault.directory.canonicalFile, "family-${FamilyNetworks.protocol(network.chainId)}-v1.sqlite3")
            FamilyJournal(path.absolutePath, FamilyNetworks.protocol(network.chainId), address(network))
        }
    }
    fun history(network: WalletNetwork): List<JSONObject> = if (isEvm(network)) evm.history(network)
        else localRecord(LocalRecordStage.HISTORY_READ) { JSONArray(journal(network).listPayments()).objects() }
    fun events(network: WalletNetwork, id: String): String = if (isEvm(network)) evm.events(network, id) else journal(network).listEvents(id)
    fun cancelPrepared(network: WalletNetwork, id: String) {
        if (isEvm(network)) evm.cancelPrepared(network, id) else journal(network).cancelPrepared(id)
    }
    fun cachedBalance(network: WalletNetwork): BalanceView? {
        if (isEvm(network)) return evm.cachedBalance(network)
        return localRecord(LocalRecordStage.BALANCE_READ) {
            val text = journal(network).latestBalance()
            if (text == "null") null else balanceView(JSONObject(text), 0)
        }
    }
    private fun balanceView(value: JSONObject, observedAt: Long): BalanceView {
        val balances = value.getJSONObject("balances")
        return BalanceView(value.getLong("block_number"), value.getString("block_hash"),
            balances.keys().asSequence().associateWith { BigInteger(balances.getString(it)) }, observedAt)
    }
    fun refreshBalance(network: WalletNetwork): BalanceView {
        if (isEvm(network)) return evm.refreshBalance(network)
        val balance = adapter(network).balance()
        val values = JSONObject().also { json -> balance.values.forEach { (symbol, units) -> json.put(symbol, units.toString()) } }
        val observation = JSONObject().put("network", FamilyNetworks.protocol(network.chainId)).put("address", address(network))
            .put("block_number", balance.height).put("block_hash", balance.blockId).put("balances", values).put("observed_at", balance.observedAt)
        return localRecord(LocalRecordStage.BALANCE_SAVE) {
            val saved = JSONObject(journal(network).recordBalance(observation.toString()))
            if (saved.getLong("block_number") != balance.height || saved.getString("block_hash") != balance.blockId) {
                throw WalletFailure("${network.name} returned older balance data. The newer saved observation has been retained.")
            }
            balanceView(saved, saved.getLong("observed_at"))
        }
    }

    // Existing EVM review/signature policy is reused without reinterpretation.
    fun preview(network: WalletNetwork, symbol: String, recipient: String, amount: String) =
        evm.preview(network, symbol, PrivateAddressRules.requirePublicEvmRecipient(recipient), amount)
    fun maxTransfer(network: WalletNetwork, symbol: String, recipient: String) =
        evm.maxTransfer(network, symbol, PrivateAddressRules.requirePublicEvmRecipient(recipient))
    fun prepare(network: WalletNetwork, symbol: String, recipient: String, amount: String, maxQuote: MaxTransferQuote? = null) =
        evm.prepare(network, symbol, PrivateAddressRules.requirePublicEvmRecipient(recipient), amount, maxQuote)
    fun revalidate(review: ReviewedPayment) = evm.revalidate(review)
    fun signAndBroadcast(review: ReviewedPayment, key: ByteArray, stillAuthorized: () -> Boolean) = evm.signAndBroadcast(review, key, stillAuthorized)

    /** Quote only: no vault access or journal writes. A new quote always requires new review. */
    fun quoteFamily(network: WalletNetwork, symbol: String, recipient: String, amount: String, maximum: Boolean): FamilyPrepared {
        val publicDestination = PrivateAddressRules.requirePublicText(recipient)
        return adapter(network).prepare(symbol, publicDestination, amount, maximum)
    }
    fun prepareFamily(network: WalletNetwork, preparation: FamilyPrepared): FamilyReview {
        if (preparation.request.getString("network") != FamilyNetworks.protocol(network.chainId) || preparation.request.getString("from") != address(network)) {
            throw WalletFailure("The transfer belongs to a different wallet or network.")
        }
        adapter(network).revalidate(preparation.json)
        journal(network).recordPrepared(preparation.json)
        return FamilyReview(network, preparation, SystemClock.elapsedRealtime())
    }
    fun revalidateFamily(review: FamilyReview): FamilyReview {
        adapter(review.network).revalidate(review.prepared.json)
        return review.copy(preflightAt = SystemClock.elapsedRealtime())
    }
    fun signFamilyAndBroadcast(review: FamilyReview, key: ByteArray, stillAuthorized: () -> Boolean): JSONObject {
        try {
            if (!stillAuthorized()) throw WalletFailure("Authentication was interrupted before signing.")
            if (SystemClock.elapsedRealtime() - review.preflightAt !in 0..30_000L) throw WalletFailure("Network checks expired. Review the transfer again.")
            val prepared = review.prepared
            val signed = try {
                if (!stillAuthorized()) throw WalletFailure("Authentication was interrupted before signing.")
                when (NetworkPolicy.family(review.network.chainId)) {
                    ChainFamily.SOLANA -> solanaSign(manifest.vaultHex, key, prepared.json, prepared.objectValue.getString("approval_digest"))
                    ChainFamily.TRON -> tronSign(manifest.vaultHex, key, prepared.json, prepared.objectValue.getString("approval_digest"))
                    else -> throw WalletFailure("Invalid signing engine.")
                }
            } finally { key.fill(0) }
            // A lifecycle interruption after signing still records the signature, then stops submission.
            val durable = JSONObject(journal(review.network).recordSigned(review.id, signed))
            if (!stillAuthorized()) return durable
            return broadcastFamily(review.network, review.id, stillAuthorized)
        } finally { key.fill(0) }
    }
    private fun broadcastFamily(network: WalletNetwork, id: String, stillAuthorized: () -> Boolean): JSONObject {
        val saved = JSONObject(journal(network).getPayment(id))
        val record = saved.getJSONObject("record")
        if (record.getString("phase") !in setOf("signed", "unknown", "pending")) throw WalletFailure("This payment cannot be resubmitted.")
        val signed = saved.optionalString("signed_json") ?: throw WalletFailure("No saved signed transaction exists.")
        val prepared = saved.getString("prepared_json")
        val transport = adapter(network)
        transport.requireRebroadcastable(prepared, signed)
        if (!stillAuthorized()) return record
        // Mark uncertainty before the one outbound attempt so a crash cannot imply non-submission.
        val unknown = JSONObject(journal(network).markSubmission(id, "unknown"))
        if (!stillAuthorized()) return unknown
        val hash = try { transport.broadcast(signed) } catch (_: Exception) { return unknown }
        if (hash != record.getString("hash")) return unknown
        return JSONObject(journal(network).markSubmission(id, "pending"))
    }
    fun rebroadcast(network: WalletNetwork, id: String, stillAuthorized: () -> Boolean): JSONObject {
        if (isEvm(network)) return evm.rebroadcast(network, id, stillAuthorized)
        if (!stillAuthorized()) throw WalletFailure("Authentication was interrupted.")
        reconcile(network)
        return broadcastFamily(network, id, stillAuthorized)
    }
    fun reconcile(network: WalletNetwork): List<JSONObject> {
        if (isEvm(network)) return evm.reconcile(network)
        val transport = adapter(network)
        for (record in history(network)) {
            if (record.getString("phase") !in setOf("signed", "unknown", "pending", "included")) continue
            val id = record.getString("intent_id")
            val saved = JSONObject(journal(network).getPayment(id))
            val evidence = transport.receiptEvidence(saved.getString("prepared_json"), saved.getString("signed_json")) ?: continue
            journal(network).observeReceipt(id, evidence)
        }
        return history(network)
    }
    fun activitySnapshot(network: WalletNetwork): ActivitySnapshot {
        if (isEvm(network)) return evm.activitySnapshot(network)
        return localRecord(LocalRecordStage.ACTIVITY_READ) {
            ActivitySnapshot(JSONArray(journal(network).listActivity()).objects().sortedByDescending { it.getLong("block_number") },
                JSONArray(journal(network).listTracked()).objects(), null, null, "Recent verified transfers. Older deposits can be checked by transaction ID.")
        }
    }
    fun trackTransaction(network: WalletNetwork, inputHash: String): ActivitySnapshot {
        if (isEvm(network)) return evm.trackTransaction(network, inputHash)
        val hash = if (network.chainId == FamilyNetworks.TRON) inputHash.trim().removePrefix("0x").lowercase(java.util.Locale.ROOT) else inputHash.trim()
        journal(network).trackTransaction(hash)
        adapter(network).activityEvidence(hash)?.let { journal(network).recordActivity(hash, it) }
        return activitySnapshot(network)
    }
    fun discoverActivity(network: WalletNetwork, stillInterested: () -> Boolean = { true }): ActivitySnapshot {
        if (isEvm(network)) return evm.discoverActivity(network, stillInterested)
        val transport = adapter(network)
        val snapshot = activitySnapshot(network)
        val pending = snapshot.tracked.filter { it.getString("status") !in setOf("verified", "failed", "not_recipient", "conflict") }.map { it.getString("hash") }
        val included = snapshot.rows.filter { it.getString("inclusion") != "finalized" }.map { it.getString("transaction_hash") }
        val finalized = snapshot.rows.filter { it.getString("inclusion") == "finalized" }.map { it.getString("transaction_hash") }.toSet()
        val terminal = snapshot.tracked.filter { it.getString("status") in setOf("failed", "not_recipient", "conflict") }.map { it.getString("hash") }.toSet()
        var discoveryWarning: String? = null
        val discovered = try { transport.recentTransactionIds() } catch (_: Exception) {
            discoveryWarning = "Recent transfer discovery is unavailable on ${network.name}. Saved transaction IDs can still be checked; find a deposit by transaction ID."
            emptyList<String>()
        }
        val fresh = discovered.filter { it !in finalized && it !in terminal }
        // Recent discoveries keep capacity even when an older provider lookup remains unavailable.
        val candidates = (fresh.take(10) + included.take(5) + pending.take(5) + fresh + included + pending)
            .distinct().filter { it !in finalized && it !in terminal }.take(20)
        for (hash in candidates) {
            if (!stillInterested()) break
            journal(network).trackTransaction(hash)
            val evidence = runCatching { transport.activityEvidence(hash) }.getOrNull()
            if (evidence != null) journal(network).recordActivity(hash, evidence)
        }
        val result = activitySnapshot(network)
        return if (discoveryWarning == null) result else result.copy(
            note = listOfNotNull(result.note, discoveryWarning).joinToString("\n"), warning = discoveryWarning)
    }
    fun auditExport(): String {
        val value = JSONObject(evm.auditExport())
        val networks = value.getJSONArray("networks")
        for (network in FamilyNetworks.all) {
            val payments = JSONArray()
            history(network).forEach { record -> payments.put(JSONObject().put("record", record)
                .put("events", JSONArray(events(network, record.getString("intent_id"))))) }
            networks.put(JSONObject().put("network", network.name).put("chain_id", network.chainId).put("address", address(network))
                .put("payments", payments).put("activity_observations", JSONArray(journal(network).listActivity())))
        }
        return value.toString(2)
    }
    override fun close() { evm.close(); journals.values.forEach { it.close() }; journals.clear(); familyAddresses = null }
}
