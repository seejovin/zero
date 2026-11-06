package app.localwallet.mainnet

import android.content.Context

/** One synchronous preference commit keeps the two public labels together. App backups remain disabled. */
internal class DisplayNamePreferences(context: Context) : DisplayNamePersistence {
    private val preferences = context.getSharedPreferences("wallet-display-names-v1", Context.MODE_PRIVATE)
    override fun read(addressScope: String): DisplayNames? {
        val wallet = preferences.getString("$addressScope.wallet", null)
        val account = preferences.getString("$addressScope.account", null)
        if (wallet == null && account == null) return null
        return DisplayNames(wallet ?: "Wallet 1", account ?: "Account 1")
    }
    override fun write(addressScope: String, names: DisplayNames): Boolean = preferences.edit()
        .putString("$addressScope.wallet", names.wallet).putString("$addressScope.account", names.account).commit()

    override fun readWallet(addressScope: String): String? = preferences.getString("$addressScope.wallet", null)
    override fun writeWallet(addressScope: String, wallet: String): Boolean = preferences.edit()
        .putString("$addressScope.wallet", wallet).commit()
}
