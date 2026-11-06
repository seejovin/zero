package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.IOException
import java.io.InputStream
import java.math.BigDecimal
import java.net.URL
import java.net.SocketTimeoutException
import java.security.cert.Certificate
import javax.net.ssl.HttpsURLConnection
import javax.net.ssl.SSLHandshakeException

class CoinbasePriceClientTest {
    private val now = 1_000_000L
    private fun body(symbol: String, price: String) = """{"data":{"amount":"$price","base":"$symbol","currency":"USD"}}"""
    // Original captured payload values plus explicit synthetic offline quotes for new symbols.
    private val liveFixtures = UsdPrices.symbols.associateWith { symbol ->
        when (symbol) { "EURC" -> "1.1224251796805455"; "ETH" -> "2247.38"; "AVAX" -> "24.123456"; "POL" -> "0.123456789"; "BNB" -> "800.1234"; else -> "1" }
    }
    private fun client(fail: Set<String> = emptySet(), clock: () -> Long = { now }) = CoinbasePriceClient(CoinbaseSpotTransport { symbol ->
        if (symbol in fail) throw IOException("provider details must not reach UI")
        body(symbol, liveFixtures.getValue(symbol))
    }, clock)

    @Test fun capturedLivePublicPayloadsForUsdcEurcAndEthPreserveDecimalPrecisionAndSource() {
        val calls = mutableListOf<String>()
        val result = CoinbasePriceClient(CoinbaseSpotTransport { symbol ->
            calls.add(symbol); body(symbol, liveFixtures.getValue(symbol))
        }, { now }).refresh()
        assertEquals(UsdPrices.symbols, calls)
        assertTrue(result.failedSymbols.isEmpty())
        assertEquals(UsdPrices.symbols.size, result.quotes.size)
        assertEquals(BigDecimal("1.1224251796805455"), result.quotes.getValue("EURC").usdPerUnit)
        assertTrue(result.quotes.values.all { it.fetchedAtMillis == now && it.source == "Coinbase" })
    }

    @Test fun partialFailureIsIsolatedAndNeverReplacedByAOneDollarStablecoinAssumption() {
        val result = client(setOf("USDC", "EURC")).refresh()
        assertEquals(setOf("USDC", "EURC"), result.failedSymbols)
        assertEquals(UsdPrices.symbols.toSet() - setOf("USDC", "EURC"), result.quotes.keys)
        assertEquals(BigDecimal("2247.38"), result.quotes.getValue("ETH").usdPerUnit)
    }

    @Test fun oneColdConnectionTimeoutRetriesTheSamePublicPairAndRecoversWithoutDuplicatingOtherReads() {
        val calls = linkedMapOf<String, Int>()
        val result = CoinbasePriceClient(CoinbaseSpotTransport { symbol ->
            calls[symbol] = (calls[symbol] ?: 0) + 1
            if (symbol == "USDC" && calls[symbol] == 1) throw SocketTimeoutException("cold fixture connection")
            body(symbol, liveFixtures.getValue(symbol))
        }, { now }).refresh()
        assertEquals(UsdPrices.symbols.associateWith { if (it == "USDC") 2 else 1 }, calls)
        assertTrue(result.failedSymbols.isEmpty())
        assertEquals(UsdPrices.symbols.size, result.quotes.size)
        assertEquals(BigDecimal.ONE, result.quotes.getValue("USDC").usdPerUnit)
    }

    @Test fun repeatedTimeoutStopsAfterTwoAttemptsAndDoesNotStopOtherPublicPairs() {
        val calls = linkedMapOf<String, Int>()
        val result = CoinbasePriceClient(CoinbaseSpotTransport { symbol ->
            calls[symbol] = (calls[symbol] ?: 0) + 1
            if (symbol == "USDC") throw SocketTimeoutException("still unavailable")
            body(symbol, liveFixtures.getValue(symbol))
        }, { now }).refresh()
        assertEquals(UsdPrices.symbols.associateWith { if (it == "USDC") 2 else 1 }, calls)
        assertEquals(setOf("USDC"), result.failedSymbols)
        assertEquals(UsdPrices.symbols.toSet() - "USDC", result.quotes.keys)
    }

    @Test fun tlsHttpAndMalformedPayloadFailuresAreNeverRetried() {
        val calls = linkedMapOf<String, Int>()
        val result = CoinbasePriceClient(CoinbaseSpotTransport { symbol ->
            calls[symbol] = (calls[symbol] ?: 0) + 1
            when (symbol) {
                "USDC" -> throw SSLHandshakeException("untrusted fixture certificate")
                "EURC" -> throw IOException("HTTP fixture rejection")
                else -> body("USDC", "1") // Wrong base for ETH: a parser/identity failure.
            }
        }, { now }).refresh()
        assertEquals(UsdPrices.symbols.associateWith { 1 }, calls)
        assertEquals(UsdPrices.symbols.toSet(), result.failedSymbols)
        assertTrue(result.quotes.isEmpty())
    }

    @Test fun strictSpotParserRejectsWrongIdentityDuplicateKeysTrailingOrOversizedData() {
        val malformed = listOf(
            body("EURC", "1"), body("USDC", "0"), body("USDC", "1").replace("USD\"", "EUR\""),
            """{"data":{"base":"USDC","currency":"USD","amount":"1","amount":"2"}}""",
            """{"data":{"base":"USDC","base":"USDC","amount":"1"}}""",
            """{"data":{"base":"USDC","currency":"USD","amount":1}}""",
            """{"data":{"base":"USDC","currency":"USD","amount":"1",}}""",
            """{"data":{"base":"USDC","currency":"USD","amount":"1"}} {}""",
            """{"data":{"base":"USDC","currency":"USD","amount":"1e0"}}""",
            """{"data":{"base":"USDC","currency":"USD","amount":"1"},"data":{}}""",
            " ".repeat(HttpsCoinbaseSpotTransport.MAX_BODY_BYTES + 1)
        )
        for (text in malformed) assertThrows(WalletFailure::class.java) { CoinbaseSpotJson.parse("USDC", text, now) }
        assertEquals(BigDecimal.ONE, CoinbaseSpotJson.parse("USDC", " \n{ \"data\" : {\"currency\":\"USD\",\"base\":\"USDC\",\"amount\":\"1\"} }\t", now).usdPerUnit)
    }

    private class Connection(url: URL, private val payload: ByteArray) : HttpsURLConnection(url) {
        var status = 200
        var type = "application/json; charset=utf-8"
        var encoding: String? = null
        var declaredLength = -1L
        var disconnected = false
        var streamOpened = false
        override fun getResponseCode() = status
        override fun getContentType() = type
        override fun getContentEncoding(): String? = encoding
        override fun getContentLengthLong() = declaredLength
        override fun getInputStream(): InputStream { streamOpened = true; return ByteArrayInputStream(payload) }
        override fun disconnect() { disconnected = true }
        override fun usingProxy() = false
        override fun connect() { connected = true }
        override fun getCipherSuite() = "fixture"
        override fun getLocalCertificates(): Array<Certificate>? = null
        override fun getServerCertificates(): Array<Certificate> = emptyArray()
    }

    @Test fun transportOnlyOpensTheThreeFixedHttpsUrlsAndSendsNoWalletOrAuthenticationData() {
        val opened = mutableListOf<Connection>()
        val transport = HttpsCoinbaseSpotTransport { url ->
            Connection(url, body("USDC", "1").toByteArray()).also { opened.add(it) }
        }
        assertEquals(body("USDC", "1"), transport.get("USDC"))
        val connection = opened.single()
        assertEquals("https://api.coinbase.com/v2/prices/USDC-USD/spot", connection.url.toString())
        assertEquals("GET", connection.requestMethod)
        assertFalse(connection.instanceFollowRedirects)
        assertFalse(connection.useCaches)
        assertFalse(connection.doOutput)
        assertEquals("identity", connection.getRequestProperty("Accept-Encoding"))
        assertEquals("application/json", connection.getRequestProperty("Accept"))
        assertNull(connection.getRequestProperty("Authorization"))
        assertNull(connection.getRequestProperty("Cookie"))
        assertTrue(connection.connectTimeout in 1..4000)
        assertTrue(connection.readTimeout in 1..4000)
        assertTrue(connection.disconnected)
        for (symbol in listOf("https://evil.invalid", "ETH?address=0x123", "../ETH", "usdc", "BTC")) {
            assertThrows(WalletFailure::class.java) { transport.get(symbol) }
        }
        assertEquals(1, opened.size)
        for (symbol in UsdPrices.symbols) assertEquals("https://api.coinbase.com/v2/prices/$symbol-USD/spot", HttpsCoinbaseSpotTransport.endpoint(symbol).toString())
    }

    @Test fun redirectsErrorsWrongContentAndOversizedBodiesFailClosedWithTheConnectionClosed() {
        val failures = listOf<(Connection) -> Unit>(
            { it.status = 301 }, { it.status = 429 }, { it.status = 503 }, { it.type = "text/html" },
            { it.encoding = "gzip" }, { it.declaredLength = HttpsCoinbaseSpotTransport.MAX_BODY_BYTES + 1L }
        )
        for (mutate in failures) {
            val connection = Connection(HttpsCoinbaseSpotTransport.endpoint("ETH"), body("ETH", "2000").toByteArray())
            mutate(connection)
            assertThrows(IOException::class.java) { HttpsCoinbaseSpotTransport { connection }.get("ETH") }
            assertTrue(connection.disconnected)
            assertFalse(connection.streamOpened)
        }
        val streamed = Connection(HttpsCoinbaseSpotTransport.endpoint("ETH"), ByteArray(HttpsCoinbaseSpotTransport.MAX_BODY_BYTES + 1) { 32 })
        assertThrows(IOException::class.java) { HttpsCoinbaseSpotTransport { streamed }.get("ETH") }
        assertTrue(streamed.disconnected)
        val invalidUtf8 = Connection(HttpsCoinbaseSpotTransport.endpoint("ETH"), byteArrayOf(0xC3.toByte(), 0x28))
        assertThrows(IOException::class.java) { HttpsCoinbaseSpotTransport { invalidUtf8 }.get("ETH") }
        assertTrue(invalidUtf8.disconnected)
    }

    private class Memory : CoinbasePricePersistence {
        var saved = emptyMap<String, UsdQuote>()
        var failWrites = false
        override fun read() = saved
        override fun write(quotes: Map<String, UsdQuote>): Boolean {
            if (failWrites) return false
            saved = quotes.toMap(); return true
        }
    }

    @Test fun publicQuoteCacheSurvivesReopenAndARefreshFailureNeverRenewsOldTimestamps() {
        var time = now
        val cache = Memory()
        val first = CoinbasePriceStore(cache, client(clock = { time }), { time }).refresh()
        assertEquals(UsdPrices.symbols.size, first.quotes.size)
        time += UsdPrices.FRESH_MILLIS + 1
        val reopened = CoinbasePriceStore(cache, client(UsdPrices.symbols.toSet(), { time }), { time })
        val loaded = reopened.load()
        assertTrue(loaded.values.all { it.fetchedAtMillis == now && UsdPrices.freshness(it, time) == PriceFreshness.STALE })
        val failed = reopened.refresh()
        assertEquals(UsdPrices.symbols.toSet(), failed.failedSymbols)
        assertEquals(loaded, failed.quotes)
        time = now + UsdPrices.MAX_CACHE_MILLIS + 1
        assertTrue(reopened.load().isEmpty())
        assertTrue(reopened.refresh().quotes.isEmpty())
    }

    @Test fun partialRefreshKeepsOnlyEligibleCachedQuotesAndDoesNotCrossBindTheirSymbols() {
        val cache = Memory()
        cache.saved = mapOf("EURC" to UsdPrices.quote("EURC", "1.1", now - 1000),
            "USDC" to UsdPrices.quote("ETH", "2000", now), "ETH" to UsdPrices.quote("ETH", "2500", now + 1))
        val store = CoinbasePriceStore(cache, client(setOf("EURC")), { now })
        assertEquals(setOf("EURC"), store.load().keys)
        val result = store.refresh()
        assertEquals(UsdPrices.symbols.size, result.quotes.size)
        assertEquals(setOf("EURC"), result.failedSymbols)
        assertEquals(now - 1000, result.quotes.getValue("EURC").fetchedAtMillis)
        assertEquals(BigDecimal("1.1"), result.quotes.getValue("EURC").usdPerUnit)
        assertEquals(BigDecimal.ONE, result.quotes.getValue("USDC").usdPerUnit)
    }

    @Test fun priceCacheWriteFailureDoesNotInvalidateLiveQuotesOrInventDurability() {
        val cache = Memory().apply {
            failWrites = true
            saved = mapOf("USDC" to UsdPrices.quote("USDC", "1", now + 1))
        }
        val store = CoinbasePriceStore(cache, client(), { now })
        assertEquals(UsdPrices.symbols.size, store.refresh().quotes.size)
        assertEquals(UsdPrices.symbols.size, store.load().size)
        assertEquals(now + 1, cache.saved.getValue("USDC").fetchedAtMillis)
        assertTrue(CoinbasePriceStore(cache, client(UsdPrices.symbols.toSet()), { now }).load().isEmpty())
    }
}
