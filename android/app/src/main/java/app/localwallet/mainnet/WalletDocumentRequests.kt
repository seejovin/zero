package app.localwallet.mainnet

import java.util.concurrent.atomic.AtomicInteger

internal enum class WalletDocumentKind { AUDIT_EXPORT, PRIVATE_EXPORT, PRIVATE_IMPORT }

/** Public request identities only; an OS picker result never grants access to another wallet. */
internal class WalletDocumentRequests {
    private data class Ticket(val owner: String, val kind: WalletDocumentKind)
    private val pending = mutableMapOf<Int, Ticket>()

    fun begin(owner: String, kind: WalletDocumentKind): Int {
        if (owner.isBlank() || pending.values.any { it.kind == kind }) {
            throw WalletFailure("Finish the current document selection before starting another.")
        }
        val code = nextCode.getAndIncrement()
        if (code !in 1_000..65_534) throw WalletFailure("Restart the app before selecting another document.")
        pending[code] = Ticket(owner, kind)
        return code
    }

    fun consume(code: Int, currentOwner: String?): WalletDocumentKind? =
        pending.remove(code)?.takeIf { it.owner == currentOwner }?.kind

    fun cancel(code: Int) { pending.remove(code) }
    fun revoke() { pending.clear() }

    companion object {
        // Never reuse a request code within a process, including across Activity recreation.
        private val nextCode = AtomicInteger(1_000)
    }
}
