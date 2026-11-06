package app.localwallet.mainnet

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import uniffi.wallet_core.revealMnemonic
import java.io.File
import java.io.IOException
import java.math.BigDecimal
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.ExecutionException
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException
import java.util.concurrent.atomic.AtomicBoolean
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Real encrypted Rust vaults/JCA keys and fault-injected local files; no device claims. */
class WalletCatalogTest {
    private val fixture = "test test test test test test test test test test test junk"
    private val idA = "11111111-1111-4111-8111-111111111111"
    private val idB = "22222222-2222-4222-8222-222222222222"

    private class Keys : VaultKeys {
        val retained = ConcurrentHashMap<String, SecretKey>()
        override fun create(mode: AppLockMode): VaultEnrollment {
            val alias = "local-wallet-v2-${UUID.randomUUID()}"
            val key = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
            retained[alias] = key
            return VaultEnrollment(alias, Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key) }, mode)
        }
        override fun decrypt(manifest: VaultManifest): Cipher = Cipher.getInstance("AES/GCM/NoPadding").apply {
            init(Cipher.DECRYPT_MODE, retained[manifest.alias] ?: throw IOException("Missing fixture key"), GCMParameterSpec(128, manifest.iv))
            updateAAD(manifest.aad())
        }
        override fun delete(alias: String) { retained.remove(alias) }
    }

    private class Environment : WalletCatalogStorage, AutoCloseable {
        val root = Files.createTempDirectory("zero-wallet-catalog-test-").toFile()
        val wallets = File(root, "wallets-v1").apply { mkdir() }
        val keys = Keys()
        val selection = File(wallets, "selection.json")
        var failBeforeSelectionWrite = false
        var failAfterSelectionWrite = false
        @Volatile var duringVaultWrite: ((File) -> Unit)? = null
        @Volatile var readDuringPendingWrite = false
        fun directory(id: String) = if (id == WalletCatalog.LEGACY_ID) File(root, "wallet-v1") else File(wallets, id)
        fun open(id: String): SecureVault {
            WalletCatalog.requireId(id)
            val vaultDirectory = directory(id).apply { mkdirs() }
            val file = object : VaultFile {
                override val directory = vaultDirectory
                private val path = File(directory, "vault.json")
                override fun exists() = path.exists()
                override fun read(): ByteArray {
                    if (File(directory, "vault.new").exists()) readDuringPendingWrite = true
                    return path.readBytes()
                }
                override fun write(value: ByteArray) {
                    val temporary = File(directory, "vault.new")
                    temporary.writeBytes(value)
                    duringVaultWrite?.invoke(directory)
                    Files.move(temporary.toPath(), path.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
                }
            }
            return SecureVault(file, keys, { 1_000_000L }) { JSONObject(it) }
        }
        fun catalog(next: () -> String = { UUID.randomUUID().toString() }, decimalJson: Boolean = false) =
            WalletCatalog(this, ::open, next) { text ->
                JSONObject(text).also { if (decimalJson) it.put("version", BigDecimal(it.get("version").toString())) }
            }
        override fun ids(): List<String> = buildList {
            if (File(directory(WalletCatalog.LEGACY_ID), "vault.json").exists()) add(WalletCatalog.LEGACY_ID)
            wallets.listFiles()!!.filter { it.isDirectory && File(it, "vault.json").exists() }.forEach { add(it.name) }
        }
        override fun reserve(id: String) = directory(id).mkdir()
        override fun readSelection(): ByteArray? = selection.takeIf { it.exists() }?.readBytes()
        override fun writeSelection(bytes: ByteArray) {
            if (failBeforeSelectionWrite) { failBeforeSelectionWrite = false; throw IOException("Injected before selection commit") }
            val temporary = File(wallets, "selection.new")
            temporary.writeBytes(bytes)
            Files.move(temporary.toPath(), selection.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
            if (failAfterSelectionWrite) { failAfterSelectionWrite = false; throw IOException("Injected after selection commit") }
        }
        override fun close() { root.deleteRecursively() }
    }

    private fun enroll(vault: SecureVault, words: String? = fixture, check: (String) -> Unit = {}): CreatedVault {
        val enrollment = vault.beginEnrollment()
        return vault.completeEnrollment(enrollment, enrollment.cipher, words, check)
    }
    private fun enroll(catalog: WalletCatalog, pending: NewWallet, words: String? = fixture): CreatedVault =
        enroll(pending.vault, words) { catalog.requireUniqueAddress(it, pending.id) }
    private fun words(vault: SecureVault): String {
        val manifest = vault.load()
        val key = vault.unwrap(manifest, vault.decryptionCipher(manifest))
        return try { revealMnemonic(manifest.vaultHex, key) } finally { key.fill(0) }
    }

    @Test fun legacyUpgradeAndSignOutPreserveVaultKeysAndJournalsAcrossRestart() = Environment().use { env ->
        val legacy = env.open(WalletCatalog.LEGACY_ID)
        val original = enroll(legacy).manifest
        val journal = File(legacy.directory, "journal.sqlite").apply { writeText("retained transaction evidence") }
        val bytes = File(legacy.directory, "vault.json").readBytes()
        val catalog = env.catalog()
        assertEquals(WalletCatalog.LEGACY_ID, catalog.selectedId())
        assertEquals(env.directory(WalletCatalog.LEGACY_ID), catalog.vault(WalletCatalog.LEGACY_ID).directory)
        catalog.signOut()
        val restarted = env.catalog()
        assertNull(restarted.selectedId())
        assertEquals(listOf(WalletCatalog.LEGACY_ID), restarted.list().map { it.id })
        assertArrayEquals(bytes, File(legacy.directory, "vault.json").readBytes())
        assertEquals("retained transaction evidence", journal.readText())
        assertTrue(env.keys.retained.containsKey(original.alias))
        assertEquals(fixture, words(restarted.vault(WalletCatalog.LEGACY_ID)))
        restarted.select(WalletCatalog.LEGACY_ID)
        assertEquals(WalletCatalog.LEGACY_ID, env.catalog().selectedId())
    }

    @Test fun twoWalletsHaveIndependentKeysStorageAndDurableSelection() = Environment().use { env ->
        val catalog = env.catalog()
        assertNull(catalog.selectedId())
        val a = catalog.allocate()
        val first = enroll(catalog, a)
        catalog.select(a.id)
        val b = catalog.allocate()
        val second = enroll(catalog, b, null)
        catalog.select(b.id)
        assertNotEquals(a.id, b.id)
        assertNotEquals(a.vault.directory, b.vault.directory)
        assertNotEquals(first.manifest.address, second.manifest.address)
        assertNotEquals(first.manifest.alias, second.manifest.alias)
        File(a.vault.directory, "journal.sqlite").writeText("wallet A signed payment")
        File(b.vault.directory, "journal.sqlite").writeText("wallet B signed payment")
        val restarted = env.catalog()
        assertEquals(b.id, restarted.selectedId())
        assertEquals(setOf(a.id, b.id), restarted.list().map { it.id }.toSet())
        assertEquals(fixture, words(restarted.vault(a.id)))
        assertEquals(second.mnemonic, words(restarted.vault(b.id)))
        assertEquals("wallet A signed payment", File(restarted.vault(a.id).directory, "journal.sqlite").readText())
        assertEquals("wallet B signed payment", File(restarted.vault(b.id).directory, "journal.sqlite").readText())
        restarted.signOut()
        assertNull(env.catalog().selectedId())
        assertEquals(2, env.keys.retained.size)
    }

    @Test fun duplicateImportIsRejectedBeforeSavingOrKeepingItsWrappingKey() = Environment().use { env ->
        val catalog = env.catalog()
        val first = catalog.allocate()
        val original = enroll(catalog, first).manifest
        catalog.select(first.id)
        val before = File(first.vault.directory, "vault.json").readBytes()
        val duplicate = catalog.allocate()
        val failure = assertThrows(WalletFailure::class.java) { enroll(catalog, duplicate) }
        assertTrue(failure.message!!.contains("already saved"))
        assertFalse(duplicate.vault.exists())
        assertEquals(setOf(original.alias), env.keys.retained.keys.toSet())
        assertArrayEquals(before, File(first.vault.directory, "vault.json").readBytes())
        assertEquals(first.id, env.catalog().selectedId())
        assertEquals(listOf(first.id), env.catalog().list().map { it.id })
        assertThrows(WalletFailure::class.java) { catalog.requireUniqueAddress(original.address.uppercase().replace("0X", "0x")) }
        Unit
    }

    @Test fun committedWalletSurvivesFailedSelectionAndRemainsDiscoverable() = Environment().use { env ->
        val catalog = env.catalog()
        val first = catalog.allocate(); enroll(catalog, first); catalog.select(first.id)
        val second = catalog.allocate(); val created = enroll(catalog, second, null)
        env.failBeforeSelectionWrite = true
        assertThrows(IOException::class.java) { catalog.select(second.id) }
        val restart = env.catalog()
        assertEquals(first.id, restart.selectedId())
        assertEquals(setOf(first.id, second.id), restart.list().map { it.id }.toSet())
        assertEquals(created.mnemonic, words(restart.vault(second.id)))
        assertEquals(fixture, words(restart.vault(first.id)))
        assertTrue(env.keys.retained.containsKey(created.manifest.alias))
    }

    @Test fun uncertainSelectionWriteKeepsBothWalletsAndReadsTheCommittedState() = Environment().use { env ->
        val catalog = env.catalog()
        val first = catalog.allocate(); enroll(catalog, first); catalog.select(first.id)
        val second = catalog.allocate(); enroll(catalog, second, null)
        env.failAfterSelectionWrite = true
        assertThrows(IOException::class.java) { catalog.select(second.id) }
        assertEquals(second.id, env.catalog().selectedId())
        assertEquals(2, env.catalog().list().size)
        env.failAfterSelectionWrite = true
        assertThrows(IOException::class.java) { catalog.signOut() }
        assertNull(env.catalog().selectedId())
        assertEquals(2, env.keys.retained.size)
    }

    @Test fun freshSignedOutStateNeverAdoptsALaterDiscoveredLegacyWallet() = Environment().use { env ->
        assertNull(env.catalog().selectedId())
        enroll(env.open(WalletCatalog.LEGACY_ID))
        assertNull(env.catalog().selectedId())
        assertEquals(WalletCatalog.LEGACY_ID, env.catalog().list().single().id)
    }

    @Test fun missingSelectionWithMultipleSavedWalletsDoesNotReopenTheLegacyWallet() = Environment().use { env ->
        enroll(env.open(WalletCatalog.LEGACY_ID))
        val catalog = env.catalog()
        assertEquals(WalletCatalog.LEGACY_ID, catalog.selectedId())
        val additional = catalog.allocate(); enroll(catalog, additional, null); catalog.select(additional.id)
        assertTrue(env.selection.delete())
        assertNull(env.catalog().selectedId())
        assertEquals(2, env.catalog().list().size)
        assertTrue(JSONObject(env.selection.readText()).isNull("active_wallet"))
    }

    @Test fun emptyReservationsAreHiddenAndNeverOverwriteAnExistingDirectory() = Environment().use { env ->
        assertTrue(env.reserve(idA))
        File(env.directory(idA), "retained.txt").writeText("do not overwrite")
        var calls = 0
        val catalog = env.catalog(next = { if (++calls == 1) idA else idB })
        val pending = catalog.allocate()
        assertEquals(idB, pending.id)
        assertTrue(catalog.list().isEmpty())
        assertEquals("do not overwrite", File(env.directory(idA), "retained.txt").readText())
        assertFalse(pending.vault.exists())
    }

    @Test fun damagedWalletAndSelectionArePreservedWithoutSilentFallback() = Environment().use { env ->
        val catalog = env.catalog()
        val good = catalog.allocate(); enroll(catalog, good); catalog.select(good.id)
        val damaged = catalog.allocate()
        File(damaged.vault.directory, "vault.json").writeText("corrupted")
        val entries = catalog.list()
        assertEquals(2, entries.size)
        assertNotNull(entries.single { it.id == good.id }.manifest)
        assertNull(entries.single { it.id == damaged.id }.manifest)
        assertNotNull(entries.single { it.id == damaged.id }.error)
        assertThrows(WalletFailure::class.java) { catalog.requireUniqueAddress("0x" + "11".repeat(20)) }
        env.selection.writeText("{\"version\":1,\"active_wallet\":\"../../other\"}")
        assertThrows(WalletFailure::class.java) { env.catalog().selectedId() }
        assertEquals(2, env.catalog().list().size)
        catalog.select(good.id)
        assertEquals(good.id, env.catalog().selectedId())
        assertEquals("corrupted", File(damaged.vault.directory, "vault.json").readText())
    }

    @Test fun selectionAcceptsStrictParserNumbersAndRejectsCoercions() = Environment().use { env ->
        assertNull(env.catalog(decimalJson = true).selectedId())
        assertNull(env.catalog(decimalJson = true).selectedId())
        for (bad in listOf("\"1\"", "true", "1.5", "2")) {
            env.selection.writeText("{\"version\":$bad,\"active_wallet\":null}")
            assertThrows(WalletFailure::class.java) { env.catalog().selectedId() }
        }
        env.selection.writeText("{\"version\":1,\"active_wallet\":null,\"unexpected\":true}")
        assertThrows(WalletFailure::class.java) { env.catalog().selectedId() }
        for (bad in listOf("{", "{}", "{\"version\":1}", "{\"version\":1,\"active_wallet\":42}")) {
            env.selection.writeText(bad)
            assertThrows(WalletFailure::class.java) { env.catalog().selectedId() }
        }
        for (id in listOf("../wallet-v1", "/tmp/wallet", "", "LEGACY", "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA")) {
            assertThrows(WalletFailure::class.java) { env.catalog().vault(id) }
        }
    }

    @Test fun simultaneousDuplicateEnrollmentsCannotCreateTwoIndependentJournals() = Environment().use { env ->
        val catalog = env.catalog()
        val a = catalog.allocate(); val b = catalog.allocate()
        val start = CountDownLatch(1)
        val workers = Executors.newFixedThreadPool(2)
        try {
            val futures = listOf(a, b).map { pending ->
                workers.submit<Boolean> {
                    start.await()
                    try { enroll(catalog, pending); true }
                    catch (failure: WalletFailure) { assertTrue(failure.message!!.contains("already saved")); false }
                }
            }
            start.countDown()
            assertEquals(1, futures.count { it.get(30, TimeUnit.SECONDS) })
            assertEquals(1, catalog.list().size)
            assertEquals(1, env.keys.retained.size)
            assertTrue(a.vault.exists() xor b.vault.exists())
        } finally { workers.shutdownNow() }
    }

    @Test fun catalogReadsFromAnotherInstanceWaitUntilTheVaultWriteFinishes() = Environment().use { env ->
        val catalog = env.catalog()
        val pending = catalog.allocate(); enroll(catalog, pending, null)
        val enteredWrite = CountDownLatch(1)
        val releaseWrite = CountDownLatch(1)
        val enteredRead = CountDownLatch(1)
        env.duringVaultWrite = { directory ->
            if (directory == pending.vault.directory) {
                enteredWrite.countDown()
                if (!releaseWrite.await(10, TimeUnit.SECONDS)) throw IOException("Write test timed out")
            }
        }
        val workers = Executors.newFixedThreadPool(2)
        try {
            val write = workers.submit<VaultManifest> { env.open(pending.id).confirmBackup() }
            assertTrue(enteredWrite.await(10, TimeUnit.SECONDS))
            val read = workers.submit<List<StoredWallet>> { enteredRead.countDown(); env.catalog().list() }
            assertTrue(enteredRead.await(10, TimeUnit.SECONDS))
            assertThrows(TimeoutException::class.java) { read.get(150, TimeUnit.MILLISECONDS) }
            releaseWrite.countDown()
            assertTrue(write.get(10, TimeUnit.SECONDS).backupConfirmed)
            assertTrue(read.get(10, TimeUnit.SECONDS).single().manifest!!.backupConfirmed)
            assertFalse(env.readDuringPendingWrite)
            assertFalse(File(pending.vault.directory, "vault.new").exists())
        } finally { releaseWrite.countDown(); workers.shutdownNow() }
    }

    @Test fun waitingOldSelectionOrSignOutCannotOverwriteANewerWallet() = Environment().use { env ->
        val catalog = env.catalog()
        val old = catalog.allocate(); enroll(catalog, old)
        val current = catalog.allocate(); enroll(catalog, current, null)
        val worker = Executors.newSingleThreadExecutor()
        try {
            for (signingOut in listOf(false, true)) {
                val allowed = AtomicBoolean(true)
                val requested = CountDownLatch(1)
                val pending = walletStorage {
                    val future = worker.submit<Unit> {
                        requested.countDown()
                        if (signingOut) env.catalog().signOut { allowed.get() }
                        else env.catalog().select(old.id) { allowed.get() }
                    }
                    assertTrue(requested.await(10, TimeUnit.SECONDS))
                    allowed.set(false)
                    catalog.select(current.id)
                    future
                }
                val failure = assertThrows(ExecutionException::class.java) { pending.get(10, TimeUnit.SECONDS) }
                assertTrue(failure.cause is WalletFailure)
                assertEquals(current.id, env.catalog().selectedId())
            }
            assertEquals(2, catalog.list().size)
            assertEquals(2, env.keys.retained.size)
        } finally { worker.shutdownNow() }
    }
}
