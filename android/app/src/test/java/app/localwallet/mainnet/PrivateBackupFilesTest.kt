package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test
import java.io.ByteArrayInputStream

class PrivateBackupFilesTest {
    private val ciphertextFixture = ByteArray(170_001) { (it % 251).toByte() }

    @Test fun exportReadbackRequiresExactCiphertextAcrossChunkBoundaries() {
        PrivateBackupFiles.verifyCopy(ByteArrayInputStream(ciphertextFixture), ciphertextFixture)
    }
    @Test fun truncatedOrExtendedDocumentsCannotBeReportedAsSaved() {
        assertThrows(PrivatePaymentFailure::class.java) { PrivateBackupFiles.verifyCopy(ByteArrayInputStream(ciphertextFixture.copyOf(ciphertextFixture.size - 1)), ciphertextFixture) }
        assertThrows(PrivatePaymentFailure::class.java) { PrivateBackupFiles.verifyCopy(ByteArrayInputStream(ciphertextFixture + byteArrayOf(1)), ciphertextFixture) }
    }
    @Test fun changedCiphertextCannotBeReportedAsSaved() {
        val modified = ciphertextFixture.copyOf().apply { this[80_000] = (this[80_000].toInt() xor 1).toByte() }
        assertThrows(PrivatePaymentFailure::class.java) { PrivateBackupFiles.verifyCopy(ByteArrayInputStream(modified), ciphertextFixture) }
    }
    @Test fun importBoundsDoNotAcceptTruncatedHeaders() {
        assertThrows(PrivatePaymentFailure::class.java) { PrivateBackupFiles.read(ByteArrayInputStream(ByteArray(75))) }
        assertArrayEquals(ciphertextFixture, PrivateBackupFiles.read(ByteArrayInputStream(ciphertextFixture)))
    }
}
