package app.localwallet.mainnet

import android.util.JsonReader
import android.util.JsonToken
import org.json.JSONArray
import org.json.JSONObject
import java.io.StringReader
import java.math.BigDecimal
import java.math.BigInteger
import java.util.Locale

open class WalletFailure(message: String) : Exception(message)
class NetworkReadUnavailable(network: String) : WalletFailure("$network is temporarily unreachable. Your saved balance has not been changed. Try refreshing again.")
enum class RpcRejectionKind { INSUFFICIENT_FUNDS, REVERTED, UNSUPPORTED, UNAVAILABLE }
class RpcRejected(val kind: RpcRejectionKind, val code: Int) : WalletFailure(when (kind) {
    RpcRejectionKind.INSUFFICIENT_FUNDS -> "The account needs more of this network's gas currency to cover this transfer's network fees."
    RpcRejectionKind.REVERTED -> "The transfer simulation was rejected by the network. Nothing was signed."
    RpcRejectionKind.UNSUPPORTED -> "This connection does not support the requested read."
    RpcRejectionKind.UNAVAILABLE -> "The network could not complete this read. Please try again."
})

data class AssetInfo(val symbol: String, val decimals: Int, val contract: String?)
data class WalletNetwork(
    val chainId: Long,
    val name: String,
    val rpcUrl: String,
    val broadcastUrl: String,
    val explorerUrl: String,
    val assets: List<AssetInfo>
) {
    val feeProfile: NetworkFeeProfile get() = NetworkPolicy.feeProfile(chainId)
    val glyph: String get() = NetworkPolicy.glyph(chainId)
    val nativeSymbol: String get() = NetworkPolicy.nativeSymbol(chainId)
    val nativeAsset: AssetInfo get() = asset(nativeSymbol)
    fun asset(symbol: String): AssetInfo {
        if (!NetworkPolicy.supports(chainId, symbol)) throw WalletFailure("This asset is not supported on the selected network.")
        return assets.firstOrNull { it.symbol == symbol } ?: throw WalletFailure("Unsupported asset.")
    }
}

data class BalanceView(val block: Long, val blockHash: String, val balances: Map<String, BigInteger>, val observedAt: Long)
data class FeePreview(
    val network: WalletNetwork, val symbol: String, val recipient: String, val amount: String,
    val gasLimit: Long, val executionEstimate: BigInteger, val executionLimit: BigInteger,
    val l1Estimate: BigInteger, val operatorEstimate: BigInteger, val reserve: BigInteger,
    val ethAvailable: BigInteger, val ethNeeded: BigInteger, val transferAvailable: Boolean,
    val simulated: Boolean, val note: String, val observedAt: Long,
    val assetAvailable: BigInteger? = null
) {
    // Historical constructor names remain source-compatible; values are the network's gas currency.
    val nativeAvailable: BigInteger get() = ethAvailable
    val nativeNeeded: BigInteger get() = ethNeeded
    val totalEstimate: BigInteger get() = executionEstimate + l1Estimate + operatorEstimate
    val shortfall: BigInteger get() = (ethNeeded - ethAvailable).max(BigInteger.ZERO)
}
data class ActivitySnapshot(val rows: List<JSONObject>, val tracked: List<JSONObject>, val fromBlock: Long?, val toBlock: Long?, val note: String?, val warning: String? = null)
data class ReviewedPayment(val network: WalletNetwork, val json: String, val preflightAt: Long) {
    val objectValue: JSONObject get() = JSONObject(json)
    val request: JSONObject get() = objectValue.getJSONObject("request")
    val id: String get() = request.getString("intent_id")
}

object Amounts {
    private val maxU256 = BigInteger.ONE.shiftLeft(256).subtract(BigInteger.ONE)
    fun parse(text: String, decimals: Int): BigInteger {
        if (decimals !in 0..18 || text.length !in 1..96 ||
            !Regex("(?:0|[1-9][0-9]*)(?:\\.[0-9]+)?").matches(text)) {
            throw WalletFailure("Enter an ordinary decimal amount without signs, spaces, commas, or exponent notation.")
        }
        val pieces = text.split('.')
        val fractional = pieces.getOrElse(1) { "" }
        if (fractional.length > decimals) throw WalletFailure("This asset supports at most $decimals decimal places.")
        val units = BigInteger(pieces[0]).multiply(BigInteger.TEN.pow(decimals))
            .add(if (fractional.isEmpty()) BigInteger.ZERO else BigInteger(fractional.padEnd(decimals, '0')))
        if (units <= BigInteger.ZERO || units > maxU256) throw WalletFailure("Amount must be positive and within the asset's integer range.")
        return units
    }
    fun display(units: BigInteger, decimals: Int): String {
        require(units >= BigInteger.ZERO && decimals in 0..18)
        if (decimals == 0) return units.toString()
        val digits = units.toString().padStart(decimals + 1, '0')
        val whole = digits.dropLast(decimals)
        val fractional = digits.takeLast(decimals).trimEnd('0')
        return if (fractional.isEmpty()) whole else "$whole.$fractional"
    }
    fun quantity(value: String): BigInteger {
        if (!Regex("0x(?:0|[1-9a-fA-F][0-9a-fA-F]{0,63})").matches(value)) {
            throw WalletFailure("The network returned a malformed integer.")
        }
        return BigInteger(value.substring(2), 16)
    }
    fun hex(value: BigInteger): String {
        if (value < BigInteger.ZERO || value > maxU256) throw WalletFailure("Integer out of range.")
        return "0x${value.toString(16)}"
    }
    fun longQuantity(value: String): Long {
        val integer = quantity(value)
        if (integer.bitLength() > 63) throw WalletFailure("The network integer exceeds supported limits.")
        return integer.toLong()
    }
    fun word(value: BigInteger): String {
        if (value < BigInteger.ZERO || value > maxU256) throw WalletFailure("Integer out of range.")
        return value.toString(16).padStart(64, '0')
    }
    fun abiInteger(value: String): BigInteger {
        if (!Regex("0x[0-9a-fA-F]{64}").matches(value)) throw WalletFailure("The network returned a malformed contract result.")
        return BigInteger(value.substring(2), 16)
    }
    fun address(value: String): String {
        if (!Regex("0x[0-9a-fA-F]{40}").matches(value) || value.substring(2).all { it == '0' }) {
            throw WalletFailure("Enter a complete nonzero Ethereum address. Names are not supported.")
        }
        return value
    }
    fun hash(value: String): String {
        if (!Regex("0x[0-9a-fA-F]{64}").matches(value)) throw WalletFailure("The network returned a malformed hash.")
        return value.lowercase(Locale.ROOT)
    }
    fun transferData(recipient: String, units: BigInteger): String =
        "0xa9059cbb${address(recipient).substring(2).lowercase(Locale.ROOT).padStart(64, '0')}${word(units)}"
    fun balanceData(address: String): String = "0x70a08231${address(address).substring(2).padStart(64, '0')}"
}

/** Bounded, strict RPC parser: duplicate members must never disappear before Rust validation. */
object RpcJson {
    fun parse(text: String, maxChars: Int = 1_048_576, maxNodes: Int = 50_000): JSONObject {
        require(maxChars in 1..4_194_304 && maxNodes in 1..200_000)
        if (text.length > maxChars) throw WalletFailure("RPC response is too large.")
        var nodes = 0
        JsonReader(StringReader(text)).use { reader ->
            reader.isLenient = false
            fun value(depth: Int): Any {
                if (depth > 32 || ++nodes > maxNodes) throw WalletFailure("RPC response exceeds structural limits.")
                return when (reader.peek()) {
                    JsonToken.BEGIN_OBJECT -> {
                        val objectValue = JSONObject()
                        val seen = HashSet<String>()
                        reader.beginObject()
                        while (reader.hasNext()) {
                            val name = reader.nextName()
                            if (!seen.add(name)) throw WalletFailure("RPC response contains duplicate fields.")
                            objectValue.put(name, value(depth + 1))
                        }
                        reader.endObject()
                        objectValue
                    }
                    JsonToken.BEGIN_ARRAY -> {
                        val values = JSONArray()
                        reader.beginArray()
                        while (reader.hasNext()) values.put(value(depth + 1))
                        reader.endArray()
                        values
                    }
                    JsonToken.STRING -> reader.nextString()
                    JsonToken.NUMBER -> {
                        val number = reader.nextString()
                        if (number.length > 80) throw WalletFailure("RPC numeric field exceeds limits.")
                        BigDecimal(number)
                    }
                    JsonToken.BOOLEAN -> reader.nextBoolean()
                    JsonToken.NULL -> { reader.nextNull(); JSONObject.NULL }
                    else -> throw WalletFailure("Malformed RPC JSON.")
                }
            }
            val result = value(0) as? JSONObject ?: throw WalletFailure("RPC response is not an object.")
            if (reader.peek() != JsonToken.END_DOCUMENT) throw WalletFailure("Trailing data in RPC response.")
            return result
        }
    }
}

fun JSONArray.objects(): List<JSONObject> = (0 until length()).map { getJSONObject(it) }
fun JSONObject.optionalString(key: String): String? = if (isNull(key)) null else getString(key)
