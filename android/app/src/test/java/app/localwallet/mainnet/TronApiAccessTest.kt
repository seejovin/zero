package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test
import javax.crypto.KeyGenerator

/** JCA encryption/storage fault tests; these do not claim Android Keystore device coverage. */
class TronApiAccessTest {
    private class MemoryFile : TronCredentialFile {
        var bytes: ByteArray? = null
        var failWrite = false
        override fun exists() = bytes != null
        override fun read() = bytes!!.copyOf()
        override fun write(bytes: ByteArray) {
            if (failWrite) throw java.io.IOException("public test fault")
            this.bytes = bytes.copyOf()
        }
        override fun clear() { bytes = null }
    }
    private fun keys() = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
    private val publicKey = "public-fixture-api-key-not-a-real-credential"
    @Test fun credentialRoundTripUsesFreshAuthenticatedEncryptionAndCanBeCleared() {
        val file = MemoryFile()
        val key = keys()
        val access = TronApiAccess(file, TronCredentialKeys { key })
        assertNull(access.read())
        access.save(publicKey)
        assertEquals(publicKey, access.read())
        val first = file.bytes!!.copyOf()
        assertFalse(first.toString(Charsets.UTF_8).contains(publicKey))
        access.save(publicKey)
        assertFalse(first.contentEquals(file.bytes!!))
        access.clear()
        assertNull(access.read())
    }
    @Test fun tamperingOrMissingDeviceKeyNeverReturnsTheCredential() {
        val file = MemoryFile()
        val key = keys()
        val access = TronApiAccess(file, TronCredentialKeys { key })
        access.save(publicKey)
        val saved = file.bytes!!.copyOf()
        for (index in listOf(0, 4, 9, saved.lastIndex)) {
            file.bytes = saved.copyOf().also { it[index] = (it[index].toInt() xor 1).toByte() }
            try { access.read(); fail("Accepted a modified credential record") } catch (_: WalletFailure) { }
        }
        file.bytes = saved
        try { TronApiAccess(file, TronCredentialKeys { keys() }).read(); fail("Accepted a different wrapping key") } catch (_: WalletFailure) { }
        assertArrayEquals(saved, file.bytes)
    }
    @Test fun failedSavePreservesThePreviousCredential() {
        val file = MemoryFile()
        val key = keys()
        val access = TronApiAccess(file, TronCredentialKeys { key })
        access.save(publicKey)
        file.failWrite = true
        try { access.save("another-public-fixture-key"); fail("Ignored write failure") } catch (_: WalletFailure) { }
        assertEquals(publicKey, access.read())
    }
    @Test fun headerInjectionWhitespaceAndOversizedKeysAreRejectedBeforeStorage() {
        val file = MemoryFile()
        val access = TronApiAccess(file, TronCredentialKeys { keys() })
        for (value in listOf("", "short", " key-with-spaces ", "line\r\nInjected: yes", "x".repeat(257))) {
            try { access.save(value); fail("Accepted an invalid provider credential") } catch (_: WalletFailure) { }
            assertFalse(file.exists())
        }
    }
    @Test fun providerTransportRejectsAnUnreviewedEndpointBeforeOpeningAConnection() {
        val transport = HttpsTronTransport(publicKey)
        for (path in listOf("https://attacker.invalid/v1", "../wallet/getaccount", "v1/accounts/address/transactions?redirect=elsewhere")) {
            try { transport.get(path); fail("Accepted an arbitrary discovery endpoint") } catch (_: WalletFailure) { }
        }
        try { HttpsTronTransport("bad\r\nheader-value"); fail("Accepted header injection") } catch (_: WalletFailure) { }
    }
}
