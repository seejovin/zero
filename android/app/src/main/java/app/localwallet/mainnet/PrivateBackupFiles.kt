package app.localwallet.mainnet

import android.content.Context
import android.net.Uri
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.security.MessageDigest

/** User-selected encrypted backup files only. No company endpoint or automatic upload exists. */
internal object PrivateBackupFiles {
    const val MAX_BYTES = 73_400_320

    fun writeAndVerify(context: Context, destination: Uri, encrypted: ByteArray) {
        if (encrypted.size !in 76..MAX_BYTES) fail()
        context.contentResolver.openOutputStream(destination, "wt")?.use { output ->
            output.write(encrypted)
            output.flush()
        } ?: fail()
        // A picker returning a URI is not evidence that the backup was saved. Read the complete
        // chosen document back and verify its exact ciphertext before reporting export success.
        context.contentResolver.openInputStream(destination)?.use { verifyCopy(it, encrypted) } ?: fail()
    }

    fun read(input: InputStream): ByteArray {
        val output = ByteArrayOutputStream()
        val buffer = ByteArray(65_536)
        try {
            while (true) {
                val count = input.read(buffer)
                if (count < 0) break
                if (count == 0) fail()
                if (output.size() + count > MAX_BYTES) fail()
                output.write(buffer, 0, count)
            }
            if (output.size() < 76) fail()
            return output.toByteArray()
        } finally { buffer.fill(0) }
    }

    internal fun verifyCopy(input: InputStream, expected: ByteArray) {
        val digest = MessageDigest.getInstance("SHA-256")
        val expectedHash = MessageDigest.getInstance("SHA-256").digest(expected)
        val buffer = ByteArray(65_536)
        var count = 0
        try {
            while (true) {
                val read = input.read(buffer)
                if (read < 0) break
                if (read == 0 || count + read > expected.size) fail()
                digest.update(buffer, 0, read)
                count += read
            }
            if (count != expected.size || !MessageDigest.isEqual(digest.digest(), expectedHash)) fail()
        } finally { expectedHash.fill(0); buffer.fill(0) }
    }
    private fun fail(): Nothing = throw PrivatePaymentFailure(PrivateFailure.STORAGE)
}
