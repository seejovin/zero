package app.localwallet.mainnet

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.system.Os
import android.system.OsConstants
import android.util.AtomicFile
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.FileOutputStream
import java.nio.file.Files
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

internal interface TronCredentialFile {
    fun exists(): Boolean
    fun read(): ByteArray
    fun write(bytes: ByteArray)
    fun clear()
}
internal fun interface TronCredentialKeys { fun key(create: Boolean): SecretKey }

/**
 * Optional provider access credential, separate from every wallet record/key.
 * This key controls TronGrid quota only; it cannot authorize a transfer.
 * The Android wrapping key never leaves Android Keystore. Nothing is backed up.
 */
class TronApiAccess internal constructor(private val file: TronCredentialFile, private val keys: TronCredentialKeys) {
    constructor(context: Context) : this(AndroidTronCredentialFile(context.applicationContext), AndroidTronCredentialKeys())

    companion object {
        private val lock = Any()
        private val header = byteArrayOf(0x54, 0x41, 0x50, 0x49, 0x01) // TAPI / v1
        private val aad = "local-wallet/provider-credential/v1/https://api.trongrid.io".toByteArray(Charsets.UTF_8)
        internal const val MAX_BYTES = 512
        fun validate(value: String): String {
            if (value.length !in 8..256 || !value.all { it.code in 33..126 }) {
                throw WalletFailure("Enter a complete TronGrid API key without spaces or line breaks.")
            }
            return value
        }
    }
    fun read(): String? = synchronized(lock) {
        if (!file.exists()) return@synchronized null
        try {
            val bytes = file.read()
            if (bytes.size !in (header.size + 12 + 16 + 8)..MAX_BYTES || !bytes.copyOfRange(0, header.size).contentEquals(header)) {
                throw WalletFailure("The TRON connection setting could not be verified. Enter the API key again in Settings.")
            }
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, keys.key(false), GCMParameterSpec(128, bytes.copyOfRange(header.size, header.size + 12)))
            cipher.updateAAD(aad)
            cipher.updateAAD(header)
            val plaintext = cipher.doFinal(bytes.copyOfRange(header.size + 12, bytes.size))
            try { validate(plaintext.toString(Charsets.UTF_8)) } finally { plaintext.fill(0) }
        } catch (failure: WalletFailure) { throw failure }
        catch (_: Exception) { throw WalletFailure("The TRON connection setting could not be decrypted. Enter the API key again in Settings.") }
    }
    fun save(value: String?) = synchronized(lock) {
        if (value == null) { clear(); return@synchronized }
        val plaintext = validate(value).toByteArray(Charsets.UTF_8)
        try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, keys.key(true))
            if (cipher.iv.size != 12) throw WalletFailure("The device could not create the TRON connection setting.")
            cipher.updateAAD(aad)
            cipher.updateAAD(header)
            val ciphertext = cipher.doFinal(plaintext)
            file.write(header + cipher.iv + ciphertext)
            if (read() != value) throw WalletFailure("The TRON connection setting could not be verified after saving.")
        } catch (failure: WalletFailure) { throw failure }
        catch (_: Exception) { throw WalletFailure("The TRON connection setting could not be saved. Reopen Settings and check its status.") }
        finally { plaintext.fill(0) }
    }
    fun clear() = synchronized(lock) {
        try {
            file.clear()
            if (file.exists()) throw WalletFailure("The TRON connection setting could not be removed.")
        } catch (failure: WalletFailure) { throw failure }
        catch (_: Exception) { throw WalletFailure("The TRON connection setting could not be removed.") }
    }
}

private class AndroidTronCredentialKeys : TronCredentialKeys {
    companion object { private const val ALIAS = "local-wallet-provider-trongrid-v1" }
    override fun key(create: Boolean): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        if (store.containsAlias(ALIAS)) {
            return store.getKey(ALIAS, null) as? SecretKey
                ?: throw WalletFailure("The TRON provider setting's device key is unavailable.")
        }
        if (!create) throw WalletFailure("The TRON provider setting's device key is missing. Enter the API key again in Settings.")
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setKeySize(256).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setRandomizedEncryptionRequired(true).setUserAuthenticationRequired(false).build())
        }.generateKey()
    }
}

private class AndroidTronCredentialFile(context: Context) : TronCredentialFile {
    private val directory = File(context.noBackupFilesDir, "provider-connections-v1").apply {
        if (Files.isSymbolicLink(toPath()) || (exists() && !isDirectory)) throw WalletFailure("Private connection settings cannot be opened safely.")
        if (!exists() && !mkdir()) throw WalletFailure("Private connection settings cannot be created.")
        Os.chmod(absolutePath, 448)
    }
    private val base = File(directory, "trongrid-v1.bin")
    private val atomic = AtomicFile(base)
    private fun paths(): List<File> = listOf(base, File(base.path + ".bak"), File(base.path + ".new"))
    private fun checkPaths() {
        if (Files.isSymbolicLink(directory.toPath()) || !directory.isDirectory || paths().any { Files.isSymbolicLink(it.toPath()) || (it.exists() && !it.isFile) }) {
            throw WalletFailure("Private connection settings cannot be opened safely.")
        }
    }
    override fun exists(): Boolean { checkPaths(); return paths().any { it.exists() } }
    override fun read(): ByteArray {
        checkPaths()
        return atomic.openRead().use { stream ->
            val output = ByteArrayOutputStream()
            val buffer = ByteArray(256)
            while (true) {
                val count = stream.read(buffer)
                if (count < 0) break
                if (output.size() + count > TronApiAccess.MAX_BYTES) throw WalletFailure("The TRON connection setting exceeds its size limit.")
                output.write(buffer, 0, count)
            }
            output.toByteArray()
        }
    }
    private fun syncDirectory() {
        val fd = Os.open(directory.absolutePath, OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW, 0)
        try {
            if (!OsConstants.S_ISDIR(Os.fstat(fd).st_mode)) throw WalletFailure("Private connection storage is not a directory.")
            Os.fsync(fd)
        } finally { Os.close(fd) }
    }
    override fun write(bytes: ByteArray) {
        checkPaths()
        if (bytes.size > TronApiAccess.MAX_BYTES) throw WalletFailure("The TRON connection setting exceeds its size limit.")
        var stream: FileOutputStream? = null
        try {
            stream = atomic.startWrite()
            stream.write(bytes)
            stream.fd.sync()
            atomic.finishWrite(stream)
            stream = null
            Os.chmod(base.absolutePath, 384)
            syncDirectory()
        } catch (failure: Exception) {
            if (stream != null) atomic.failWrite(stream)
            throw failure
        }
    }
    override fun clear() {
        checkPaths()
        atomic.delete()
        syncDirectory()
    }
}
