package app.localwallet.mainnet

import android.content.Context
import android.system.Os
import android.system.OsConstants
import android.util.AtomicFile
import android.util.Base64
import org.json.JSONObject
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.nio.file.Files
import java.nio.file.LinkOption
import java.security.MessageDigest

/**
 * Opaque encrypted SDK cache, separate from the authoritative native transfer journal. JavaScript
 * must encrypt/authenticate the complete snapshot with its independent state key before writing.
 * This store never accepts a file path, mnemonic or a generic key/value database entry. A missing
 * cache triggers a canonical rescan; it never clears a native reservation or recovery quarantine.
 */
internal class PrivateSdkSnapshotStore(context: Context, privateWalletIdentity: String) : AutoCloseable {
    private data class Snapshot(val bytes: Long, val sha256: String)
    private class Upload(val id: String, val total: Long, val sha256: String, val output: FileOutputStream) {
        val digest: MessageDigest = MessageDigest.getInstance("SHA-256")
        var offset = 0L
    }
    private val directory: File
    private val file: File
    private val atomic: AtomicFile
    private var snapshot: Snapshot? = null
    private var upload: Upload? = null
    private var closed = false

    companion object {
        const val KEY = "private-engine-state-v1"
        const val MAX_BYTES = 67_108_864L
        const val MAX_CHUNK = 262_144
        private fun hash(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it.toInt() and 255) }
    }

    init {
        PrivatePaymentRelease.requireAvailable()
        PrivateIdentifiers.require(privateWalletIdentity)
        val root = context.noBackupFilesDir.canonicalFile
        directory = File(File(root, "private-payments-v1"), privateWalletIdentity)
        if (!Files.isDirectory(directory.toPath(), LinkOption.NOFOLLOW_LINKS) || Files.isSymbolicLink(directory.toPath()) ||
            !Files.isDirectory(directory.parentFile!!.toPath(), LinkOption.NOFOLLOW_LINKS)) fail()
        file = File(directory, "engine-state-v1.encrypted")
        atomic = AtomicFile(file)
        listOf(file, File(file.path + ".bak"), File(file.path + ".new")).forEach(::requireRegularOrMissing)
        if (file.exists() || File(file.path + ".bak").exists()) {
            snapshot = atomic.openRead().use { inspect(it) }
        }
    }

    @Synchronized fun read(request: JSONObject): JSONObject {
        requireOpen()
        PrivateEngineWire.fields(request, "key", "offset", "length", "sha256")
        requireKey(request)
        val offset = PrivateEngineWire.number(request, "offset", 0, MAX_BYTES)
        val count = PrivateEngineWire.number(request, "length", 1, MAX_CHUNK.toLong()).toInt()
        val expected = if (request.isNull("sha256")) null else PrivateEngineWire.string(request, "sha256").also(::requireHash)
        val current = snapshot ?: return JSONObject().put("exists", false).put("base64", JSONObject.NULL)
            .put("offset", offset).put("total_bytes", 0).put("sha256", JSONObject.NULL)
        if (expected != null && expected != current.sha256 || offset > current.bytes) fail()
        val length = minOf(count.toLong(), current.bytes - offset).toInt()
        requireRegularOrMissing(file)
        val bytes = FileInputStream(file).use { input ->
            if (input.channel.size() != current.bytes) fail()
            input.channel.position(offset)
            val result = ByteArray(length)
            var cursor = 0
            while (cursor < result.size) {
                val read = input.read(result, cursor, result.size - cursor)
                if (read <= 0) fail()
                cursor += read
            }
            result
        }
        return JSONObject().put("exists", true).put("base64", Base64.encodeToString(bytes, Base64.NO_WRAP))
            .put("offset", offset).put("total_bytes", current.bytes).put("sha256", current.sha256)
    }

    @Synchronized fun write(request: JSONObject): JSONObject {
        requireOpen()
        requireKey(request)
        return when (PrivateEngineWire.string(request, "action")) {
            "begin" -> {
                PrivateEngineWire.fields(request, "action", "key", "total_bytes", "sha256")
                if (upload != null) fail()
                val total = PrivateEngineWire.number(request, "total_bytes", 32, MAX_BYTES)
                val hash = PrivateEngineWire.string(request, "sha256").also(::requireHash)
                listOf(file, File(file.path + ".bak"), File(file.path + ".new")).forEach(::requireRegularOrMissing)
                val current = Upload(PrivateIdentifiers.create(), total, hash, atomic.startWrite())
                upload = current
                JSONObject().put("upload_id", current.id).put("offset", 0)
            }
            "write" -> {
                PrivateEngineWire.fields(request, "action", "key", "upload_id", "offset", "base64")
                val current = requireUpload(request)
                val offset = PrivateEngineWire.number(request, "offset", 0, MAX_BYTES)
                val encoded = PrivateEngineWire.string(request, "base64")
                if (offset != current.offset || encoded.length !in 4..349_528 || !Regex("[A-Za-z0-9+/]*={0,2}").matches(encoded)) fail()
                val bytes = Base64.decode(encoded, Base64.NO_WRAP)
                try {
                    if (bytes.size !in 1..MAX_CHUNK || bytes.size > current.total - current.offset ||
                        Base64.encodeToString(bytes, Base64.NO_WRAP) != encoded) fail()
                    try {
                        current.output.write(bytes)
                        current.digest.update(bytes)
                        current.offset += bytes.size
                    } catch (_: Exception) {
                        runCatching { atomic.failWrite(current.output) }
                        upload = null
                        fail()
                    }
                } finally { bytes.fill(0) }
                JSONObject().put("upload_id", current.id).put("offset", current.offset)
            }
            "commit" -> {
                PrivateEngineWire.fields(request, "action", "key", "upload_id")
                val current = requireUpload(request)
                if (current.offset != current.total || hash(current.digest.digest()) != current.sha256) {
                    atomic.failWrite(current.output); upload = null; fail()
                }
                // Never replace the previous complete snapshot with a partial or mismatched one.
                current.output.fd.sync()
                atomic.finishWrite(current.output)
                upload = null
                Os.chmod(file.absolutePath, 0x180)
                syncDirectory()
                val saved = atomic.openRead().use { inspect(it) }
                if (saved.bytes != current.total || saved.sha256 != current.sha256) fail()
                snapshot = saved
                JSONObject().put("committed", true).put("total_bytes", saved.bytes).put("sha256", saved.sha256)
            }
            "abort" -> {
                PrivateEngineWire.fields(request, "action", "key", "upload_id")
                val current = requireUpload(request)
                atomic.failWrite(current.output)
                upload = null
                JSONObject().put("aborted", true)
            }
            else -> fail()
        }
    }

    private fun requireUpload(request: JSONObject): Upload {
        val current = upload ?: fail()
        if (PrivateEngineWire.string(request, "upload_id") != current.id) fail()
        return current
    }
    private fun inspect(input: FileInputStream): Snapshot {
        val digest = MessageDigest.getInstance("SHA-256")
        val buffer = ByteArray(65_536)
        var total = 0L
        while (true) {
            val count = input.read(buffer)
            if (count < 0) break
            total += count
            if (total > MAX_BYTES) fail()
            digest.update(buffer, 0, count)
        }
        if (total < 32) fail()
        return Snapshot(total, hash(digest.digest()))
    }
    private fun requireRegularOrMissing(value: File) {
        if (Files.exists(value.toPath(), LinkOption.NOFOLLOW_LINKS) && !Files.isRegularFile(value.toPath(), LinkOption.NOFOLLOW_LINKS)) fail()
    }
    private fun requireHash(value: String) { if (!Regex("[0-9a-f]{64}").matches(value)) fail() }
    private fun requireKey(value: JSONObject) { if (PrivateEngineWire.string(value, "key") != KEY) fail() }
    private fun requireOpen() { if (closed) fail() }
    private fun syncDirectory() {
        val descriptor = Os.open(directory.absolutePath, OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW, 0)
        try {
            if (!OsConstants.S_ISDIR(Os.fstat(descriptor).st_mode)) fail()
            Os.fsync(descriptor)
        } finally { Os.close(descriptor) }
    }
    private fun fail(): Nothing = throw PrivatePaymentFailure(PrivateFailure.STORAGE)
    @Synchronized override fun close() {
        if (closed) return
        closed = true
        upload?.let { runCatching { atomic.failWrite(it.output) } }
        upload = null
    }
}
