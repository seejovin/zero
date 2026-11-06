package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.nio.file.Files
import javax.crypto.Cipher

/** Repository routing regression: no network, signing key access or real wallet is used. */
class PrivatePublicRouteGuardTest {
    private fun vault(directory: File): SecureVault {
        val files = object : VaultFile {
            override val directory: File = directory
            override fun exists(): Boolean = error("Recipient rejection must precede any vault access")
            override fun read(): ByteArray = error("No recovery or signing key access")
            override fun write(value: ByteArray) = error("No wallet mutation")
        }
        val keys = object : VaultKeys {
            override fun create(mode: AppLockMode): VaultEnrollment = error("No key creation")
            override fun decrypt(manifest: VaultManifest): Cipher = error("No signing")
            override fun delete(alias: String) = error("No deletion")
        }
        return SecureVault(files, keys, { 1_000L }) { error("No vault decoding") }
    }

    private fun manifest() = VaultManifest("0x1111111111111111111111111111111111111111", "unused", "unused",
        ByteArray(12), ByteArray(48), true)

    @Test fun directEvmPreviewMaxAndPrepareRejectPrivateDestinationsBeforeCreatingAnyRpcClient() {
        val directory = Files.createTempDirectory("private-public-route-fixture-").toFile()
        var clients = 0
        try {
            WalletRepository(vault(directory), manifest(), rpcFactory = {
                clients++
                error("Public RPC must not receive a private recipient")
            }, elapsedMillis = { 1_000L }).use { repository ->
                for (network in repository.networks) {
                    for (destination in listOf("0zkfixture-recipient", "railgun:fixture", "private:fixture", "aztec:fixture")) {
                        val preview = assertThrows(PrivatePaymentFailure::class.java) { repository.preview(network, "USDC", destination, "1") }
                        val maximum = assertThrows(PrivatePaymentFailure::class.java) { repository.maxTransfer(network, "USDC", destination) }
                        val prepared = assertThrows(PrivatePaymentFailure::class.java) { repository.prepare(network, "USDC", destination, "1") }
                        for (failure in listOf(preview, maximum, prepared)) {
                            assertEquals(PrivateFailure.PRIVATE_DESTINATION_IN_PUBLIC_FLOW, failure.failure)
                        }
                    }
                }
            }
            assertEquals(0, clients)
            assertTrue(directory.listFiles()!!.isEmpty())
        } finally { directory.deleteRecursively() }
    }

    @Test fun familyFacadeRejectsBeforeOpeningAnAdapterOrAccessingFamilyAddressesOrApiKeys() {
        val directory = Files.createTempDirectory("private-family-route-fixture-").toFile()
        var credentialReads = 0
        try {
            MultiNetworkRepository(vault(directory), manifest(), tronApiKey = {
                credentialReads++
                error("A private destination must not trigger public provider credential access")
            }).use { repository ->
                for (network in FamilyNetworks.all) {
                    val failure = assertThrows(PrivatePaymentFailure::class.java) {
                        repository.quoteFamily(network, "USDC", "0zkfixture-recipient", "1", false)
                    }
                    assertEquals(PrivateFailure.PRIVATE_DESTINATION_IN_PUBLIC_FLOW, failure.failure)
                }
            }
            assertEquals(0, credentialReads)
            assertTrue(directory.listFiles()!!.isEmpty())
        } finally { directory.deleteRecursively() }
    }
}
