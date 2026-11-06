package app.localwallet.mainnet

import android.content.Context
import android.util.Base64
import android.webkit.WebResourceResponse
import org.json.JSONObject
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.security.MessageDigest

/** Hash-verified assets from the installed APK only. A manifest fetched beside a bundle is not trust. */
internal class PrivateEngineAssets private constructor(
    private val context: Context,
    private val entries: Map<String, Entry>
) {
    private data class Entry(val path: String, val mime: String, val sha256: String, val bytes: Long)
    private val verified = HashSet<String>()

    companion object {
        const val ORIGIN = "https://private.local-wallet.invalid"
        const val DOCUMENT = "$ORIGIN/index.html"
        private const val PREFIX = "private-engine/"
        private val allowedMime = setOf("text/html", "text/plain", "application/javascript", "application/wasm", "application/octet-stream", "application/json")
        const val CSP = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'none'; " +
            "worker-src blob:; child-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; " +
            "form-action 'none'; img-src 'none'; style-src 'none'; frame-ancestors 'none'"

        fun open(context: Context, trustedManifestSha256: String): PrivateEngineAssets {
            try {
                requireDigest(trustedManifestSha256)
                val bytes = context.assets.open(PREFIX + "manifest.json").use { readBounded(it, 131_072) }
                if (digest(bytes) != trustedManifestSha256) unavailable()
                val manifest = RpcJson.parse(bytes.toString(Charsets.UTF_8), maxChars = 131_072, maxNodes = 5_000)
                PrivateEngineWire.fields(manifest, "schema_version", "sdk_version", "engine_version", "entry", "files")
                PrivateEngineWire.number(manifest, "schema_version", 1, 1)
                if (PrivateEngineWire.string(manifest, "sdk_version") != PrivateEngineWire.SDK_VERSION ||
                    PrivateEngineWire.string(manifest, "engine_version") != PrivateEngineWire.ENGINE_VERSION ||
                    PrivateEngineWire.string(manifest, "entry") != "index.html") unavailable()
                val files = manifest.getJSONArray("files")
                if (files.length() !in 2..256) unavailable()
                val entries = LinkedHashMap<String, Entry>()
                var total = 0L
                for (index in 0 until files.length()) {
                    val item = files.getJSONObject(index)
                    PrivateEngineWire.fields(item, "path", "mime", "sha256", "bytes")
                    val path = PrivateEngineWire.string(item, "path")
                    val mime = PrivateEngineWire.string(item, "mime")
                    val hash = PrivateEngineWire.string(item, "sha256")
                    val length = PrivateEngineWire.number(item, "bytes", 1, 268_435_456)
                    if (!Regex("[a-zA-Z0-9][a-zA-Z0-9._/-]{0,180}").matches(path) ||
                        path.split('/').any { it.isEmpty() || it == "." || it == ".." } ||
                        path == "manifest.json" || mime !in allowedMime || (mime == "text/html" && path != "index.html")) unavailable()
                    requireDigest(hash)
                    if (entries.put(path, Entry(path, mime, hash, length)) != null) unavailable()
                    total += length
                    if (total > 536_870_912) unavailable()
                }
                if (entries["index.html"]?.mime != "text/html") unavailable()
                return PrivateEngineAssets(context.applicationContext, entries)
            } catch (_: Exception) { unavailable() }
        }

        private fun requireDigest(value: String) { if (!Regex("[0-9a-f]{64}").matches(value)) unavailable() }
        private fun digest(bytes: ByteArray): String = hex(MessageDigest.getInstance("SHA-256").digest(bytes))
        private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it.toInt() and 255) }
        private fun readBounded(input: InputStream, maximum: Int): ByteArray {
            val output = ByteArrayOutputStream()
            val buffer = ByteArray(8_192)
            while (true) {
                val count = input.read(buffer)
                if (count < 0) break
                if (output.size() + count > maximum) unavailable()
                output.write(buffer, 0, count)
            }
            return output.toByteArray()
        }
        private fun unavailable(): Nothing = throw PrivatePaymentFailure(PrivateFailure.UNAVAILABLE)
    }

    /** Worker thread only: verify all proving resources before supplying any recovery material. */
    fun verifyAll() { entries.values.forEach(::verify) }

    @Synchronized private fun verify(entry: Entry) {
        if (entry.path in verified) return
        val digest = MessageDigest.getInstance("SHA-256")
        var read = 0L
        context.assets.open(PREFIX + entry.path).use { input ->
            val buffer = ByteArray(65_536)
            while (true) {
                val count = input.read(buffer)
                if (count < 0) break
                read += count
                if (read > entry.bytes) unavailable()
                digest.update(buffer, 0, count)
            }
        }
        if (read != entry.bytes || hex(digest.digest()) != entry.sha256) unavailable()
        verified.add(entry.path)
    }

    fun document(): String {
        val entry = entries.getValue("index.html")
        verify(entry)
        if (entry.bytes > 131_072) unavailable()
        return context.assets.open(PREFIX + entry.path).use { readBounded(it, 131_072) }.toString(Charsets.UTF_8)
    }

    /** Requests outside this immutable package always receive an empty denial; never the network. */
    fun response(url: String): WebResourceResponse {
        if (!url.startsWith("$ORIGIN/") || url.contains('?') || url.contains('#') || url.contains('%')) return denied()
        val path = url.removePrefix("$ORIGIN/")
        val entry = entries[path] ?: return denied()
        return try {
            verify(entry)
            WebResourceResponse(entry.mime, if (entry.mime in setOf("text/html", "text/plain", "application/javascript", "application/json")) "UTF-8" else null,
                200, "OK", headers(), context.assets.open(PREFIX + entry.path))
        } catch (_: Exception) { denied() }
    }

    /** Artifact ports use bounded chunks; no arbitrary filesystem or asset names are accepted. */
    fun artifact(path: String, offset: Long, length: Int): JSONObject {
        val entry = entries[path] ?: unavailable()
        if (entry.mime !in setOf("application/wasm", "application/octet-stream", "application/json") ||
            offset < 0 || length !in 1..262_144 || offset > entry.bytes || length > entry.bytes - offset) unavailable()
        verify(entry)
        val bytes = context.assets.open(PREFIX + path).use { input ->
            var remaining = offset
            while (remaining > 0) {
                val skipped = input.skip(remaining)
                if (skipped <= 0) unavailable()
                remaining -= skipped
            }
            val out = ByteArray(length)
            var cursor = 0
            while (cursor < length) {
                val count = input.read(out, cursor, length - cursor)
                if (count <= 0) unavailable()
                cursor += count
            }
            out
        }
        return JSONObject().put("offset", offset).put("total_bytes", entry.bytes)
            .put("sha256", entry.sha256).put("base64", Base64.encodeToString(bytes, Base64.NO_WRAP))
    }

    private fun headers() = mapOf("Content-Security-Policy" to CSP, "Cache-Control" to "no-store", "X-Content-Type-Options" to "nosniff",
        "Cross-Origin-Opener-Policy" to "same-origin", "Cross-Origin-Embedder-Policy" to "require-corp", "Cross-Origin-Resource-Policy" to "same-origin",
        "Permissions-Policy" to "camera=(), microphone=(), geolocation=(), usb=(), payment=()")
    private fun denied() = WebResourceResponse("text/plain", "UTF-8", 403, "Forbidden", headers(), ByteArrayInputStream(byteArrayOf()))
}
