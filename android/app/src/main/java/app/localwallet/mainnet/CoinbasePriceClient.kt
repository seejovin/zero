package app.localwallet.mainnet

import java.io.ByteArrayOutputStream
import java.io.IOException
import java.net.URL
import java.net.SocketTimeoutException
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets
import java.util.Locale
import javax.net.ssl.HttpsURLConnection

fun interface CoinbaseSpotTransport {
    fun get(symbol: String): String
}

data class PriceRefresh(val quotes: Map<String, UsdQuote>, val failedSymbols: Set<String>)

/** Only public asset symbols are sent. This client has no account, address, amount, or signing inputs. */
class CoinbasePriceClient(
    private val transport: CoinbaseSpotTransport = HttpsCoinbaseSpotTransport(),
    private val clock: () -> Long = System::currentTimeMillis
) {
    fun refresh(): PriceRefresh {
        val quotes = linkedMapOf<String, UsdQuote>()
        val failed = linkedSetOf<String>()
        for (symbol in UsdPrices.symbols) {
            try { quotes[symbol] = CoinbaseSpotJson.parse(symbol, read(symbol), clock()) }
            catch (_: Exception) { failed.add(symbol) }
        }
        return PriceRefresh(quotes, failed)
    }

    private fun read(symbol: String): String = try { transport.get(symbol) }
    catch (_: SocketTimeoutException) {
        // A public read has no financial side effect. Retry a timeout once, then keep it unavailable.
        // TLS, HTTP, redirect, authentication and parser failures never enter this retry path.
        transport.get(symbol)
    }
}

class HttpsCoinbaseSpotTransport internal constructor(
    private val open: (URL) -> HttpsURLConnection = { it.openConnection() as HttpsURLConnection }
) : CoinbaseSpotTransport {
    companion object {
        const val MAX_BODY_BYTES = 8192
        private const val REQUEST_MILLIS = 8000L

        fun endpoint(symbol: String): URL {
            if (symbol !in UsdPrices.symbols) throw WalletFailure("Unsupported price asset.")
            return URL("https://api.coinbase.com/v2/prices/$symbol-USD/spot")
        }
    }

    override fun get(symbol: String): String {
        val endpoint = endpoint(symbol)
        val connection = open(endpoint)
        val started = System.nanoTime()
        fun remaining(): Int {
            val millis = REQUEST_MILLIS - (System.nanoTime() - started) / 1_000_000L
            if (millis <= 0) throw SocketTimeoutException("Coinbase price request timed out.")
            return millis.coerceAtMost(4000L).toInt().coerceAtLeast(1)
        }
        try {
            if (connection.url != endpoint) throw IOException("Unexpected price endpoint.")
            connection.requestMethod = "GET"
            connection.connectTimeout = 4000
            connection.readTimeout = remaining()
            connection.instanceFollowRedirects = false
            connection.useCaches = false
            connection.doOutput = false
            connection.setRequestProperty("Accept", "application/json")
            connection.setRequestProperty("Accept-Encoding", "identity")
            if (connection.responseCode != 200) throw IOException("Coinbase prices are unavailable.")
            if (connection.contentType?.substringBefore(';')?.trim()?.lowercase(Locale.ROOT) != "application/json" ||
                connection.contentEncoding?.takeIf { it.isNotBlank() }?.lowercase(Locale.ROOT) !in setOf(null, "identity") ||
                connection.contentLengthLong > MAX_BODY_BYTES) throw IOException("Unexpected Coinbase price response.")
            val bytes = ByteArrayOutputStream()
            connection.inputStream.use { stream ->
                val buffer = ByteArray(1024)
                while (true) {
                    connection.readTimeout = remaining()
                    val count = stream.read(buffer)
                    if (count == -1) break
                    if (bytes.size() + count > MAX_BODY_BYTES) throw IOException("Coinbase price response is too large.")
                    bytes.write(buffer, 0, count)
                }
            }
            remaining()
            return StandardCharsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(bytes.toByteArray())).toString()
        } finally { connection.disconnect() }
    }
}

/** Deliberately small strict parser for Coinbase's three-string spot payload, not a general JSON parser. */
internal object CoinbaseSpotJson {
    fun parse(symbol: String, body: String, fetchedAtMillis: Long): UsdQuote {
        if (body.length > HttpsCoinbaseSpotTransport.MAX_BODY_BYTES) fail()
        var index = 0
        fun whitespace() { while (index < body.length && body[index] in " \r\n\t") index++ }
        fun next(character: Char) {
            whitespace()
            if (index >= body.length || body[index++] != character) fail()
        }
        fun string(): String {
            next('"')
            val start = index
            while (index < body.length && body[index] != '"') {
                // Currency identifiers and decimal prices need neither escapes nor non-ASCII data.
                if (body[index] !in ' '..'~' || body[index] == '\\' || index - start >= 48) fail()
                index++
            }
            if (index >= body.length) fail()
            return body.substring(start, index).also { index++ }
        }
        next('{')
        if (string() != "data") fail()
        next(':'); next('{')
        val values = linkedMapOf<String, String>()
        repeat(3) { field ->
            if (field > 0) next(',')
            val key = string()
            if (key !in setOf("base", "currency", "amount") || key in values) fail()
            next(':')
            values[key] = string()
        }
        next('}'); next('}'); whitespace()
        if (index != body.length || values["base"] != symbol || values["currency"] != "USD") fail()
        return UsdPrices.quote(symbol, values["amount"] ?: fail(), fetchedAtMillis)
    }

    private fun fail(): Nothing = throw WalletFailure("Coinbase returned an invalid USD quote.")
}
