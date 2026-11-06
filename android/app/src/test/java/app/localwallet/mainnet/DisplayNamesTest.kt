package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test

class DisplayNamesTest {
    private val address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
    private class Memory : DisplayNamePersistence {
        val rows = mutableMapOf<String, DisplayNames>()
        var writes = 0
        var rejectWrite = false
        override fun read(addressScope: String): DisplayNames? = rows[addressScope]
        override fun write(addressScope: String, names: DisplayNames): Boolean {
            if (rejectWrite) return false
            rows[addressScope] = names; writes++; return true
        }
        override fun readWallet(addressScope: String): String? = rows[addressScope]?.wallet
        override fun writeWallet(addressScope: String, wallet: String): Boolean {
            if (rejectWrite) return false
            rows[addressScope] = (rows[addressScope] ?: DisplayNames()).copy(wallet = wallet); writes++; return true
        }
    }

    @Test fun defaultsAndRenamesPersistAcrossInstancesUnderTheSameAccountScope() {
        val storage = Memory()
        assertEquals(DisplayNames(), WalletDisplayNames(storage).load(address))
        val saved = WalletDisplayNames(storage).save(address, DisplayNames("  Travel  ", "  Daily  "))
        assertEquals(DisplayNames("Travel", "Daily"), saved)
        assertEquals(saved, WalletDisplayNames(storage).load(address.lowercase()))
        assertEquals(setOf(address.lowercase()), storage.rows.keys)
        assertEquals(DisplayNames(), WalletDisplayNames(storage).load("0x1111111111111111111111111111111111111111"))
    }

    @Test fun namesUseNfcAndUnicodeCodepointsRatherThanUtf16Length() {
        assertEquals("Café", DisplayNameRules.normalize("  Cafe\u0301  "))
        val emoji = "\uD83D\uDCBC"
        assertEquals(emoji.repeat(32), DisplayNameRules.normalize(emoji.repeat(32)))
        assertThrows(WalletFailure::class.java) { DisplayNameRules.normalize(emoji.repeat(33)) }
        assertThrows(WalletFailure::class.java) { DisplayNameRules.normalize(" ") }
        assertThrows(WalletFailure::class.java) { DisplayNameRules.normalize("a".repeat(257)) }
    }

    @Test fun rawControlsBidiAndUnpairedSurrogatesAreRejectedEvenAtTheEdges() {
        for (invalid in listOf("\nWallet", "Wallet\n", "\tWallet", "Wal\u0000let", "Wal\u007Flet", "Wal\u0085let",
            "Wal\u202Elet", "Wal\u2067let", "Wal\u200Dlet", "Wal\u2028let", "Wal\u2029let", "Wal\uD800let", "Wal\uDC00let")) {
            assertThrows(WalletFailure::class.java) { DisplayNameRules.normalize(invalid) }
        }
    }

    @Test fun invalidSavedCosmeticMetadataDoesNotBlockTheWalletOrGetSilentlyRewritten() {
        val storage = Memory()
        storage.rows[address.lowercase()] = DisplayNames("\u202Ebad", "Account")
        assertEquals(DisplayNames(), WalletDisplayNames(storage).load(address))
        assertEquals(0, storage.writes)
        assertEquals("\u202Ebad", storage.rows[address.lowercase()]!!.wallet)
    }

    @Test fun failedOrInvalidRenameDoesNotReportASuccessfulSave() {
        val storage = Memory()
        val names = WalletDisplayNames(storage)
        names.save(address, DisplayNames("Main", "Savings"))
        storage.rejectWrite = true
        assertThrows(WalletFailure::class.java) { names.save(address, DisplayNames("New", "Daily")) }
        assertEquals(DisplayNames("Main", "Savings"), names.load(address))
        assertThrows(WalletFailure::class.java) { names.save(address, DisplayNames("\nBad", "Daily")) }
        assertEquals(1, storage.writes)
    }

    @Test fun walletOnlyRenamePreservesLegacyAccountTextWithoutValidatingOrReplacingIt() {
        val storage = Memory()
        val legacyAccount = "\u202E legacy\naccount "
        storage.rows[address.lowercase()] = DisplayNames("Existing wallet", legacyAccount)
        val names = WalletDisplayNames(storage)
        assertEquals("Existing wallet", names.loadWallet(address))
        assertEquals("Reserve", names.saveWallet(address, "  Reserve  "))
        assertEquals(legacyAccount, storage.rows[address.lowercase()]!!.account)
        assertEquals("Reserve", WalletDisplayNames(storage).loadWallet(address.lowercase()))
        assertEquals("Wallet 1", names.loadWallet("0x1111111111111111111111111111111111111111"))
        assertThrows(WalletFailure::class.java) { names.saveWallet(address, "\nInvalid") }
        assertEquals(legacyAccount, storage.rows[address.lowercase()]!!.account)
        assertEquals(1, storage.writes)
    }

    @Test fun walletOnlyApiNeverCallsLegacyCombinedRecordMethods() {
        var stored: String? = "Travel"
        var writes = 0
        val persistence = object : DisplayNamePersistence {
            override fun read(addressScope: String): DisplayNames? = throw AssertionError("Hidden account metadata must not be read")
            override fun write(addressScope: String, names: DisplayNames): Boolean = throw AssertionError("Hidden account metadata must not be replaced")
            override fun readWallet(addressScope: String): String? = stored
            override fun writeWallet(addressScope: String, wallet: String): Boolean { stored = wallet; writes++; return true }
        }
        val names = WalletDisplayNames(persistence)
        assertEquals("Travel", names.loadWallet(address))
        assertEquals("Café", names.saveWallet(address, "Cafe\u0301"))
        assertEquals("Café", names.loadWallet(address))
        assertEquals(1, writes)
    }

    @Test fun walletOnlyFailuresAreSanitizedAndBadSavedNamesAreNotRewritten() {
        val storage = Memory()
        storage.rows[address.lowercase()] = DisplayNames("\u202Ebad", "Unchanged")
        assertEquals("Wallet 1", WalletDisplayNames(storage).loadWallet(address))
        assertEquals("\u202Ebad", storage.rows[address.lowercase()]!!.wallet)
        assertEquals(0, storage.writes)
        storage.rejectWrite = true
        assertThrows(WalletFailure::class.java) { WalletDisplayNames(storage).saveWallet(address, "New") }
        assertEquals("Unchanged", storage.rows[address.lowercase()]!!.account)
        assertEquals(0, storage.writes)
    }
}
