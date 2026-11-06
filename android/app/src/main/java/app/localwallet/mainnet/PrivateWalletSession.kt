package app.localwallet.mainnet

import android.content.Context
import android.system.Os
import android.system.OsConstants
import android.util.AtomicFile
import org.json.JSONArray
import org.json.JSONObject
import uniffi.wallet_core.PrivatePaymentJournal
import uniffi.wallet_core.PrivateRecoveryBackup
import uniffi.wallet_core.privateCreateRecoveryJournal
import uniffi.wallet_core.privateAuthenticateJournal
import uniffi.wallet_core.privateEngineKeys
import uniffi.wallet_core.privateExportRecoveryBackup
import uniffi.wallet_core.privateImportRecoveryBackup
import uniffi.wallet_core.privateInspectRecoveryBackup
import uniffi.wallet_core.privateRecoveryStatus
import java.io.ByteArrayOutputStream
import java.io.File
import java.nio.file.Files
import java.nio.file.LinkOption
import java.security.MessageDigest
import java.util.Locale

internal data class PrivateWalletProfile(val publicWalletAddress: String, val identity: String, val storageSlot: String)

/** Device-local routing metadata, never uploaded and never used as evidence of recovery safety. */
internal class PrivateWalletCatalog(context: Context) {
    private val directory = File(context.noBackupFilesDir.canonicalFile, "private-payments-v1")

    init {
        PrivatePaymentRelease.requireAvailable()
        if (!Files.exists(directory.toPath(), LinkOption.NOFOLLOW_LINKS)) Files.createDirectory(directory.toPath())
        if (!Files.isDirectory(directory.toPath(), LinkOption.NOFOLLOW_LINKS)) fail()
        Os.chmod(directory.absolutePath, 0x1c0)
    }

    fun load(publicWalletAddress: String): PrivateWalletProfile? {
        val address = canonicalAddress(publicWalletAddress)
        read(profileFile(address))?.let { saved ->
            if (canonicalAddress(saved.publicWalletAddress) != address) fail()
            return saved
        }
        // Older candidate builds had one profile file. Only its exact owner may copy
        // it to the new namespace. Another wallet never overwrites or adopts it.
        val legacy = read(File(directory, "profile-v1.json")) ?: return null
        if (canonicalAddress(legacy.publicWalletAddress) != address) return null
        save(legacy)
        return legacy
    }

    /** Import calls this only after a new isolated journal has authenticated and committed. */
    fun save(profile: PrivateWalletProfile) {
        val canonical = profile.copy(publicWalletAddress = canonicalAddress(profile.publicWalletAddress))
        val file = profileFile(canonical.publicWalletAddress)
        checkLeaves(file)
        val atomic = AtomicFile(file)
        val bytes = JSONObject().put("schema_version", 1).put("public_wallet_address", canonical.publicWalletAddress)
            .put("identity", PrivateIdentifiers.require(canonical.identity)).put("storage_slot", PrivateIdentifiers.require(canonical.storageSlot))
            .toString().toByteArray(Charsets.UTF_8)
        val output = atomic.startWrite()
        try {
            output.write(bytes); output.fd.sync(); atomic.finishWrite(output)
            Os.chmod(file.absolutePath, 0x180)
            val fd = Os.open(directory.absolutePath, OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW, 0)
            try {
                if (!OsConstants.S_ISDIR(Os.fstat(fd).st_mode)) fail()
                Os.fsync(fd)
            } finally { Os.close(fd) }
            if (read(file) != canonical) fail()
        } catch (error: Exception) { runCatching { atomic.failWrite(output) }; throw error }
    }

    private fun profileFile(address: String): File {
        val input = ("LocalWallet/private-profile/v1\u0000" + canonicalAddress(address)).toByteArray(Charsets.UTF_8)
        val hash = MessageDigest.getInstance("SHA-256").digest(input).joinToString("") { "%02x".format(it.toInt() and 255) }
        return File(directory, "profile-$hash-v1.json")
    }

    private fun read(file: File): PrivateWalletProfile? {
        checkLeaves(file)
        if (!file.exists() && !File(file.path + ".bak").exists()) return null
        val raw = AtomicFile(file).openRead().use { input ->
            val out = ByteArrayOutputStream(); val chunk = ByteArray(1024)
            while (true) {
                val count = input.read(chunk)
                if (count < 0) break
                if (out.size() + count > 8_192) fail()
                out.write(chunk, 0, count)
            }
            out.toString("UTF-8")
        }
        val value = RpcJson.parse(raw, maxChars = 8_192, maxNodes = 20)
        PrivateEngineWire.fields(value, "schema_version", "public_wallet_address", "identity", "storage_slot")
        PrivateEngineWire.number(value, "schema_version", 1, 1)
        return PrivateWalletProfile(canonicalAddress(PrivateEngineWire.string(value, "public_wallet_address")),
            PrivateIdentifiers.require(PrivateEngineWire.string(value, "identity")),
            PrivateIdentifiers.require(PrivateEngineWire.string(value, "storage_slot")))
    }

    private fun canonicalAddress(address: String) = Amounts.address(address).lowercase(Locale.ROOT)
    private fun checkLeaves(file: File) {
        listOf(file, File(file.path + ".bak"), File(file.path + ".new")).forEach {
            if (Files.exists(it.toPath(), LinkOption.NOFOLLOW_LINKS) && !Files.isRegularFile(it.toPath(), LinkOption.NOFOLLOW_LINKS)) fail()
        }
    }
    private fun fail(): Nothing = throw PrivatePaymentFailure(PrivateFailure.STORAGE)
}

internal data class PrivateRecoveryState(val quarantined: Boolean, val incident: Boolean, val recoveryId: String?, val localJson: String) {
    override fun toString() = "PrivateRecoveryState(quarantined=$quarantined, incident=$incident)"
}

/** One local private wallet, deliberately distinct from the existing public wallet repository. */
internal class PrivateWalletSession private constructor(
    val profile: PrivateWalletProfile,
    val configuration: PrivateEngineConfiguration,
    val engine: PrivateWebLocalEngine,
    val journal: PrivateNativeJournal,
    val coordinator: PrivatePaymentCoordinator,
    val recoveryActions: PrivateRecoveryController,
    private val ports: PrivateEngineNativePorts
) : AutoCloseable {
    @Volatile var balance: PrivateScannedBalance? = null
        private set
    @Volatile var recovery: PrivateRecoveryState = recoveryState(journal)
        private set
    @Volatile var payments: List<PrivateLocalPayment> = emptyList()
        private set
    @Volatile private var localStateReady = false
    private val completedCancellationCleanup = HashSet<String>()
    val privateAddress: String get() = engine.address()
    val canSend: Boolean get() {
        val cursor = coordinator.snapshot()
        val completed = setOf(PrivatePaymentStage.FINALIZED, PrivatePaymentStage.CANCELLED, PrivatePaymentStage.SUPERSEDED)
        val unresolved = payments.any { payment ->
            payment.entry.phase !in setOf(PrivateJournalPhase.FINALIZED, PrivateJournalPhase.CANCELLED, PrivateJournalPhase.SUPERSEDED) &&
                !(payment.entry.intentId == cursor.intentId && cursor.stage in completed)
        }
        return localStateReady && !recovery.quarantined && !recovery.incident && balance?.scanComplete == true && !unresolved
    }

    companion object {
        /** Called under the existing wallet's app-lock authorization, on its serialized worker. */
        fun open(context: Context, manifest: VaultManifest, vaultKey: ByteArray, stillAuthorized: () -> Boolean): PrivateWalletSession {
            PrivateNativeRelease.requireQualified()
            val config = PrivateEngineConfiguration.reviewedRelease()
            val assets = PrivateEngineAssets.open(context, config.artifactManifestSha256)
            assets.verifyAll()
            if (!stillAuthorized()) throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED)
            val catalog = PrivateWalletCatalog(context)
            val saved = catalog.load(manifest.address)
            val profile = saved ?: PrivateIdentifiers.create().let { PrivateWalletProfile(Amounts.address(manifest.address).lowercase(Locale.ROOT), it, it) }
            val storage = PrivateJournalStorage(context, profile.identity, profile.storageSlot)
            val deviceKey = storage.loadOrCreateKey()
            val native = try {
                if (storage.journalFile.exists()) PrivatePaymentJournal(storage.journalFile.absolutePath, profile.identity, deviceKey)
                else privateCreateRecoveryJournal(storage.journalFile.absolutePath, profile.identity, deviceKey, manifest.vaultHex, vaultKey)
            } finally { deviceKey.fill(0) }
            val journal = PrivateNativeJournal(native)
            var nativePorts: PrivateEngineNativePorts? = null
            var runtime: PrivateEngineRuntime? = null
            try {
                // A partially created or manually removed journal must not turn into a new,
                // unquarantined namespace merely because a SQLite file now exists.
                val bound = RpcJson.parse(privateAuthenticateJournal(native, manifest.vaultHex, vaultKey), maxChars = 65_536)
                if (bound.isNull("wallet_binding")) throw PrivatePaymentFailure(PrivateFailure.STORAGE)
                PrivateIdentifiers.require(PrivateEngineWire.string(bound, "wallet_binding"))
                // Missing history always starts quarantined; this metadata record cannot unlock it.
                if (saved == null) catalog.save(profile)
                val network = PrivateObliviousSubmissionTransport(config.relay)
                nativePorts = PrivateEngineNativePorts(assets, PrivateSdkSnapshotStore(context, profile.storageSlot), network)
                runtime = PrivateWebViewEngineRuntime.open(context, assets, nativePorts)
                val fees = PrivateVerifiedFeeScheduleSource(config, network) { System.currentTimeMillis() / 1000 }
                val engine = PrivateWebLocalEngine(runtime, config, fees, { digest ->
                    network.advisoryStatus(digest)
                }) { System.currentTimeMillis() / 1000 }
                if (!stillAuthorized()) throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED)
                val keys = privateEngineKeys(manifest.vaultHex, vaultKey, profile.identity)
                try { engine.initialize(profile.identity, keys) } finally { keys.fill(0) }
                val records = journal.localPayments()
                engine.restoreReservations(journal.reservations(records))
                if (!stillAuthorized()) throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED)
                val coordinator = PrivatePaymentFeature.create(engine, journal, network) { System.currentTimeMillis() / 1000 }
                val recoveryActions = PrivateRecoveryController(engine, journal, network) { System.currentTimeMillis() / 1000 }
                return PrivateWalletSession(profile, config, engine, journal, coordinator, recoveryActions, nativePorts).also { session ->
                    session.payments = records
                    val recoveryWorkspace = recoveryActions.reload()
                    if (recoveryWorkspace.quarantined && !recoveryWorkspace.incident) {
                        recoveryWorkspace.batches.firstOrNull { it.phase !in setOf(PrivateJournalPhase.FINALIZED, PrivateJournalPhase.CANCELLED, PrivateJournalPhase.SUPERSEDED) }
                            ?.let { recoveryActions.resumeBatch(it.intentId) }
                    }
                    records.firstOrNull { it.entry.phase !in setOf(PrivateJournalPhase.FINALIZED, PrivateJournalPhase.CANCELLED, PrivateJournalPhase.SUPERSEDED) }
                        ?.let { coordinator.restore(it.entry.intentId) }
                    session.reloadLocalRecords()
                }
            } catch (error: Exception) {
                runtime?.close(); nativePorts?.close(); journal.close()
                throw error
            }
        }

        /** Restores into a new device namespace; previous local files are retained intact. */
        fun importBackup(context: Context, manifest: VaultManifest, vaultKey: ByteArray, encrypted: ByteArray): PrivateWalletProfile {
            PrivateNativeRelease.requireQualified()
            if (encrypted.size !in 76..73_400_320) throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
            val checkpoint = RpcJson.parse(privateInspectRecoveryBackup(encrypted, manifest.vaultHex, vaultKey), maxChars = 16_384)
            val identity = PrivateIdentifiers.require(PrivateEngineWire.string(checkpoint, "journal_identity"))
            val profile = PrivateWalletProfile(Amounts.address(manifest.address).lowercase(Locale.ROOT), identity, PrivateIdentifiers.create())
            val storage = PrivateJournalStorage(context, identity, profile.storageSlot)
            if (storage.journalFile.exists()) throw PrivatePaymentFailure(PrivateFailure.STORAGE)
            val deviceKey = storage.loadOrCreateKey()
            try {
                privateImportRecoveryBackup(encrypted, manifest.vaultHex, vaultKey, storage.journalFile.absolutePath, identity, deviceKey).use { native ->
                    val status = RpcJson.parse(privateAuthenticateJournal(native, manifest.vaultHex, vaultKey), maxChars = 65_536)
                    if (!PrivateEngineWire.boolean(status, "quarantined")) throw PrivatePaymentFailure(PrivateFailure.STORAGE)
                }
                PrivateWalletCatalog(context).save(profile)
                return profile
            } finally { deviceKey.fill(0) }
        }

        private fun recoveryState(journal: PrivateNativeJournal): PrivateRecoveryState {
            val json = privateRecoveryStatus(journal.recoveryHandle())
            val status = RpcJson.parse(json, maxChars = 65_536)
            val id = status.optionalString("recovery_id")?.let(PrivateIdentifiers::require)
            return PrivateRecoveryState(PrivateEngineWire.boolean(status, "quarantined"), PrivateEngineWire.boolean(status, "incident"), id, json)
        }
    }

    fun refresh(progress: (Int) -> Unit = {}): PrivateScannedBalance {
        reloadLocalRecords()
        return engine.synchronize(progress).also { balance = it }
    }
    fun reloadRecovery(): PrivateRecoveryState = recoveryState(journal).also { recovery = it }
    fun reloadLocalRecords(): List<PrivateLocalPayment> {
        localStateReady = false
        recovery = recoveryState(journal)
        recoveryActions.reload()
        val status = RpcJson.parse(recovery.localJson, maxChars = 65_536)
        val importedThrough = if (status.has("source_checkpoint") && !status.isNull("source_checkpoint"))
            PrivateEngineWire.number(status.getJSONObject("source_checkpoint"), "source_sequence", 0, Long.MAX_VALUE) else 0L
        return journal.localPayments().also { records ->
            PrivateCancellationCleanup.pending(records, importedThrough, completedCancellationCleanup).forEach { release ->
                engine.releaseCancelledReservations(release)
                // A newer live intent may still own some inputs from this old cancellation.
                // Retain partial cleanup for a later authoritative snapshot instead of forgetting it.
                if (release.completesCleanup) completedCancellationCleanup.add(release.payment.entry.intentId)
            }
            payments = records
            val id = coordinator.snapshot().intentId
            if (id != null && records.any { it.entry.intentId == id }) coordinator.restore(id)
            localStateReady = true
        }
    }
    fun selectPayment(intentId: String): PrivatePaymentState {
        PrivateIdentifiers.require(intentId)
        val state = coordinator.restore(intentId)
        reloadLocalRecords()
        return state
    }
    fun exportBackup(manifest: VaultManifest, vaultKey: ByteArray): PrivateRecoveryBackup {
        if (!manifest.address.equals(profile.publicWalletAddress, true)) throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
        return privateExportRecoveryBackup(journal.recoveryHandle(), manifest.vaultHex, vaultKey)
    }
    fun setForeground(value: Boolean) { coordinator.setForeground(value); recoveryActions.setForeground(value); engine.setForeground(value) }
    override fun close() { engine.close(); ports.close(); journal.close() }
}
