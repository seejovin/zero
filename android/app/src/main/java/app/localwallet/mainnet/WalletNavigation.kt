package app.localwallet.mainnet

internal enum class WalletDestination { HOME, HISTORY, PROFILE, SETTINGS, RECOVERY, APP_LOCK, SEND, DEPOSIT, ASSET, ASSET_PICKER, TRACK }

/** Public navigation metadata only. No recovery words, PIN, transaction review, or signing capability. */
internal data class WalletResumeRoute(
    val address: String,
    val destination: WalletDestination,
    val chainId: Long?,
    val symbol: String,
    val pickerAction: String = "deposit"
)

/** Identity for a public UI read; a pause invalidates it even when Android never calls onStop. */
internal data class WalletUiRequest(val address: String, val epoch: Int, val page: Long, val lifecycle: Long) {
    fun accepts(currentAddress: String?, currentEpoch: Int, currentPage: Long, currentLifecycle: Long,
                foreground: Boolean, unlocked: Boolean): Boolean = foreground && unlocked &&
        address.equals(currentAddress, ignoreCase = true) && epoch == currentEpoch && page == currentPage && lifecycle == currentLifecycle
}

internal object WalletNavigation {
    val stateKeys = listOf("wallet.route.version", "wallet.route.address", "wallet.route.destination",
        "wallet.route.chain", "wallet.route.asset", "wallet.route.picker")

    fun capture(address: String?, screen: String, chainId: Long?, symbol: String, pickerAction: String): WalletResumeRoute? {
        val destination = when (screen) {
            "home" -> WalletDestination.HOME
            "history", "result", "review" -> WalletDestination.HISTORY
            "profile" -> WalletDestination.PROFILE
            "security", "tron_connection" -> WalletDestination.SETTINGS
            "reveal", "backup", "backup_check", "resume_backup" -> WalletDestination.RECOVERY
            "app_lock", "set_pin" -> WalletDestination.APP_LOCK
            // Prepared intents resume in Activity for explicit cancellation; no approval is restored.
            "send", "fee_preview" -> WalletDestination.SEND
            "receive" -> WalletDestination.DEPOSIT
            "asset" -> WalletDestination.ASSET
            "asset_picker" -> WalletDestination.ASSET_PICKER
            "track" -> WalletDestination.TRACK
            else -> return null
        }
        return validated(address, destination, chainId, symbol, pickerAction)
    }

    fun forWallet(route: WalletResumeRoute?, address: String): WalletResumeRoute? = route?.let {
        val checked = validated(it.address, it.destination, it.chainId, it.symbol, it.pickerAction)
        checked?.takeIf { saved -> saved.address.equals(address, ignoreCase = true) }
    }

    fun encode(route: WalletResumeRoute?): Map<String, String> {
        val saved = route?.let { validated(it.address, it.destination, it.chainId, it.symbol, it.pickerAction) } ?: return emptyMap()
        return mapOf(stateKeys[0] to "2", stateKeys[1] to saved.address, stateKeys[2] to saved.destination.name,
            stateKeys[3] to (saved.chainId?.toString() ?: ""), stateKeys[4] to saved.symbol, stateKeys[5] to saved.pickerAction)
    }

    fun decode(values: Map<String, String?>): WalletResumeRoute? = runCatching {
        val version = values[stateKeys[0]]
        if (version !in setOf("1", "2")) return null
        val destination = WalletDestination.valueOf(values[stateKeys[2]] ?: return null)
        val rawChain = values[stateKeys[3]] ?: return null
        // Version 1 always represented a chosen chain. Version 2 also preserves the explicit
        // unchosen state; restoring a public route must never silently pick a network.
        val chain: Long? = if (version == "2" && rawChain.isEmpty()) null else (rawChain.toLongOrNull() ?: return null)
        validated(values[stateKeys[1]], destination, chain, values[stateKeys[4]] ?: return null, values[stateKeys[5]] ?: return null)
    }.getOrNull()

    private fun validated(address: String?, destination: WalletDestination, chainId: Long?, symbol: String, pickerAction: String): WalletResumeRoute? {
        if (address == null || symbol !in NetworkPolicy.assetSymbols || pickerAction !in setOf("send", "deposit")) return null
        if (chainId == null) {
            if (destination !in setOf(WalletDestination.SEND, WalletDestination.DEPOSIT, WalletDestination.ASSET_PICKER)) return null
        } else if (!NetworkPolicy.supports(chainId, symbol)) return null
        val canonical = runCatching { Amounts.address(address) }.getOrNull() ?: return null
        return WalletResumeRoute(canonical, destination, chainId, symbol, pickerAction)
    }
}
