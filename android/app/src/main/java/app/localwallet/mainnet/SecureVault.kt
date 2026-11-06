package app.localwallet.mainnet

import android.app.KeyguardManager
import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.system.Os
import android.system.OsConstants
import android.util.AtomicFile
import org.json.JSONObject
import uniffi.wallet_core.generateWallet
import uniffi.wallet_core.restoreWallet
import uniffi.wallet_core.walletAddress
import java.io.File
import java.security.KeyStore
import java.security.SecureRandom
import java.util.UUID
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

data class VaultEnrollment(val alias: String, val cipher: Cipher, val lockMode: AppLockMode, var commitAttempted: Boolean = false)
data class CreatedVault(val manifest: VaultManifest, val mnemonic: String?)

internal interface VaultFile {
    val directory: File
    fun exists(): Boolean
    fun read(): ByteArray
    fun write(value: ByteArray)
}

internal interface VaultKeys {
    fun create(mode: AppLockMode): VaultEnrollment
    fun decrypt(manifest: VaultManifest): Cipher
    fun delete(alias: String)
}

/** The device wrapping key never leaves Android Keystore. Vault keys are operation-scoped. */
class SecureVault internal constructor(
    private val file: VaultFile,
    private val keys: VaultKeys,
    private val now: () -> Long = System::currentTimeMillis,
    private val decode: (String) -> JSONObject = RpcJson::parse
) {
    constructor(context: Context) : this(AndroidVaultFile(context, WalletCatalog.LEGACY_ID), AndroidVaultKeys(context))
    internal constructor(context: Context, walletId: String) : this(AndroidVaultFile(context, walletId), AndroidVaultKeys(context))
    val directory: File get() = file.directory
    fun exists(): Boolean = walletStorage { file.exists() }

    fun load(): VaultManifest = walletStorage {
        if (!exists()) throw WalletFailure("No wallet exists on this device.")
        VaultManifest.parse(decode(file.read().toString(Charsets.UTF_8)))
    }

    fun beginEnrollment(): VaultEnrollment = walletStorage {
        if (exists()) throw WalletFailure("A wallet already exists. Open it to continue; it cannot be overwritten.")
        keys.create(AppLockMode.OFF)
    }

    fun beginProtectionChange(mode: AppLockMode): VaultEnrollment = walletStorage {
        if (!exists()) throw WalletFailure("Open the existing wallet before changing app lock.")
        keys.create(mode)
    }

    fun abortEnrollment(enrollment: VaultEnrollment): Unit = walletStorage {
        // A late fsync error may follow a committed rename. Never remove either key in that case.
        if (!enrollment.commitAttempted) runCatching { keys.delete(enrollment.alias) }
        Unit
    }

    fun completeEnrollment(enrollment: VaultEnrollment, authorizedCipher: Cipher, restoreWords: String?,
                           validateAddress: (String) -> Unit = {}): CreatedVault = walletStorage {
        if (exists()) throw WalletFailure("A wallet already exists. Open it to continue; it cannot be overwritten.")
        if (authorizedCipher !== enrollment.cipher || enrollment.lockMode != AppLockMode.OFF) throw WalletFailure("Wallet creation was not bound to its device key operation.")
        val vaultKey = ByteArray(32).also { SecureRandom().nextBytes(it) }
        try {
            val created = JSONObject(if (restoreWords == null) generateWallet(vaultKey) else restoreWallet(restoreWords, vaultKey))
            validateAddress(created.getString("address"))
            val manifest = wrap(enrollment, authorizedCipher, created.getString("address"), created.getString("vault_hex"), vaultKey, restoreWords != null, null)
            enrollment.commitAttempted = true
            save(manifest)
            val readBack = load()
            if (!manifest.sameProtection(readBack) || readBack.backupConfirmed != manifest.backupConfirmed) {
                throw WalletFailure("The new wallet could not be verified after saving. Its device key has been retained; reopen the wallet.")
            }
            return@walletStorage CreatedVault(readBack, created.optionalString("mnemonic"))
        } catch (error: Exception) { abortEnrollment(enrollment); throw error }
        finally { vaultKey.fill(0) }
    }

    /** Current-key authentication is completed by the caller before entering this operation. */
    fun changeProtection(current: VaultManifest, key: ByteArray, enrollment: VaultEnrollment, authorizedCipher: Cipher, pin: CharArray?, stillAuthorized: () -> Boolean = { true }): VaultManifest = walletStorage {
        val saved = currentProtection(current)
        if (authorizedCipher !== enrollment.cipher || key.size != 32 || !walletAddress(saved.vaultHex, key).equals(saved.address, true)) {
            throw WalletFailure("The app-lock change was not authorized for this wallet.")
        }
        try {
            if (!stillAuthorized()) throw WalletFailure("App-lock change was interrupted before saving.")
            val replacement = wrap(enrollment, authorizedCipher, saved.address, saved.vaultHex, key, saved.backupConfirmed, pin)
            if (!stillAuthorized()) throw WalletFailure("App-lock change was interrupted before saving.")
            enrollment.commitAttempted = true
            save(replacement)
            val readBack = load()
            if (!replacement.sameProtection(readBack) || readBack.backupConfirmed != saved.backupConfirmed) {
                throw WalletFailure("App-lock settings could not be verified after saving. Both device keys have been retained; reopen the wallet.")
            }
            // Only after durable success and read-back can the old wrapping key be retired.
            runCatching { keys.delete(saved.alias) }
            return@walletStorage readBack
        } catch (error: Exception) { abortEnrollment(enrollment); throw error }
    }

    private fun wrap(enrollment: VaultEnrollment, cipher: Cipher, address: String, vaultHex: String, key: ByteArray, confirmed: Boolean, pin: CharArray?): VaultManifest {
        val salt = if (enrollment.lockMode == AppLockMode.PIN) ByteArray(16).also { SecureRandom().nextBytes(it) } else null
        val draft = VaultManifest(address, vaultHex, enrollment.alias, cipher.iv.copyOf(), byteArrayOf(), confirmed, 2, enrollment.lockMode, salt)
        val inner = if (salt == null) key.copyOf() else VaultPin.seal(key, pin ?: throw WalletFailure("Choose a 6-digit app PIN."), salt, draft.aad())
        try {
            cipher.updateAAD(draft.aad())
            return draft.copy(wrappedKey = cipher.doFinal(inner))
        } finally { inner.fill(0) }
    }

    fun decryptionCipher(manifest: VaultManifest): Cipher = walletStorage { keys.decrypt(manifest) }

    fun unwrap(manifest: VaultManifest, authorizedCipher: Cipher, pin: CharArray? = null): ByteArray = walletStorage {
        val current = currentProtection(manifest)
        if (current.lockMode == AppLockMode.PIN) {
            VaultPin.requireSixDigits(pin ?: throw WalletFailure("Enter your 6-digit app PIN."))
            val at = now()
            if (at < current.pinRetryAfter) {
                val seconds = (current.pinRetryAfter - at + 999) / 1_000
                throw WalletFailure("Too many PIN attempts. Try again in $seconds seconds.")
            }
            val attempt = minOf(1_000, current.pinAttempts + 1)
            // Reserve before any decryption. Process death and failed disk writes cannot grant free guesses.
            save(current.copy(pinAttempts = attempt, pinRetryAfter = at + VaultPin.retryDelayMillis(attempt)))
        }
        val unwrapped = authorizedCipher.doFinal(current.wrappedKey)
        val key = try {
            if (current.lockMode == AppLockMode.PIN) VaultPin.open(unwrapped, pin!!, current.pinSalt!!, current.aad()) else unwrapped.copyOf()
        } finally { unwrapped.fill(0) }
        try {
            if (key.size != 32 || !walletAddress(current.vaultHex, key).equals(current.address, ignoreCase = true)) {
                throw WalletFailure("The encrypted vault does not match the stored receiving address.")
            }
            if (current.lockMode == AppLockMode.PIN) {
                val saved = currentProtection(current)
                save(saved.copy(pinAttempts = 0, pinRetryAfter = 0))
            }
            return@walletStorage key
        } catch (error: Exception) { key.fill(0); throw error }
    }

    fun confirmBackup(): VaultManifest = walletStorage {
        val manifest = load().copy(backupConfirmed = true)
        save(manifest)
        manifest
    }

    private fun currentProtection(expected: VaultManifest): VaultManifest {
        val current = load()
        if (!expected.sameProtection(current)) throw WalletFailure("Wallet protection changed during this operation. Reopen the wallet before continuing.")
        return current
    }

    private fun save(manifest: VaultManifest) { file.write(manifest.json().toString().toByteArray(Charsets.UTF_8)) }
}

private class AndroidVaultFile(context: Context, walletId: String) : VaultFile {
    override val directory: File = WalletStoragePaths.directory(context.noBackupFilesDir, walletId)
    private val atomic = AtomicFile(File(directory, "vault.json"))
    private fun paths() = listOf(atomic.baseFile, File(atomic.baseFile.path + ".bak"), File(atomic.baseFile.path + ".new"))
    override fun exists(): Boolean {
        paths().forEach(WalletStoragePaths::requireRegularOrMissing)
        return paths().any(WalletStoragePaths::exists)
    }
    override fun read(): ByteArray = try {
        paths().forEach(WalletStoragePaths::requireRegularOrMissing)
        atomic.openRead().use { stream ->
            val result = java.io.ByteArrayOutputStream()
            val buffer = ByteArray(1_024)
            while (true) {
                val count = stream.read(buffer)
                if (count == -1) break
                if (result.size() + count > 16_384) throw WalletFailure("Wallet metadata exceeds its size limit.")
                result.write(buffer, 0, count)
            }
            result.toByteArray()
        }
    } catch (error: WalletFailure) { throw error }
    catch (_: Exception) { throw WalletFailure("The saved wallet could not be opened. Its files have been preserved. Retry opening the wallet.") }

    override fun write(value: ByteArray) {
        paths().forEach(WalletStoragePaths::requireRegularOrMissing)
        var stream: java.io.FileOutputStream? = null
        try {
            stream = atomic.startWrite()
            stream.write(value)
            stream.fd.sync()
            atomic.finishWrite(stream)
            stream = null
            Os.chmod(atomic.baseFile.absolutePath, 384)
            val descriptor = Os.open(directory.absolutePath, OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW, 0)
            try {
                if (!OsConstants.S_ISDIR(Os.fstat(descriptor).st_mode)) throw WalletFailure("Private storage path is not a directory.")
                Os.fsync(descriptor)
            } finally { Os.close(descriptor) }
        } catch (error: Exception) {
            if (stream != null) atomic.failWrite(stream)
            throw error
        }
    }
}

private class AndroidVaultKeys(private val context: Context) : VaultKeys {
    private fun store(): KeyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    override fun create(mode: AppLockMode): VaultEnrollment {
        if (mode == AppLockMode.DEVICE && !context.getSystemService(KeyguardManager::class.java).isDeviceSecure) {
            throw WalletFailure("To enable biometrics or device screen lock, first set a screen lock in Android Settings. App lock can remain off.")
        }
        val alias = "local-wallet-v2-${UUID.randomUUID()}"
        try {
            val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
            val specification = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setKeySize(256).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).setRandomizedEncryptionRequired(true)
                .setUserAuthenticationRequired(mode == AppLockMode.DEVICE)
            if (mode == AppLockMode.DEVICE) specification.setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL)
            generator.init(specification.build())
            val key = generator.generateKey()
            return VaultEnrollment(alias, Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key) }, mode)
        } catch (error: Exception) { runCatching { delete(alias) }; throw error }
    }

    override fun decrypt(manifest: VaultManifest): Cipher {
        val key = store().getKey(manifest.alias, null) as? SecretKey
            ?: throw WalletFailure("The device key is unavailable. Existing wallet files are preserved; your recovery words can restore the account on another installation.")
        return Cipher.getInstance("AES/GCM/NoPadding").apply {
            init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, manifest.iv)); updateAAD(manifest.aad())
        }
    }
    override fun delete(alias: String) { store().deleteEntry(alias) }
}
