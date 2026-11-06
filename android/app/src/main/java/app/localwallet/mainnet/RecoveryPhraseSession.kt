package app.localwallet.mainnet

/** Public identity only. A reveal belongs to one visible page in one unlocked wallet session. */
internal data class RecoveryPage(val address: String, val epoch: Int, val page: Long)
internal class RecoveryRevealRequest internal constructor(val context: RecoveryPage, val generation: Long)

/**
 * No words exist in this object until an explicit request succeeds. Results from a revoked
 * request are consumed and erased. UI/clipboard Strings are unavoidable platform copies;
 * the retained phrase and the verification challenge use erasable character buffers.
 */
internal class RecoveryPhraseSession(private val elapsedMillis: () -> Long = { System.nanoTime() / 1_000_000L }) {
    private var generation = 0L
    private var pending: RecoveryRevealRequest? = null
    private var requestExpiresAt = 0L
    private var foregroundConfirmationRequired = false
    private var confirmationExpiresAt: Long? = null
    private var phraseWords = emptyList<CharArray>()

    @get:Synchronized val isVisible: Boolean get() = phraseWords.isNotEmpty()
    @get:Synchronized val isPending: Boolean get() = pending != null
    @get:Synchronized val wordCount: Int get() = phraseWords.size

    @Synchronized fun request(context: RecoveryPage): RecoveryRevealRequest {
        conceal()
        requestExpiresAt = elapsedMillis() + 120_000L
        return RecoveryRevealRequest(context, generation).also { pending = it }
    }

    @Synchronized fun isCurrent(request: RecoveryRevealRequest, context: RecoveryPage, foreground: Boolean): Boolean =
        foreground && pending == request && request.generation == generation &&
            elapsedMillis() < requestExpiresAt &&
            request.context.epoch == context.epoch && request.context.page == context.page &&
            request.context.address.equals(context.address, ignoreCase = true)

    @Synchronized fun needsForegroundConfirmation(request: RecoveryRevealRequest): Boolean =
        pending == request && foregroundConfirmationRequired

    /** Called only after system authentication has returned, before any key is unwrapped. */
    @Synchronized fun beginForegroundConfirmation(request: RecoveryRevealRequest, context: RecoveryPage, foreground: Boolean): Boolean {
        if (!isCurrent(request, context, foreground) || !foregroundConfirmationRequired) {
            cancel(request)
            return false
        }
        if (confirmationExpiresAt == null) confirmationExpiresAt = elapsedMillis() + 30_000L
        if (elapsedMillis() >= confirmationExpiresAt!!) { cancel(request); return false }
        return true
    }

    /** A fresh user tap is required after an authentication-related Activity pause. */
    @Synchronized fun confirmForeground(request: RecoveryRevealRequest, context: RecoveryPage, foreground: Boolean): Boolean {
        val deadline = confirmationExpiresAt
        if (!isCurrent(request, context, foreground) || !foregroundConfirmationRequired || deadline == null || elapsedMillis() >= deadline) {
            cancel(request)
            return false
        }
        foregroundConfirmationRequired = false
        confirmationExpiresAt = null
        return true
    }

    @Synchronized fun accepts(request: RecoveryRevealRequest, context: RecoveryPage, foreground: Boolean): Boolean =
        isCurrent(request, context, foreground) && !foregroundConfirmationRequired

    /** Takes ownership of [phrase] and always clears it, including stale/invalid results. */
    @Synchronized fun reveal(request: RecoveryRevealRequest, context: RecoveryPage, foreground: Boolean, phrase: CharArray): Boolean {
        var parsed = emptyList<CharArray>()
        try {
            if (!accepts(request, context, foreground)) {
                // Rejecting this delivery consumes its authority, but cannot revoke a newer request.
                if (pending == request) conceal()
                return false
            }
            val starts = listOf(0) + phrase.indices.filter { phrase[it] == ' ' }.map { it + 1 }
            if (starts.size !in setOf(12, 15, 18, 21, 24) || phrase.any { it != ' ' && it !in 'a'..'z' }) {
                throw WalletFailure("The recovery phrase could not be displayed safely.")
            }
            parsed = starts.mapIndexed { index, start ->
                val end = if (index == starts.lastIndex) phrase.size else starts[index + 1] - 1
                phrase.copyOfRange(start, end)
            }
            if (parsed.any { it.isEmpty() }) throw WalletFailure("The recovery phrase could not be displayed safely.")
            phraseWords = parsed
            parsed = emptyList()
            pending = null
            requestExpiresAt = 0L
            return true
        } catch (error: Exception) {
            conceal()
            throw error
        } finally {
            parsed.forEach { it.fill('\u0000') }
            phrase.fill('\u0000')
        }
    }

    @Synchronized fun word(index: Int): String = phraseWords.getOrNull(index)?.concatToString()
        ?: throw WalletFailure("Reveal the recovery phrase first.")

    @Synchronized fun clipboardText(): String {
        if (!isVisible) throw WalletFailure("Reveal the recovery phrase first.")
        return phraseWords.joinToString(" ") { it.concatToString() }
    }

    /** Transfers only three requested words to verification, then erases the complete phrase. */
    @Synchronized fun challenge(positions: List<Int>): RecoveryChallenge {
        if (!isVisible || positions.size != 3 || positions.distinct().size != 3 || positions.any { it !in phraseWords.indices }) {
            throw WalletFailure("Reveal the recovery phrase before checking your backup.")
        }
        val expected = positions.sorted().associateWith { phraseWords[it].copyOf() }
        conceal()
        return RecoveryChallenge(expected)
    }

    @Synchronized fun cancel(request: RecoveryRevealRequest) {
        if (pending == request) conceal()
    }

    /**
     * A system prompt may pause the Activity before any words are decrypted. Preserve only the
     * request, require another explicit foreground tap, and revoke an already deferred operation
     * on any subsequent pause. authPending alone must never authorize a later automatic reveal.
     */
    @Synchronized fun pause(systemAuthenticationPending: Boolean = false) {
        phraseWords.forEach { it.fill('\u0000') }
        phraseWords = emptyList()
        if (systemAuthenticationPending && pending != null && confirmationExpiresAt == null) {
            foregroundConfirmationRequired = true
        } else {
            generation++
            pending = null
            requestExpiresAt = 0L
            foregroundConfirmationRequired = false
            confirmationExpiresAt = null
        }
    }

    @Synchronized fun conceal() = pause()
}

internal class RecoveryChallenge internal constructor(private var expected: Map<Int, CharArray>) {
    val positions: List<Int> get() = expected.keys.sorted()
    val retainedWordCount: Int get() = expected.size

    /** Consumes input buffers even when verification fails. No phrase is captured by a UI callback. */
    fun matches(answers: Map<Int, CharArray>): Boolean = try {
        expected.size == 3 && answers.keys == expected.keys && expected.all { (index, value) ->
            answers[index]?.contentEquals(value) == true
        }
    } finally { answers.values.forEach { it.fill('\u0000') } }

    fun clear() {
        expected.values.forEach { it.fill('\u0000') }
        expected = emptyMap()
    }
}

/**
 * Deferred system-auth capability: owns only the unexecuted operation closure. It never runs
 * automatically, and releases both closures on confirmation/cancellation. Android supplies a
 * closure that first closes the confirmation UI and then unwraps the key on its worker.
 */
internal class RecoveryRevealContinuation(
    private val session: RecoveryPhraseSession,
    private val request: RecoveryRevealRequest,
    context: RecoveryPage,
    foreground: Boolean,
    proceed: () -> Unit,
    cancel: () -> Unit
) {
    private var continuation: (() -> Unit)? = proceed
    private var cancellation: (() -> Unit)? = cancel
    init {
        if (!session.beginForegroundConfirmation(request, context, foreground)) this.cancel()
    }
    @get:Synchronized val isPending: Boolean get() = continuation != null

    @Synchronized fun confirm(context: RecoveryPage, foreground: Boolean): Boolean {
        if (continuation == null) return false
        if (!session.confirmForeground(request, context, foreground)) { cancel(); return false }
        val operation = continuation
        continuation = null; cancellation = null
        operation?.invoke()
        return true
    }

    @Synchronized fun cancel() {
        if (continuation == null && cancellation == null) return
        val abandon = cancellation
        continuation = null; cancellation = null
        session.cancel(request)
        abandon?.invoke()
    }
}
