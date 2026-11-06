package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test

class WalletAccessPolicyTest {
    private fun manifest(mode: AppLockMode = AppLockMode.OFF) = VaultManifest(
        "0x1111111111111111111111111111111111111111", "unused", "unused",
        ByteArray(12), ByteArray(48), true, lockMode = mode)

    @Test fun signedOutNeverAutoOpensAnUnlockedSavedVault() {
        // Covers a stale retained manifest after sign-out, resume and Activity reconstruction.
        for (foreground in listOf(false, true)) {
            for (mode in AppLockMode.entries) {
                assertEquals(WalletAccessDestination.PICKER,
                    WalletAccessPolicy.startup(null, manifest(mode), true, foreground))
            }
        }
        assertEquals(WalletAccessDestination.WELCOME, WalletAccessPolicy.startup(null, null, false, true))
    }

    @Test fun selectingWalletRetainsItsOwnProtectionAndRequiresForeground() {
        assertEquals(WalletAccessDestination.OPEN, WalletAccessPolicy.startup("legacy", manifest(), true, true))
        assertEquals(WalletAccessDestination.LOCKED, WalletAccessPolicy.startup("legacy", manifest(), true, false))
        for (mode in listOf(AppLockMode.PIN, AppLockMode.DEVICE)) {
            assertEquals(WalletAccessDestination.LOCKED, WalletAccessPolicy.startup("legacy", manifest(mode), true, true))
        }
        assertEquals(WalletAccessDestination.LOCKED, WalletAccessPolicy.startup("legacy", null, true, true))
    }

    @Test fun pendingSignOutCannotRaceStartupOrAuthentication() {
        assertTrue(WalletAccessPolicy.mayInitialize(false, false, false))
        assertFalse(WalletAccessPolicy.mayInitialize(false, false, true))
        assertFalse(WalletAccessPolicy.mayInitialize(true, false, false))
        assertFalse(WalletAccessPolicy.mayInitialize(false, true, false))
    }

    @Test fun newWalletNamesAreDistinctWithoutRenamingExistingWallets() {
        assertEquals("Wallet 1", WalletAccessPolicy.nextWalletName(emptyList()))
        assertEquals("Wallet 2", WalletAccessPolicy.nextWalletName(listOf("Savings")))
        assertEquals("Wallet 3", WalletAccessPolicy.nextWalletName(listOf("wallet 2")))
        assertEquals("Wallet 4", WalletAccessPolicy.nextWalletName(listOf("Wallet 3", "Trading")))
    }

    @Test fun staleReadsCannotCrossWalletSwitchOrReturnToTheSameWallet() {
        val addressA = "0x1111111111111111111111111111111111111111"
        val addressB = "0x2222222222222222222222222222222222222222"
        val request = WalletUiRequest(addressA, 5, 9, 11)
        assertTrue(request.accepts(addressA, 5, 9, 11, true, true))
        assertFalse(request.accepts(null, 6, 9, 12, true, false))
        assertFalse(request.accepts(addressB, 7, 9, 12, true, true))
        assertFalse(request.accepts(addressA, 8, 9, 12, true, true))
    }

    @Test fun oldQueuedSignOutCannotClearANewerSelection() {
        val authority = WalletSelectionAuthority()
        val activityA = authority.claimActivity(false)
        val oldSignOut = authority.requestSignOut()
        assertFalse(authority.isCurrent(activityA))
        assertTrue(authority.requiresSignOut())
        authority.claimActivity(false)
        val chooseB = authority.requestSelection()
        assertTrue(authority.selected(chooseB))
        assertFalse(authority.isCurrent(oldSignOut))
        assertFalse(authority.selected(oldSignOut))
        assertFalse(authority.requiresSignOut())
    }

    @Test fun pendingSignOutSurvivesRecreationButStaleSavedStateCannotOverrideANewerChoice() {
        val authority = WalletSelectionAuthority()
        authority.claimActivity(false)
        authority.requestSignOut()
        authority.claimActivity(false)
        assertTrue(authority.requiresSignOut())
        assertTrue(authority.selected(authority.requestSelection()))
        authority.claimActivity(true)
        assertFalse(authority.requiresSignOut())

        val restartedProcess = WalletSelectionAuthority()
        restartedProcess.claimActivity(true)
        assertTrue(restartedProcess.requiresSignOut())
    }

    @Test fun staleCallbacksCannotReclaimSelectionOrFinishAnotherActivitysRequest() {
        val authority = WalletSelectionAuthority()
        val firstActivity = authority.claimActivity(false)
        val returningActivity = authority.claimActivity(false)
        repeat(3) {
            assertFalse(authority.isCurrent(firstActivity))
            assertFalse(authority.selected(firstActivity))
        }
        assertTrue(authority.isCurrent(returningActivity))
        val signOut = authority.requestSignOut()
        val interruptedSelection = authority.requestSelection()
        assertFalse(authority.isCurrent(signOut))
        assertTrue(authority.requiresSignOut())
        authority.claimActivity(false)
        assertFalse(authority.selected(interruptedSelection))
        assertTrue(authority.requiresSignOut())
    }
}
