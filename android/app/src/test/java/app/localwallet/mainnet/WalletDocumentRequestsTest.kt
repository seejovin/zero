package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test

class WalletDocumentRequestsTest {
    @Test fun normalPickerPauseKeepsOwnerButResultIsConsumedOnce() {
        val requests = WalletDocumentRequests()
        val request = requests.begin("wallet-a", WalletDocumentKind.PRIVATE_IMPORT)
        assertEquals(WalletDocumentKind.PRIVATE_IMPORT, requests.consume(request, "wallet-a"))
        assertNull(requests.consume(request, "wallet-a"))
    }

    @Test fun switchingWalletRevokesEveryKindOfOutstandingDocumentResult() {
        for (kind in WalletDocumentKind.entries) {
            val requests = WalletDocumentRequests()
            val request = requests.begin("wallet-a", kind)
            requests.revoke()
            assertNull(requests.consume(request, "wallet-b"))
            assertNull(requests.consume(request, "wallet-a"))
        }
    }

    @Test fun lateResultFromAIsRejectedEvenAfterSwitchingBackToA() {
        val requests = WalletDocumentRequests()
        val old = requests.begin("wallet-a", WalletDocumentKind.PRIVATE_IMPORT)
        requests.revoke()
        val fresh = requests.begin("wallet-a", WalletDocumentKind.PRIVATE_IMPORT)
        assertNotEquals(old, fresh)
        assertNull(requests.consume(old, "wallet-a"))
        assertEquals(WalletDocumentKind.PRIVATE_IMPORT, requests.consume(fresh, "wallet-a"))
    }

    @Test fun recreationDoesNotReuseCodesOrAcceptAnotherInstancesResult() {
        val old = WalletDocumentRequests().begin("wallet-a", WalletDocumentKind.AUDIT_EXPORT)
        val recreated = WalletDocumentRequests()
        val fresh = recreated.begin("wallet-a", WalletDocumentKind.AUDIT_EXPORT)
        assertNotEquals(old, fresh)
        assertNull(recreated.consume(old, "wallet-a"))
        assertNull(recreated.consume(fresh, null))
    }

    @Test fun mismatchedOwnerAndDuplicateLaunchCannotGrantAuthority() {
        val requests = WalletDocumentRequests()
        val request = requests.begin("wallet-a", WalletDocumentKind.PRIVATE_EXPORT)
        assertThrows(WalletFailure::class.java) { requests.begin("wallet-a", WalletDocumentKind.PRIVATE_EXPORT) }
        assertNull(requests.consume(request, "wallet-b"))
        assertNull(requests.consume(request, "wallet-a"))
    }
}
