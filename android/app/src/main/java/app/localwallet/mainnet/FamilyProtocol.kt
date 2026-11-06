package app.localwallet.mainnet

import org.json.JSONObject
import java.math.BigInteger

/** Protocol identity is fixed by the app; negative keys are local IDs, never EVM chain IDs. */
enum class ChainFamily { EVM, SOLANA, TRON }

data class FamilyBalance(
    val height: Long,
    val blockId: String,
    val values: Map<String, BigInteger>,
    val observedAt: Long
)

/** Public preparation only. The adapter has no vault, key, signing or journal capability. */
data class FamilyPrepared(
    val json: String,
    val estimatedFee: BigInteger,
    val feeReserve: BigInteger,
    val nativeAvailable: BigInteger,
    val assetAvailable: BigInteger,
    val note: String,
    val validUntilMillis: Long
) {
    val objectValue: JSONObject get() = JSONObject(json)
    val request: JSONObject get() = objectValue.getJSONObject("request")
    val id: String get() = request.getString("intent_id")
    val amountUnits: BigInteger get() = BigInteger(objectValue.getString("amount_units"))
}

/**
 * Read/submit boundary for a specific non-EVM mainnet. Implementations must bind chain
 * identity, token identity, fee evidence and freshness themselves. Submission is one attempt;
 * the caller has already committed and revalidated exact signed bytes before calling it.
 */
interface FamilyNetworkAdapter {
    val network: WalletNetwork
    val address: String
    fun balance(): FamilyBalance
    fun prepare(symbol: String, recipient: String, amount: String, maximum: Boolean): FamilyPrepared
    /** Revalidate the exact prepared payload; never refresh its blockhash/TAPOS or amount. */
    fun revalidate(preparedJson: String)
    /** Return the network's transaction identifier. Never retry or sign in this method. */
    fun broadcast(signedJson: String): String
    /** Null means not yet found, not proof of failure. The Rust validator checks returned evidence. */
    fun receiptEvidence(preparedJson: String, signedJson: String): String?
    /** Check chain lifetime before explicit same-byte rebroadcast. No automatic replacement. */
    fun requireRebroadcastable(preparedJson: String, signedJson: String)
    /** Read-only transaction evidence for validating an incoming deposit; no journal mutation. */
    fun activityEvidence(transactionId: String): String?
    /** Discovery hints only; every result still requires complete validated transaction evidence. */
    fun recentTransactionIds(): List<String> = emptyList()
}

/** Only public addresses; derived from the authenticated vault on each wallet opening. */
data class FamilyAddresses(val solana: String, val tron: String)
