package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test

class WalletNavigationTest {
    private val address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
    private fun capture(screen: String) = WalletNavigation.capture(address, screen, 8453, "USDC", "deposit")!!

    @Test fun recoveryAndLegacyBackupRoutesResumeAsConcealedRecoveryWithoutAChallenge() {
        for (screen in listOf("reveal", "backup", "backup_check", "resume_backup")) {
            val restored = WalletNavigation.decode(WalletNavigation.encode(capture(screen)))!!
            assertEquals(WalletDestination.RECOVERY, restored.destination)
            assertEquals(address, restored.address)
        }
    }

    @Test fun priorApprovalsAndPinFormsCannotBeRestoredAsAuthorizedOperations() {
        assertEquals(WalletDestination.HISTORY, capture("review").destination)
        assertEquals(WalletDestination.SEND, capture("fee_preview").destination)
        assertEquals(WalletDestination.HISTORY, capture("result").destination)
        assertEquals(WalletDestination.APP_LOCK, capture("set_pin").destination)
    }

    @Test fun sameWalletCanReturnToItsExactPublicAssetAndNetworkRoute() {
        val original = WalletNavigation.capture(address, "receive", 1, "EURC", "deposit")!!
        val restored = WalletNavigation.decode(WalletNavigation.encode(original))!!
        assertEquals(WalletDestination.DEPOSIT, restored.destination)
        assertEquals(1L, restored.chainId)
        assertEquals("EURC", restored.symbol)
        assertEquals(restored, WalletNavigation.forWallet(restored, address.lowercase()))
        assertNull(WalletNavigation.forWallet(restored, "0x1111111111111111111111111111111111111111"))
    }

    @Test fun serializationIncludesOnlyAllowlistedPublicRouteFields() {
        val publicState = WalletNavigation.encode(capture("reveal"))
        assertEquals(WalletNavigation.stateKeys.toSet(), publicState.keys)
        val injected = publicState + mapOf("mnemonic" to "not retained", "pin" to "not retained", "review" to "not retained", "rawSigned" to "not retained")
        assertEquals(publicState, WalletNavigation.encode(WalletNavigation.decode(injected)))
        assertFalse(publicState.keys.any { it.contains("phrase") || it.contains("pin") || it.contains("review") })
    }

    @Test fun invalidRouteMetadataCannotRestoreAChangedOrUnsupportedOperation() {
        val state = WalletNavigation.encode(capture("send"))
        for ((field, invalid) in listOf("wallet.route.version" to "3", "wallet.route.chain" to "84532",
            "wallet.route.asset" to "USDT", "wallet.route.destination" to "SIGN", "wallet.route.picker" to "approve",
            "wallet.route.address" to "not-an-address")) {
            assertNull(WalletNavigation.decode(state + (field to invalid)))
        }
        assertNull(WalletNavigation.decode(emptyMap()))
        assertNull(WalletNavigation.capture(null, "reveal", 8453, "USDC", "deposit"))
    }

    @Test fun lockedAndStartupScreensDoNotReplaceTheRouteWaitingForUnlock() {
        val recovery = capture("reveal")
        var pending = recovery
        for (screen in listOf("locked", "opening", "welcome", "restore", "unknown")) {
            pending = WalletNavigation.capture(address, screen, 8453, "USDC", "deposit") ?: pending
        }
        assertEquals(recovery, pending)
    }

    @Test fun quoteCallbacksCannotReturnIntoAnotherWalletPageOrForegroundSession() {
        val read = WalletUiRequest(address, 3, 8, 11)
        assertTrue(read.accepts(address.lowercase(), 3, 8, 11, true, true))
        assertFalse(read.accepts(address, 3, 8, 12, true, true))
        assertFalse(read.accepts(address, 4, 8, 11, true, true))
        assertFalse(read.accepts(address, 3, 9, 11, true, true))
        assertFalse(read.accepts(address, 3, 8, 11, false, true))
        assertFalse(read.accepts(address, 3, 8, 11, true, false))
        assertFalse(read.accepts("0x1111111111111111111111111111111111111111", 3, 8, 11, true, true))
    }

    @Test fun genericTransferRoutesRemainWithoutANetworkAcrossResume() {
        for ((screen, destination) in listOf("send" to WalletDestination.SEND, "receive" to WalletDestination.DEPOSIT,
            "asset_picker" to WalletDestination.ASSET_PICKER)) {
            val route = WalletNavigation.capture(address, screen, null, "EURC", "deposit")!!
            val restored = WalletNavigation.forWallet(WalletNavigation.decode(WalletNavigation.encode(route)), address)!!
            assertEquals(destination, restored.destination)
            assertEquals("EURC", restored.symbol)
            assertNull(restored.chainId)
        }
    }

    @Test fun onlyUnselectedActionRoutesMayOmitANetwork() {
        for (screen in listOf("home", "history", "asset", "track", "review", "reveal")) {
            assertNull(WalletNavigation.capture(address, screen, null, "USDC", "send"))
        }
        val state = WalletNavigation.encode(capture("send"))
        assertNull(WalletNavigation.decode(state - "wallet.route.chain"))
        assertNull(WalletNavigation.decode(state + ("wallet.route.chain" to "null")))
        assertNull(WalletNavigation.decode(state + ("wallet.route.chain" to "0")))
    }

    @Test fun legacyExplicitNetworkRoutesStillRestoreWithoutInventingASelection() {
        val explicit = WalletNavigation.capture(address, "receive", 1, "EURC", "deposit")!!
        val oldState = WalletNavigation.encode(explicit) + ("wallet.route.version" to "1")
        assertEquals(explicit, WalletNavigation.decode(oldState))
        assertNull(WalletNavigation.decode(oldState + ("wallet.route.chain" to "")))
    }
}
