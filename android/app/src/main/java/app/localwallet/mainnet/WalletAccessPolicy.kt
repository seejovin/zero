package app.localwallet.mainnet

internal enum class WalletAccessDestination { WELCOME, PICKER, LOCKED, OPEN }

/** A saved vault is not an active session. Explicit sign-out always wins over app-lock OFF. */
internal object WalletAccessPolicy {
    fun startup(selectedId: String?, manifest: VaultManifest?, hasSavedWallets: Boolean,
                foreground: Boolean): WalletAccessDestination {
        if (selectedId == null) return if (hasSavedWallets) WalletAccessDestination.PICKER else WalletAccessDestination.WELCOME
        if (manifest == null || !foreground || manifest.lockMode != AppLockMode.OFF) return WalletAccessDestination.LOCKED
        return WalletAccessDestination.OPEN
    }

    fun mayInitialize(busy: Boolean, authenticating: Boolean, changingWallet: Boolean): Boolean =
        !busy && !authenticating && !changingWallet

    fun nextWalletName(existingNames: List<String>): String {
        var number = existingNames.size + 1
        while (existingNames.any { it.equals("Wallet $number", ignoreCase = true) }) number++
        return "Wallet $number"
    }
}

/** Orders selection requests from overlapping/recreated Activities in this process. */
internal class WalletSelectionAuthority {
    private var generation = 0L
    private var signedOutRequested = false

    @Synchronized fun claimActivity(restoredSignOut: Boolean): Long {
        // Restored state carries an unfinished sign-out across process death. Within the same
        // process the newer global intent wins over a stale Activity's saved state.
        if (generation == 0L) signedOutRequested = restoredSignOut
        return ++generation
    }

    @Synchronized fun requestSignOut(): Long {
        signedOutRequested = true
        return ++generation
    }

    @Synchronized fun requestSelection(): Long = ++generation
    @Synchronized fun isCurrent(token: Long): Boolean = token == generation
    @Synchronized fun requiresSignOut(): Boolean = signedOutRequested
    @Synchronized fun selected(token: Long): Boolean {
        if (token != generation) return false
        signedOutRequested = false
        return true
    }

    companion object { val process = WalletSelectionAuthority() }
}
