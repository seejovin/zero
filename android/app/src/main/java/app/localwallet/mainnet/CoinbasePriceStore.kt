package app.localwallet.mainnet

import android.content.Context

internal interface CoinbasePricePersistence {
    fun read(): Map<String, UsdQuote>
    fun write(quotes: Map<String, UsdQuote>): Boolean
}

/** Public prices only. No wallet identifier, balances, recovery material, or transactions are cached here. */
class CoinbasePriceStore internal constructor(
    private val persistence: CoinbasePricePersistence,
    private val client: CoinbasePriceClient,
    private val clock: () -> Long = System::currentTimeMillis
) {
    constructor(context: Context) : this(CoinbasePricePreferences(context), CoinbasePriceClient())
    private var memory: Map<String, UsdQuote> = emptyMap()

    @Synchronized fun load(): Map<String, UsdQuote> {
        val candidates = usable(runCatching { persistence.read() }.getOrDefault(emptyMap())).toMutableMap()
        for ((symbol, quote) in usable(memory)) {
            if ((candidates[symbol]?.fetchedAtMillis ?: 0) <= quote.fetchedAtMillis) candidates[symbol] = quote
        }
        memory = usable(candidates)
        return memory.toMap()
    }

    @Synchronized fun refresh(): PriceRefresh {
        val previous = load()
        val fetched = client.refresh()
        memory = usable(previous + fetched.quotes)
        // A price-cache failure must not invalidate a usable live quote or any wallet operation.
        runCatching { persistence.write(memory) }
        return PriceRefresh(memory.toMap(), fetched.failedSymbols)
    }

    private fun usable(quotes: Map<String, UsdQuote>): Map<String, UsdQuote> {
        val now = clock()
        return UsdPrices.symbols.mapNotNull { symbol ->
            quotes[symbol]?.takeIf { it.symbol == symbol && UsdPrices.freshness(it, now) != PriceFreshness.UNAVAILABLE }
                ?.let { symbol to it }
        }.toMap()
    }
}

private class CoinbasePricePreferences(context: Context) : CoinbasePricePersistence {
    private val preferences = context.getSharedPreferences("coinbase-public-usd-prices-v1", Context.MODE_PRIVATE)
    override fun read(): Map<String, UsdQuote> = UsdPrices.symbols.mapNotNull { symbol ->
        runCatching {
            val amount = preferences.getString("$symbol.usd", null) ?: return@runCatching null
            val received = preferences.getLong("$symbol.received-at", 0L)
            symbol to UsdPrices.quote(symbol, amount, received)
        }.getOrNull()
    }.toMap()

    override fun write(quotes: Map<String, UsdQuote>): Boolean {
        val edit = preferences.edit().clear()
        for (symbol in UsdPrices.symbols) {
            quotes[symbol]?.let { quote ->
                edit.putString("$symbol.usd", quote.usdPerUnit.toPlainString())
                edit.putLong("$symbol.received-at", quote.fetchedAtMillis)
            }
        }
        return edit.commit()
    }
}
