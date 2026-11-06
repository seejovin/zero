package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test
import java.math.BigDecimal

class PortfolioPreferencesTest {
    private val address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
    private val other = "0x1111111111111111111111111111111111111111"
    private class Memory : PortfolioPreferencePersistence {
        val values = mutableMapOf<String, Boolean>()
        val thresholds = mutableMapOf<String, String>()
        var writes = 0
        var refuse = false
        override fun read(addressScope: String): Boolean? = values[addressScope]
        override fun write(addressScope: String, hideSmallBalances: Boolean): Boolean {
            if (refuse) return false
            values[addressScope] = hideSmallBalances; writes++; return true
        }
        override fun readThreshold(addressScope: String): String? = thresholds[addressScope]
        override fun writeSettings(addressScope: String, hideSmallBalances: Boolean, thresholdUsd: String): Boolean {
            if (refuse) return false
            values[addressScope] = hideSmallBalances; thresholds[addressScope] = thresholdUsd; writes++; return true
        }
    }

    @Test fun defaultOffDoesNotWriteAndAnExplicitChoiceSurvivesReopenWithinTheSameWalletScope() {
        val storage = Memory()
        assertFalse(WalletPortfolioPreferences(storage).load(address))
        assertEquals(0, storage.writes)
        assertTrue(WalletPortfolioPreferences(storage).save(address, true))
        assertTrue(WalletPortfolioPreferences(storage).load(address.lowercase()))
        assertFalse(WalletPortfolioPreferences(storage).load(other))
        assertFalse(WalletPortfolioPreferences(storage).save(address, false))
        assertFalse(WalletPortfolioPreferences(storage).load(address))
        assertEquals(setOf(address.lowercase()), storage.values.keys)
        assertEquals(2, storage.writes)
    }

    @Test fun failedSavingIsNotReportedAsSuccessAndUnavailableReadingDefaultsToShowingSmallHoldings() {
        val storage = Memory()
        val preferences = WalletPortfolioPreferences(storage)
        preferences.save(address, false)
        storage.refuse = true
        assertThrows(WalletFailure::class.java) { preferences.save(address, true) }
        assertFalse(preferences.load(address))
        assertEquals(1, storage.writes)
        val unavailable = object : PortfolioPreferencePersistence {
            override fun read(addressScope: String): Boolean? = throw IllegalStateException("private fixture storage details")
            override fun write(addressScope: String, hideSmallBalances: Boolean): Boolean = throw IllegalStateException("private fixture storage details")
            override fun readThreshold(addressScope: String): String? = throw IllegalStateException("private fixture storage details")
            override fun writeSettings(addressScope: String, hideSmallBalances: Boolean, thresholdUsd: String): Boolean = throw IllegalStateException("private fixture storage details")
        }
        assertFalse(WalletPortfolioPreferences(unavailable).load(address))
        val error = assertThrows(WalletFailure::class.java) { WalletPortfolioPreferences(unavailable).save(address, true) }
        assertEquals("The display setting could not be saved. Try again.", error.message)
    }

    @Test fun invalidWalletScopesCannotCreatePreferencesOrAffectExistingLabels() {
        val storage = Memory()
        val preferences = WalletPortfolioPreferences(storage)
        preferences.save(address, true)
        for (invalid in listOf("", "not-an-address", address + "\n", "0x" + "0".repeat(40))) {
            assertThrows(WalletFailure::class.java) { preferences.save(invalid, false) }
        }
        assertEquals(1, storage.writes)
        assertTrue(preferences.load(address))
    }

    @Test fun usdMigrationRetainsTheExistingToggleAndNewThresholdSurvivesReopenWithoutAffectingAnotherWallet() {
        val storage = Memory()
        val preferences = WalletPortfolioPreferences(storage)
        assertEquals(PortfolioDisplaySettings(false, BigDecimal.ONE), preferences.loadSettings(address))
        preferences.save(address, true)
        assertEquals(PortfolioDisplaySettings(true, BigDecimal.ONE), preferences.loadSettings(address))
        assertEquals(PortfolioDisplaySettings(false, BigDecimal("0.25")), preferences.saveSettings(address, false, "0.25"))
        assertEquals(PortfolioDisplaySettings(false, BigDecimal("0.25")), WalletPortfolioPreferences(storage).loadSettings(address.lowercase()))
        assertEquals(PortfolioDisplaySettings(false, BigDecimal.ONE), preferences.loadSettings(other))
        assertEquals(PortfolioDisplaySettings(true, BigDecimal("0.25")), preferences.saveSettings(address, true, "0.25"))
    }

    @Test fun invalidUsdThresholdOrFailedCommitDoesNotPartiallyChangeTheSavedSetting() {
        val storage = Memory()
        val preferences = WalletPortfolioPreferences(storage)
        preferences.saveSettings(address, false, "2.50")
        val original = PortfolioDisplaySettings(false, BigDecimal("2.5"))
        for (invalid in listOf("0", "-1", "1e3", "NaN", "1000000.01", "0.001", "1\n", "9".repeat(100))) {
            assertThrows(WalletFailure::class.java) { preferences.saveSettings(address, true, invalid) }
            assertEquals(original, preferences.loadSettings(address))
        }
        storage.refuse = true
        assertThrows(WalletFailure::class.java) { preferences.saveSettings(address, true, "10") }
        assertEquals(original, preferences.loadSettings(address))
        assertEquals(1, storage.writes)
    }

    @Test fun corruptThresholdDefaultsToOneDollarWithoutSilentlyRewritingTheExistingToggle() {
        val storage = Memory()
        storage.values[address.lowercase()] = true
        storage.thresholds[address.lowercase()] = "not a dollar amount"
        assertEquals(PortfolioDisplaySettings(true, BigDecimal.ONE), WalletPortfolioPreferences(storage).loadSettings(address))
        assertEquals(0, storage.writes)
        assertEquals("not a dollar amount", storage.thresholds[address.lowercase()])
    }
}
