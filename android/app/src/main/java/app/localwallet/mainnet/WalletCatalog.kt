package app.localwallet.mainnet

import android.content.Context
import android.system.Os
import android.system.OsConstants
import android.util.AtomicFile
import org.json.JSONObject
import java.io.File
import java.nio.file.Files
import java.nio.file.LinkOption
import java.util.UUID

// AtomicFile is not thread-safe across instances. One reentrant process-wide lock
// covers catalog discovery/selection and every vault read or mutation, including
// callbacks that validate a new address before its durable first write.
internal object WalletStorageSynchronization { val lock = Any() }
internal inline fun <T> walletStorage(block: () -> T): T = synchronized(WalletStorageSynchronization.lock, block)

data class StoredWallet(val id: String, val manifest: VaultManifest?, val error: String? = null)
data class NewWallet(val id: String, val vault: SecureVault)

internal interface WalletCatalogStorage {
    fun ids(): List<String>
    fun reserve(id: String): Boolean
    fun readSelection(): ByteArray?
    fun writeSelection(bytes: ByteArray)
}

/** A selection is only a local session preference, never ownership or wallet deletion. */
class WalletCatalog internal constructor(
    private val storage: WalletCatalogStorage,
    private val openVault: (String) -> SecureVault,
    private val nextId: () -> String = { UUID.randomUUID().toString() },
    private val decode: (String) -> JSONObject = RpcJson::parse
) {
    constructor(context: Context) : this(
        AndroidWalletCatalogStorage(context.applicationContext),
        { id -> SecureVault(context.applicationContext, id) }
    )

    companion object {
        const val LEGACY_ID = "legacy"
        private val uuid = Regex("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
        internal fun requireId(id: String) {
            if (id != LEGACY_ID && !uuid.matches(id)) throw WalletFailure("Invalid saved wallet identifier.")
        }
    }

    fun list(): List<StoredWallet> = walletStorage {
        val ids = storage.ids()
        if (ids.distinct().size != ids.size) throw WalletFailure("Saved wallet identifiers are inconsistent. Wallet data has been preserved.")
        ids.forEach(::requireId)
        ids.sortedWith(compareBy<String> { it != LEGACY_ID }.thenBy { it }).map { id ->
            try { StoredWallet(id, openVault(id).load()) }
            catch (_: Exception) { StoredWallet(id, null, "This saved wallet could not be opened. Its data has been preserved.") }
        }
    }

    /** Missing pre-upgrade selection adopts the old wallet once. Null is durable sign-out. */
    fun selectedId(): String? = walletStorage {
        val saved = storage.readSelection()
        if (saved != null) return@walletStorage parseSelection(saved)
        val ids = storage.ids()
        // Losing a later selector must not silently reopen the old wallet.
        val legacy = LEGACY_ID.takeIf { ids == listOf(LEGACY_ID) }
        persistSelection(legacy)
        legacy
    }

    fun vault(id: String): SecureVault = walletStorage {
        requireId(id)
        openVault(id)
    }

    /** Reserve storage without changing the current selection or creating wallet secrets. */
    fun allocate(): NewWallet = walletStorage {
        repeat(32) {
            val id = nextId()
            requireId(id)
            if (id == LEGACY_ID) throw WalletFailure("New wallets cannot replace legacy storage.")
            if (storage.reserve(id)) return@walletStorage NewWallet(id, openVault(id))
        }
        throw WalletFailure("A new wallet storage location could not be reserved. Existing wallets are unchanged.")
    }

    fun select(id: String, stillAuthorized: () -> Boolean = { true }) = walletStorage {
        requireId(id)
        if (id !in storage.ids()) throw WalletFailure("The selected wallet is not saved on this device.")
        openVault(id).load()
        persistSelection(id, stillAuthorized)
    }

    fun signOut(stillAuthorized: () -> Boolean = { true }) = walletStorage { persistSelection(null, stillAuthorized) }

    /** Called by SecureVault before its first write, inside the shared storage lock. */
    fun requireUniqueAddress(address: String, excludingId: String? = null) = walletStorage {
        if (!Regex("0x[0-9a-fA-F]{40}").matches(address)) throw WalletFailure("The new wallet address is invalid.")
        excludingId?.let(::requireId)
        val saved = list().filter { it.id != excludingId }
        if (saved.any { it.manifest == null }) {
            throw WalletFailure("A saved wallet cannot be checked for duplicates. Its data has been preserved; reopen it before adding another wallet.")
        }
        if (saved.any { it.manifest!!.address.equals(address, ignoreCase = true) }) {
            throw WalletFailure("This wallet is already saved on this device. Select it from your wallets instead of importing it again.")
        }
    }

    private fun parseSelection(bytes: ByteArray): String? {
        try {
            if (bytes.isEmpty() || bytes.size > 4_096) throw WalletFailure("Saved wallet selection is invalid. Wallet data has been preserved.")
            val value = decode(bytes.toString(Charsets.UTF_8))
            val version = value.get("version")
            if (value.keys().asSequence().toSet() != setOf("version", "active_wallet") || version !is Number || version.toString() != "1") {
                throw WalletFailure("Saved wallet selection is invalid. Wallet data has been preserved.")
            }
            val selected = value.get("active_wallet")
            if (selected == JSONObject.NULL) return null
            if (selected !is String) throw WalletFailure("Saved wallet selection is invalid. Wallet data has been preserved.")
            requireId(selected)
            return selected
        } catch (failure: WalletFailure) { throw failure }
        catch (_: Exception) { throw WalletFailure("Saved wallet selection is invalid. Wallet data has been preserved.") }
    }

    private fun persistSelection(id: String?, stillAuthorized: () -> Boolean = { true }) {
        id?.let(::requireId)
        val bytes = JSONObject().put("version", 1).put("active_wallet", id ?: JSONObject.NULL)
            .toString().toByteArray(Charsets.UTF_8)
        // UI authority owns lifecycle/intents. Check it only after acquiring the
        // same lock that protects the write, so queued old Activities cannot
        // replace a newer selection after waiting for another vault operation.
        if (!stillAuthorized()) throw WalletFailure("Wallet selection was interrupted. Reopen your wallets to continue.")
        storage.writeSelection(bytes)
        val saved = storage.readSelection() ?: throw WalletFailure("Wallet selection could not be verified after saving.")
        if (parseSelection(saved) != id) throw WalletFailure("Wallet selection changed while saving. Reopen your wallets.")
    }
}

/** Shared path checks keep old data in place and prohibit caller-selected paths. */
internal object WalletStoragePaths {
    fun exists(file: File): Boolean = Files.exists(file.toPath(), LinkOption.NOFOLLOW_LINKS)

    fun requireRegularOrMissing(file: File) {
        if (exists(file) && !Files.isRegularFile(file.toPath(), LinkOption.NOFOLLOW_LINKS)) {
            throw WalletFailure("Private wallet storage contains an unexpected file. Existing data has been preserved.")
        }
    }

    fun ensureDirectory(file: File): Unit = walletStorage {
        val created = !exists(file) && file.mkdir()
        if (!created && !exists(file)) throw WalletFailure("Private wallet storage cannot be created.")
        if (!Files.isDirectory(file.toPath(), LinkOption.NOFOLLOW_LINKS)) {
            throw WalletFailure("Private wallet storage path is not a directory.")
        }
        Os.chmod(file.absolutePath, 448)
        if (created) file.parentFile?.let(::syncDirectory)
    }

    fun directory(root: File, id: String): File = walletStorage {
        WalletCatalog.requireId(id)
        ensureDirectory(root)
        val parent = if (id == WalletCatalog.LEGACY_ID) root else File(root, "wallets-v1").also(::ensureDirectory)
        File(parent, if (id == WalletCatalog.LEGACY_ID) "wallet-v1" else id).also(::ensureDirectory)
    }

    fun syncDirectory(directory: File) {
        val descriptor = Os.open(directory.absolutePath, OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW, 0)
        try {
            if (!OsConstants.S_ISDIR(Os.fstat(descriptor).st_mode)) throw WalletFailure("Private wallet storage path is not a directory.")
            Os.fsync(descriptor)
        } finally { Os.close(descriptor) }
    }
}

private class AndroidWalletCatalogStorage(context: Context) : WalletCatalogStorage {
    private val root = context.noBackupFilesDir.also(WalletStoragePaths::ensureDirectory)
    private val wallets = File(root, "wallets-v1").also(WalletStoragePaths::ensureDirectory)
    private val atomic = AtomicFile(File(wallets, "selection.json"))
    private fun selectionPaths() = listOf(atomic.baseFile, File(atomic.baseFile.path + ".bak"), File(atomic.baseFile.path + ".new"))

    private fun containsVault(directory: File): Boolean {
        if (!WalletStoragePaths.exists(directory)) return false
        if (!Files.isDirectory(directory.toPath(), LinkOption.NOFOLLOW_LINKS)) {
            throw WalletFailure("Private wallet storage contains an unexpected directory.")
        }
        return listOf("vault.json", "vault.json.bak", "vault.json.new")
            .any { WalletStoragePaths.exists(File(directory, it)) }
    }

    override fun ids(): List<String> {
        val result = ArrayList<String>()
        if (containsVault(File(root, "wallet-v1"))) result += WalletCatalog.LEGACY_ID
        val entries = wallets.listFiles() ?: throw WalletFailure("Saved wallets could not be listed. Their files have been preserved.")
        for (entry in entries) {
            if (entry.name in setOf("selection.json", "selection.json.bak", "selection.json.new")) {
                WalletStoragePaths.requireRegularOrMissing(entry)
                continue
            }
            WalletCatalog.requireId(entry.name)
            if (entry.name == WalletCatalog.LEGACY_ID) throw WalletFailure("Unexpected legacy wallet location.")
            if (containsVault(entry)) result += entry.name
        }
        return result
    }

    override fun reserve(id: String): Boolean {
        WalletCatalog.requireId(id)
        if (id == WalletCatalog.LEGACY_ID) throw WalletFailure("Legacy wallet storage cannot be allocated.")
        val directory = File(wallets, id)
        if (WalletStoragePaths.exists(directory)) return false
        if (!directory.mkdir()) {
            if (WalletStoragePaths.exists(directory)) return false
            throw WalletFailure("New wallet storage could not be created.")
        }
        WalletStoragePaths.ensureDirectory(directory)
        WalletStoragePaths.syncDirectory(wallets)
        return true
    }

    override fun readSelection(): ByteArray? {
        selectionPaths().forEach(WalletStoragePaths::requireRegularOrMissing)
        if (selectionPaths().none(WalletStoragePaths::exists)) return null
        return try {
            atomic.openRead().use { stream ->
                val bytes = java.io.ByteArrayOutputStream()
                val buffer = ByteArray(512)
                while (true) {
                    val count = stream.read(buffer)
                    if (count == -1) break
                    if (bytes.size() + count > 4_096) throw WalletFailure("Saved wallet selection exceeds its size limit.")
                    bytes.write(buffer, 0, count)
                }
                bytes.toByteArray()
            }
        } catch (error: WalletFailure) { throw error }
        catch (_: Exception) { throw WalletFailure("Saved wallet selection could not be read. Wallet data has been preserved.") }
    }

    override fun writeSelection(bytes: ByteArray) {
        if (bytes.size > 4_096) throw WalletFailure("Saved wallet selection exceeds its size limit.")
        selectionPaths().forEach(WalletStoragePaths::requireRegularOrMissing)
        var stream: java.io.FileOutputStream? = null
        try {
            stream = atomic.startWrite()
            stream.write(bytes)
            stream.fd.sync()
            atomic.finishWrite(stream)
            stream = null
            Os.chmod(atomic.baseFile.absolutePath, 384)
            WalletStoragePaths.syncDirectory(wallets)
        } catch (error: Exception) {
            if (stream != null) atomic.failWrite(stream)
            throw error
        }
    }
}
