package app.localwallet.mainnet

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.system.Os
import android.system.OsConstants
import android.util.AtomicFile
import android.util.Base64
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.InputStream
import java.nio.file.Files
import java.nio.file.LinkOption
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Independent, random journal key wrapped by Android Keystore. This namespace never opens the
 * public wallet vault, public transaction journal or mnemonic. The device-local private catalog
 * persists its random 32-byte identity; public addresses are not used as journal storage identities.
 * No call site is active before private release review. Device/Keystore verification is outstanding.
 */
internal class PrivateJournalStorage(context: Context, private val identity: String, private val storageSlot: String = identity) {
    private val directory: File
    private val keyFile: File
    val journalFile: File
    private val alias: String
    private val aad: ByteArray

    init {
        PrivatePaymentRelease.requireAvailable()
        try {
            if (!Regex("0x[0-9a-f]{64}").matches(identity) || identity.drop(2).all { it == '0' }) {
                throw PrivatePaymentFailure(PrivateFailure.STORAGE)
            }
            PrivateIdentifiers.require(storageSlot)
            val root = context.noBackupFilesDir.canonicalFile
            val namespace = File(root, "private-payments-v1")
            secureDirectory(namespace)
            directory = File(namespace, storageSlot)
            secureDirectory(directory)
            keyFile = File(directory, "journal-key.json")
            journalFile = File(directory, "payments.sqlite3")
            alias = "app.localwallet.private-journal-v1.$storageSlot"
            aad = ("LocalWallet/private-journal-key/v1\u0000" + identity + "\u0000" + storageSlot).toByteArray(Charsets.UTF_8)
        } catch (_: Exception) { throw PrivatePaymentFailure(PrivateFailure.STORAGE) }
    }

    fun loadOrCreateKey(): ByteArray = try {
        PrivatePaymentRelease.requireAvailable()
        rejectUnexpectedFiles()
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        val atomic = AtomicFile(keyFile)
        val hasRecord = keyFile.exists() || File(keyFile.path + ".bak").exists()
        if (hasRecord) {
            val key = store.getKey(alias, null) as? SecretKey ?: throw PrivatePaymentFailure(PrivateFailure.STORAGE)
            val bytes = atomic.openRead().use(::readKeyRecord)
            val text = try { String(bytes, Charsets.UTF_8) } finally { bytes.fill(0) }
            val record = RpcJson.parse(text, maxChars = 8_192, maxNodes = 20)
            if (record.keys().asSequence().toSet() != setOf("schema_version", "identity", "iv", "ciphertext") ||
                record.getInt("schema_version") != 1 || record.getString("identity") != identity) {
                throw PrivatePaymentFailure(PrivateFailure.STORAGE)
            }
            val iv = Base64.decode(record.getString("iv"), Base64.NO_WRAP)
            val encrypted = Base64.decode(record.getString("ciphertext"), Base64.NO_WRAP)
            try {
                if (iv.size != 12 || encrypted.size != 48) throw PrivatePaymentFailure(PrivateFailure.STORAGE)
                Cipher.getInstance("AES/GCM/NoPadding").run {
                    init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, iv))
                    updateAAD(aad)
                    doFinal(encrypted).also { if (it.size != 32) { it.fill(0); throw PrivatePaymentFailure(PrivateFailure.STORAGE) } }
                }
            } finally { iv.fill(0); encrypted.fill(0) }
        } else {
            // A partial enrollment or missing wrapping record must never regenerate the key for
            // an existing encrypted journal. Preserve all bytes and stop for controlled recovery.
            if (store.containsAlias(alias) || journalFile.exists() || File(journalFile.path + "-wal").exists() ||
                File(journalFile.path + "-shm").exists() || File(keyFile.path + ".new").exists()) {
                throw PrivatePaymentFailure(PrivateFailure.STORAGE)
            }
            val wrappingKey = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
                init(KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                    .setKeySize(256).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .setRandomizedEncryptionRequired(true).build())
                generateKey()
            }
            val privateKey = ByteArray(32).also { SecureRandom().nextBytes(it) }
            try {
                val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply {
                    init(Cipher.ENCRYPT_MODE, wrappingKey); updateAAD(aad)
                }
                val wrapped = cipher.doFinal(privateKey)
                val record = JSONObject().put("schema_version", 1).put("identity", identity)
                    .put("iv", Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
                    .put("ciphertext", Base64.encodeToString(wrapped, Base64.NO_WRAP)).toString().toByteArray(Charsets.UTF_8)
                wrapped.fill(0)
                var output: java.io.FileOutputStream? = null
                try {
                    output = atomic.startWrite()
                    output.write(record)
                    output.fd.sync()
                    atomic.finishWrite(output)
                    output = null
                    Os.chmod(keyFile.absolutePath, 0x180) // 0600
                    syncDirectory(directory)
                    val saved = atomic.openRead().use(::readKeyRecord)
                    try {
                        if (!saved.contentEquals(record)) throw PrivatePaymentFailure(PrivateFailure.STORAGE)
                    } finally { saved.fill(0) }
                } catch (error: Exception) {
                    if (output != null) atomic.failWrite(output)
                    throw error
                } finally { record.fill(0) }
                privateKey.copyOf()
            } finally { privateKey.fill(0) }
        }
    } catch (_: Exception) { throw PrivatePaymentFailure(PrivateFailure.STORAGE) }

    private fun readKeyRecord(input: InputStream): ByteArray {
        val bounded = ByteArrayOutputStream()
        val buffer = ByteArray(1024)
        try {
            while (true) {
                val count = input.read(buffer)
                if (count < 0) break
                if (bounded.size() + count > 8_192) throw PrivatePaymentFailure(PrivateFailure.STORAGE)
                bounded.write(buffer, 0, count)
            }
            return bounded.toByteArray()
        } finally { buffer.fill(0) }
    }

    private fun secureDirectory(value: File) {
        val path = value.toPath()
        if (Files.exists(path, LinkOption.NOFOLLOW_LINKS)) {
            if (!Files.isDirectory(path, LinkOption.NOFOLLOW_LINKS)) throw PrivatePaymentFailure(PrivateFailure.STORAGE)
        } else Files.createDirectory(path)
        if (Files.isSymbolicLink(path)) throw PrivatePaymentFailure(PrivateFailure.STORAGE)
        Os.chmod(value.absolutePath, 0x1c0) // 0700
        syncDirectory(value)
        syncDirectory(value.parentFile!!)
    }

    private fun syncDirectory(value: File) {
        val descriptor = Os.open(value.absolutePath, OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW, 0)
        try {
            if (!OsConstants.S_ISDIR(Os.fstat(descriptor).st_mode)) throw PrivatePaymentFailure(PrivateFailure.STORAGE)
            Os.fsync(descriptor)
        } finally { Os.close(descriptor) }
    }

    private fun rejectUnexpectedFiles() {
        listOf(keyFile, File(keyFile.path + ".bak"), File(keyFile.path + ".new"), journalFile,
            File(journalFile.path + "-wal"), File(journalFile.path + "-shm")).forEach { value ->
            val path = value.toPath()
            if (Files.exists(path, LinkOption.NOFOLLOW_LINKS) && !Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) {
                throw PrivatePaymentFailure(PrivateFailure.STORAGE)
            }
        }
    }
}
