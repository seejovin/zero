package app.localwallet.mainnet

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import uniffi.wallet_core.generateWallet
import uniffi.wallet_core.revealMnemonic
import uniffi.wallet_core.restoreWallet
import uniffi.wallet_core.walletAddress
import java.io.File
import java.io.IOException
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.UUID
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Real Rust vaults and JCA ciphers; storage/key adapters model faults without claiming Android device coverage. */
class SecureVaultTest {
    private val publicFixture = "test test test test test test test test test test test junk"
    private val publicAddress = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"

    private class Disk : VaultFile, AutoCloseable {
        override val directory: File = Files.createTempDirectory("wallet-v05-test-").toFile()
        private val path = File(directory, "vault.json").toPath()
        var failBeforeNextWrite = false
        var failAfterNextWrite = false
        var failNextRead = false
        var mutateNextRead: ((ByteArray) -> ByteArray)? = null
        var writes = 0
        override fun exists() = Files.exists(path)
        override fun read(): ByteArray {
            if (failNextRead) { failNextRead = false; throw IOException("Injected read failure") }
            val bytes = Files.readAllBytes(path)
            val mutate = mutateNextRead
            mutateNextRead = null
            return mutate?.invoke(bytes) ?: bytes
        }
        override fun write(value: ByteArray) {
            if (failBeforeNextWrite) { failBeforeNextWrite = false; throw IOException("Injected before commit") }
            val temp = File(directory, "vault.new").toPath()
            Files.write(temp, value)
            Files.move(temp, path, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
            writes++
            if (failAfterNextWrite) { failAfterNextWrite = false; throw IOException("Injected after commit") }
        }
        override fun close() { directory.deleteRecursively() }
    }

    private class Keys : VaultKeys {
        val keys = HashMap<String, SecretKey>()
        val modes = HashMap<String, AppLockMode>()
        override fun create(mode: AppLockMode): VaultEnrollment = create(mode, 2)
        fun create(mode: AppLockMode, version: Int): VaultEnrollment {
            val alias = "local-wallet-v$version-${UUID.randomUUID()}"
            val key = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
            keys[alias] = key; modes[alias] = mode
            return VaultEnrollment(alias, Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key) }, mode)
        }
        override fun decrypt(manifest: VaultManifest): Cipher = Cipher.getInstance("AES/GCM/NoPadding").apply {
            init(Cipher.DECRYPT_MODE, keys[manifest.alias] ?: throw WalletFailure("Missing test device key"), GCMParameterSpec(128, manifest.iv))
            updateAAD(manifest.aad())
        }
        override fun delete(alias: String) { keys.remove(alias); modes.remove(alias) }
    }

    private fun vault(disk: Disk, keys: Keys, now: () -> Long = { 1_000_000L }) = SecureVault(disk, keys, now) { JSONObject(it) }
    private fun create(vault: SecureVault, restore: String? = publicFixture): CreatedVault {
        val enrollment = vault.beginEnrollment()
        return vault.completeEnrollment(enrollment, enrollment.cipher, restore)
    }
    private fun unlock(vault: SecureVault, manifest: VaultManifest, pin: String? = null): ByteArray {
        val secret = pin?.toCharArray()
        try { return vault.unwrap(manifest, vault.decryptionCipher(manifest), secret) }
        finally { secret?.fill('\u0000') }
    }
    private fun change(vault: SecureVault, current: VaultManifest, mode: AppLockMode, oldPin: String? = null, newPin: String? = null): VaultManifest {
        val key = unlock(vault, current, oldPin)
        val pin = newPin?.toCharArray()
        val enrollment = vault.beginProtectionChange(mode)
        try { return vault.changeProtection(current, key, enrollment, enrollment.cipher, pin) }
        finally { key.fill(0); pin?.fill('\u0000') }
    }

    @Test fun newWalletOpensImmediatelyAndRestartKeepsSameWordsWithoutClaimingBackup() = Disk().use { disk ->
        val keys = Keys()
        val first = vault(disk, keys)
        val created = create(first, null)
        assertEquals(24, created.mnemonic!!.split(' ').size)
        assertFalse(created.manifest.backupConfirmed)
        assertEquals(AppLockMode.OFF, created.manifest.lockMode)
        val durable = disk.read().copyOf()
        val restarted = vault(disk, keys)
        val loaded = restarted.load()
        WalletRepository(restarted, loaded).use {
            assertEquals(setOf(1L, 8453L, 10L, 42161L, 43114L, 137L, 56L), it.networks.map { network -> network.chainId }.toSet())
        }
        assertEquals(WalletStart.OPEN, walletStart(loaded))
        assertEquals(created.manifest.address, loaded.address)
        assertEquals(created.manifest.vaultHex, loaded.vaultHex)
        val key = unlock(restarted, loaded)
        try {
            assertEquals(created.mnemonic, revealMnemonic(loaded.vaultHex, key))
            assertEquals(loaded.address, walletAddress(loaded.vaultHex, key))
        } finally { key.fill(0) }
        assertArrayEquals(durable, disk.read())
        assertThrows(WalletFailure::class.java) { restarted.beginEnrollment() }
        assertArrayEquals(durable, disk.read())
        assertFalse(restarted.load().backupConfirmed)
        restarted.confirmBackup()
        assertEquals(WalletStart.OPEN, walletStart(vault(disk, keys).load()))
    }

    @Test fun legacyUnverifiedManifestKeepsDeviceAuthenticationAndSameSeed() = Disk().use { disk ->
        val keys = Keys()
        val enrollment = keys.create(AppLockMode.DEVICE, 1)
        val key = ByteArray(32) { (it + 1).toByte() }
        try {
            val core = JSONObject(generateWallet(key))
            val draft = VaultManifest(core.getString("address"), core.getString("vault_hex"), enrollment.alias, enrollment.cipher.iv, byteArrayOf(), false)
            enrollment.cipher.updateAAD(draft.aad())
            val original = draft.copy(wrappedKey = enrollment.cipher.doFinal(key))
            disk.write(original.json().toString().toByteArray())
            val reopened = vault(disk, keys)
            val loaded = reopened.load()
            assertEquals(1, loaded.version)
            assertEquals(AppLockMode.DEVICE, loaded.lockMode)
            assertEquals(WalletStart.UNLOCK, walletStart(loaded))
            val recovered = unlock(reopened, loaded)
            try { assertEquals(core.getString("mnemonic"), revealMnemonic(loaded.vaultHex, recovered)) }
            finally { recovered.fill(0) }
            reopened.confirmBackup()
            assertEquals(WalletStart.UNLOCK, walletStart(reopened.load()))
            assertEquals(AppLockMode.DEVICE, reopened.load().lockMode)
        } finally { key.fill(0) }
    }

    @Test fun creationReadFailureDoesNotReportSuccessOrDeleteTheCommittedWalletKey() = Disk().use { disk ->
        val keys = Keys(); val first = vault(disk, keys)
        val enrollment = first.beginEnrollment()
        disk.failNextRead = true
        assertThrows(IOException::class.java) { first.completeEnrollment(enrollment, enrollment.cipher, null) }
        first.abortEnrollment(enrollment)
        assertTrue(disk.exists())
        assertTrue(keys.keys.containsKey(enrollment.alias))
        val restarted = vault(disk, keys)
        val saved = restarted.load()
        assertEquals(enrollment.alias, saved.alias)
        assertFalse(saved.backupConfirmed)
        assertEquals(WalletStart.OPEN, walletStart(saved))
        val key = unlock(restarted, saved)
        try { assertEquals(24, revealMnemonic(saved.vaultHex, key).split(' ').size) }
        finally { key.fill(0) }
        assertThrows(WalletFailure::class.java) { restarted.beginEnrollment() }
        Unit
    }

    @Test fun creationRejectsMismatchedReadbackWithoutChangingTheSavedBackupState() = Disk().use { disk ->
        val keys = Keys(); val first = vault(disk, keys)
        val enrollment = first.beginEnrollment()
        disk.mutateNextRead = { bytes ->
            JSONObject(bytes.toString(Charsets.UTF_8)).put("backup_confirmed", true).toString().toByteArray(Charsets.UTF_8)
        }
        assertThrows(WalletFailure::class.java) { first.completeEnrollment(enrollment, enrollment.cipher, null) }
        first.abortEnrollment(enrollment)
        assertTrue(keys.keys.containsKey(enrollment.alias))
        val restarted = vault(disk, keys)
        val saved = restarted.load()
        assertFalse(saved.backupConfirmed)
        assertEquals(WalletStart.OPEN, walletStart(saved))
        unlock(restarted, saved).fill(0)
    }

    @Test fun pinAndOffChangesPreserveVaultAddressAndBackupState() = Disk().use { disk ->
        val keys = Keys(); val vault = vault(disk, keys)
        val original = create(vault).manifest
        val pinned = change(vault, original, AppLockMode.PIN, newPin = "012345")
        assertEquals(original.address, pinned.address)
        assertEquals(original.vaultHex, pinned.vaultHex)
        assertTrue(pinned.backupConfirmed)
        assertEquals(WalletStart.UNLOCK, walletStart(pinned))
        assertFalse(keys.keys.containsKey(original.alias))
        assertThrows(WalletFailure::class.java) { unlock(vault, pinned, "112345") }
        val off = change(vault, pinned, AppLockMode.OFF, oldPin = "012345")
        assertEquals(original.address, off.address)
        assertEquals(original.vaultHex, off.vaultHex)
        assertEquals(WalletStart.OPEN, walletStart(off))
        val restored = unlock(vault, off)
        try { assertEquals(publicAddress, walletAddress(off.vaultHex, restored)) }
        finally { restored.fill(0) }
    }

    @Test fun enablingPinNeverTurnsIncompleteBackupIntoConfirmed() = Disk().use { disk ->
        val keys = Keys(); val vault = vault(disk, keys)
        val original = create(vault, null).manifest
        val pinned = change(vault, original, AppLockMode.PIN, newPin = "567890")
        assertFalse(pinned.backupConfirmed)
        assertEquals(WalletStart.UNLOCK, walletStart(vault.load()))
    }

    @Test fun pinLockoutSurvivesRestartAndSuccessfulUnlockResetsIt() = Disk().use { disk ->
        val keys = Keys(); var now = 1_000_000L
        var vault = vault(disk, keys) { now }
        val pinned = change(vault, create(vault).manifest, AppLockMode.PIN, newPin = "012345")
        repeat(5) { assertThrows(WalletFailure::class.java) { unlock(vault, pinned, "999999") } }
        vault = vault(disk, keys) { now }
        assertEquals(5, vault.load().pinAttempts)
        assertEquals(now + 30_000, vault.load().pinRetryAfter)
        val writes = disk.writes
        assertTrue(assertThrows(WalletFailure::class.java) { unlock(vault, vault.load(), "012345") }.message!!.contains("30 seconds"))
        assertEquals(writes, disk.writes)
        now += 30_001
        unlock(vault, vault.load(), "012345").fill(0)
        assertEquals(0, vault.load().pinAttempts)
        assertEquals(0, vault.load().pinRetryAfter)
    }

    @Test fun failedAttemptReservationDoesNotConsumeCipherOrDecrypt() = Disk().use { disk ->
        val keys = Keys(); val vault = vault(disk, keys)
        val pinned = change(vault, create(vault).manifest, AppLockMode.PIN, newPin = "012345")
        val cipher = vault.decryptionCipher(pinned)
        disk.failBeforeNextWrite = true
        assertThrows(IOException::class.java) { vault.unwrap(pinned, cipher, "012345".toCharArray()) }
        assertEquals(0, vault.load().pinAttempts)
        // The one-use GCM operation remains untouched: no PIN envelope was opened before durable reservation.
        val stillEncryptedByPin = cipher.doFinal(pinned.wrappedKey)
        assertEquals(60, stillEncryptedByPin.size)
        stillEncryptedByPin.fill(0)
    }

    @Test fun uncertainProtectionWriteRetainsBothKeysAndReopensCommittedVersion() = Disk().use { disk ->
        val keys = Keys(); val vault = vault(disk, keys)
        val original = create(vault).manifest
        val key = unlock(vault, original)
        val enrollment = vault.beginProtectionChange(AppLockMode.PIN)
        try {
            disk.failAfterNextWrite = true
            assertThrows(IOException::class.java) { vault.changeProtection(original, key, enrollment, enrollment.cipher, "012345".toCharArray()) }
            vault.abortEnrollment(enrollment)
            assertTrue(keys.keys.containsKey(original.alias))
            assertTrue(keys.keys.containsKey(enrollment.alias))
            val restart = vault(disk, keys)
            val current = restart.load()
            assertEquals(enrollment.alias, current.alias)
            assertEquals(original.vaultHex, current.vaultHex)
            unlock(restart, current, "012345").fill(0)
        } finally { key.fill(0) }
    }

    @Test fun failedProtectionWriteBeforeCommitPreservesOldWalletAndItsKey() = Disk().use { disk ->
        val keys = Keys(); val vault = vault(disk, keys)
        val original = create(vault).manifest
        val saved = disk.read()
        val key = unlock(vault, original)
        val enrollment = vault.beginProtectionChange(AppLockMode.PIN)
        try {
            disk.failBeforeNextWrite = true
            assertThrows(IOException::class.java) { vault.changeProtection(original, key, enrollment, enrollment.cipher, "012345".toCharArray()) }
            vault.abortEnrollment(enrollment)
            assertArrayEquals(saved, disk.read())
            assertTrue(keys.keys.containsKey(original.alias))
            unlock(vault(disk, keys), original).fill(0)
        } finally { key.fill(0) }
    }

    @Test fun cancelledProtectionChangeCannotReplaceManifestOrDeleteCurrentKey() = Disk().use { disk ->
        val keys = Keys(); val vault = vault(disk, keys)
        val original = create(vault).manifest
        val before = disk.read(); val key = unlock(vault, original)
        val enrollment = vault.beginProtectionChange(AppLockMode.PIN)
        var checkpoints = 0
        try {
            assertThrows(WalletFailure::class.java) { vault.changeProtection(original, key, enrollment, enrollment.cipher, "012345".toCharArray()) { ++checkpoints == 1 } }
            assertArrayEquals(before, disk.read())
            assertTrue(keys.keys.containsKey(original.alias))
            assertFalse(keys.keys.containsKey(enrollment.alias))
        } finally { key.fill(0) }
    }

    @Test fun wrapperAuthenticationBindsSelectedLockMethod() = Disk().use { disk ->
        val keys = Keys(); val vault = vault(disk, keys)
        val original = create(vault).manifest
        val edited = original.copy(lockMode = AppLockMode.DEVICE)
        disk.write(edited.json().toString().toByteArray())
        assertThrows(AEADBadTagException::class.java) { unlock(vault, edited) }
        assertEquals(original.vaultHex, vault.load().vaultHex)
    }

    @Test fun staleWrapperCannotOpenAfterSettingsChanged() = Disk().use { disk ->
        val keys = Keys(); val vault = vault(disk, keys)
        val original = create(vault).manifest
        val oldCipher = vault.decryptionCipher(original)
        change(vault, original, AppLockMode.PIN, newPin = "012345")
        assertThrows(WalletFailure::class.java) { vault.unwrap(original, oldCipher) }
        Unit
    }

    @Test fun savedDataContainsNeitherRecoveryWordsNorPin() = Disk().use { disk ->
        val keys = Keys(); val vault = vault(disk, keys)
        val generated = create(vault, null)
        change(vault, generated.manifest, AppLockMode.PIN, newPin = "012345")
        val saved = disk.read().toString(Charsets.UTF_8)
        assertFalse(saved.contains(generated.mnemonic!!))
        assertFalse(saved.contains("012345"))
    }

    @Test fun exactPinLengthAndLeadingZeroAreUnambiguous() {
        VaultPin.requireSixDigits("012345".toCharArray())
        for (bad in listOf("1234", "12345", "1234567", "12345678", "1".repeat(100), "１２３４５６", "abc123")) {
            assertThrows(WalletFailure::class.java) { VaultPin.requireSixDigits(bad.toCharArray()) }
        }
        assertEquals(0, VaultPin.retryDelayMillis(4))
        assertEquals(30_000, VaultPin.retryDelayMillis(5))
        assertEquals(3_600_000, VaultPin.retryDelayMillis(1_000))
    }

    @Test fun malformedMetadataCannotSilentlyDisableAppLockOrChangeBackupState() = Disk().use { disk ->
        val keys = Keys(); val vault = vault(disk, keys)
        val original = create(vault).manifest
        for (mutate in listOf<(JSONObject) -> Unit>(
            { it.remove("backup_confirmed") },
            { it.put("backup_confirmed", "true") },
            { it.remove("lock_mode") },
            { it.put("lock_mode", "unknown") },
            { it.put("version", 1.5) },
            { it.put("pin_attempts", 1.5) },
            { it.put("unexpected", true) }
        )) {
            val changed = JSONObject(original.json().toString()).also(mutate)
            disk.write(changed.toString().toByteArray())
            val before = disk.read()
            assertThrows(WalletFailure::class.java) { vault.load() }
            assertArrayEquals(before, disk.read())
        }
    }
}
