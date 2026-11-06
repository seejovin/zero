package app.localwallet.mainnet

import uniffi.wallet_core.WalletException
import java.io.File
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.attribute.BasicFileAttributes

/** Codes describe the failed step without exposing paths, addresses, or record contents. */
enum class LocalRecordStage(val code: String, val explanation: String) {
    JOURNAL_PATH("LOCAL_JOURNAL_PATH", "The private wallet journal folder could not be opened safely."),
    JOURNAL_OPEN("LOCAL_JOURNAL_OPEN", "This device could not open the wallet's local transaction journal."),
    BALANCE_READ("LOCAL_BALANCE_READ", "This device could not read or verify the saved balance record."),
    BALANCE_SAVE("LOCAL_BALANCE_SAVE", "Network balances were checked, but this device could not save their verified record."),
    HISTORY_READ("LOCAL_HISTORY_READ", "This device could not read or verify the saved transaction history."),
    ACTIVITY_READ("LOCAL_ACTIVITY_READ", "This device could not read or verify the saved activity records."),
    JOURNAL_IO("LOCAL_JOURNAL_IO", "This device could not read, verify, or save a local transaction record.")
}

class LocalRecordFailure(val stage: LocalRecordStage) : WalletFailure(
    "${stage.explanation} Close and reopen the app. If this repeats, share diagnostic code: ${stage.code}. Do not clear app data."
)

internal inline fun <T> localRecord(stage: LocalRecordStage, operation: () -> T): T = try {
    operation()
} catch (failure: LocalRecordFailure) {
    throw failure
} catch (_: Exception) {
    throw LocalRecordFailure(stage)
}

/**
 * Android may expose an app-private directory through a framework-owned parent
 * alias (for example, /data/user/0). SQLite NOFOLLOW rejects that alias too.
 * Resolve only the trusted parent, never the database filename. Reject a link
 * at the app-controlled wallet directory itself before resolving its parents.
 */
internal fun journalFile(directory: File, chainId: Long): File = localRecord(LocalRecordStage.JOURNAL_PATH) {
    require(chainId in NetworkPolicy.evmChainOrder)
    val attributes = Files.readAttributes(directory.toPath(), BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS)
    if (!attributes.isDirectory || attributes.isSymbolicLink) throw LocalRecordFailure(LocalRecordStage.JOURNAL_PATH)
    File(directory.canonicalFile, "journal-$chainId.sqlite3")
}

object WalletDiagnostics {
    fun isLocalRecordFailure(error: Throwable): Boolean = error is LocalRecordFailure || error is WalletException.Storage

    fun localRecordMessage(error: Throwable): String =
        if (error is LocalRecordFailure) error.message!! else LocalRecordFailure(LocalRecordStage.JOURNAL_IO).message!!

    fun refreshMessage(network: String, error: Throwable): String = when {
        isLocalRecordFailure(error) -> localRecordMessage(error)
        error is NetworkReadUnavailable || error is java.io.IOException ->
            "$network could not be reached. Check your connection and try again. Your saved balance has been retained. Diagnostic code: RPC_UNAVAILABLE."
        error is WalletFailure -> "${error.message} Diagnostic code: BALANCE_VERIFY."
        else -> "The $network balance response could not be verified. Try refreshing again. Diagnostic code: BALANCE_VERIFY."
    }
}
