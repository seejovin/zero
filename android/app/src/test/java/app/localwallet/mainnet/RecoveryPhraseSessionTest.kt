package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test

class RecoveryPhraseSessionTest {
    // Public deterministic fixture, never a user's recovery phrase.
    private val fixture = listOf("abandon", "ability", "able", "about", "above", "absent", "absorb", "abstract",
        "absurd", "abuse", "access", "accident", "account", "accuse", "achieve", "acid", "acoustic", "acquire",
        "across", "act", "action", "actor", "actress", "actual")
    private val page = RecoveryPage("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266", 4, 12)
    private fun phrase() = fixture.joinToString(" ").toCharArray()
    private fun assertErased(value: CharArray) = assertTrue(value.all { it == '\u0000' })
    private fun reveal(session: RecoveryPhraseSession): RecoveryRevealRequest {
        val request = session.request(page)
        val value = phrase()
        assertTrue(session.reveal(request, page, true, value))
        assertErased(value)
        return request
    }

    @Test fun creationAndReopenedBackupBeginConcealedAndUnrequestedDataIsRejected() {
        repeat(2) {
            val session = RecoveryPhraseSession()
            assertFalse(session.isVisible)
            assertFalse(session.isPending)
            assertEquals(0, session.wordCount)
            assertThrows(WalletFailure::class.java) { session.word(0) }
            assertThrows(WalletFailure::class.java) { session.clipboardText() }
            val unsolicited = phrase()
            assertFalse(session.reveal(RecoveryRevealRequest(page, 0), page, true, unsolicited))
            assertErased(unsolicited)
        }
    }

    @Test fun explicitRevealConsumesAuthorizationAndErasesInputBeforeWordsCanBeRead() {
        val session = RecoveryPhraseSession()
        val request = reveal(session)
        assertTrue(session.isVisible)
        assertFalse(session.isPending)
        assertEquals(24, session.wordCount)
        assertEquals(fixture[23], session.word(23))
        assertEquals(fixture.joinToString(" "), session.clipboardText())
        val duplicate = phrase()
        assertFalse(session.reveal(request, page, true, duplicate))
        assertErased(duplicate)
        assertTrue(session.isVisible)
    }

    @Test fun wrongWalletEpochOrPageCannotReceiveDecryptedWords() {
        for (changed in listOf(page.copy(address = "0x1111111111111111111111111111111111111111"),
            page.copy(epoch = 5), page.copy(page = 13))) {
            val session = RecoveryPhraseSession()
            val request = session.request(page)
            val value = phrase()
            assertFalse(session.reveal(request, changed, true, value))
            assertFalse(session.isVisible)
            assertFalse(session.isPending)
            assertErased(value)
            val replay = phrase()
            assertFalse(session.reveal(request, page, true, replay))
            assertErased(replay)
        }
    }

    @Test fun backgroundDeliveryIsConsumedAndCannotRevealOnLaterResume() {
        val session = RecoveryPhraseSession()
        val request = session.request(page)
        val value = phrase()
        assertFalse(session.reveal(request, page, false, value))
        assertErased(value)
        assertFalse(session.isPending)
        val replay = phrase()
        assertFalse(session.reveal(request, page, true, replay))
        assertErased(replay)
    }

    @Test fun hideOrOrdinaryPauseRevokesAnInFlightDecryptionWithoutOnStop() {
        for (hide in listOf<(RecoveryPhraseSession) -> Unit>({ it.conceal() }, { it.pause() })) {
            val session = RecoveryPhraseSession()
            val request = session.request(page)
            hide(session)
            val completed = phrase()
            assertFalse(session.reveal(request, page, true, completed))
            assertErased(completed)
            assertFalse(session.isVisible)
            reveal(session)
            hide(session)
            assertEquals(0, session.wordCount)
            assertThrows(WalletFailure::class.java) { session.clipboardText() }
        }
    }

    @Test fun aSystemPromptMayResumeOnlyTheSamePendingRequestAndNeverPreservesVisibleWords() {
        val session = RecoveryPhraseSession()
        val request = session.request(page)
        session.pause(systemAuthenticationPending = true)
        assertTrue(session.isPending)
        assertFalse(session.accepts(request, page, foreground = false))
        assertFalse(session.accepts(request, page, foreground = true))
        assertTrue(session.needsForegroundConfirmation(request))
        assertTrue(session.beginForegroundConfirmation(request, page, true))
        assertTrue(session.confirmForeground(request, page, true))
        val value = phrase()
        assertTrue(session.reveal(request, page.copy(address = page.address.lowercase()), true, value))
        assertErased(value)
        session.pause(systemAuthenticationPending = true)
        assertFalse(session.isVisible)
        assertFalse(session.isPending)
        assertEquals(0, session.wordCount)

        val cancelled = session.request(page)
        session.pause(systemAuthenticationPending = true)
        session.cancel(cancelled)
        val late = phrase()
        assertFalse(session.reveal(cancelled, page, true, late))
        assertErased(late)
    }

    @Test fun pausedAuthenticationDefersActualOperationUntilOneFreshForegroundConfirmation() {
        val session = RecoveryPhraseSession()
        val request = session.request(page)
        session.pause(systemAuthenticationPending = true)
        var decryptions = 0
        var cancellations = 0
        val deferred = RecoveryRevealContinuation(session, request, page, true, {
            // This is the exact continuation invoked by MainActivity's pre-decrypt gate.
            assertTrue(session.accepts(request, page, true))
            decryptions++
        }, { cancellations++ })
        assertTrue(deferred.isPending)
        assertEquals(0, decryptions)
        assertFalse(session.isVisible)
        assertTrue(deferred.confirm(page, true))
        assertEquals(1, decryptions)
        assertFalse(deferred.isPending)
        assertFalse(deferred.confirm(page, true))
        deferred.cancel()
        assertEquals(1, decryptions)
        assertEquals(0, cancellations)
    }

    @Test fun deferredOperationCannotRunAfterAnotherPauseCancelWrongContextOrExpiry() {
        for (reason in listOf("pause", "pause-again", "cancel", "page", "wallet", "epoch", "expiry", "background")) {
            var elapsed = 1_000L
            val session = RecoveryPhraseSession { elapsed }
            val request = session.request(page)
            session.pause(systemAuthenticationPending = true)
            var decryptions = 0
            var cancellations = 0
            val deferred = RecoveryRevealContinuation(session, request, page, true, { decryptions++ }, { cancellations++ })
            var current = page
            when (reason) {
                "pause" -> session.pause()
                "pause-again" -> session.pause(systemAuthenticationPending = true)
                "cancel" -> deferred.cancel()
                "page" -> current = page.copy(page = 13)
                "wallet" -> current = page.copy(address = "0x1111111111111111111111111111111111111111")
                "epoch" -> current = page.copy(epoch = 5)
                "expiry" -> elapsed += 30_000L
            }
            assertFalse(reason, deferred.confirm(current, foreground = reason != "background"))
            assertFalse(deferred.isPending)
            assertEquals(reason, 0, decryptions)
            assertEquals(reason, 1, cancellations)
            assertFalse(session.isPending)
            assertFalse(deferred.confirm(page, true))
        }
    }

    @Test fun invalidDeferredCapabilityAndOldCancellationCannotConsumeNewerAuthorization() {
        var elapsed = 0L
        val session = RecoveryPhraseSession { elapsed }
        val old = session.request(page)
        session.pause(systemAuthenticationPending = true)
        var oldDecryptions = 0
        val oldDeferred = RecoveryRevealContinuation(session, old, page, true, { oldDecryptions++ }, {})
        session.conceal()
        val currentPage = page.copy(page = 13)
        val current = session.request(currentPage)
        session.pause(systemAuthenticationPending = true)
        var currentDecryptions = 0
        val actual = RecoveryRevealContinuation(session, current, currentPage, true, { currentDecryptions++ }, {})
        assertFalse(oldDeferred.confirm(page, true))
        assertTrue(actual.confirm(currentPage, true))
        assertEquals(0, oldDecryptions)
        assertEquals(1, currentDecryptions)

        val expired = session.request(page)
        session.pause(systemAuthenticationPending = true)
        elapsed += 120_000L
        var expiredDecryptions = 0
        val invalid = RecoveryRevealContinuation(session, expired, page, true, { expiredDecryptions++ }, {})
        assertFalse(invalid.isPending)
        assertFalse(invalid.confirm(page, true))
        assertEquals(0, expiredDecryptions)
    }

    @Test fun staleSuccessAndCancellationCannotRevokeANewerRevealRequest() {
        val session = RecoveryPhraseSession()
        val old = session.request(page)
        session.conceal()
        val current = session.request(page.copy(page = 13))
        session.cancel(old)
        val late = phrase()
        assertFalse(session.reveal(old, page.copy(page = 13), true, late))
        assertErased(late)
        assertTrue(session.accepts(current, page.copy(page = 13), true))
        val actual = phrase()
        assertTrue(session.reveal(current, page.copy(page = 13), true, actual))
        assertErased(actual)
    }

    @Test fun publicContextAndGenerationCannotSubstituteForTheIssuedRequestInstance() {
        val session = RecoveryPhraseSession()
        val issued = session.request(page)
        val reconstructed = RecoveryRevealRequest(issued.context, issued.generation)
        val attempt = phrase()
        assertFalse(session.reveal(reconstructed, page, true, attempt))
        assertErased(attempt)
        assertTrue(session.accepts(issued, page, true))
        val authorized = phrase()
        assertTrue(session.reveal(issued, page, true, authorized))
        assertErased(authorized)
    }

    @Test fun verificationRetainsOnlyThreeRequestedWordsAndNoCompletePhrase() {
        val session = RecoveryPhraseSession()
        reveal(session)
        val challenge = session.challenge(listOf(23, 0, 7))
        assertEquals(listOf(0, 7, 23), challenge.positions)
        assertEquals(3, challenge.retainedWordCount)
        assertEquals(0, session.wordCount)
        assertFalse(session.isVisible)
        assertThrows(WalletFailure::class.java) { session.clipboardText() }
        assertThrows(WalletFailure::class.java) { session.word(7) }
        val answers = listOf(0, 7, 23).associateWith { fixture[it].toCharArray() }
        assertTrue(challenge.matches(answers))
        answers.values.forEach(::assertErased)
        val incorrect = listOf(0, 7, 23).associateWith { "wrong".toCharArray() }
        assertFalse(challenge.matches(incorrect))
        incorrect.values.forEach(::assertErased)
        challenge.clear()
        assertEquals(0, challenge.retainedWordCount)
        val afterClear = listOf(0, 7, 23).associateWith { fixture[it].toCharArray() }
        assertFalse(challenge.matches(afterClear))
        afterClear.values.forEach(::assertErased)
    }

    @Test fun clearingChallengeErasesRetainedBuffersAndMalformedResultsAreErased() {
        val retained = listOf(0, 7, 23).associateWith { fixture[it].toCharArray() }
        RecoveryChallenge(retained).clear()
        retained.values.forEach(::assertErased)
        val session = RecoveryPhraseSession()
        val request = session.request(page)
        val malformed = "not a recovery phrase".toCharArray()
        assertThrows(WalletFailure::class.java) { session.reveal(request, page, true, malformed) }
        assertErased(malformed)
        assertFalse(session.isVisible)
        assertFalse(session.isPending)
    }
}
