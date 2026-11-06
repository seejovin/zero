package app.localwallet.mainnet

import android.content.Context

/** A display preference only; no wallet or transaction data shares this preference file. */
internal class PortfolioPreferences(context: Context) : PortfolioPreferencePersistence {
    private val preferences = context.getSharedPreferences("wallet-portfolio-display-v1", Context.MODE_PRIVATE)
    override fun read(addressScope: String): Boolean? =
        if (preferences.contains("$addressScope.hide-small")) preferences.getBoolean("$addressScope.hide-small", false) else null

    override fun write(addressScope: String, hideSmallBalances: Boolean): Boolean = preferences.edit()
        .putBoolean("$addressScope.hide-small", hideSmallBalances).commit()

    override fun readThreshold(addressScope: String): String? = preferences.getString("$addressScope.usd-threshold-v1", null)

    override fun writeSettings(addressScope: String, hideSmallBalances: Boolean, thresholdUsd: String): Boolean = preferences.edit()
        .putBoolean("$addressScope.hide-small", hideSmallBalances)
        .putString("$addressScope.usd-threshold-v1", thresholdUsd).commit()
}
