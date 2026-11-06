package app.localwallet.mainnet

import android.app.Activity
import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Intent
import android.content.res.ColorStateList
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.os.PersistableBundle
import android.text.InputType
import android.text.InputFilter
import android.text.Editable
import android.text.TextWatcher
import android.text.TextUtils
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.GridLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.MultiFormatWriter
import org.json.JSONObject
import uniffi.wallet_core.WalletException
import uniffi.wallet_core.revealMnemonic
import java.math.BigInteger
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import javax.crypto.Cipher

/** Native UI. The separate private engine uses only hash-verified bundled JS/WASM; no remote code. */
class MainActivity : Activity() {
    private val work = Executors.newSingleThreadExecutor()
    private val priceWork = Executors.newSingleThreadExecutor()
    private val withdrawalQuoteWork = Executors.newSingleThreadExecutor()
    private val handler = Handler(Looper.getMainLooper())
    private val background = Color.rgb(245, 243, 238)
    private val surface = Color.WHITE
    private val elevated = Color.rgb(238, 241, 233)
    private val line = Color.rgb(223, 228, 219)
    private val ink = Color.rgb(43, 57, 49)
    private val muted = Color.rgb(104, 115, 108)
    private val blue = Color.rgb(63, 110, 23)
    private val warning = Color.rgb(138, 86, 28)
    private var balancesHidden = false
    private var detailAsset = "USDC"
    private var assetPickerReturn = "home"
    private var assetPickerAction = "deposit"
    private var resumeRoute: WalletResumeRoute? = null
    private var createdMessagePending = false
    private lateinit var page: LinearLayout
    private lateinit var status: TextView
    private var pageScroll: ScrollView? = null
    private var pageShell: LinearLayout? = null
    private var withdrawalFlow: WithdrawalFlow? = null
    private var withdrawalDebounce: Runnable? = null
    private var withdrawalExpiry: Runnable? = null
    private var withdrawalQuoteRunning = false
    private var pendingWithdrawalQuote: (() -> Unit)? = null
    private var visibleDataUpdate: (() -> Unit)? = null
    private lateinit var contentHost: FrameLayout
    private var contentPage: View? = null
    private var openingTransition: ZeroOpeningTransition? = null
    private val displayNameStore by lazy { WalletDisplayNames(DisplayNamePreferences(this)) }
    private val portfolioPreferences by lazy { WalletPortfolioPreferences(PortfolioPreferences(this)) }
    private val priceStore by lazy { CoinbasePriceStore(applicationContext) }
    private var walletName = "Wallet 1"
    private var portfolioSettings = PortfolioDisplaySettings()
    private var usdQuotes: Map<String, UsdQuote> = emptyMap()
    private var priceRefreshing = false
    private var lastPriceRefreshElapsed = 0L
    private var pageSequence = 0L
    @Volatile private var financialViewGeneration = 0L
    private var preparingReview = false
    private val recoverySession = RecoveryPhraseSession { android.os.SystemClock.elapsedRealtime() }
    private var recoveryContent: LinearLayout? = null
    private var recoveryTimeout: Runnable? = null
    private var recoveryConfirmation: AlertDialog? = null
    private lateinit var vault: SecureVault
    private lateinit var walletCatalog: WalletCatalog
    private var activeWalletId: String? = null
    private var savedWallets: List<WalletChoice> = emptyList()
    // Keeps a failed durable sign-out from reopening the old wallet in this Activity.
    private var requireSignedOut = false
    private var walletTransitionPending = false
    private var selectionClaim = 0L
    private val selectionAuthority = WalletSelectionAuthority.process
    private val documentRequests = WalletDocumentRequests()
    private data class WalletChoice(val saved: StoredWallet, val name: String)
    private data class OpenedWallet(val id: String, val vault: SecureVault, val manifest: VaultManifest,
                                    val repository: MultiNetworkRepository, val name: String,
                                    val settings: PortfolioDisplaySettings)
    private data class WalletStartup(val choices: List<WalletChoice>, val opened: OpenedWallet?)
    private data class RetiredWallet(val repository: MultiNetworkRepository?, val privateWorkspace: PrivateWalletWorkspace?) {
        fun close() { try { privateWorkspace?.close() } finally { repository?.close() } }
    }
    private var repository: MultiNetworkRepository? = null
    private var manifest: VaultManifest? = null
    private var familyReview: FamilyReview? = null
    private var selectedChain = 8453L
    // Action flows never inherit selectedChain. Null means the user must choose a network.
    private var transferChain: Long? = null
    private var networkDropdown: WithdrawalSheet? = null
    private var withdrawalDetails: WithdrawalSheet? = null
    private var selectedAsset = "USDC"
    private var unlocked = false
    @Volatile private var busy = false
    @Volatile private var resumed = false
    @Volatile private var epoch = 0
    private var screen = "welcome"
    private var authPending = false
    private var authCancel: CancellationSignal? = null
    private var pinDialog: AlertDialog? = null
    private var sensitivePaused = false
    private var copiedPhraseToken: String? = null
    private var pendingAuthorizedAction: (() -> Unit)? = null
    private val sensitiveViews = mutableListOf<TextView>()
    private var review: ReviewedPayment? = null
    private var reviewEditBlocked = false
    private var reviewControlsUpdate: (() -> Unit)? = null
    private val balanceCache = HashMap<Long, BalanceView>()
    private val activityCache = HashMap<Long, ActivitySnapshot>()
    private val historyCache = HashMap<Long, List<JSONObject>>()
    private val syncState = HashMap<Long, String>()
    private val syncMessage = HashMap<Long, String>()
    private var refreshing = false
    private var sendRecipient = ""
    private var sendAmount = ""
    private var transferDrafts = TransferDrafts()
    private val unboundSendRecipients = HashMap<String, String>()
    private var receiveAsset = "USDC"
    private var exportText: String? = null
    private var privateWorkspace: PrivateWalletWorkspace? = null
    private var privateWorkspaceOwner: String? = null
    private var privateWorkspaceSelection: Long? = null
    private var privateResumePending = false
    private var pendingPrivateBackup: uniffi.wallet_core.PrivateRecoveryBackup? = null
    private var pendingPrivateImport: ByteArray? = null
    private val autoRefresh = object : Runnable {
        override fun run() {
            if (isDestroyed) return
            if (resumed && unlocked && !busy && screen in setOf("home", "history", "receive", "asset", "asset_picker")) {
                if (!refreshing) refresh()
                refreshPrices()
            }
            handler.postDelayed(this, 30_000)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.setDecorFitsSystemWindows(false)
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        if (Build.VERSION.SDK_INT >= 33) setRecentsScreenshotEnabled(false)
        if (Build.VERSION.SDK_INT >= 31) window.setHideOverlayWindows(true)
        selectionClaim = selectionAuthority.claimActivity(savedInstanceState?.getBoolean("wallet.session.signed_out", false) ?: false)
        requireSignedOut = selectionAuthority.requiresSignOut()
        resumeRoute = savedInstanceState?.let { saved ->
            runCatching { WalletNavigation.decode(WalletNavigation.stateKeys.associateWith { saved.getString(it) }) }.getOrNull()
        }
        renderOpening(animate = savedInstanceState == null)
        initialize()
        handler.postDelayed(autoRefresh, 30_000)
    }

    override fun onResume() {
        super.onResume()
        resumed = true
        if (!selectionAuthority.isCurrent(selectionClaim)) {
            // Only an actual foreground lifecycle event may take ownership back from another
            // Activity. A delayed callback must never reclaim a newer wallet selection.
            val retired = retireWallet()
            work.execute { retired.close() }
            selectionClaim = selectionAuthority.claimActivity(false)
            requireSignedOut = selectionAuthority.requiresSignOut()
        }
        val authorized = pendingAuthorizedAction
        pendingAuthorizedAction = null
        if (authorized != null) handler.post(authorized)
        else if (WalletAccessPolicy.mayInitialize(busy, authPending, walletTransitionPending) && ::walletCatalog.isInitialized) {
            sensitivePaused = false
            initialize()
        } else if (sensitivePaused && !busy && !authPending) {
            sensitivePaused = false
            renderSignedOut()
        }
    }

    override fun onPause() {
        openingTransition?.finishImmediately()
        if (screen == "private_wallet") privateResumePending = true
        privateWorkspace?.setForeground(false)
        rememberResumeRoute()
        dismissNetworkDropdown()
        invalidateWithdrawalForm()
        resumed = false
        financialViewGeneration++
        sensitivePaused = sensitiveViews.isNotEmpty()
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        clearSensitiveViews(systemAuthenticationPending = authPending && authCancel != null)
        if (authCancel == null) pinDialog?.cancel()
        super.onPause()
    }

    override fun onStop() {
        unlocked = false
        if (!authPending) {
            epoch++
            review = null
            if (!walletTransitionPending) {
                if (activeWalletId != null && manifest != null) renderLocked() else renderSignedOut()
            }
        }
        super.onStop()
    }

    override fun onDestroy() {
        epoch++
        financialViewGeneration++
        openingTransition?.finishImmediately()
        handler.removeCallbacks(autoRefresh)
        authCancel?.cancel()
        pinDialog?.cancel()
        pendingAuthorizedAction = null
        clearSensitiveViews()
        pendingPrivateBackup?.encryptedBytes?.fill(0); pendingPrivateBackup = null
        pendingPrivateImport?.fill(0); pendingPrivateImport = null
        val retired = RetiredWallet(repository, privateWorkspace)
        repository = null
        privateWorkspace = null
        documentRequests.revoke()
        work.execute { retired.close() }
        work.shutdown()
        priceWork.shutdownNow()
        withdrawalQuoteWork.shutdownNow()
        super.onDestroy()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        rememberResumeRoute()
        // Only allowlisted public route metadata is saved. No form, recovery words, PIN or review.
        super.onSaveInstanceState(outState)
        outState.clear()
        WalletNavigation.encode(resumeRoute).forEach { (key, value) -> outState.putString(key, value) }
        outState.putBoolean("wallet.session.signed_out", requireSignedOut)
    }

    override fun dispatchTouchEvent(event: MotionEvent): Boolean {
        if (event.flags and (MotionEvent.FLAG_WINDOW_IS_OBSCURED or MotionEvent.FLAG_WINDOW_IS_PARTIALLY_OBSCURED) != 0) return false
        return super.dispatchTouchEvent(event)
    }

    @Deprecated("Legacy callback is retained for API 30 compatibility")
    override fun onBackPressed() {
        if (busy || authPending) return
        if (activeWalletId == null) {
            if (screen in setOf("restore", "add_wallet")) { clearSensitiveViews(); renderSignedOut() }
            else super.onBackPressed()
            return
        }
        if (unlocked && screen != "home") {
            when (screen) {
                "asset_picker" -> returnFromAssetPicker()
                "security" -> renderProfile()
                "tron_connection" -> renderSecurity()
                "private_wallet" -> { privateResumePending = false; renderSecurity() }
                "app_lock" -> renderSecurity()
                "set_pin" -> renderAppLock()
                "reveal" -> { clearSensitiveViews(); renderSecurity() }
                "review" -> cancelReviewForEditing()
                else -> renderHome()
            }
        } else {
            lock()
            super.onBackPressed()
        }
    }

    private fun renderOpening(animate: Boolean) {
        screen = "opening"
        val shell = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL; gravity = Gravity.CENTER
            setPadding(dp(28), dp(28), dp(28), dp(28)); setBackgroundColor(this@MainActivity.background)
            isSaveEnabled = false
        }
        shell.addView(ZeroBrand.wordmark(this, ink))
        status = textView("", 13, muted).apply { visibility = View.GONE; gravity = Gravity.CENTER; isSaveEnabled = false }
        shell.addView(status, rowParams(24))
        contentHost = FrameLayout(this).apply {
            setBackgroundColor(this@MainActivity.background)
            isSaveEnabled = false
        }
        contentPage = shell
        contentHost.addView(shell, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        setContentView(contentHost)
        if (animate && ZeroBrand.animationsEnabled(this)) {
            openingTransition = ZeroOpeningTransition(this, background, ink) { openingTransition = null }.also {
                contentHost.addView(it, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
            }
        }
    }

    private fun showPageContent(view: View) {
        contentPage?.let(contentHost::removeView)
        contentPage = view
        contentHost.addView(view, 0, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        view.requestApplyInsets()
        openingTransition?.contentReady()
    }

    private fun walletChoices(): List<WalletChoice> = walletCatalog.list().map { stored ->
        WalletChoice(stored, stored.manifest?.let { displayNameStore.loadWallet(it.address) } ?: "Saved wallet")
    }

    /** Runs only on the serial wallet executor. No currently active Activity fields are read. */
    private fun loadStoredWallet(id: String): OpenedWallet {
        val selectedVault = walletCatalog.vault(id)
        val selectedManifest = selectedVault.load()
        val selectedRepository = MultiNetworkRepository(selectedVault, selectedManifest, { TronApiAccess(applicationContext).read() })
        try {
            return OpenedWallet(id, selectedVault, selectedManifest, selectedRepository,
                displayNameStore.loadWallet(selectedManifest.address), portfolioPreferences.loadSettings(selectedManifest.address))
        } catch (error: Exception) { selectedRepository.close(); throw error }
    }

    private fun discardOpenedWallet(opened: OpenedWallet) {
        val close = Runnable { runCatching { opened.repository.close() } }
        try { work.execute(close) }
        catch (_: RejectedExecutionException) {
            // The result was never installed and has no queued operations. Activity destruction
            // may already have shut down its executor; still release it off the UI thread.
            Thread(close, "zero-wallet-disposal").start()
        }
    }

    private fun installWallet(opened: OpenedWallet) {
        if (!selectionAuthority.selected(selectionClaim)) throw WalletFailure("Wallet selection changed. Choose the wallet again.")
        activeWalletId = opened.id
        vault = opened.vault
        manifest = opened.manifest
        repository = opened.repository
        walletName = opened.name
        portfolioSettings = opened.settings
        requireSignedOut = false
        resumeRoute = WalletNavigation.forWallet(resumeRoute, opened.manifest.address)
    }

    private fun initialize() {
        if (!WalletAccessPolicy.mayInitialize(busy, authPending, walletTransitionPending)) return
        if (!selectionAuthority.isCurrent(selectionClaim)) return
        val oldRepository = repository
        repository = null
        unlocked = false
        epoch++
        financialViewGeneration++
        val forceSignedOut = requireSignedOut
        val operationEpoch = epoch
        val claim = selectionClaim
        val current = { !isDestroyed && operationEpoch == epoch && selectionAuthority.isCurrent(claim) }
        asyncWithDiscard("Opening private storage…", {
            oldRepository?.close()
            if (!::walletCatalog.isInitialized) walletCatalog = WalletCatalog(applicationContext)
            if (forceSignedOut) walletCatalog.signOut(current)
            val choices = walletChoices()
            val selected = walletCatalog.selectedId()
            WalletStartup(choices, selected?.let(::loadStoredWallet))
        }, { result -> result.opened?.let(::discardOpenedWallet) }) { loaded ->
            savedWallets = loaded.choices
            if (activeWalletId != null && activeWalletId != loaded.opened?.id) {
                val retired = retireWallet()
                work.execute { retired.close() }
            }
            if (loaded.opened == null) {
                activeWalletId = null
                manifest = null
                resumeRoute = null
                renderSignedOut()
            } else {
                installWallet(loaded.opened)
                renderStartup()
            }
        }
    }

    /** Removes only in-memory capabilities. Saved vaults and transaction journals are untouched. */
    private fun retireWallet(): RetiredWallet {
        privateWorkspace?.setForeground(false)
        epoch++
        financialViewGeneration++
        unlocked = false
        authCancel?.cancel(); authCancel = null
        pinDialog?.cancel(); pinDialog = null
        authPending = false
        pendingAuthorizedAction = null
        clearSensitiveViews()
        copiedPhraseToken?.let(::clearOwnedPhraseClipboard)
        dismissNetworkDropdown()
        invalidateWithdrawalForm()
        val retired = RetiredWallet(repository, privateWorkspace)
        repository = null
        privateWorkspace = null
        privateWorkspaceOwner = null
        privateWorkspaceSelection = null
        documentRequests.revoke()
        privateResumePending = false
        pendingPrivateBackup?.encryptedBytes?.fill(0); pendingPrivateBackup = null
        pendingPrivateImport?.fill(0); pendingPrivateImport = null
        exportText = null
        activeWalletId = null
        manifest = null
        resumeRoute = null
        review = null
        familyReview = null
        reviewEditBlocked = false
        reviewControlsUpdate = null
        visibleDataUpdate = null
        preparingReview = false
        balanceCache.clear(); activityCache.clear(); historyCache.clear()
        syncState.clear(); syncMessage.clear()
        refreshing = false
        transferDrafts = TransferDrafts()
        unboundSendRecipients.clear()
        sendRecipient = ""; sendAmount = ""; transferChain = null
        selectedChain = 8453L; selectedAsset = "USDC"; receiveAsset = "USDC"; detailAsset = "USDC"
        assetPickerReturn = "home"; assetPickerAction = "deposit"
        createdMessagePending = false
        walletName = "Wallet 1"
        portfolioSettings = PortfolioDisplaySettings()
        hideKeyboard()
        return retired
    }

    private fun showWallets(add: Boolean = false) {
        if (walletTransitionPending) return
        val retired = retireWallet()
        requireSignedOut = true
        selectionClaim = selectionAuthority.requestSignOut()
        val claim = selectionClaim
        walletTransitionPending = true
        busy = true
        screen = "wallet_transition"
        newPage("Signing out…", "")
        paragraph("Closing the current session. Your saved wallets and transaction records stay on this device.", 15, muted)
        work.execute {
            val result = runCatching {
                try {
                    if (!::walletCatalog.isInitialized) walletCatalog = WalletCatalog(applicationContext)
                    walletCatalog.signOut { !isDestroyed && selectionAuthority.isCurrent(claim) }
                } finally { retired.close() }
                walletChoices()
            }
            runOnUiThread {
                walletTransitionPending = false
                busy = false
                if (isDestroyed) return@runOnUiThread
                if (!selectionAuthority.isCurrent(claim)) {
                    if (resumed) initialize()
                    return@runOnUiThread
                }
                result.fold({ choices ->
                    savedWallets = choices
                    if (add && resumed) renderAddWallet() else renderSignedOut()
                }, { error ->
                    screen = "wallets"
                    newPage("Sign out needs to finish", "")
                    paragraph("The session is closed, but the saved selection could not be verified. Retry sign out before closing the app.", 15, muted)
                    paragraph(errorMessage(error), 13, warning)
                    button("Retry sign out") { showWallets() }
                })
            }
        }
    }

    private fun chooseWallet(id: String) {
        if (busy || authPending) return
        val retired = retireWallet()
        requireSignedOut = true
        selectionClaim = selectionAuthority.requestSelection()
        val claim = selectionClaim
        val operationEpoch = epoch
        val current = { !isDestroyed && resumed && operationEpoch == epoch && selectionAuthority.isCurrent(claim) }
        screen = "wallets"
        newPage("Wallets", "")
        asyncWithDiscard("Opening selected wallet…", {
            retired.close()
            walletCatalog.select(id, current)
            loadStoredWallet(id)
        }, ::discardOpenedWallet) { opened ->
            installWallet(opened)
            renderStartup()
        }
    }

    private fun renderSignedOut() {
        if (savedWallets.isEmpty()) { renderWelcome(); return }
        screen = "wallets"
        newPage("Your wallets", "")
        paragraph("Choose a saved wallet to open. Signing out keeps your wallets and recovery phrases encrypted on this device.", 15, muted)
        for (choice in savedWallets) {
            val saved = choice.saved
            val value = saved.manifest
            card {
                label(choice.name, 19, true)
                if (value != null) {
                    label(shortAddress(value.address), 13, false, muted, monospace = true)
                    label("App lock: ${value.lockMode.label}", 12, false, muted)
                    button("Open ${choice.name}", secondary = true, parent = this) { chooseWallet(saved.id) }
                } else {
                    label(saved.error ?: "This saved wallet needs attention. Its files have been preserved.", 13, false, warning)
                }
            }
        }
        button("Add wallet") { renderAddWallet() }
        linkButton("Refresh saved wallets") { initialize() }
    }

    private fun renderAddWallet() {
        screen = "add_wallet"
        newPage("Add wallet", "")
        paragraph("Create a separate wallet or import another recovery phrase. Your existing wallets stay saved.", 16, muted)
        button("Create wallet") { startCreation() }
        button("Import wallet", secondary = true) { renderRestore() }
        linkButton("Back to wallets") { renderSignedOut() }
    }

    private fun errorMessage(error: Throwable): String = when (error) {
        is WalletFailure -> error.message ?: "Wallet operation was blocked."
        is WalletException.InvalidInput -> error.reason
        is WalletException.Policy -> error.reason
        is WalletException.Conflict -> error.reason
        is WalletException.Storage -> WalletDiagnostics.localRecordMessage(error)
        is WalletException.Crypto -> "The encrypted wallet could not be authenticated. Existing data has not been changed."
        is java.net.SocketTimeoutException -> "The network read timed out. Your saved balance has not been changed. Try again."
        is java.io.IOException -> "The network or local storage is unavailable. Existing records have been retained."
        is android.security.keystore.KeyPermanentlyInvalidatedException -> "The device lock key is no longer valid. Existing wallet files are preserved. Your recovery words can restore this wallet on another installation."
        is android.security.keystore.UserNotAuthenticatedException -> "Device authentication expired. Open the wallet again and approve the device prompt."
        is org.json.JSONException -> "Saved wallet information could not be read. Your wallet files are preserved; retry opening the existing wallet."
        is javax.crypto.AEADBadTagException -> "The selected unlock method could not open this encrypted wallet. Existing files are preserved."
        else -> "This step could not finish. Your saved wallet is preserved. Reopen it to continue."
    }

    private fun <T> async(label: String, action: () -> T, done: (T) -> Unit) = asyncWithDiscard(label, action, {}, done)

    private fun <T> asyncWithDiscard(label: String, action: () -> T, discard: (T) -> Unit, done: (T) -> Unit) {
        if (busy) return
        busy = true
        val started = epoch
        val startedSelection = selectionClaim
        status.text = label
        status.visibility = if (screen == "opening" || label in setOf("Opening wallet…", "Opening activity…")) View.GONE else View.VISIBLE
        status.setTextColor(muted)
        work.execute {
            val result = runCatching(action)
            runOnUiThread {
                if (walletTransitionPending) {
                    result.getOrNull()?.let(discard)
                    return@runOnUiThread
                }
                busy = false
                if (isDestroyed || started != epoch || !selectionAuthority.isCurrent(startedSelection)) {
                    result.getOrNull()?.let(discard)
                    if (isDestroyed) return@runOnUiThread
                    if (resumed && ::walletCatalog.isInitialized) initialize()
                    return@runOnUiThread
                }
                result.fold(onSuccess = { value ->
                    status.text = ""
                    status.visibility = View.GONE
                    try { done(value) } catch (error: Exception) { discard(value); showError(error) }
                }, onFailure = { error -> showError(error) })
            }
        }
    }

    private fun showError(error: Throwable) {
        openingTransition?.finishImmediately()
        if (unlocked && repository != null && WalletDiagnostics.isLocalRecordFailure(error)) {
            syncState[selectedChain] = "storage"
            syncMessage[selectedChain] = WalletDiagnostics.localRecordMessage(error)
            refreshVisibleScreen()
        }
        status.text = errorMessage(error)
        status.visibility = View.VISIBLE
        status.setTextColor(Color.rgb(164, 46, 35))
        if (activeWalletId == null && screen in setOf("opening", "welcome", "restore", "add_wallet", "wallets", "locked")) {
            // Enrollment may have committed before a late write/read-back error. Discover saved
            // wallets again instead of retrying against or replacing that storage directory.
            screen = "wallets"
            newPage("Wallet action paused", "")
            paragraph(errorMessage(error))
            button("Return to saved wallets") { showWallets() }
        } else if (activeWalletId != null && (manifest == null || repository == null)) {
            screen = "locked"
            newPage("Wallet data exists", "")
            paragraph(errorMessage(error))
            button("Reopen existing wallet") { initialize() }
            button("Choose another wallet", secondary = true, allowWhileBusy = true) { showWallets() }
        }
        // async clears busy before showing its failure. Refresh review controls after a key-open
        // cancellation callback may have run earlier on the worker while busy was still true.
        reviewControlsUpdate?.invoke()
    }

    private fun authorize(title: String, description: String, cipher: Cipher, cancelled: () -> Unit = {}, done: (Cipher) -> Unit) {
        if (busy || authPending || !selectionAuthority.isCurrent(selectionClaim)) return
        openingTransition?.finishImmediately()
        try {
            val authenticators = BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.DEVICE_CREDENTIAL
            val manager = getSystemService(BiometricManager::class.java)
            if (manager.canAuthenticate(authenticators) != BiometricManager.BIOMETRIC_SUCCESS) {
                throw WalletFailure("Enable a secure device PIN, password, pattern, or strong biometric before continuing.")
            }
            authPending = true
            val cancellation = CancellationSignal()
            val authorizationEpoch = epoch
            val authorizationSelection = selectionClaim
            authCancel = cancellation
            val prompt = BiometricPrompt.Builder(this).setTitle(title).setDescription(description)
                .setAllowedAuthenticators(authenticators).setConfirmationRequired(true).build()
            prompt.authenticate(BiometricPrompt.CryptoObject(cipher), cancellation, mainExecutor,
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        val action = action@{
                            if (epoch != authorizationEpoch || !selectionAuthority.isCurrent(authorizationSelection) ||
                                authCancel !== cancellation || cancellation.isCanceled || isDestroyed) {
                                cancelled()
                                return@action
                            }
                            authPending = false
                            authCancel = null
                            if (isDestroyed || !resumed) { cancelled(); return@action }
                            val authenticated = result.cryptoObject?.cipher
                            if (authenticated == null || authenticated !== cipher) {
                                cancelled()
                                showError(WalletFailure("Authentication did not authorize the required cryptographic operation."))
                                return@action
                            }
                            done(authenticated)
                        }
                        if (resumed) handler.post(action) else pendingAuthorizedAction = action
                    }
                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        if (epoch != authorizationEpoch || !selectionAuthority.isCurrent(authorizationSelection) ||
                            authCancel !== cancellation) { cancelled(); return }
                        authPending = false
                        authCancel = null
                        cancelled()
                        if (!isDestroyed) {
                            if (activeWalletId != null && manifest != null) renderLocked() else renderSignedOut()
                            status.text = "Authentication cancelled or unavailable. Nothing was signed."
                            status.visibility = View.VISIBLE
                        }
                    }
                })
        } catch (error: Exception) {
            authPending = false
            authCancel?.cancel()
            authCancel = null
            pendingAuthorizedAction = null
            cancelled()
            showError(error)
        }
    }

    private fun <T> withVaultKey(title: String, description: String, action: (ByteArray, () -> Boolean) -> T,
                                 cancelled: () -> Unit = {}, discard: (T) -> Unit = {},
                                 beforeDecrypt: ((proceed: () -> Unit, cancel: () -> Unit) -> Unit)? = null,
                                 operationAuthorized: () -> Boolean = { true },
                                 done: (T) -> Unit) {
        if (busy || authPending || !selectionAuthority.isCurrent(selectionClaim)) return
        try {
            rememberResumeRoute()
            val current = manifest ?: throw WalletFailure("Open the existing wallet first.")
            val operationVault = vault
            val operationSelection = selectionClaim
            fun execute(cipher: Cipher, pin: CharArray? = null) {
                val authorizationEpoch = epoch
                // Captured after a device prompt returns. A later pause invalidates queued work
                // even if Android never stops the Activity before it is resumed again.
                val authorizationLifecycle = financialViewGeneration
                val stillAuthorized = { resumed && epoch == authorizationEpoch &&
                    selectionAuthority.isCurrent(operationSelection) &&
                    financialViewGeneration == authorizationLifecycle && operationAuthorized() }
                var consumed = false
                val cancelOperation = {
                    if (!consumed) { consumed = true; pin?.fill('\u0000'); cancelled() }
                }
                val proceed = operation@{
                    if (consumed) return@operation
                    if (busy || !stillAuthorized()) { cancelOperation(); return@operation }
                    consumed = true
                    asyncWithDiscard("Opening your encrypted wallet…", {
                        try {
                            if (!stillAuthorized()) throw WalletFailure("This operation was interrupted. Reopen the wallet to continue.")
                            val key = operationVault.unwrap(current, cipher, pin)
                            try {
                                if (!stillAuthorized()) throw WalletFailure("This operation was interrupted. Reopen the wallet to continue.")
                                action(key, stillAuthorized)
                            } finally { key.fill(0) }
                        } catch (error: Exception) { cancelled(); throw error }
                        finally { pin?.fill('\u0000') }
                    }, discard, done)
                }
                if (beforeDecrypt != null) beforeDecrypt(proceed, cancelOperation) else proceed()
            }
            when (current.lockMode) {
                AppLockMode.OFF -> execute(operationVault.decryptionCipher(current))
                AppLockMode.DEVICE -> authorize(title, "$description\nUse your phone's biometrics or screen lock.", operationVault.decryptionCipher(current), cancelled) { execute(it) }
                AppLockMode.PIN -> requestPin(title, description, cancelled) { pin ->
                    try { execute(operationVault.decryptionCipher(current), pin) }
                    catch (error: Exception) { pin.fill('\u0000'); cancelled(); showError(error) }
                }
            }
        } catch (error: Exception) { cancelled(); showError(error) }
    }

    private fun requestPin(title: String, description: String, cancelled: () -> Unit, done: (CharArray) -> Unit) {
        if (busy || authPending || !resumed || !selectionAuthority.isCurrent(selectionClaim)) { cancelled(); return }
        openingTransition?.finishImmediately()
        val started = epoch
        val startedSelection = selectionClaim
        val input = EditText(this).apply {
            hint = "6-digit app PIN"; textSize = 22f
            inputType = InputType.TYPE_CLASS_NUMBER or InputType.TYPE_NUMBER_VARIATION_PASSWORD
            filters = arrayOf(InputFilter.LengthFilter(6))
            imeOptions = EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING or EditorInfo.IME_ACTION_DONE
            importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
            isSaveEnabled = false; filterTouchesWhenObscured = true
            setTextColor(ink); setHintTextColor(muted); setPadding(dp(20), dp(14), dp(20), dp(14))
        }
        sensitiveViews.add(input)
        authPending = true
        var completed = false
        val dialog = AlertDialog.Builder(this).setTitle(title).setMessage("$description\n\nEnter your 6-digit app PIN.")
            .setView(input).setNegativeButton("Cancel", null).setPositiveButton("Continue", null).create()
        pinDialog = dialog
        dialog.setOnShowListener {
            dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener {
                val pin = input.text.toString().toCharArray()
                if (pin.size != 6 || pin.any { it !in '0'..'9' }) {
                    pin.fill('\u0000'); input.error = "Enter exactly 6 digits"; return@setOnClickListener
                }
                if (started != epoch || !resumed || !selectionAuthority.isCurrent(startedSelection)) {
                    pin.fill('\u0000'); dialog.cancel(); return@setOnClickListener
                }
                completed = true
                input.text.clear(); dialog.dismiss(); authPending = false; pinDialog = null
                done(pin)
            }
        }
        dialog.setOnDismissListener {
            input.text.clear(); sensitiveViews.remove(input)
            if (pinDialog === dialog) pinDialog = null
            if (!completed) { authPending = false; cancelled() }
        }
        dialog.show()
    }

    private fun startCreation(restore: String? = null) {
        if (activeWalletId != null || manifest != null) return
        val newlyCreated = restore == null
        requireSignedOut = true
        selectionClaim = selectionAuthority.requestSelection()
        val claim = selectionClaim
        val operationEpoch = epoch
        val current = { !isDestroyed && resumed && operationEpoch == epoch && selectionAuthority.isCurrent(claim) }
        asyncWithDiscard("Creating your encrypted wallet…", {
            val choices = walletChoices()
            val defaultName = WalletAccessPolicy.nextWalletName(choices.map { it.name })
            val pending = walletCatalog.allocate()
            val enrollment = pending.vault.beginEnrollment()
            // Duplicate imports are rejected before any vault/journal is saved for the new slot.
            val created = pending.vault.completeEnrollment(enrollment, enrollment.cipher, restore) { address ->
                walletCatalog.requireUniqueAddress(address, pending.id)
            }.manifest
            displayNameStore.saveWallet(created.address, defaultName)
            walletCatalog.select(pending.id, current)
            loadStoredWallet(pending.id)
        }, ::discardOpenedWallet) { opened ->
            installWallet(opened)
            resumeRoute = null
            createdMessagePending = newlyCreated
            openSavedWallet()
        }
    }

    private fun renderStartup() {
        when (WalletAccessPolicy.startup(activeWalletId, manifest, savedWallets.isNotEmpty(), resumed)) {
            WalletAccessDestination.WELCOME -> renderWelcome()
            WalletAccessDestination.PICKER -> renderSignedOut()
            WalletAccessDestination.OPEN -> openSavedWallet()
            WalletAccessDestination.LOCKED -> renderLocked()
        }
    }

    private fun openSavedWallet() {
        val selectedRepository = repository ?: return
        withVaultKey("Open zerø", "Open the wallet saved on this device.", { key, _ -> selectedRepository.initializeFamilyAddresses(key) }) {
            unlocked = true
            if (privateResumePending || pendingPrivateImport != null) {
                renderPrivateWorkspace()
                if (pendingPrivateImport != null) continuePrivateImport()
            } else openWallet(WalletNavigation.forWallet(resumeRoute, manifest!!.address))
        }
    }

    private fun rememberResumeRoute() {
        val symbol = when (screen) { "receive", "track" -> receiveAsset; "asset" -> detailAsset; else -> selectedAsset }
        val destination = if (preparingReview) "review" else screen
        val chain = when (destination) {
            "send", "receive" -> transferChain
            "asset_picker" -> null
            else -> selectedChain
        }
        WalletNavigation.capture(manifest?.address, destination, chain, symbol, assetPickerAction)?.let { resumeRoute = it }
    }

    private fun renderDestination(route: WalletResumeRoute?) {
        val saved = WalletNavigation.forWallet(route, manifest!!.address)
        if (saved == null) { renderHome(); return }
        saved.chainId?.let { selectedChain = it }
        when (saved.destination) {
            WalletDestination.HOME -> renderHome()
            WalletDestination.HISTORY -> renderHistory()
            WalletDestination.PROFILE -> renderProfile()
            WalletDestination.SETTINGS -> renderSecurity()
            WalletDestination.RECOVERY -> renderRecoveryPage()
            WalletDestination.APP_LOCK -> renderAppLock()
            WalletDestination.SEND -> {
                selectedAsset = saved.symbol
                transferChain = saved.chainId
                loadSendDraft()
                renderSend()
            }
            WalletDestination.DEPOSIT -> { receiveAsset = saved.symbol; transferChain = saved.chainId; renderReceive() }
            WalletDestination.ASSET -> { detailAsset = saved.symbol; renderAsset() }
            WalletDestination.ASSET_PICKER -> { assetPickerReturn = "home"; renderAssetSelection(saved.pickerAction) }
            WalletDestination.TRACK -> { receiveAsset = saved.symbol; renderTrack() }
        }
    }

    private fun showCreatedMessage() {
        if (!createdMessagePending || !resumed || !unlocked || isDestroyed) return
        createdMessagePending = false
        AlertDialog.Builder(this).setTitle("Congratulations — your wallet is ready")
            .setMessage("Your private keys and 24-word recovery phrase stay encrypted on this device.\n\nWe do not receive your keys or recovery phrase, hold your money, or provide key recovery.\n\nFind your phrase in Profile → Settings → Recovery phrase. Save a private backup when you are ready.")
            .setPositiveButton("Continue", null).show()
    }

    private fun renderWelcome() {
        screen = "welcome"
        newPage("Your crypto.\nYour control.", "")
        paragraph("One simple place to hold, send and receive.", 17, muted)
        val art = FrameLayout(this).apply {
            background = rounded(surface, 28, line)
            addView(ZeroBrand.mark(this@MainActivity, ink, 82f), FrameLayout.LayoutParams(dp(96), dp(110), Gravity.CENTER))
        }
        page.addView(art, LinearLayout.LayoutParams(dp(156), dp(156)).apply { gravity = Gravity.CENTER_HORIZONTAL; topMargin = dp(28); bottomMargin = dp(18) })
        card {
            infoRow("Networks", "Ethereum, Base, OP, Arbitrum, Avalanche, Polygon, BNB, Solana and TRON")
            infoRow("Assets", NetworkPolicy.assetSymbols.joinToString(" · "))
            gap(5)
            label("Your recovery phrase is your backup. Keep it private and offline.", 13, false, muted)
        }
        button("Create wallet") { startCreation() }
        button("I already have a wallet", secondary = true) { renderRestore() }
        paragraph(ZeroBrand.releaseLabel(this), 11, muted).gravity = Gravity.CENTER
    }

    private fun renderRestore() {
        screen = "restore"
        newPage("Import your wallet", "")
        paragraph("Enter your recovery phrase in the original order. Your words are encrypted on this device.", 15, muted)
        val input = field("Recovery words", secret = true, multiline = true)
        sensitiveViews.add(input)
        paragraph("Supports standard English recovery phrases with standard Ethereum, Solana and TRON derivation paths, without an extra BIP-39 passphrase.", 12, muted)
        button("Import wallet") {
            val value = input.text.toString().trim().replace(Regex("\\s+"), " ")
            input.text.clear()
            if (value.split(' ').size !in setOf(12, 15, 18, 21, 24)) showError(WalletFailure("Enter 12, 15, 18, 21, or 24 recovery words."))
            else startCreation(value)
        }
    }

    private fun renderLocked() {
        if (activeWalletId == null) { renderSignedOut(); return }
        if (manifest == null) {
            screen = "locked"
            newPage("Your wallet is saved", "")
            paragraph("Reopen the existing wallet to continue. Its data has not been replaced.", 16, muted)
            button("Reopen existing wallet") { initialize() }
            return
        }
        screen = "locked"
        newPage("Welcome back", "")
        val mode = manifest!!.lockMode
        paragraph(if (mode == AppLockMode.PIN) "Enter your 6-digit app PIN to open this wallet." else if (mode == AppLockMode.DEVICE) "Use your phone's biometrics or existing screen lock." else "Your wallet is ready when you are.", 17, muted)
        val emblem = FrameLayout(this).apply {
            background = rounded(surface, 48, line)
            addView(WalletGlyph(this@MainActivity, "lock", ink), FrameLayout.LayoutParams(dp(42), dp(42), Gravity.CENTER))
        }
        page.addView(emblem, LinearLayout.LayoutParams(dp(96), dp(96)).apply { gravity = Gravity.CENTER_HORIZONTAL; topMargin = dp(48); bottomMargin = dp(40) })
        button(when (mode) { AppLockMode.PIN -> "Unlock with 6-digit PIN"; AppLockMode.DEVICE -> "Unlock with device authentication"; AppLockMode.OFF -> "Open wallet" }) { openSavedWallet() }
        paragraph("App lock: ${mode.label}", 12, muted).gravity = Gravity.CENTER
        button("Choose another wallet", secondary = true, allowWhileBusy = true) { showWallets() }
        button("Sign out", secondary = true, allowWhileBusy = true) { showWallets() }
    }

    /** Opening or resuming this page never decrypts the recovery phrase. */
    private fun renderRecoveryPage() {
        screen = "reveal"
        newPage("Recovery phrase", "")
        val content = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; isSaveEnabled = false }
        recoveryContent = content
        page.addView(content, rowParams(0))
        renderRecoveryContents(content)
    }

    private fun recoveryPage(): RecoveryPage = RecoveryPage(manifest?.address.orEmpty(), epoch, pageSequence)

    private fun renderRecoveryContents(content: LinearLayout) {
        clearSensitiveText()
        content.removeAllViews()
        if (!recoverySession.isVisible) {
            card(content) {
                label("Your recovery phrase is hidden", 20, true)
                label("Reveal it when you are ready to view it privately.", 14, false, muted)
            }
            button("Reveal recovery phrase", parent = content) { revealRecoveryWords() }
            paragraph("Anyone with your recovery phrase can move your assets.", 13, muted, parent = content)
        } else {
            paragraph("Save all ${recoverySession.wordCount} words in order, somewhere private.", 15, muted, parent = content)
            recoveryWordsGrid(content)
            allowPhraseCapture()
            button("Copy ${recoverySession.wordCount} words", secondary = true, parent = content) { copyRecoveryWords() }
            button("Hide recovery phrase", secondary = true, parent = content) { renderRecoveryPage() }
            paragraph("Screenshots are allowed while the words are visible. Keep copies private.", 12, muted, parent = content)
        }
    }

    private fun revealRecoveryWords() {
        if (!resumed || busy || authPending || screen != "reveal") return
        val saved = manifest ?: return
        val context = recoveryPage()
        val request = recoverySession.request(context)
        val expectedScreen = screen
        withVaultKey("Reveal recovery phrase", "Authorize viewing this wallet's recovery phrase.", { key, stillAuthorized ->
            if (!stillAuthorized() || !recoverySession.accepts(request, context, resumed)) {
                throw WalletFailure("Recovery display was interrupted. Tap Reveal again when ready.")
            }
            val phrase = revealMnemonic(saved.vaultHex, key).toCharArray()
            if (!stillAuthorized() || !recoverySession.accepts(request, context, resumed)) {
                phrase.fill('\u0000')
                throw WalletFailure("Recovery display was interrupted. Tap Reveal again when ready.")
            }
            phrase
        }, cancelled = { recoverySession.cancel(request) }, discard = { it.fill('\u0000') },
            beforeDecrypt = { proceed, cancel -> confirmRecoveryAfterAuthentication(request, proceed, cancel) },
            operationAuthorized = { recoverySession.accepts(request, context, resumed) }) { phrase ->
            val visible = recoverySession.reveal(request, recoveryPage(), resumed && screen == expectedScreen, phrase)
            if (visible) {
                unlocked = true
                recoveryContent?.let { renderRecoveryContents(it) }
                armRecoveryTimeout()
            }
        }
    }

    /** Holds only the authenticated operation closure; unwrap/decryption has not started. */
    private fun confirmRecoveryAfterAuthentication(request: RecoveryRevealRequest, proceed: () -> Unit, cancel: () -> Unit) {
        if (!recoverySession.isCurrent(request, recoveryPage(), resumed)) { cancel(); return }
        if (!recoverySession.needsForegroundConfirmation(request)) { proceed(); return }
        val dialog = AlertDialog.Builder(this).setTitle("Reveal recovery phrase?")
            .setMessage("Your phrase is still hidden. Reveal it now?")
            .setNegativeButton("Keep hidden", null).setPositiveButton("Reveal", null).create()
        val deferred = RecoveryRevealContinuation(recoverySession, request, recoveryPage(), resumed,
            proceed = { dialog.dismiss(); proceed() }, cancel = cancel)
        if (!deferred.isPending) return
        val expires = Runnable { dialog.cancel() }
        recoveryConfirmation = dialog
        authPending = true
        dialog.setOnShowListener {
            dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener {
                if (!deferred.confirm(recoveryPage(), resumed)) dialog.cancel()
            }
        }
        dialog.setOnDismissListener {
            handler.removeCallbacks(expires)
            if (recoveryConfirmation === dialog) { recoveryConfirmation = null; authPending = false }
            deferred.cancel()
        }
        try {
            dialog.show()
            handler.postDelayed(expires, 30_000)
        } catch (error: Exception) {
            recoveryConfirmation = null; authPending = false
            deferred.cancel()
            throw error
        }
    }

    private fun armRecoveryTimeout() {
        recoveryTimeout?.let(handler::removeCallbacks)
        val startedPage = pageSequence
        val timeout = Runnable {
            if (resumed && pageSequence == startedPage && screen == "reveal") renderRecoveryPage()
        }
        recoveryTimeout = timeout
        handler.postDelayed(timeout, 120_000)
    }

    private fun network(): WalletNetwork = network(selectedChain)
    private fun network(chainId: Long): WalletNetwork = repository!!.networks.first { it.chainId == chainId }
    private fun portfolioNetworks(): List<WalletNetwork> = WalletPortfolio.chainOrder.map(::network)
    private fun portfolioAssets(): List<PortfolioAsset> = WalletPortfolio.assets(repository!!.networks)

    private fun openWallet(destination: WalletResumeRoute? = null) {
        renderDestination(destination)
        showCreatedMessage()
        refreshPrices()
        val repo = repository!!
        val networks = portfolioNetworks()
        async("Opening wallet…", {
            networks.map { selected -> selected to Triple(
                runCatching { repo.cachedBalance(selected) },
                runCatching { repo.activitySnapshot(selected) },
                runCatching { repo.history(selected) }) }
        }) { cached ->
            for ((selected, result) in cached) {
                result.first.fold({ snapshot ->
                    snapshot?.let { balanceCache[selected.chainId] = it; syncState[selected.chainId] = "stale" }
                }, { error ->
                    syncState[selected.chainId] = if (WalletDiagnostics.isLocalRecordFailure(error)) "storage" else "unavailable"
                    syncMessage[selected.chainId] = WalletDiagnostics.refreshMessage(selected.name, error)
                })
                result.second.getOrNull()?.let { activityCache[selected.chainId] = it }
                result.third.getOrNull()?.let { historyCache[selected.chainId] = it }
            }
            refreshVisibleScreen()
            refresh()
        }
    }

    private fun renderHome() {
        if (!unlocked) { renderLocked(); return }
        screen = "home"
        newPage("Wallet", "", tabs = true)
        val heading = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL }
        val totalLabel = textView("Portfolio value", 14, muted)
        heading.addView(totalLabel, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        val visibility = iconControl(if (balancesHidden) "hidden" else "eye", if (balancesHidden) "Show balances" else "Hide balances") {
            balancesHidden = !balancesHidden
            refreshVisibleScreen()
        }
        heading.addView(visibility, LinearLayout.LayoutParams(dp(44), dp(44)))
        page.addView(heading, rowParams(12))
        val total = heroAmount("—")
        val caption = paragraph("USD estimate · Coinbase prices", 13, muted).apply { minHeight = dp(32) }
        walletActions({ renderAssetSelection("send") }, { renderAssetSelection("deposit") })
        val updateStatus = portfolioConnectionBanner()
        val title = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL; setPadding(0, dp(20), 0, dp(5)) }
        title.addView(textView("Assets", 19, ink, true), LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        title.addView(iconControl("refresh", "Refresh balances and USD prices") { refresh(); refreshPrices(force = true) }, LinearLayout.LayoutParams(dp(44), dp(44)))
        page.addView(title, rowParams(4))
        val bindings = portfolioAssets().map { position ->
            position to portfolioAssetRow(position) { selectPortfolioAsset(position, "asset") }
        }
        val empty = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL; gravity = Gravity.CENTER; minimumHeight = dp(174)
            setPadding(dp(20), dp(26), dp(20), dp(26)); isSaveEnabled = false
        }
        val emptyTitle = textView("", 18, ink, true).apply { gravity = Gravity.CENTER }
        val emptyNote = textView("", 13, muted).apply { gravity = Gravity.CENTER }
        empty.addView(emptyTitle, rowParams(0)); empty.addView(emptyNote, rowParams(10))
        page.addView(empty, rowParams(0))
        var lastHidden = !balancesHidden
        visibleDataUpdate = {
            val now = System.currentTimeMillis()
            val summary = WalletPortfolio.totalUsd(portfolioNetworks(), balanceCache, syncState, usdQuotes, now)
            val holdings = WalletPortfolio.holdings(portfolioNetworks(), balanceCache, syncState,
                portfolioSettings.hideSmallBalances, portfolioSettings.thresholdUsd, usdQuotes, now)
            val visibleKeys = holdings.visible.map { it.position.key }.toSet()
            totalLabel.updateText(if (summary.coverage == UsdCoverage.PARTIAL) "Known value" else "Portfolio value")
            val value = when (summary.coverage) {
                UsdCoverage.COMPLETE -> summary.amount?.let(UsdPrices::formatUsd) ?: "—"
                UsdCoverage.PARTIAL -> UsdPrices.formatUsd(summary.knownSubtotal)
                UsdCoverage.UNAVAILABLE -> "—"
            }
            total.updateText(if (balancesHidden) "••••••" else value)
            caption.updateText(when {
                summary.coverage == UsdCoverage.UNAVAILABLE -> if (priceRefreshing) "Checking USD prices and balances" else "USD estimate unavailable"
                summary.coverage == UsdCoverage.PARTIAL -> if (summary.stale) "Partial estimate · Last checked values" else "Partial estimate · Some prices or balances unavailable"
                summary.stale -> "Last checked estimate · Coinbase prices"
                else -> "USD estimate · Coinbase prices"
            })
            if (lastHidden != balancesHidden) {
                (visibility as FrameLayout).removeAllViews()
                visibility.addView(WalletGlyph(this, if (balancesHidden) "hidden" else "eye", muted), FrameLayout.LayoutParams(dp(21), dp(21), Gravity.CENTER))
                visibility.contentDescription = if (balancesHidden) "Show balances" else "Hide balances"
                lastHidden = balancesHidden
            }
            bindings.forEach { (position, binding) ->
                val visibilityState = if (position.key in visibleKeys) View.VISIBLE else View.GONE
                binding.row.visibility = visibilityState
                binding.divider.visibility = visibilityState
                updateAssetRow(position, binding)
            }
            empty.visibility = if (holdings.visible.isEmpty()) View.VISIBLE else View.GONE
            when (holdings.emptyState) {
                PortfolioEmptyState.ZERO -> {
                    emptyTitle.updateText(if (holdings.stale) "No assets at last check" else "No assets yet")
                    emptyNote.updateText(if (holdings.stale) "Refresh to check your balances." else "Deposit crypto to get started.")
                }
                PortfolioEmptyState.SMALL_BALANCES_HIDDEN -> {
                    emptyTitle.updateText("Small balances are hidden")
                    emptyNote.updateText("Below ${UsdPrices.formatUsd(portfolioSettings.thresholdUsd)} per asset. Change this in Settings." +
                        if (holdings.unknown.isNotEmpty()) "\nSome balances are unavailable."
                        else if (holdings.stale) "\nBased on last checked balances." else "")
                }
                PortfolioEmptyState.UNKNOWN -> {
                    val loading = syncState.values.any { it == "loading" }
                    emptyTitle.updateText(if (loading) "Checking your balances" else "Balances unavailable")
                    emptyNote.updateText(if (loading) "Your assets will appear here." else "Refresh to check your assets. Your receiving address is still available.")
                }
                PortfolioEmptyState.NONE -> Unit
            }
            updateStatus()
        }
        refreshVisibleScreen()
    }

    private fun renderAsset() {
        if (!unlocked) { renderLocked(); return }
        val selected = network()
        val symbol = detailAsset
        screen = "asset"
        newPage(symbol, selected.name)
        val asset = selected.asset(symbol)
        page.addView(AssetGlyph(this, symbol), LinearLayout.LayoutParams(dp(52), dp(52)).apply { topMargin = dp(20) })
        paragraph("$symbol (${selected.name})", 16, muted)
        val amount = heroAmount("—")
        val dollarValue = paragraph("", 16, muted)
        paragraph(assetName(symbol), 13, muted)
        val position = PortfolioAsset(selected, asset)
        walletActions({ selectPortfolioAsset(position, "send") }, { selectPortfolioAsset(position, "deposit") })
        val updateStatus = connectionBanner(selected)
        sectionTitle("Asset details")
        card {
            infoRow("Network", selected.name)
            infoRow("Asset", symbol)
            if (asset.contract != null) {
                gap(10); label("Token contract", 12, false, muted)
                label(asset.contract, 12, false, muted, monospace = true)
            } else label("${selected.nativeSymbol} pays network fees on ${selected.name}.", 13, false, muted)
        }
        linkButton("View activity") { openHistory() }
        visibleDataUpdate = {
            val units = balanceCache[selected.chainId]?.balances?.get(symbol)
            amount.updateText(if (balancesHidden) "••••••" else units?.let { Amounts.display(it, asset.decimals) } ?: "—")
            val usd = WalletPortfolio.usdValue(position, units, usdQuotes, System.currentTimeMillis())
            dollarValue.updateText(if (balancesHidden) "USD value hidden" else usd.amount?.let {
                "${UsdPrices.formatUsd(it)} USD estimate" + if (usd.freshness == PriceFreshness.STALE || syncState[selected.chainId] != "live") " · Last checked" else ""
            } ?: "USD value unavailable")
            updateStatus()
        }
        refreshVisibleScreen()
    }

    private fun renderAssetSelection(action: String) {
        if (screen == "send") saveSendDraft()
        assetPickerAction = action
        if (screen != "asset_picker") assetPickerReturn = screen
        review = null
        screen = "asset_picker"
        newPage(if (action == "send") "Send" else "Deposit", "")
        val context = transferViewRequest()
        paragraph("Choose an asset", 23, ink).apply { typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL); setPadding(0, dp(14), 0, dp(12)) }
        AssetSelection.assets(repository!!.networks).forEach { choice ->
            assetChoiceRow(choice.symbol) { if (transferViewCurrent(context)) selectTransferAsset(choice.symbol, action) }
        }
    }

    private fun returnFromAssetPicker() {
        when (assetPickerReturn) { "send" -> renderSend(); "receive" -> renderReceive(); "asset" -> renderAsset(); else -> renderHome() }
    }

    private fun selectPortfolioAsset(position: PortfolioAsset, action: String) {
        if (screen == "send") saveSendDraft()
        val canonical = AssetSelection.requireSelected(repository!!.networks, position.asset.symbol, position.network.chainId)
        review = null
        selectedChain = canonical.network.chainId
        when (action) {
            "send" -> {
                selectedAsset = canonical.asset.symbol
                transferChain = canonical.network.chainId
                loadSendDraft()
                renderSend()
            }
            "deposit" -> { receiveAsset = canonical.asset.symbol; transferChain = canonical.network.chainId; renderReceive() }
            else -> { detailAsset = canonical.asset.symbol; renderAsset() }
        }
    }

    private fun selectTransferAsset(symbol: String, action: String) {
        // Resolve the symbol even though the network is deliberately still unchosen.
        AssetSelection.resolve(repository!!.networks, symbol, null)
        review = null
        transferChain = null
        when (action) {
            "send" -> { selectedAsset = symbol; loadSendDraft(); renderSend() }
            "deposit" -> { receiveAsset = symbol; renderReceive() }
            else -> throw WalletFailure("Choose Send or Deposit to continue.")
        }
    }

    private fun selectTransferNetwork(symbol: String, action: String, chainId: Long) {
        val position = AssetSelection.requireSelected(repository!!.networks, symbol, chainId)
        val unboundRecipient = if (screen == "send" && transferChain == null) sendRecipient.takeIf { it.isNotBlank() } else null
        if (screen == "send") saveSendDraft()
        review = null
        transferChain = position.network.chainId
        selectedChain = position.network.chainId
        when (action) {
            "send" -> {
                selectedAsset = symbol
                loadSendDraft()
                // The reference flow accepts Address before Network. Bind only that explicitly
                // entered recipient on the first choice; never carry an amount/Max across chains.
                if (unboundRecipient != null) { sendRecipient = unboundRecipient; sendAmount = ""; saveSendDraft() }
                unboundSendRecipients.remove(unboundSendScope())
                renderSend()
            }
            "deposit" -> { receiveAsset = symbol; renderReceive() }
            else -> throw WalletFailure("Choose Send or Deposit to continue.")
        }
    }

    private fun unboundSendScope() = "${manifest!!.address.lowercase()}:$selectedAsset"

    private fun saveSendDraft() {
        val chain = transferChain
        if (chain == null) {
            if (manifest != null) unboundSendRecipients[unboundSendScope()] = sendRecipient
            return
        }
        manifest?.let { transferDrafts.save(it.address, PortfolioAssetKey(chain, selectedAsset), TransferDraft(sendRecipient, sendAmount)) }
    }

    private fun loadSendDraft() {
        val draft = transferChain?.let { transferDrafts.get(manifest!!.address, PortfolioAssetKey(it, selectedAsset)) }
            ?: TransferDraft(unboundSendRecipients[unboundSendScope()].orEmpty(), "")
        sendRecipient = draft.recipient; sendAmount = draft.amount
    }

    private fun transferViewRequest(): WalletUiRequest = WalletUiRequest(manifest!!.address, epoch, pageSequence, financialViewGeneration)

    private fun transferViewCurrent(context: WalletUiRequest): Boolean = context.accepts(
        manifest?.address, epoch, pageSequence, financialViewGeneration, resumed, unlocked) &&
        selectionAuthority.isCurrent(selectionClaim)

    private fun resumeAfterDiscardedQuote(context: WalletUiRequest) {
        // A pause can occur without onStop. Restore the captured public route after the read
        // finishes, with the configured app lock; never revive a stale Max/review capability.
        if (resumed && !isDestroyed && !busy && !authPending && context.lifecycle != financialViewGeneration) initialize()
    }

    private fun networkStatus(selected: WalletNetwork): String = if (refreshing) "Updating" else when (syncState[selected.chainId]) {
        "live" -> "Updated"
        "loading" -> "Updating"
        "stale" -> "Last checked balance"
        "storage" -> "Local records need attention"
        else -> "Balance unavailable"
    }

    private fun showNetworkStatus(selected: WalletNetwork) {
        val snapshot = balanceCache[selected.chainId]
        val observation = snapshot?.let { "\n\nLast checked block: ${it.block}" }.orEmpty()
        val message = syncMessage[selected.chainId] ?: when (syncState[selected.chainId]) {
            "live" -> "Balances were checked on ${selected.name}."
            "loading" -> "Checking ${selected.name}. Any previous balance stays visible until the update is verified."
            else -> "No fresh balance is available on ${selected.name}. An unavailable balance is not a zero balance."
        }
        AlertDialog.Builder(this).setTitle("${selected.name} · ${networkStatus(selected)}")
            .setMessage(message + observation).setNegativeButton("Close", null)
            .setPositiveButton("Refresh") { _, _ -> refresh() }.show()
    }

    /** Builds a small status view once. Polling changes only its text and indicator. */
    private fun connectionBanner(selected: WalletNetwork = network()): () -> Unit {
        val dot = View(this)
        val label = textView("", 12, muted)
        val box = LinearLayout(this).apply {
            gravity = Gravity.CENTER_VERTICAL; minimumHeight = dp(40)
            addView(dot, LinearLayout.LayoutParams(dp(5), dp(5)).apply { marginEnd = dp(8) })
            addView(label, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            addView(WalletGlyph(this@MainActivity, "chevron", muted), LinearLayout.LayoutParams(dp(11), dp(11)))
            isFocusable = true; filterTouchesWhenObscured = true
            setOnClickListener { if (!busy && !authPending) showNetworkStatus(selected) }
        }
        page.addView(box, rowParams(6))
        val update: () -> Unit = {
            val state = syncState[selected.chainId]
            label.updateText(networkStatus(selected))
            dot.background = rounded(if (state == "live") blue else if (state == "loading") muted else warning, 3)
            box.contentDescription = "${selected.name}: ${networkStatus(selected)}. Show details."
        }
        update()
        return update
    }

    private fun portfolioConnectionBanner(): () -> Unit {
        val dot = View(this)
        val label = textView("", 12, muted)
        val box = LinearLayout(this).apply {
            gravity = Gravity.CENTER_VERTICAL; minimumHeight = dp(40)
            addView(dot, LinearLayout.LayoutParams(dp(5), dp(5)).apply { marginEnd = dp(8) })
            addView(label, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            addView(WalletGlyph(this@MainActivity, "chevron", muted), LinearLayout.LayoutParams(dp(11), dp(11)))
            isFocusable = true; filterTouchesWhenObscured = true
            setOnClickListener {
                if (!busy && !authPending) {
                    val networks = portfolioNetworks()
                    AlertDialog.Builder(this@MainActivity).setTitle("Balance updates")
                        .setItems(networks.map { "${it.name} · ${networkStatus(it)}" }.toTypedArray()) { _, index -> showNetworkStatus(networks[index]) }
                        .setNegativeButton("Close", null).show()
                }
            }
        }
        page.addView(box, rowParams(6))
        val update: () -> Unit = {
            val states = WalletPortfolio.chainOrder.map { syncState[it] }
            val allReady = states.all { it == "live" }
            val loading = refreshing || states.any { it == "loading" }
            label.updateText(when {
                loading -> "Updating balances"
                allReady -> "All balances updated"
                states.any { it == "storage" } -> "Local records need attention"
                WalletPortfolio.chainOrder.any { !balanceCache.containsKey(it) } -> "Some balances unavailable"
                else -> "Last checked balances"
            })
            dot.background = rounded(if (allReady) blue else if (loading) muted else warning, 3)
            box.contentDescription = "${label.text}. Show network details."
        }
        update()
        return update
    }

    /** Public price requests never share the transaction/signing worker. */
    private fun refreshPrices(force: Boolean = false) {
        if (priceRefreshing || !unlocked || !resumed || isDestroyed) return
        val now = android.os.SystemClock.elapsedRealtime()
        if (!force && lastPriceRefreshElapsed > 0 && now - lastPriceRefreshElapsed < 60_000L) return
        lastPriceRefreshElapsed = now
        priceRefreshing = true
        priceWork.execute {
            val cached = runCatching { priceStore.load() }.getOrDefault(emptyMap())
            runOnUiThread {
                if (!isDestroyed && resumed && unlocked) { usdQuotes = cached; refreshVisibleScreen() }
            }
            val result = runCatching { priceStore.refresh() }
            runOnUiThread {
                priceRefreshing = false
                // Quotes contain no wallet identity and are usable by any currently open wallet.
                if (!isDestroyed && resumed && unlocked) {
                    result.getOrNull()?.let { usdQuotes = it.quotes }
                    refreshVisibleScreen()
                }
            }
        }
    }

    private fun refresh() {
        if (refreshing || busy || !unlocked || !resumed) return
        val networks = portfolioNetworks()
        val repo = repository!!
        val context = PortfolioRefreshContext(manifest!!.address, epoch)
        val priorStates = syncState.toMap()
        refreshing = true
        networks.forEach {
            // Keep a last successful live observation stable while checking again. Failure marks it
            // stale below; loading indicators are separate so filtering does not flash every poll.
            if (syncState[it.chainId] != "live") syncState[it.chainId] = "loading"
            syncMessage.remove(it.chainId)
        }
        refreshVisibleScreen()
        work.execute {
            val successful = mutableListOf<WalletNetwork>()
            // All balances get a turn before slower transfer discovery. Each chain fails independently.
            for (selected in networks) {
                if (busy || context.epoch != epoch || !resumed) break
                val balance = runCatching { repo.refreshBalance(selected) }
                if (balance.isSuccess) successful.add(selected)
                runOnUiThread {
                    if (!isDestroyed && context.accepts(selected.chainId, manifest?.address, epoch, unlocked, resumed)) {
                        balance.fold({ snapshot -> balanceCache[selected.chainId] = snapshot; syncState[selected.chainId] = "live" }, { failure ->
                            syncState[selected.chainId] = if (WalletDiagnostics.isLocalRecordFailure(failure)) "storage" else if (balanceCache.containsKey(selected.chainId)) "stale" else "unavailable"
                            syncMessage[selected.chainId] = WalletDiagnostics.refreshMessage(selected.name, failure)
                        })
                        refreshVisibleScreen()
                    }
                }
            }
            for (selected in successful) {
                if (busy || context.epoch != epoch || !resumed) break
                val history = runCatching { repo.reconcile(selected) }
                val incoming = if (!busy && context.epoch == epoch && resumed) runCatching { repo.discoverActivity(selected) { !busy && context.epoch == epoch && resumed } } else null
                runOnUiThread {
                    if (!isDestroyed && context.accepts(selected.chainId, manifest?.address, epoch, unlocked, resumed)) {
                        history.getOrNull()?.let { historyCache[selected.chainId] = it }
                        incoming?.getOrNull()?.let { activityCache[selected.chainId] = it }
                        if (history.isFailure || incoming?.isFailure == true) {
                            val localFailure = listOfNotNull(history.exceptionOrNull(), incoming?.exceptionOrNull()).firstOrNull(WalletDiagnostics::isLocalRecordFailure)
                            syncMessage[selected.chainId] = if (localFailure != null) "Balance updated. ${WalletDiagnostics.localRecordMessage(localFailure)}"
                                else "Balance updated. Some transfer details are still unavailable; track a transaction hash to check a deposit."
                        } else {
                            incoming?.getOrNull()?.warning?.let { syncMessage[selected.chainId] = "Balance updated. $it" }
                        }
                        refreshVisibleScreen()
                    }
                }
            }
            runOnUiThread {
                refreshing = false
                if (!isDestroyed && context.accepts(networks.first().chainId, manifest?.address, epoch, unlocked, resumed)) {
                    for (selected in networks) if (syncState[selected.chainId] == "loading") {
                        syncState[selected.chainId] = priorStates[selected.chainId]?.takeIf { it != "loading" }
                            ?: if (balanceCache.containsKey(selected.chainId)) "stale" else "unavailable"
                    }
                    refreshVisibleScreen()
                }
            }
        }
    }

    /** Never replaces the view tree during polling; form text, scroll, selection and QR remain in place. */
    private fun refreshVisibleScreen() {
        if (!unlocked || authPending || busy || !resumed) return
        visibleDataUpdate?.invoke()
    }

    private fun renderReceive() {
        val symbol = receiveAsset
        val position = AssetSelection.resolve(repository!!.networks, symbol, transferChain)
        screen = "receive"
        newPage("Deposit", "")
        assetPicker(symbol, "deposit")
        networkPicker(symbol, "deposit", position?.network)
        if (position == null) {
            paragraph("Choose the same network as the wallet or exchange sending your deposit.", 14, muted)
            return
        }
        val selected = position.network
        val context = transferViewRequest()
        val address = repository!!.address(selected)
        val asset = position.asset
        val uri = when (NetworkPolicy.family(selected.chainId)) {
            ChainFamily.EVM -> if (asset.contract == null) "ethereum:$address@${selected.chainId}" else "ethereum:${asset.contract}@${selected.chainId}/transfer?address=$address"
            ChainFamily.SOLANA -> if (asset.contract == null) "solana:$address" else "solana:$address?spl-token=${asset.contract}"
            ChainFamily.TRON -> address
        }
        val matrix = MultiFormatWriter().encode(uri, BarcodeFormat.QR_CODE, 720, 720, mapOf(EncodeHintType.MARGIN to 4))
        val bitmap = Bitmap.createBitmap(matrix.width, matrix.height, Bitmap.Config.ARGB_8888)
        val pixels = IntArray(matrix.width * matrix.height) { index -> if (matrix[index % matrix.width, index / matrix.width]) Color.BLACK else Color.WHITE }
        bitmap.setPixels(pixels, 0, matrix.width, 0, 0, matrix.width, matrix.height)
        val image = ImageView(this).apply { setImageBitmap(bitmap); contentDescription = "Deposit $symbol on ${selected.name}"; adjustViewBounds = true }
        val qr = LinearLayout(this).apply {
            gravity = Gravity.CENTER; setPadding(dp(15), dp(15), dp(15), dp(15)); background = rounded(Color.WHITE, 24)
            addView(image, LinearLayout.LayoutParams(dp(228), dp(228)))
        }
        page.addView(qr, LinearLayout.LayoutParams(dp(258), dp(258)).apply { gravity = Gravity.CENTER_HORIZONTAL; topMargin = dp(26); bottomMargin = dp(10) })
        paragraph("Only send $symbol on ${selected.name}.", 13, warning).gravity = Gravity.CENTER
        paragraph("Your receiving address", 12, muted).apply { gravity = Gravity.CENTER; setPadding(0, dp(10), 0, 0) }
        paragraph(address, 14, ink, monospace = true).apply { gravity = Gravity.CENTER; setTextIsSelectable(true) }
        actionRow("Copy address", { if (transferViewCurrent(context)) copyAddress(address, selected.name) }, "Share", {
            if (transferViewCurrent(context)) startActivity(Intent.createChooser(Intent(Intent.ACTION_SEND).apply {
                    type = "text/plain"; putExtra(Intent.EXTRA_TEXT, "Deposit $symbol on ${selected.name}\n$address")
                }, "Share receiving address"))
        })
        val updateStatus = connectionBanner(selected)
        val latest = paragraph("", 13, blue).apply { minHeight = dp(20) }
        linkButton("Waiting for a deposit? Check a transaction") { if (transferViewCurrent(context)) renderTrack() }
        visibleDataUpdate = {
            updateStatus()
            val received = activityCache[selected.chainId]?.rows?.firstOrNull {
                it.getString("verification_state") == "observed" && it.getString("direction") == "received" && it.getString("asset") == symbol
            }
            latest.updateText(received?.let { "Latest deposit · ${it.getString("amount")} $symbol" }.orEmpty())
        }
        refreshVisibleScreen()
    }

    private fun renderSend() {
        val symbol = selectedAsset
        val position = AssetSelection.resolve(repository!!.networks, symbol, transferChain)
        if (position != null && NetworkPolicy.family(position.network.chainId) != ChainFamily.EVM) { renderFamilySend(position); return }
        screen = "send"
        newPage("Send", "")
        val context = transferViewRequest()
        val flow = WithdrawalFlow().also { withdrawalFlow = it }
        val form = WithdrawalFormViews(this, withdrawalPalette(), symbol, assetName(symbol), position?.network)
        page.addView(form, rowParams(0))
        val dock = WithdrawalDock(this, withdrawalPalette())
        attachWithdrawalDock(dock)
        var mode = WithdrawalAmountMode.MANUAL
        var input: WithdrawalRequest? = null
        var inputProblem: String? = null
        var quoteProblem: String? = null
        var estimating = false
        var hadQuote = false
        var updatingAmount = false
        var preparing = false
        form.recipient.setText(sendRecipient)
        form.amount.setText(sendAmount)

        fun current() = !isDestroyed && transferViewCurrent(context) && withdrawalFlow === flow
        fun elapsed() = android.os.SystemClock.elapsedRealtime()
        fun amountUnits(): BigInteger? = when {
            position == null -> null
            mode == WithdrawalAmountMode.MAX -> flow.currentQuote(elapsed())?.recipientUnits
            else -> runCatching { Amounts.parse(sendAmount.trim(), position.asset.decimals) }.getOrNull()
        }
        fun updateSummary() {
            if (!current()) return
            val now = elapsed()
            val quote = flow.currentQuote(now)
            val units = amountUnits()
            form.editable(!preparing && !busy && !authPending, position != null)
            form.amount.textSize = if (sendAmount.length > 16) 21f else 30f
            form.maximum.contentDescription = if (mode == WithdrawalAmountMode.MAX) "Max selected. Recalculate the full available amount." else "Use the full available amount"
            dock.received.updateText(units?.let { "${Amounts.display(it, position!!.asset.decimals)} $symbol" } ?: "—")
            val balance = quote?.assetAvailable ?: position?.let { balanceCache[it.network.chainId]?.balances?.get(symbol) }
            form.available.updateText(when {
                position == null -> "Choose a network to see your balance"
                balance == null -> "Available balance · not verified"
                else -> "${if (quote != null) "Available" else "Last checked"} · ${Amounts.display(balance, position.asset.decimals)} $symbol"
            })
            val valuation = position?.let { UsdPrices.value(it, units, usdQuotes, System.currentTimeMillis()) }
            form.amountUsd.updateText(when {
                units == null -> ""
                valuation?.freshness == PriceFreshness.FRESH && valuation.amount != null -> "≈ ${UsdPrices.formatUsd(valuation.amount)} USD"
                else -> "USD estimate unavailable"
            })
            dock.fee.updateText(when {
                preparing -> "Checking current fees…"
                estimating -> "Estimating…"
                quote != null -> {
                    val usd = quote.estimatedFeeUsd(usdQuotes, System.currentTimeMillis())
                    val dollars = if (usd.freshness == PriceFreshness.FRESH && usd.amount != null) "≈ ${UsdPrices.formatUsd(usd.amount)} · " else ""
                    "$dollars${feeText(quote.fee.totalEstimate, quote.request.network)}"
                }
                quoteProblem != null -> "Unavailable"
                input == null -> if (position == null) "Choose a network" else "Add recipient and amount"
                hadQuote -> "Estimate expired"
                else -> "Not estimated"
            })
            dock.fee.contentDescription = if (quote != null) "Estimated network fee ${dock.fee.text}. Open fee details." else "Network fee: ${dock.fee.text}"
            form.estimateAction.visibility = if (input != null && !preparing) View.VISIBLE else View.GONE
            form.estimateAction.updateText(if (quoteProblem != null) "Try fee estimate again" else if (estimating) "Estimating fee…" else "Refresh fee")
            form.estimateAction.isEnabled = !estimating && !preparing && !busy && !authPending
            form.depositFees.visibility = if (quote != null && quote.fee.shortfall > BigInteger.ZERO && !preparing) View.VISIBLE else View.GONE
            val message = when {
                preparing -> "Checking this transfer before the final review."
                quoteProblem != null -> quoteProblem!!
                estimating && mode == WithdrawalAmountMode.MAX -> "Calculating your full available amount and network fee…"
                quote != null && quote.fee.shortfall > BigInteger.ZERO ->
                    "Add ${feeText(quote.fee.shortfall, quote.request.network)} on ${quote.request.network.name} to cover ${if (symbol == quote.request.network.nativeSymbol) "this amount and its fees" else "network fees"}."
                quote != null && !quote.fee.transferAvailable -> "There is not enough $symbol on ${quote.request.network.name} for this amount."
                quote != null && !quote.fee.simulated -> "A conservative fee budget is shown. The transfer is checked again before review."
                inputProblem != null -> inputProblem!!
                hadQuote && quote == null && !estimating -> "The fee estimate expired. Refresh it to continue."
                else -> ""
            }
            form.message(message, quoteProblem != null || quote?.readyToPrepare == false)
            dock.button(if (preparing) "Preparing review…" else "Review transfer",
                !preparing && !estimating && !busy && !authPending && quote?.readyToPrepare == true)
        }
        fun quoteNow() {
            withdrawalDebounce?.let(handler::removeCallbacks)
            withdrawalDebounce = null
            if (!current() || input == null || preparing) return
            estimating = true; quoteProblem = null
            updateSummary()
            val launch = launch@{
                if (!current() || input == null || preparing) return@launch
                val ticket = flow.beginQuote(elapsed()) ?: run {
                    estimating = false; updateSummary(); return@launch
                }
                estimating = true; quoteProblem = null
                updateSummary()
                val repo = repository!!
                withdrawalQuoteRunning = true
                withdrawalQuoteWork.execute {
                    val result = runCatching {
                        val request = ticket.request
                        if (request.mode == WithdrawalAmountMode.MAX) repo.maxTransfer(request.network, request.symbol, request.recipient).let { it.fee to it }
                        else repo.preview(request.network, request.symbol, request.recipient, request.amount!!) to null
                    }
                    runOnUiThread {
                        withdrawalQuoteRunning = false
                        if (current()) {
                            result.fold({ (fee, maximum) ->
                                try {
                                    if (flow.complete(ticket, fee, maximum, elapsed())) {
                                        estimating = false; quoteProblem = null; hadQuote = true
                                        val accepted = flow.currentQuote(elapsed())
                                        if (accepted != null && accepted.maximum != null) {
                                            updatingAmount = true
                                            sendAmount = accepted.amount
                                            form.amount.setText(sendAmount)
                                            updatingAmount = false
                                            saveSendDraft()
                                        }
                                        withdrawalExpiry?.let(handler::removeCallbacks)
                                        withdrawalExpiry = accepted?.let { quote -> Runnable {
                                            if (current()) updateSummary()
                                        }.also { handler.postDelayed(it, quote.remainingMillis(elapsed()) + 1L) } }
                                        updateSummary()
                                    }
                                } catch (error: Exception) {
                                    // complete consumes the current ticket before validating its payload.
                                    estimating = false; quoteProblem = errorMessage(error)
                                    updateSummary()
                                }
                            }, { error ->
                                if (flow.fail(ticket)) {
                                    estimating = false; quoteProblem = errorMessage(error)
                                    updateSummary()
                                }
                            })
                        }
                        val next = pendingWithdrawalQuote
                        pendingWithdrawalQuote = null
                        if (!isDestroyed) next?.invoke()
                    }
                }
            }
            // One running read and one latest pending job. Edits never create an unbounded queue.
            if (withdrawalQuoteRunning) pendingWithdrawalQuote = launch else launch()
        }
        fun updateInput(immediate: Boolean = false) {
            withdrawalDebounce?.let(handler::removeCallbacks)
            withdrawalDebounce = null
            pendingWithdrawalQuote = null
            withdrawalExpiry?.let(handler::removeCallbacks)
            withdrawalExpiry = null
            quoteProblem = null; hadQuote = false; estimating = false
            inputProblem = null
            input = try {
                when {
                    position == null -> null
                    sendRecipient.isBlank() -> null
                    mode == WithdrawalAmountMode.MANUAL && sendAmount.isBlank() -> null
                    else -> WithdrawalRequest.create(repository!!.networks, manifest!!.address,
                        position.network.chainId, symbol, sendRecipient, sendAmount, mode)
                }
            } catch (error: WalletFailure) {
                inputProblem = error.message
                null
            }
            flow.update(input)
            if (input != null) {
                estimating = true
                val task = Runnable { if (current()) quoteNow() }
                withdrawalDebounce = task
                handler.postDelayed(task, if (immediate) 0L else 550L)
            }
            updateSummary()
        }
        watch(form.recipient) {
            if (current() && !preparing) {
                sendRecipient = it
                if (mode == WithdrawalAmountMode.MAX) {
                    updatingAmount = true; sendAmount = ""; form.amount.setText(""); updatingAmount = false
                }
                saveSendDraft(); updateInput()
            }
        }
        watch(form.amount) {
            if (current() && !preparing && !updatingAmount) {
                mode = WithdrawalAmountMode.MANUAL; sendAmount = it
                saveSendDraft(); updateInput()
            }
        }
        form.assetSelector.setOnClickListener {
            if (current() && !busy && !authPending && !preparing) { hideKeyboard(); renderAssetSelection("send") }
        }
        form.networkSelector.setOnClickListener {
            if (current() && !busy && !authPending && !preparing) { hideKeyboard(); showNetworkPicker(symbol, "send", position?.network, context) }
        }
        form.paste.setOnClickListener {
            if (!current() || busy || authPending || preparing) return@setOnClickListener
            val clipboard = getSystemService(ClipboardManager::class.java)
            val pasted = clipboard.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.text?.toString()?.trim()
            val address = pasted?.let { runCatching { Amounts.address(it) }.getOrNull() }
            if (address == null) {
                inputProblem = "The clipboard does not contain a complete wallet address."
                form.message(inputProblem!!, true)
            } else {
                form.recipient.setText(address)
                form.recipient.setSelection(form.recipient.text.length)
            }
        }
        form.maximum.setOnClickListener {
            if (!current() || busy || authPending || preparing || position == null) return@setOnClickListener
            try {
                WithdrawalRequest.create(repository!!.networks, manifest!!.address, position.network.chainId,
                    symbol, sendRecipient, "", WithdrawalAmountMode.MAX)
                mode = WithdrawalAmountMode.MAX
                updatingAmount = true; sendAmount = ""; form.amount.setText(""); updatingAmount = false
                saveSendDraft(); updateInput(immediate = true)
            } catch (error: WalletFailure) {
                form.message(error.message ?: "Enter a complete recipient address before using Max.", true)
                if (runCatching { Amounts.address(sendRecipient.trim()) }.isFailure) form.recipient.requestFocus()
            }
        }
        form.amount.setOnEditorActionListener { _, action, _ ->
            if (action == EditorInfo.IME_ACTION_DONE) { hideKeyboard(); true } else false
        }
        form.estimateAction.setOnClickListener { if (current() && !busy && !authPending && !preparing && !estimating) quoteNow() }
        form.depositFees.setOnClickListener {
            val quote = flow.currentQuote(elapsed())
            if (current() && !busy && !authPending && !preparing && quote != null) {
                hideKeyboard()
                selectPortfolioAsset(PortfolioAsset(quote.request.network, quote.request.network.nativeAsset), "deposit")
            }
        }
        dock.fee.setOnClickListener {
            if (current() && !busy && !authPending && !preparing) flow.currentQuote(elapsed())?.let { showWithdrawalFeeDetails(it, context) }
        }
        dock.primary.setOnClickListener {
            if (!current() || busy || authPending || preparing) return@setOnClickListener
            val approved = flow.claimPreparation(elapsed()) ?: run { updateSummary(); return@setOnClickListener }
            preparing = true; preparingReview = true
            withdrawalDebounce?.let(handler::removeCallbacks); withdrawalDebounce = null
            pendingWithdrawalQuote = null
            hideKeyboard(); updateSummary()
            val repo = repository!!
            asyncWithDiscard("Checking your transfer…", {
                runCatching { repo.prepare(approved.request.network, approved.request.symbol, approved.request.recipient,
                    approved.amount, maxQuote = approved.maximum) }
            }, { preparingReview = false }) { result ->
                preparingReview = false
                if (!current() || !flow.preparationCurrent(approved)) {
                    if (result.isSuccess) resumeRoute = WalletNavigation.capture(context.address, "review",
                        approved.request.network.chainId, approved.request.symbol, "send")
                    resumeAfterDiscardedQuote(context)
                    return@asyncWithDiscard
                }
                result.fold({ prepared ->
                    sendRecipient = prepared.request.getString("to")
                    sendAmount = prepared.request.getString("amount")
                    saveSendDraft()
                    review = prepared
                    renderReview(prepared)
                }, { error ->
                    preparing = false; flow.releasePreparation()
                    quoteProblem = errorMessage(error); estimating = false
                    updateSummary()
                })
            }
        }
        visibleDataUpdate = { updateSummary() }
        refreshPrices()
        updateInput()
    }

    private fun familyRecipient(network: WalletNetwork, input: String): String {
        val publicDestination = PrivateAddressRules.requirePublicText(input)
        return when (NetworkPolicy.family(network.chainId)) {
            ChainFamily.SOLANA -> uniffi.wallet_core.solanaValidateAddress(publicDestination)
            ChainFamily.TRON -> uniffi.wallet_core.tronNormalizeAddress(publicDestination)
            else -> throw WalletFailure("Choose a supported network.")
        }
    }

    /** The same asset-first form, with protocol-specific preparation behind it. */
    private fun renderFamilySend(position: PortfolioAsset) {
        val selected = position.network
        val symbol = position.asset.symbol
        screen = "send"
        newPage("Send", "")
        familyReview = null; review = null
        val context = transferViewRequest()
        val repo = repository!!
        val form = WithdrawalFormViews(this, withdrawalPalette(), symbol, assetName(symbol), selected)
        page.addView(form, rowParams(0))
        val dock = WithdrawalDock(this, withdrawalPalette())
        attachWithdrawalDock(dock)
        var quote: FamilyPrepared? = null
        var quoteReceivedAt = 0L
        var sequence = 0
        var maximum = false
        var updating = false
        var estimating = false
        var preparing = false
        var problem: String? = null
        form.recipient.setText(sendRecipient)
        form.amount.setText(sendAmount)
        fun current() = !isDestroyed && transferViewCurrent(context)
        fun fresh(): FamilyPrepared? = quote?.takeIf {
            System.currentTimeMillis() < it.validUntilMillis && SystemClock.elapsedRealtime() - quoteReceivedAt in 0..30_000L
        }
        fun update() {
            if (!current()) return
            val active = fresh()
            form.editable(!busy && !authPending && !preparing, true)
            val units = active?.amountUnits ?: runCatching { Amounts.parse(sendAmount, position.asset.decimals) }.getOrNull()
            dock.received.updateText(units?.let { "${Amounts.display(it, position.asset.decimals)} $symbol" } ?: "—")
            val balance = active?.assetAvailable ?: balanceCache[selected.chainId]?.balances?.get(symbol)
            form.available.updateText(balance?.let { "${if (active == null) "Last checked" else "Available"} · ${Amounts.display(it, position.asset.decimals)} $symbol" } ?: "Available balance · not verified")
            form.amount.textSize = if (sendAmount.length > 16) 21f else 30f
            val usd = UsdPrices.value(position, units, usdQuotes, System.currentTimeMillis())
            form.amountUsd.updateText(if (usd.amount != null && usd.freshness == PriceFreshness.FRESH) "≈ ${UsdPrices.formatUsd(usd.amount)} USD" else if (units != null) "USD estimate unavailable" else "")
            dock.fee.updateText(when {
                estimating -> "Estimating…"
                active != null -> feeText(active.estimatedFee, selected)
                quote != null -> "Estimate expired"
                problem != null -> "Unavailable"
                else -> "Add recipient and amount"
            })
            form.message(problem ?: if (quote != null && active == null) "Refresh the fee estimate to continue." else "", problem != null)
            form.estimateAction.visibility = if (sendRecipient.isNotBlank()) View.VISIBLE else View.GONE
            form.estimateAction.isEnabled = !busy && !authPending && !preparing && !estimating
            dock.button(if (preparing) "Preparing review…" else "Review transfer", active != null && !busy && !authPending && !preparing && !estimating)
        }
        fun estimate(immediate: Boolean = false) {
            sequence++
            val ticket = sequence
            withdrawalDebounce?.let(handler::removeCallbacks); withdrawalDebounce = null
            pendingWithdrawalQuote = null
            withdrawalExpiry?.let(handler::removeCallbacks); withdrawalExpiry = null
            quote = null; problem = null; estimating = false
            val recipient = runCatching { familyRecipient(selected, sendRecipient) }.getOrNull()
            val amount = sendAmount.trim()
            if (recipient == null || (!maximum && runCatching { Amounts.parse(amount, position.asset.decimals) }.isFailure)) { update(); return }
            val useMaximum = maximum
            estimating = true; update()
            val launch = launch@{
                if (!current() || sequence != ticket) return@launch
                withdrawalQuoteRunning = true
                withdrawalQuoteWork.execute {
                    val result = runCatching { repo.quoteFamily(selected, symbol, recipient, amount, useMaximum) }
                    runOnUiThread {
                        withdrawalQuoteRunning = false
                        if (current() && sequence == ticket) {
                            estimating = false
                            result.fold({ ready ->
                                quote = ready; quoteReceivedAt = SystemClock.elapsedRealtime()
                                if (useMaximum) {
                                    updating = true
                                    sendAmount = Amounts.display(ready.amountUnits, position.asset.decimals)
                                    form.amount.setText(sendAmount); updating = false; saveSendDraft()
                                }
                                val delay = minOf(30_001L, (ready.validUntilMillis - System.currentTimeMillis() + 1).coerceAtLeast(1L))
                                withdrawalExpiry = Runnable { update() }.also { handler.postDelayed(it, delay) }
                            }, { problem = errorMessage(it) })
                            update()
                        }
                        val next = pendingWithdrawalQuote
                        pendingWithdrawalQuote = null
                        if (!isDestroyed) next?.invoke()
                    }
                }
            }
            withdrawalDebounce = Runnable {
                if (!current() || sequence != ticket) return@Runnable
                if (withdrawalQuoteRunning) pendingWithdrawalQuote = launch else launch()
            }.also { handler.postDelayed(it, if (immediate) 0L else 550L) }
        }
        watch(form.recipient) { if (current() && !preparing) { sendRecipient = it; saveSendDraft(); estimate() } }
        watch(form.amount) { if (current() && !preparing && !updating) { maximum = false; sendAmount = it; saveSendDraft(); estimate() } }
        form.assetSelector.setOnClickListener { if (current() && !busy && !authPending && !preparing) { hideKeyboard(); renderAssetSelection("send") } }
        form.networkSelector.setOnClickListener { if (current() && !busy && !authPending && !preparing) { hideKeyboard(); showNetworkPicker(symbol, "send", selected, context) } }
        if (symbol != selected.nativeSymbol) {
            form.depositFees.visibility = View.VISIBLE
            form.depositFees.setOnClickListener {
                if (current() && !busy && !authPending && !preparing) selectPortfolioAsset(PortfolioAsset(selected, selected.nativeAsset), "deposit")
            }
        }
        form.paste.setOnClickListener {
            if (!current() || busy || authPending || preparing) return@setOnClickListener
            val pasted = getSystemService(ClipboardManager::class.java).primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.text?.toString().orEmpty()
            runCatching { familyRecipient(selected, pasted) }.fold({ form.recipient.setText(it) }, { form.message("The clipboard does not contain a valid ${selected.name} address.", true) })
        }
        form.maximum.setOnClickListener {
            if (!current() || busy || authPending || preparing) return@setOnClickListener
            if (runCatching { familyRecipient(selected, sendRecipient) }.isFailure) { form.message("Enter the receiving address before using Max.", true); form.recipient.requestFocus() }
            else { maximum = true; estimate(true) }
        }
        form.estimateAction.setOnClickListener { if (current() && !busy && !authPending && !preparing && !estimating) estimate(true) }
        dock.fee.setOnClickListener {
            fresh()?.let { ready -> if (current() && !busy && !authPending) AlertDialog.Builder(this)
                .setTitle("Cost to transfer").setMessage("Estimated fee: ${feeText(ready.estimatedFee, selected)}\nFee reserve: ${feeText(ready.feeReserve, selected)}\n\n${ready.note}")
                .setPositiveButton("Done", null).show() }
        }
        dock.primary.setOnClickListener {
            if (!current() || busy || authPending || preparing || estimating) return@setOnClickListener
            val ready = fresh() ?: run { update(); return@setOnClickListener }
            preparing = true; preparingReview = true; sequence++
            hideKeyboard(); update()
            asyncWithDiscard("Checking your transfer…", { runCatching { repo.prepareFamily(selected, ready) } }, { preparingReview = false }) { result ->
                preparingReview = false
                if (!current()) {
                    if (result.isSuccess) resumeRoute = WalletNavigation.capture(context.address, "review", selected.chainId, symbol, "send")
                    resumeAfterDiscardedQuote(context); return@asyncWithDiscard
                }
                result.fold({ renderFamilyReview(it) }, { preparing = false; problem = errorMessage(it); update() })
            }
        }
        visibleDataUpdate = { update() }
        estimate()
    }

    private fun renderFamilyReview(payment: FamilyReview) {
        screen = "review"
        newPage("Review transfer", payment.network.name)
        review = null; familyReview = payment
        val selected = payment.network
        val prepared = payment.prepared
        val request = prepared.request
        val symbol = request.getString("asset")
        val amount = Amounts.display(prepared.amountUnits, selected.asset(symbol).decimals)
        val recipient = request.getString("to")
        val context = transferViewRequest()
        var claimed = false
        var editing = false
        paragraph("You're sending", 13, muted).gravity = Gravity.CENTER
        paragraph(amount, if (amount.length > 16) 24 else 37, ink, monospace = amount.length > 16).apply { gravity = Gravity.CENTER; setTextIsSelectable(true) }
        paragraph("$symbol · ${selected.name}", 14, muted).gravity = Gravity.CENTER
        card {
            label("Recipient", 12, false, muted)
            label(recipient, 16, false, ink, monospace = true)
            gap(12); infoRow("Network", selected.name)
            label("Verify the entire address and receiving network.", 12, false, muted)
        }
        card {
            infoRow("Cost to transfer", feeText(prepared.estimatedFee, selected))
            infoRow("Fee reserve", feeText(prepared.feeReserve, selected))
            label(prepared.note, 12, false, muted)
            gap(12); label("From", 12, false, muted)
            label(request.getString("from"), 12, false, ink, monospace = true)
            selected.asset(symbol).contract?.let { label(if (selected.chainId == FamilyNetworks.SOLANA) "Token mint" else "Token contract", 12, false, muted); label(it, 12, false, ink, monospace = true) }
        }
        paragraph("Nothing is sent until you confirm. If this review expires, the app requires a new review.", 12, muted)
        val dock = WithdrawalDock(this, withdrawalPalette(), "Fee reserve")
        dock.received.text = "$amount $symbol"
        dock.fee.text = feeText(prepared.feeReserve, selected)
        attachWithdrawalDock(dock)
        fun current() = !isDestroyed && transferViewCurrent(context) && familyReview?.id == payment.id
        fun update() {
            if (!current()) return
            val fresh = System.currentTimeMillis() < prepared.validUntilMillis
            dock.button(when { claimed -> "Sending…"; editing -> "Editing…"; !fresh -> "Review expired"; else -> "Confirm and send" },
                fresh && !claimed && !editing && !busy && !authPending)
        }
        linkButton("Edit details") { if (current() && !claimed && !editing) { editing = true; update(); cancelFamilyReview(payment) } }
        dock.primary.setOnClickListener {
            if (!current() || claimed || editing || busy || authPending || System.currentTimeMillis() >= prepared.validUntilMillis) return@setOnClickListener
            claimed = true; update()
            val repo = repository!!
            async("Rechecking this exact transfer…", { runCatching { repo.revalidateFamily(payment) } }) { checked ->
                if (!current()) { resumeAfterDiscardedQuote(context); return@async }
                checked.fold({ rechecked ->
                    withVaultKey("Send $amount $symbol", "${selected.name}\nTo $recipient", { key, stillAuthorized ->
                        runCatching { repo.signFamilyAndBroadcast(rechecked, key, stillAuthorized) }
                    }, cancelled = { runOnUiThread { if (current()) { claimed = false; update() } } }) { outcome ->
                        unlocked = true; familyReview = null
                        outcome.fold(::renderPaymentResult) { error ->
                            selectedChain = selected.chainId; screen = "result"
                            newPage("Check transfer status", selected.name)
                            paragraph("Check Activity before trying again", 25, ink)
                            paragraph(errorMessage(error), 14, muted)
                            paragraph("The saved record determines whether this transfer is unsigned, submitted or still unresolved.", 13, muted)
                            button("View activity") { openHistory(reconcile = true) }
                        }
                    }
                }, { claimed = false; showError(it); update() })
            }
        }
        reviewControlsUpdate = { update() }; visibleDataUpdate = { update() }
        withdrawalExpiry = Runnable { update() }.also { handler.postDelayed(it, (prepared.validUntilMillis - System.currentTimeMillis() + 1L).coerceAtLeast(1L)) }
        update()
    }

    private fun cancelFamilyReview(payment: FamilyReview) {
        if (busy || authPending || !resumed || !unlocked) return
        val context = transferViewRequest()
        val repo = repository!!
        async("Releasing this unsigned transfer…", { runCatching { repo.cancelPrepared(payment.network, payment.id) } }) { result ->
            if (!transferViewCurrent(context)) { resumeAfterDiscardedQuote(context); return@async }
            result.fold({
                familyReview = null
                transferChain = payment.network.chainId; selectedChain = payment.network.chainId
                selectedAsset = payment.prepared.request.getString("asset")
                sendRecipient = payment.prepared.request.getString("to")
                sendAmount = Amounts.display(payment.prepared.amountUnits, payment.network.asset(selectedAsset).decimals)
                saveSendDraft(); renderSend()
            }, { showError(it); openHistory() })
        }
    }

    private fun withdrawalPalette() = WithdrawalPalette(background, surface, elevated, line, ink, muted, blue, warning)

    private fun attachWithdrawalDock(dock: View) {
        pageShell?.addView(View(this).apply { setBackgroundColor(line) }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(1)))
        pageShell?.addView(dock, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
    }

    private fun invalidateWithdrawalForm() {
        withdrawalDebounce?.let(handler::removeCallbacks); withdrawalDebounce = null
        withdrawalExpiry?.let(handler::removeCallbacks); withdrawalExpiry = null
        pendingWithdrawalQuote = null
        withdrawalFlow?.invalidate(); withdrawalFlow = null
        withdrawalDetails?.dismiss(); withdrawalDetails = null
    }

    private fun hideKeyboard() {
        currentFocus?.let { getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(it.windowToken, 0) }
    }

    private fun showWithdrawalFeeDetails(quote: WithdrawalQuote, context: WalletUiRequest) {
        if (!transferViewCurrent(context)) return
        withdrawalDetails?.dismiss()
        val sheet = WithdrawalSheet(this, withdrawalPalette(), "Network fee", "Paid in ${quote.request.network.nativeSymbol} on ${quote.request.network.name}.")
        sheet.body.apply {
            infoRow(if (quote.fee.simulated) "Estimated cost" else "Conservative budget", feeText(quote.fee.totalEstimate, quote.request.network))
            val dollars = quote.estimatedFeeUsd(usdQuotes, System.currentTimeMillis())
            if (dollars.freshness == PriceFreshness.FRESH && dollars.amount != null) infoRow("USD estimate", "≈ ${UsdPrices.formatUsd(dollars.amount)}")
            infoRow("${quote.request.network.nativeSymbol} available", feeText(quote.fee.nativeAvailable, quote.request.network))
            infoRow("Fee reserve", feeText(quote.fee.reserve, quote.request.network))
            if (quote.request.network.feeProfile == NetworkFeeProfile.OP_STACK) {
                infoRow("L1 data estimate", "${Amounts.display(quote.fee.l1Estimate, 18)} ETH")
                infoRow("Operator estimate", "${Amounts.display(quote.fee.operatorEstimate, 18)} ETH")
            } else if (quote.request.network.feeProfile == NetworkFeeProfile.ARBITRUM) {
                label("The estimate includes execution and L1 data posting in a single fee.", 13, false, muted)
            }
            gap(8)
            label("Fees can change. The final review shows the amount and fee budget you approve.", 13, false, muted)
            if (quote.request.symbol != quote.request.network.nativeSymbol) label("No network fee is deducted from the $selectedAsset sent to the recipient.", 13, false, muted)
            smallButton("Done") { sheet.dismiss() }
        }
        sheet.setOnDismissListener { if (withdrawalDetails === sheet) withdrawalDetails = null }
        withdrawalDetails = sheet
        sheet.show()
    }

    private fun renderTrack() {
        // Entry is either a chosen Deposit network or an explicit Activity network choice.
        transferChain = selectedChain
        screen = "track"
        newPage("Track a deposit", network().name.uppercase())
        paragraph("Paste the transaction hash from the sending wallet or exchange. We check the receipt for this address on ${network().name}.", 16)
        val input = field(if (NetworkPolicy.family(network().chainId) == ChainFamily.EVM) "Transaction hash · 0x…" else "Transaction ID")
        button("Check transaction") {
            val selected = network()
            val hash = input.text.toString().trim()
            val selectedRepository = repository ?: return@button
            async("Looking up and verifying this transaction…", { selectedRepository.trackTransaction(selected, hash) }) { snapshot ->
                activityCache[selected.chainId] = snapshot
                renderHistory()
            }
        }
        paragraph("A connection failure means the transaction could not be checked. It does not mean the deposit failed.", 13, muted)
        button("Back to deposit", secondary = true) { renderReceive() }
    }

    private fun watch(input: EditText, changed: (String) -> Unit) {
        input.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}
            override fun afterTextChanged(value: Editable?) { changed(value?.toString().orEmpty()) }
        })
    }

    private fun renderReview(payment: ReviewedPayment) {
        screen = "review"
        val prepared = payment.objectValue
        val request = payment.request
        val symbol = request.getString("asset")
        val amount = request.getString("amount")
        val recipient = request.getString("to")
        newPage("Review transfer", payment.network.name)
        review = payment; reviewEditBlocked = false
        val context = transferViewRequest()
        var submissionClaimed = false
        val logo = FrameLayout(this).apply {
            addView(AssetGlyph(this@MainActivity, symbol), FrameLayout.LayoutParams(dp(46), dp(46), Gravity.CENTER))
        }
        page.addView(logo, rowParams(14).apply { height = dp(56) })
        paragraph("You're sending", 13, muted).gravity = Gravity.CENTER
        paragraph(amount, if (amount.length > 16) 24 else 37, ink, monospace = amount.length > 16).apply {
            gravity = Gravity.CENTER; typeface = Typeface.create(if (amount.length > 16) "monospace" else "sans-serif-medium", Typeface.NORMAL)
            fontFeatureSettings = "tnum"; setTextIsSelectable(true)
            breakStrategy = android.graphics.text.LineBreaker.BREAK_STRATEGY_SIMPLE
        }
        paragraph("$symbol · ${payment.network.name}", 14, muted).gravity = Gravity.CENTER
        val amountUsd = paragraph("", 13, muted).apply { gravity = Gravity.CENTER }
        card {
            label("Recipient", 12, false, muted)
            paragraph(recipient, 16, ink, monospace = true, parent = this).apply { setTextIsSelectable(true) }
            gap(12)
            infoRow("Network", payment.network.name)
            label("Verify the entire address and receiving network.", 12, false, muted)
        }
        val details = LinearLayout(this).apply {
            gravity = Gravity.CENTER_VERTICAL; minimumHeight = dp(52); setPadding(dp(2), dp(7), dp(2), dp(7))
            isFocusable = true; isClickable = true; filterTouchesWhenObscured = true
            contentDescription = "Transfer and fee details. Inspect the token contract, sender, gas and fee budget."
            addView(textView("Transfer & fee details", 14, ink, true), LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            addView(WalletGlyph(this@MainActivity, "chevron", muted), LinearLayout.LayoutParams(dp(15), dp(15)))
        }
        page.addView(details, rowParams(10))
        paragraph(when (payment.network.feeProfile) {
            NetworkFeeProfile.OP_STACK -> "The ETH reserve is a fee budget. ${payment.network.name} data and operator fees can change."
            NetworkFeeProfile.ARBITRUM -> "The ETH fee reserve includes execution and L1 data posting. The final fee can be lower."
            NetworkFeeProfile.ETHEREUM, NetworkFeeProfile.AVALANCHE, NetworkFeeProfile.POLYGON, NetworkFeeProfile.BNB -> "${payment.network.nativeSymbol} is reserved for network fees. The final fee can be lower."
        }, 12, muted)
        paragraph("Nothing is sent until you confirm. This review expires after five minutes.", 12, muted)
        val reserve = BigInteger(prepared.getString("fee_reserve_wei"))
        val dock = WithdrawalDock(this, withdrawalPalette(), "Fee reserve")
        dock.received.text = "$amount $symbol"
        val edit = textView("Edit details", 13, ink, true).apply {
            gravity = Gravity.CENTER; minHeight = dp(44); setPadding(dp(12), dp(12), dp(12), dp(7))
            background = touchBackground(Color.TRANSPARENT, 22)
            isFocusable = true; isClickable = true; filterTouchesWhenObscured = true
        }
        dock.addView(edit, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(4) })
        attachWithdrawalDock(dock)
        fun reviewCurrent() = !isDestroyed && transferViewCurrent(context) && review?.id == payment.id
        fun updateReview() {
            if (!reviewCurrent()) return
            val now = System.currentTimeMillis()
            val asset = PortfolioAsset(payment.network, payment.network.asset(symbol))
            val value = UsdPrices.value(asset, Amounts.parse(amount, asset.asset.decimals), usdQuotes, now)
            amountUsd.updateText(if (value.freshness == PriceFreshness.FRESH && value.amount != null) "≈ ${UsdPrices.formatUsd(value.amount)} USD" else "")
            val feeUsd = UsdPrices.value(PortfolioAsset(payment.network, payment.network.nativeAsset), reserve, usdQuotes, now)
            val dollarFee = if (feeUsd.freshness == PriceFreshness.FRESH && feeUsd.amount != null) "≈ ${UsdPrices.formatUsd(feeUsd.amount)} · " else ""
            dock.fee.updateText("$dollarFee${feeText(reserve, payment.network)}")
            dock.fee.contentDescription = "${payment.network.nativeSymbol} reserved for network fees: ${dock.fee.text}. Open fee details."
            val fresh = now / 1000L < prepared.getLong("expires_at")
            dock.button(when {
                reviewEditBlocked -> "Editing is paused"
                submissionClaimed -> "Sending…"
                !fresh -> "Review expired"
                else -> "Confirm and send"
            }, !reviewEditBlocked && !submissionClaimed && !busy && !authPending && fresh)
            edit.updateText(if (reviewEditBlocked) "Retry editing" else "Edit details")
            edit.isEnabled = !busy && !authPending && !submissionClaimed
        }
        details.setOnClickListener { if (reviewCurrent() && !busy && !authPending) showReviewedTransferDetails(payment, context) }
        dock.fee.setOnClickListener { if (reviewCurrent() && !busy && !authPending) showReviewedTransferDetails(payment, context) }
        edit.setOnClickListener { if (reviewCurrent() && !busy && !authPending && !submissionClaimed) cancelReviewForEditing() }
        dock.primary.setOnClickListener {
            if (!reviewCurrent() || busy || authPending || submissionClaimed || reviewEditBlocked) return@setOnClickListener
            if (System.currentTimeMillis() / 1000L >= prepared.getLong("expires_at")) { updateReview(); return@setOnClickListener }
            submissionClaimed = true; updateReview()
            val repo = repository!!
            async("Rechecking this exact transfer…", { runCatching { repo.revalidate(payment) } }) { result ->
                if (!reviewCurrent()) { resumeAfterDiscardedQuote(context); return@async }
                result.fold({ rechecked ->
                    review = rechecked
                    withVaultKey("Send $amount $symbol", "${payment.network.name}\nTo $recipient",
                        { key, stillAuthorized -> runCatching { repo.signAndBroadcast(rechecked, key, stillAuthorized) } },
                        cancelled = { runOnUiThread { if (reviewCurrent()) { submissionClaimed = false; updateReview() } } }) { outcome ->
                        unlocked = true
                        review = null
                        outcome.fold(::renderPaymentResult) { error -> renderSubmissionProblem(payment, error) }
                    }
                }, { error ->
                    submissionClaimed = false; showError(error); updateReview()
                })
            }
        }
        reviewControlsUpdate = { updateReview() }
        visibleDataUpdate = { updateReview() }
        val remaining = (prepared.getLong("expires_at") * 1000L - System.currentTimeMillis()).coerceAtLeast(0L)
        withdrawalExpiry = Runnable { if (reviewCurrent()) updateReview() }.also { handler.postDelayed(it, remaining + 1L) }
        updateReview()
    }

    private fun cancelReviewForEditing() {
        familyReview?.let { cancelFamilyReview(it); return }
        if (busy || authPending || !resumed || !unlocked) return
        val payment = review ?: run { openHistory(); return }
        val context = transferViewRequest()
        reviewEditBlocked = true
        reviewControlsUpdate?.invoke()
        val repo = repository!!
        async("Releasing this unsigned transfer…", { runCatching { repo.cancelPrepared(payment.network, payment.id) } }) { outcome ->
            if (!transferViewCurrent(context)) { resumeAfterDiscardedQuote(context); return@async }
            outcome.fold({
                review = null; reviewEditBlocked = false
                selectedChain = payment.network.chainId; transferChain = payment.network.chainId
                selectedAsset = payment.request.getString("asset")
                sendRecipient = payment.request.getString("to"); sendAmount = payment.request.getString("amount")
                saveSendDraft()
                renderSend()
            }, { error ->
                // An uncertain local cancellation must not restore a signing button or editable form.
                showError(error)
                reviewControlsUpdate?.invoke()
            })
        }
    }

    private fun showReviewedTransferDetails(payment: ReviewedPayment, context: WalletUiRequest) {
        if (!transferViewCurrent(context)) return
        withdrawalDetails?.dismiss()
        val prepared = payment.objectValue
        val request = payment.request
        val sheet = WithdrawalSheet(this, withdrawalPalette(), "Transfer details", "The exact transfer you are reviewing.")
        sheet.body.apply {
            infoRow("Network", payment.network.name)
            infoRow("Asset", request.getString("asset"))
            label("From", 12, false, muted)
            label(request.getString("from"), 13, false, ink, monospace = true)
            payment.network.asset(request.getString("asset")).contract?.let { contract ->
                gap(12); label("Token contract", 12, false, muted); label(contract, 13, false, ink, monospace = true)
            }
            gap(12)
            infoRow(if (payment.network.feeProfile == NetworkFeeProfile.ARBITRUM) "Combined gas fee limit" else "Execution fee limit", feeText(BigInteger(prepared.getString("max_fee_wei")), payment.network))
            if (payment.network.feeProfile == NetworkFeeProfile.OP_STACK) {
                infoRow("L1 data estimate", "${eth(request.getString("l1_fee_estimate_wei"))} ETH")
                infoRow("Operator estimate", "${eth(request.getString("operator_fee_estimate_wei"))} ETH")
                infoRow("Fee budget estimate", "${eth(prepared.getString("estimated_total_fee_wei"))} ETH")
            }
            infoRow("Fee reserve", feeText(BigInteger(prepared.getString("fee_reserve_wei")), payment.network))
            infoRow("Gas limit", request.getLong("gas_limit").toString())
            infoRow("Nonce", request.getLong("nonce").toString())
            gap(8)
            label(when (payment.network.feeProfile) {
                NetworkFeeProfile.OP_STACK -> "The fee budget includes the execution limit and estimated data/operator fees. The reserve is not an on-chain cap on ${payment.network.name}'s data or operator fees."
                NetworkFeeProfile.ARBITRUM -> "The gas fee limit includes execution and L1 data posting. The final fee is based on gas used; no separate L1 fee is added."
                NetworkFeeProfile.ETHEREUM, NetworkFeeProfile.AVALANCHE, NetworkFeeProfile.POLYGON, NetworkFeeProfile.BNB -> "The execution limit is a budget, not a prediction of the final fee."
            }, 12, false, muted)
            smallButton("Done") { sheet.dismiss() }
        }
        sheet.setOnDismissListener { if (withdrawalDetails === sheet) withdrawalDetails = null }
        withdrawalDetails = sheet
        sheet.show()
    }

    private fun renderSubmissionProblem(payment: ReviewedPayment, error: Throwable) {
        selectedChain = payment.network.chainId
        screen = "result"
        newPage("Check transfer status", payment.network.name)
        paragraph("Check Activity before trying again", 25, ink)
        paragraph("This send did not finish normally. Its saved record determines whether it can be cancelled or retried.", 15, muted)
        card {
            infoRow("Amount", "${payment.request.getString("amount")} ${payment.request.getString("asset")}")
            infoRow("Network", payment.network.name)
            label("Recipient", 12, false, muted)
            label(payment.request.getString("to"), 14, false, ink, monospace = true)
            gap(8); label(errorMessage(error), 13, false, warning)
        }
        button("View activity") { openHistory(reconcile = true) }
        linkButton("Back to wallet") { renderHome() }
    }

    private fun renderPaymentResult(record: JSONObject) {
        selectedChain = record.getLong("chain_id")
        screen = "result"
        val phase = record.getString("phase")
        val pending = phase == "pending"
        val signed = phase == "signed"
        newPage("Transfer status", network().name)
        val icon = FrameLayout(this).apply {
            background = rounded(elevated, 34)
            addView(WalletGlyph(this@MainActivity, "history", ink), FrameLayout.LayoutParams(dp(29), dp(29), Gravity.CENTER))
        }
        page.addView(icon, LinearLayout.LayoutParams(dp(68), dp(68)).apply { gravity = Gravity.CENTER_HORIZONTAL; topMargin = dp(36); bottomMargin = dp(15) })
        paragraph(if (pending) "Transfer submitted" else if (signed) "Ready to submit" else "Submission unconfirmed", 28, ink).gravity = Gravity.CENTER
        paragraph(if (pending) "Awaiting network confirmation"
            else if (signed) "Your signed transfer is saved on this device"
            else "The submission result is unknown", 14, muted).gravity = Gravity.CENTER
        card {
            infoRow("Amount", "${record.getString("amount")} ${record.getString("asset")}")
            infoRow("Network", network().name)
            label("Recipient", 12, false, muted)
            label(record.getString("to"), 14, false, ink, monospace = true)
            gap(10); label("Transaction", 12, false, muted)
            label(record.getString("hash"), 12, false, muted, monospace = true)
        }
        paragraph(if (pending) "Submission is not final confirmation. Activity checks the receipt and confirmation state."
            else if (signed) "Open Activity to check the connection and submit the same saved transaction."
            else "Open Activity before retrying. Any retry must use this same saved transaction, not create another payment.", 13, muted)
        button("View activity") { openHistory(reconcile = true) }
        linkButton("Back to wallet") { renderHome() }
    }

    private fun openHistory(reconcile: Boolean = false) {
        renderHistory()
        val repo = repository!!
        val networks = portfolioNetworks()
        async("Opening activity…", {
            networks.map { selected -> selected to Pair(runCatching { repo.history(selected) }, runCatching { repo.activitySnapshot(selected) }) }
        }) { results ->
            for ((selected, result) in results) {
                result.first.getOrNull()?.let { historyCache[selected.chainId] = it }
                result.second.getOrNull()?.let { activityCache[selected.chainId] = it }
                listOfNotNull(result.first.exceptionOrNull(), result.second.exceptionOrNull()).firstOrNull()?.let {
                    syncMessage[selected.chainId] = WalletDiagnostics.refreshMessage(selected.name, it)
                }
            }
            refreshVisibleScreen()
            if (reconcile) refresh()
        }
    }

    private fun phase(record: JSONObject): String = when (record.getString("phase")) {
        "prepared" -> "Unsigned review"
        "signed" -> "Submission unconfirmed"
        "unknown" -> "Submission outcome unknown"
        "pending" -> "Submitted · awaiting inclusion"
        "included" -> if (record.getString("execution") == "failed") "Execution failed · awaiting finality" else "Included · awaiting finality"
        "finalized" -> if (record.getString("execution") == "failed") "Failed · gas charged · finalized" else "Transfer finalized"
        "cancelled" -> "Unsigned review cancelled"
        else -> "Unresolved · investigation needed"
    }

    private fun chooseTrackingNetwork() {
        val networks = portfolioNetworks()
        val context = transferViewRequest()
        AlertDialog.Builder(this).setTitle("Deposit network")
            .setItems(networks.map { it.name }.toTypedArray()) { _, index ->
                if (transferViewCurrent(context)) { selectedChain = networks[index].chainId; renderTrack() }
            }
            .setNegativeButton("Cancel", null).show()
    }

    private fun renderHistory() {
        screen = "history"
        newPage("Activity", "", tabs = true)
        sectionTitle("Activity")
        val updateStatus = portfolioConnectionBanner()
        linkButton("Find a transaction") { chooseTrackingNetwork() }
        val content = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; isSaveEnabled = false }
        page.addView(content, rowParams(0))
        linkButton("Activity coverage") {
            val message = portfolioNetworks().joinToString("\n\n") { selected ->
                val snapshot = activityCache[selected.chainId]
                val coverage = if (NetworkPolicy.family(selected.chainId) == ChainFamily.EVM) {
                    snapshot?.fromBlock?.let { "Token scan: blocks $it–${snapshot.toBlock}." } ?: "Token history has not been refreshed."
                } else "Bounded recent transaction discovery; transfers are checked against transaction and block evidence."
                "${selected.name}\n" + coverage +
                    snapshot?.note?.let { "\n$it" }.orEmpty()
            }
            AlertDialog.Builder(this).setTitle("Activity coverage")
                .setMessage("$message\n\nEarlier deposits can be checked by transaction ID on the matching network. Internal EVM native-asset transfers may appear only in balances.")
                .setPositiveButton("Done", null).show()
        }
        var previous = ""
        visibleDataUpdate = {
            updateStatus()
            val records = portfolioNetworks().flatMap { selected -> historyCache[selected.chainId].orEmpty().map { selected to it } }
                .sortedByDescending { it.second.optLong("updated_at", 0) }
            val fingerprint = portfolioNetworks().joinToString("|") { selected ->
                val local = historyCache[selected.chainId].orEmpty().joinToString { record ->
                    listOf("intent_id", "phase", "execution", "hash", "amount", "asset", "to", "updated_at", "actual_fee_wei", "resolution_note").joinToString { record.opt(it)?.toString() ?: "null" }
                }
                val snapshot = activityCache[selected.chainId]
                "${selected.chainId}:$local:${snapshot?.rows}:${snapshot?.tracked}:${snapshot?.note}"
            }
            if (fingerprint != previous) {
                previous = fingerprint
                val scroll = pageScroll
                val priorY = scroll?.scrollY ?: 0
                content.removeAllViews()
                populateHistory(content, records)
                scroll?.post { if (screen == "history" && content.isAttachedToWindow) scroll.scrollTo(0, priorY) }
            }
        }
        refreshVisibleScreen()
    }

    private fun populateHistory(content: LinearLayout, records: List<Pair<WalletNetwork, JSONObject>>) {
        val localHashes = records.mapNotNull { (selected, record) -> record.optionalString("hash")?.let { selected.chainId to canonicalTransactionId(selected, it) } }.toSet()
        val incoming = portfolioNetworks().flatMap { selected -> activityCache[selected.chainId]?.rows.orEmpty()
            .filter { it.getString("direction") == "received" || (selected.chainId to canonicalTransactionId(selected, it.getString("transaction_hash"))) !in localHashes }
            .map { selected to it } }.sortedByDescending { it.second.optLong("timestamp", 0) }
        if (incoming.isNotEmpty()) {
            content.label("Verified transfers", 16, true)
            incoming.forEach { (selected, activity) -> receivedCard(activity, selected, content) }
        }
        val unresolved = portfolioNetworks().flatMap { selected -> activityCache[selected.chainId]?.tracked.orEmpty()
            .filter { it.getString("status") != "verified" }.map { selected to it } }
        if (unresolved.isNotEmpty()) {
            content.gap(14); content.label("Tracking", 16, true)
            for ((selected, tracked) in unresolved) card(content) {
                val message = when (tracked.getString("status")) {
                    "pending" -> "Pending confirmation"
                    "not_found" -> "Not found on ${selected.name}"
                    "failed" -> "Transaction failed on-chain"
                    "not_recipient" -> "No supported transfer for this address"
                    "conflict" -> "Conflicting finality evidence"
                    else -> "Couldn't check this transaction"
                }
                label(message, 15, true)
                label(selected.name, 12, false, muted)
                label(shortAddress(tracked.getString("hash")), 12, false, muted, monospace = true)
                if (tracked.getString("status") == "not_found") label("Check the hash and network. This is not proof that a transfer failed.", 12, false, muted)
            }
        }
        if (records.isNotEmpty()) { content.gap(14); content.label("Sent from this wallet", 16, true) }
        for ((selected, record) in records) card(content) {
            label("↑  ${record.getString("amount")} ${record.getString("asset")}", 20, true)
            label("${selected.name} · ${phase(record)}", 12, true, if (record.getString("execution") == "failed") warning else muted)
            label("To ${shortAddress(record.getString("to"))}", 12, false, muted, monospace = true)
            record.optionalString("actual_fee_wei")?.let { label("Fee paid: ${feeText(BigInteger(it), selected)}", 12, false, muted) }
            record.optionalString("actual_account_rent_units")?.let { if (BigInteger(it) > BigInteger.ZERO) label("Account rent: ${feeText(BigInteger(it), selected)}", 12, false, muted) }
            record.optionalString("resolution_note")?.let { label(it, 12, false, muted) }
            record.optionalString("hash")?.let { hash ->
                setOnClickListener { if (!busy && !authPending) showTransactionDetails(selected, record.getString("amount"), record.getString("asset"), record.getString("to"), hash, phase(record)) }
                isClickable = true; filterTouchesWhenObscured = true
            }
            when (record.getString("phase")) {
                "prepared" -> smallButton("Cancel unsigned review") {
                    selectedChain = selected.chainId
                    val repo = repository!!
                    async("Cancelling unsigned intent…", { repo.cancelPrepared(selected, record.getString("intent_id")); repo.history(selected) }) {
                        historyCache[selected.chainId] = it; refreshVisibleScreen()
                    }
                }
                "signed", "unknown", "pending" -> smallButton("Retry identical transaction") {
                    AlertDialog.Builder(this@MainActivity).setTitle("Resubmit saved payment?")
                        .setMessage("${record.getString("amount")} ${record.getString("asset")} on ${selected.name}\n\nTo ${record.getString("to")}\n\nThe app checks the chain, then resubmits only the same signed transaction.")
                        .setNegativeButton("Back", null).setPositiveButton("Retry") { _, _ ->
                            selectedChain = selected.chainId
                            val repo = repository!!
                            withVaultKey("Resubmit saved payment", "Authorize retrying this same signed transaction on ${selected.name}.", { _, stillAuthorized ->
                                repo.rebroadcast(selected, record.getString("intent_id"), stillAuthorized)
                            }) { unlocked = true; renderPaymentResult(it) }
                        }.show()
                }
            }
        }
        if (incoming.isEmpty() && records.isEmpty() && unresolved.isEmpty()) card(content) {
            label("Your activity will appear here", 19, true)
            label("Transfers across your supported networks.", 13, false, muted)
        }
    }

    private fun receivedCard(activity: JSONObject, selected: WalletNetwork = network(activity.getLong("chain_id")), parent: LinearLayout = page) {
        val direction = activity.getString("direction")
        val state = activity.getString("verification_state")
        val statusText = when (state) {
            "conflict" -> "Conflicting finality · needs investigation"
            "reorg" -> "Reorganized · no longer included"
            else -> if (activity.getString("inclusion") == "finalized") "Finalized" else "Included · awaiting finality"
        }
        val title = when (direction) { "received" -> "Received"; "sent" -> "Sent"; else -> "Self transfer" }
        card(parent) {
            val participant = if (direction == "received") activity.getString("from") else activity.getString("to")
            val row = LinearLayout(this@MainActivity).apply { gravity = Gravity.CENTER_VERTICAL }
            val mark = FrameLayout(this@MainActivity).apply {
                background = rounded(elevated, 21)
                addView(WalletGlyph(this@MainActivity, if (direction == "received") "receive" else "send", if (state == "observed" && direction == "received") blue else ink), FrameLayout.LayoutParams(dp(19), dp(19), Gravity.CENTER))
            }
            row.addView(mark, LinearLayout.LayoutParams(dp(40), dp(40)))
            val left = LinearLayout(this@MainActivity).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(11), 0, dp(6), 0) }
            left.label(title, 14, true)
            left.label("${activity.getString("asset")} · ${selected.name}", 12, false, muted)
            row.addView(left, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            val right = LinearLayout(this@MainActivity).apply { orientation = LinearLayout.VERTICAL; gravity = Gravity.END }
            val prefix = when (direction) { "received" -> "+"; "sent" -> "−"; else -> "" }
            right.addView(textView("$prefix${activity.getString("amount")}", 16, if (state == "observed" && direction == "received") blue else ink, true).apply { gravity = Gravity.END; fontFeatureSettings = "tnum" }, rowParams(0))
            right.addView(textView(statusText, 11, if (state == "observed") muted else warning).apply { gravity = Gravity.END }, rowParams(6))
            row.addView(right, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.35f))
            addView(row)
            setOnClickListener { if (!busy && !authPending) showTransactionDetails(selected, activity.getString("amount"), activity.getString("asset"), participant, activity.getString("transaction_hash"), statusText) }
            isClickable = true; isFocusable = true
            contentDescription = "$title ${activity.getString("amount")} ${activity.getString("asset")} on ${selected.name}. $statusText. Show transaction details."
            filterTouchesWhenObscured = true
        }
    }

    private fun shortAddress(value: String) = if (value.length > 20) value.take(9) + "…" + value.takeLast(7) else value
    private fun showTransactionDetails(selected: WalletNetwork, amount: String, asset: String, participant: String, hash: String, statusText: String) {
        AlertDialog.Builder(this).setTitle("$amount $asset").setMessage("${selected.name}\n$statusText\n\nAddress\n$participant\n\nTransaction hash\n$hash")
            .setNegativeButton("Close", null).setPositiveButton("Explorer") { _, _ -> startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(explorerTransaction(selected, hash)))) }.show()
    }

    private fun renderProfile() {
        if (!unlocked || manifest == null) { renderStartup(); return }
        screen = "profile"
        newPage("Profile", "", tabs = true)
        card {
            label(walletName, 23, true)
            gap(12)
            label("Public address", 12, false, muted)
            label(manifest!!.address, 12, false, muted, monospace = true)
            val address = manifest!!.address
            addView(iconControl("copy", "Copy public address") { copyAddress(address) },
                LinearLayout.LayoutParams(dp(44), dp(44)).apply { gravity = Gravity.END; topMargin = dp(6) })
        }
        button("Wallets", secondary = true, allowWhileBusy = true) { showWallets() }
        button("Add wallet", secondary = true, allowWhileBusy = true) { showWallets(add = true) }
        button("Settings", secondary = true) { renderSecurity() }
        button("Sign out", secondary = true, allowWhileBusy = true) { showWallets() }
    }

    private fun nameRow(label: String, value: String, action: () -> Unit) {
        val row = LinearLayout(this).apply {
            gravity = Gravity.CENTER_VERTICAL; minimumHeight = dp(74)
            setPadding(dp(18), dp(12), dp(14), dp(12)); background = touchBackground(surface, 18, line)
            isFocusable = true; isClickable = true; filterTouchesWhenObscured = true
            contentDescription = "$label: $value. Rename."
            val labels = LinearLayout(this@MainActivity).apply { orientation = LinearLayout.VERTICAL }
            labels.addView(textView(label, 12, muted))
            labels.addView(textView(value, 16, ink, true).apply { maxLines = 1; ellipsize = TextUtils.TruncateAt.END }, rowParams(5))
            addView(labels, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            addView(WalletGlyph(this@MainActivity, "edit", muted), LinearLayout.LayoutParams(dp(18), dp(18)).apply { marginStart = dp(12) })
            setOnClickListener { if (!busy && !authPending) action() }
        }
        page.addView(row, rowParams())
    }

    private fun renameWallet() {
        val address = manifest!!.address
        val started = epoch
        val input = EditText(this).apply {
            setText(walletName)
            setSingleLine(true); setSelection(text.length); textSize = 18f
            filters = arrayOf(InputFilter.LengthFilter(256))
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
            imeOptions = EditorInfo.IME_ACTION_DONE or EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
            isSaveEnabled = false; filterTouchesWhenObscured = true
            setTextColor(ink); setPadding(dp(20), dp(14), dp(20), dp(14))
        }
        val dialog = AlertDialog.Builder(this).setTitle("Rename wallet")
            .setView(input).setNegativeButton("Cancel", null).setPositiveButton("Save", null).create()
        dialog.setOnShowListener {
            dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener {
                if (started != epoch || !unlocked || !resumed || !address.equals(manifest?.address, true)) { dialog.dismiss(); return@setOnClickListener }
                val value = try { DisplayNameRules.normalize(input.text.toString()) } catch (error: WalletFailure) { input.error = error.message; return@setOnClickListener }
                dialog.dismiss()
                async("Saving name…", { displayNameStore.saveWallet(address, value) }) { saved -> walletName = saved; renderSecurity() }
            }
        }
        dialog.show()
    }

    private fun renderSecurity() {
        screen = "security"
        newPage("Settings", "")
        sectionTitle("Wallet")
        nameRow("Wallet name", walletName) { renameWallet() }
        button("Wallets", secondary = true, allowWhileBusy = true) { showWallets() }
        button("Add wallet", secondary = true, allowWhileBusy = true) { showWallets(add = true) }
        button("Sign out", secondary = true, allowWhileBusy = true) { showWallets() }
        sectionTitle("Display")
        val smallBalances = CheckBox(this).apply {
            text = "Hide small balances"; textSize = 16f; setTextColor(ink)
            buttonTintList = ColorStateList.valueOf(blue); isSaveEnabled = false
            isChecked = portfolioSettings.hideSmallBalances; minHeight = dp(48); filterTouchesWhenObscured = true
            setOnClickListener {
                val requested = isChecked
                isChecked = portfolioSettings.hideSmallBalances
                if (!busy && !authPending) {
                    val address = manifest!!.address
                    val threshold = portfolioSettings.thresholdUsd.toPlainString()
                    async("Saving display setting…", { portfolioPreferences.saveSettings(address, requested, threshold) }) { saved ->
                        portfolioSettings = saved; isChecked = saved.hideSmallBalances
                    }
                }
            }
        }
        page.addView(smallBalances, rowParams(0))
        val threshold = field("Hide assets below (USD)", decimal = true).apply {
            setText(portfolioSettings.thresholdUsd.toPlainString())
        }
        button("Save threshold", secondary = true) {
            val input = threshold.text.toString()
            val checked = try { UsdPrices.parseThreshold(input) } catch (error: WalletFailure) { threshold.error = error.message; null }
            if (checked != null) {
                val address = manifest!!.address
                val enabled = portfolioSettings.hideSmallBalances
                async("Saving USD threshold…", { portfolioPreferences.saveSettings(address, enabled, checked.toPlainString()) }) { saved ->
                    portfolioSettings = saved
                    if (threshold.text.toString() == input) threshold.setText(saved.thresholdUsd.toPlainString())
                }
            }
        }
        paragraph(WalletPortfolio.SMALL_BALANCE_DESCRIPTION, 12, muted)
        sectionTitle("Security")
        card {
            infoRow("App lock", manifest!!.lockMode.label)
            label(if (manifest!!.lockMode == AppLockMode.OFF) "App lock is off. Anyone who can open this app can send assets or view recovery words." else "App lock protects opening, sending and viewing recovery words.", 13, false, muted)
        }
        button("App lock", secondary = true) { renderAppLock() }
        button("Recovery phrase", secondary = true) { renderRecoveryPage() }
        if (manifest!!.lockMode != AppLockMode.OFF) button("Lock wallet", secondary = true, allowWhileBusy = true) { lock() }
        sectionTitle("Connections")
        button("TRON connection", secondary = true) { renderTronConnection() }
        button("Private transfers", secondary = true) { renderPrivateWorkspace() }
        sectionTitle("Wallet information")
        card {
            infoRow("Wallet", walletName)
            label(manifest!!.address, 12, false, muted, monospace = true)
            gap(8)
            infoRow("Networks", "9 mainnet networks")
            infoRow("Version", ZeroBrand.releaseLabel(this@MainActivity))
            label("Your recovery words restore this wallet if you lose your device or remove the app. An app PIN does not replace that backup.", 13, false, muted)
        }
        button("Export transaction history", secondary = true) {
            val selectedRepository = repository ?: return@button
            async("Preparing public audit records…", { selectedRepository.auditExport() }) { text ->
                val owner = activeWalletId ?: return@async
                val request = documentRequests.begin(owner, WalletDocumentKind.AUDIT_EXPORT)
                exportText = text
                try {
                    startActivityForResult(Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                        addCategory(Intent.CATEGORY_OPENABLE); type = "application/json"; putExtra(Intent.EXTRA_TITLE, "local-wallet-public-audit.json")
                    }, request)
                } catch (error: Exception) { documentRequests.cancel(request); exportText = null; throw error }
            }
        }
        paragraph("History export includes public addresses and transfers. It contains no recovery words or signing keys.", 12, muted)
        paragraph("Private keys and recovery words are not uploaded. Blockchain providers receive public address and transaction requests; Coinbase receives asset-price requests. Public transactions remain on the blockchain.", 12, muted)
        linkButton("Recovery compatibility") {
            AlertDialog.Builder(this).setTitle("Recovery compatibility")
                .setMessage("Standard English BIP-39 recovery words\n\nEVM: m/44'/60'/0'/0/0\nSolana: m/44'/501'/0'/0'\nTRON: m/44'/195'/0'/0/0\nBIP-39 passphrase: empty\n\nEach family has its own address. Choose the matching derivation path in a compatible wallet. The local transfer journal is specific to this installation.")
                .setPositiveButton("Done", null).show()
        }
        paragraph("Engineering release · Not independently audited", 11, muted).gravity = Gravity.CENTER
    }

    private fun renderTronConnection() {
        screen = "tron_connection"
        newPage("TRON connection", "")
        paragraph("TronGrid may require an API key to read balances and submit transfers. Use a key from your own TronGrid account.", 15, muted)
        paragraph("This key controls access to TronGrid. It cannot sign transfers or recover your wallet. It is encrypted on this device and sent only to api.trongrid.io.", 13, muted)
        val input = field("TronGrid API key", secret = true).apply {
            filters = arrayOf(InputFilter.LengthFilter(256))
            sensitiveViews.add(this)
            setSingleLine(true)
        }
        button("Save connection key") {
            val key = input.text.toString().trim()
            TronApiAccess.validate(key)
            input.text.clear()
            async("Saving the encrypted connection key…", { TronApiAccess(applicationContext).save(key) }) {
                renderSecurity()
                Toast.makeText(this, "TRON connection key saved", Toast.LENGTH_SHORT).show()
            }
        }
        button("Remove saved connection key", secondary = true) {
            input.text.clear()
            async("Removing the connection key…", { TronApiAccess(applicationContext).clear() }) { renderSecurity() }
        }
        linkButton("Open TronGrid") { startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("https://www.trongrid.io"))) }
        linkButton("Back to settings") { renderSecurity() }
    }

    private fun renderPrivateWorkspace() {
        if (!unlocked || manifest == null) { renderStartup(); return }
        val address = manifest!!.address
        val owner = activeWalletId ?: return
        val ownerSelection = selectionClaim
        if (!selectionAuthority.isCurrent(ownerSelection)) return
        if (privateWorkspaceOwner != owner || privateWorkspaceSelection != ownerSelection) {
            val old = privateWorkspace
            old?.setForeground(false)
            privateWorkspace = null
            if (old != null) work.execute { old.close() }
        }
        screen = "private_wallet"
        privateResumePending = true
        newPage("Private wallet", "")
        val workspace = privateWorkspace ?: PrivateWalletWorkspace(this, object : PrivateWorkspaceHost {
            override fun canAct(): Boolean = selectionAuthority.isCurrent(ownerSelection) &&
                activeWalletId == owner && manifest?.address == address &&
                screen == "private_wallet" && resumed && unlocked && !busy && !authPending
            override fun <T> work(label: String, task: () -> T, discard: (T) -> Unit, complete: (Result<T>) -> Unit) {
                if (!canAct()) return
                asyncWithDiscard(label, { runCatching(task) }, { result -> result.getOrNull()?.let(discard) }, complete)
            }
            override fun <T> authorized(label: String, task: (VaultManifest, ByteArray, () -> Boolean) -> T, discard: (T) -> Unit, complete: (Result<T>) -> Unit) {
                if (!canAct()) return
                val saved = manifest ?: return
                var authorizationStillValid: (() -> Boolean)? = null
                withVaultKey(label, "Authorize this private-wallet operation.", { key, valid ->
                    authorizationStillValid = valid
                    runCatching {
                        privateWorkspace?.authorizedForeground(valid)
                        task(saved, key, valid)
                    }
                }, discard = { result -> result.getOrNull()?.let(discard) },
                    operationAuthorized = { selectionAuthority.isCurrent(ownerSelection) &&
                        activeWalletId == owner && screen == "private_wallet" && manifest?.address == saved.address }) { result ->
                    if (authorizationStillValid?.invoke() == true) {
                        unlocked = true
                        privateWorkspace?.setForeground(true)
                        complete(result)
                    } else {
                        result.getOrNull()?.let(discard)
                        privateWorkspace?.setForeground(false)
                        if (resumed) initialize()
                    }
                }
            }
            override fun exportBackup(value: uniffi.wallet_core.PrivateRecoveryBackup) {
                if (!canAct()) { value.encryptedBytes.fill(0); return }
                val request = try { documentRequests.begin(owner, WalletDocumentKind.PRIVATE_EXPORT) }
                    catch (error: Exception) { value.encryptedBytes.fill(0); throw error }
                pendingPrivateBackup?.encryptedBytes?.fill(0)
                pendingPrivateBackup = value
                try {
                    startActivityForResult(Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                        addCategory(Intent.CATEGORY_OPENABLE); type = "application/octet-stream"
                        putExtra(Intent.EXTRA_TITLE, "local-wallet-private-backup.lwprivate")
                    }, request)
                } catch (error: Exception) {
                    documentRequests.cancel(request); value.encryptedBytes.fill(0); pendingPrivateBackup = null
                    throw error
                }
            }
            override fun chooseBackup() {
                if (!canAct()) return
                PrivatePaymentRelease.requireAvailable()
                val request = documentRequests.begin(owner, WalletDocumentKind.PRIVATE_IMPORT)
                try {
                    startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                        addCategory(Intent.CATEGORY_OPENABLE); type = "application/octet-stream"
                    }, request)
                } catch (error: Exception) { documentRequests.cancel(request); throw error }
            }
            override fun usd(units: BigInteger): String? {
                if (!selectionAuthority.isCurrent(ownerSelection) || activeWalletId != owner || manifest?.address != address) return null
                val selected = network(42161L)
                return UsdPrices.value(PortfolioAsset(selected, selected.asset("USDC")), units, usdQuotes, System.currentTimeMillis())
                    .amount?.let(UsdPrices::formatUsd)
            }
            override fun settings() { if (canAct()) { privateResumePending = false; renderSecurity() } }
        }).also { privateWorkspace = it; privateWorkspaceOwner = owner; privateWorkspaceSelection = ownerSelection }
        (workspace.view.parent as? ViewGroup)?.removeView(workspace.view)
        page.addView(workspace.view, rowParams(0))
        workspace.setForeground(resumed)
        workspace.render()
        visibleDataUpdate = { workspace.refreshVisible() }
        refreshPrices()
    }

    private fun continuePrivateImport() {
        if (!resumed || !unlocked || busy || authPending) return
        val encrypted = pendingPrivateImport ?: return
        pendingPrivateImport = null
        val saved = manifest ?: run { encrypted.fill(0); return }
        var importAuthorization: (() -> Boolean)? = null
        withVaultKey("Restore private backup", "Restore encrypted history into a new recovery namespace. Existing journals are retained.", { key, valid ->
            importAuthorization = valid
            try {
                if (!valid()) throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED)
                PrivateWalletSession.importBackup(applicationContext, saved, key, encrypted)
            } finally { encrypted.fill(0) }
        }, cancelled = { encrypted.fill(0) }, operationAuthorized = { screen == "private_wallet" && manifest?.address == saved.address }) {
            if (importAuthorization?.invoke() == true) {
                unlocked = true
                privateWorkspace?.setForeground(true)
                privateWorkspace?.imported()
            } else if (resumed) initialize()
        }
    }

    private fun renderAppLock() {
        screen = "app_lock"
        newPage("App lock", "")
        paragraph("Choose how this wallet opens. You can change this later.", 16, muted)
        val mode = manifest!!.lockMode
        card { infoRow("Current setting", mode.label) }
        button(if (mode == AppLockMode.OFF) "Off · selected" else "Turn app lock off", secondary = true) {
            if (mode != AppLockMode.OFF) changeAppLock(AppLockMode.OFF)
        }
        paragraph("Your private keys stay encrypted with Android Keystore. With app lock off, anyone who can open the app can use the wallet.", 12, muted)
        button(if (mode == AppLockMode.PIN) "Change 6-digit app PIN" else "Use a 6-digit app PIN", secondary = true) { renderSetPin() }
        paragraph("Exactly 6 numbers, separate from your phone's screen-lock PIN.", 12, muted)
        button(if (mode == AppLockMode.DEVICE) "Biometrics or device screen lock · selected" else "Use biometrics or device screen lock", secondary = true) {
            if (mode != AppLockMode.DEVICE) changeAppLock(AppLockMode.DEVICE)
        }
        paragraph("Uses Android's secure prompt: strong biometrics, or your phone's existing PIN, password or pattern. No new app code is created for this option.", 12, muted)
    }

    private fun pinField(label: String): EditText = field(label, secret = true).apply {
        hint = "6 digits"
        inputType = InputType.TYPE_CLASS_NUMBER or InputType.TYPE_NUMBER_VARIATION_PASSWORD
        filters = arrayOf(InputFilter.LengthFilter(6))
        sensitiveViews.add(this)
    }

    private fun renderSetPin() {
        screen = "set_pin"
        newPage("Create a 6-digit app PIN", "")
        paragraph("Use exactly 6 numbers. This PIN is only for zerø on this device.", 15, muted)
        val first = pinField("New 6-digit app PIN")
        val confirm = pinField("Confirm 6-digit app PIN")
        button("Save 6-digit app PIN") {
            val pin = first.text.toString().toCharArray()
            val repeated = confirm.text.toString().toCharArray()
            try {
                VaultPin.requireSixDigits(pin)
                if (!pin.contentEquals(repeated)) throw WalletFailure("The two 6-digit PINs do not match.")
                first.text.clear(); confirm.text.clear()
                changeAppLock(AppLockMode.PIN, pin.copyOf())
            } finally { pin.fill('\u0000'); repeated.fill('\u0000') }
        }
        paragraph("If you forget this PIN, your recovery words can restore the wallet on another installation. The app does not upload or recover your PIN.", 12, muted)
    }

    private fun changeAppLock(mode: AppLockMode, pin: CharArray? = null) {
        if (busy || authPending) { pin?.fill('\u0000'); return }
        val current = manifest ?: run { pin?.fill('\u0000'); return }
        val operationVault = vault
        val changeEpoch = epoch
        fun cleanup(enrollment: VaultEnrollment? = null) {
            pin?.fill('\u0000')
            enrollment?.let { operationVault.abortEnrollment(it) }
        }
        asyncWithDiscard("Preparing app-lock change…", {
            try { operationVault.beginProtectionChange(mode) }
            catch (error: Exception) { cleanup(); throw error }
        }, { cleanup(it) }) { enrollment ->
            fun commit(cipher: Cipher) {
                if (epoch != changeEpoch || !resumed) { cleanup(enrollment); return }
                withVaultKey("Change app lock", "Authorize changing how this wallet is protected.", { key, stillAuthorized ->
                    try {
                        if (!stillAuthorized() || epoch != changeEpoch) throw WalletFailure("App-lock change was interrupted.")
                        operationVault.changeProtection(current, key, enrollment, cipher, pin, stillAuthorized)
                    } finally { cleanup(enrollment) }
                }, cancelled = { cleanup(enrollment) }) { changed ->
                    manifest = changed
                    val previous = repository!!
                    repository = MultiNetworkRepository(operationVault, changed, { TronApiAccess(applicationContext).read() }).also { it.retainAuthenticatedAddresses(previous) }
                    work.execute { previous.close() }
                    unlocked = true
                    renderSecurity()
                    status.text = "App lock updated: ${changed.lockMode.label}"
                    status.visibility = View.VISIBLE
                    status.setTextColor(blue)
                }
            }
            if (epoch != changeEpoch || !resumed) cleanup(enrollment)
            else if (mode == AppLockMode.DEVICE) authorize("Enable device authentication", "Use your phone's biometrics or existing screen lock.", enrollment.cipher, cancelled = { cleanup(enrollment) }) { commit(it) }
            else commit(enrollment.cipher)
        }
    }

    private fun recoveryWordsGrid(parent: LinearLayout = page) {
        val grid = GridLayout(this).apply {
            columnCount = if (resources.configuration.fontScale > 1.4f) 1 else 2
            setPadding(dp(8), dp(8), dp(8), dp(8)); background = rounded(surface, 18, line)
        }
        repeat(recoverySession.wordCount) { index ->
            val cell = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(8), dp(7), dp(8), dp(7)); isSaveEnabled = false }
            cell.addView(textView("${index + 1}", 10, muted))
            val value = textView(recoverySession.word(index), 16, ink).apply {
                typeface = Typeface.MONOSPACE; setSingleLine(true)
                setAutoSizeTextTypeUniformWithConfiguration(13, 16, 1, android.util.TypedValue.COMPLEX_UNIT_SP)
            }
            sensitiveViews.add(value)
            cell.addView(value, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(3) })
            grid.addView(cell, GridLayout.LayoutParams().apply { width = 0; columnSpec = GridLayout.spec(GridLayout.UNDEFINED, 1f) })
        }
        parent.addView(grid, rowParams())
    }

    private fun allowPhraseCapture() {
        if (resumed && screen == "reveal" && recoverySession.isVisible) window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
    }

    private fun copyRecoveryWords() {
        if (!resumed || screen != "reveal" || !recoverySession.isVisible) return
        val token = UUID.randomUUID().toString()
        val clip = ClipData.newPlainText("zerø recovery phrase", recoverySession.clipboardText()).apply {
            description.extras = PersistableBundle().apply {
                putBoolean(if (Build.VERSION.SDK_INT >= 33) ClipDescription.EXTRA_IS_SENSITIVE else "android.content.extra.IS_SENSITIVE", true)
                putString("local_wallet_phrase_token", token)
            }
        }
        getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
        copiedPhraseToken = token
        Toast.makeText(this, "${recoverySession.wordCount} words copied. Keep them private.", Toast.LENGTH_SHORT).show()
        handler.postDelayed({ clearOwnedPhraseClipboard(token) }, 60_000)
    }

    private fun clearOwnedPhraseClipboard(token: String) {
        if (copiedPhraseToken != token) return
        val clipboard = getSystemService(ClipboardManager::class.java)
        runCatching {
            if (clipboard.primaryClipDescription?.extras?.getString("local_wallet_phrase_token") == token) clipboard.clearPrimaryClip()
        }
        copiedPhraseToken = null
    }

    @Deprecated("Platform document picker callback for API 30 compatibility")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (!selectionAuthority.isCurrent(selectionClaim)) {
            documentRequests.revoke()
            pendingPrivateBackup?.encryptedBytes?.fill(0); pendingPrivateBackup = null
            pendingPrivateImport?.fill(0); pendingPrivateImport = null
            exportText = null
            return
        }
        val document = documentRequests.consume(requestCode, activeWalletId) ?: return
        if (document == WalletDocumentKind.PRIVATE_EXPORT) {
            val backup = pendingPrivateBackup
            pendingPrivateBackup = null
            val destination = data?.data
            if (resultCode != RESULT_OK || backup == null || destination == null) {
                backup?.encryptedBytes?.fill(0)
                return
            }
            async("Saving encrypted private backup…", {
                try {
                    PrivateBackupFiles.writeAndVerify(applicationContext, destination, backup.encryptedBytes)
                } finally { backup.encryptedBytes.fill(0) }
            }) {
                Toast.makeText(this, "Encrypted private backup saved", Toast.LENGTH_SHORT).show()
                if (resumed && !unlocked) initialize()
            }
            return
        }
        if (document == WalletDocumentKind.PRIVATE_IMPORT) {
            val source = data?.data
            if (resultCode != RESULT_OK || source == null) return
            asyncWithDiscard("Reading encrypted private backup…", {
                contentResolver.openInputStream(source)?.use(PrivateBackupFiles::read)
                    ?: throw PrivatePaymentFailure(PrivateFailure.STORAGE)
            }, { it.fill(0) }) { encrypted ->
                pendingPrivateImport?.fill(0)
                pendingPrivateImport = encrypted
                privateResumePending = true
                if (resumed && unlocked) { renderPrivateWorkspace(); continuePrivateImport() }
                else if (resumed) initialize()
            }
            return
        }
        if (document == WalletDocumentKind.AUDIT_EXPORT) {
            val text = exportText
            exportText = null
            if (resultCode == RESULT_OK && text != null && data?.data != null) {
                try {
                    contentResolver.openOutputStream(data.data!!, "wt")?.use { it.write(text.toByteArray(Charsets.UTF_8)) }
                        ?: throw WalletFailure("The selected export location is unavailable.")
                    Toast.makeText(this, "Public audit exported", Toast.LENGTH_SHORT).show()
                } catch (error: Exception) { showError(error) }
            }
        }
    }

    private fun lock() {
        privateWorkspace?.setForeground(false)
        rememberResumeRoute()
        epoch++
        unlocked = false
        authCancel?.cancel()
        authCancel = null
        pinDialog?.cancel()
        pinDialog = null
        authPending = false
        pendingAuthorizedAction = null
        review = null
        clearSensitiveViews()
        if (activeWalletId != null && manifest != null) renderLocked() else renderSignedOut()
    }

    private fun clearSensitiveViews(systemAuthenticationPending: Boolean = false) {
        recoveryConfirmation?.cancel()
        clearSensitiveText()
        recoverySession.pause(systemAuthenticationPending)
        recoveryTimeout?.let(handler::removeCallbacks)
        recoveryTimeout = null
    }

    private fun clearSensitiveText() {
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        sensitiveViews.forEach { it.text = ""; it.contentDescription = null }
        sensitiveViews.clear()
    }
    private fun eth(value: String) = Amounts.display(BigInteger(value), 18)
    private fun feeText(units: BigInteger, network: WalletNetwork) = "${Amounts.display(units, network.nativeAsset.decimals)} ${network.nativeSymbol}"
    private fun canonicalTransactionId(network: WalletNetwork, value: String) = if (NetworkPolicy.family(network.chainId) == ChainFamily.EVM) value.lowercase(java.util.Locale.ROOT) else value
    private fun explorerTransaction(network: WalletNetwork, hash: String): String = if (network.chainId == FamilyNetworks.TRON) "https://tronscan.org/#/transaction/$hash" else "${network.explorerUrl.trimEnd('/')}/tx/$hash"
    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()
    private fun rounded(color: Int, radius: Int, border: Int? = null) = GradientDrawable().apply {
        setColor(color); cornerRadius = dp(radius).toFloat(); border?.let { setStroke(dp(1), it) }
    }
    private fun touchBackground(color: Int, radius: Int, border: Int? = null) = RippleDrawable(ColorStateList.valueOf(Color.argb(28, 170, 181, 198)), rounded(color, radius, border), rounded(Color.WHITE, radius))
    private fun rowParams(top: Int = 12) = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(top) }
    private fun textView(text: String, size: Int, color: Int = ink, bold: Boolean = false) = TextView(this).apply {
        this.text = text; textSize = size.toFloat(); setTextColor(color); isSaveEnabled = false; includeFontPadding = false
        typeface = Typeface.create(if (bold) "sans-serif-medium" else "sans-serif", Typeface.NORMAL)
        setLineSpacing(dp(3).toFloat(), 1f)
    }
    private fun iconControl(glyph: String, description: String, action: () -> Unit): View = FrameLayout(this).apply {
        contentDescription = description; isFocusable = true; isClickable = true; filterTouchesWhenObscured = true
        background = touchBackground(Color.TRANSPARENT, 22)
        addView(WalletGlyph(this@MainActivity, glyph, muted), FrameLayout.LayoutParams(dp(21), dp(21), Gravity.CENTER))
        setOnClickListener { if (!busy && !authPending) { try { action() } catch (error: Exception) { showError(error) } } }
    }
    private fun copyAddress(address: String, networkName: String? = null) {
        getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText(networkName?.let { "$it receiving address" } ?: "Receiving address", address))
        Toast.makeText(this, networkName?.let { "Address copied · $it" } ?: "Address copied", Toast.LENGTH_SHORT).show()
    }
    private fun assetName(symbol: String) = when (symbol) { "USDC" -> "USD Coin"; "EURC" -> "Euro Coin"; "USDT" -> "Tether USD"; "SOL" -> "Solana"; "TRX" -> "TRON"; "AVAX" -> "Avalanche"; "POL" -> "Polygon"; "BNB" -> "BNB"; else -> "Ether" }
    private fun TextView.updateText(value: String) { if (text.toString() != value) text = value }

    private fun heroAmount(text: String): TextView {
        val view = textView(text, 50, ink, true).apply {
            letterSpacing = -0.035f; fontFeatureSettings = "tnum"; setSingleLine(true)
            setAutoSizeTextTypeUniformWithConfiguration(26, 50, 1, android.util.TypedValue.COMPLEX_UNIT_SP)
            minHeight = dp(66)
        }
        page.addView(view, rowParams(0))
        return view
    }

    private fun transferAmount(amount: String, symbol: String) {
        if (amount.length <= 16) heroAmount(amount)
        else paragraph(amount, 27, ink, monospace = true).apply {
            setTextIsSelectable(true); setSingleLine(false); breakStrategy = android.graphics.text.LineBreaker.BREAK_STRATEGY_SIMPLE
            // Long exact ETH amounts wrap; they are never rounded or ellipsized on approval pages.
        }
        paragraph(symbol, 15, muted)
    }

    private fun walletActions(send: () -> Unit, deposit: () -> Unit) {
        val row = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL }
        for ((index, item) in listOf(Triple("DEPOSIT", "receive", deposit), Triple("SEND", "send", send)).withIndex()) {
            val primary = index == 1
            val foreground = if (primary) Color.WHITE else ink
            val action = LinearLayout(this).apply {
                gravity = Gravity.CENTER; minimumHeight = dp(56)
                setPadding(dp(10), dp(12), dp(10), dp(12))
                background = touchBackground(if (primary) ink else Color.rgb(238, 243, 232), 28, if (primary) null else Color.rgb(222, 230, 215))
                isClickable = true; isFocusable = true; contentDescription = item.first
                filterTouchesWhenObscured = true
                addView(WalletGlyph(this@MainActivity, item.second, foreground), LinearLayout.LayoutParams(dp(20), dp(20)))
                addView(textView(item.first, 13, foreground, true).apply { letterSpacing = 0.055f }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { marginStart = dp(9) })
                setOnClickListener { if (!busy && !authPending) { try { item.third() } catch (error: Exception) { showError(error) } } }
            }
            row.addView(action, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { if (index == 0) marginEnd = dp(6) else marginStart = dp(6) })
        }
        page.addView(row, rowParams(25))
    }

    private data class AssetRowViews(val amount: TextView, val note: TextView, val row: View, val divider: View)

    private fun portfolioAssetRow(position: PortfolioAsset, action: () -> Unit): AssetRowViews {
        val amount = textView("—", 17, ink, true).apply {
            gravity = Gravity.END; fontFeatureSettings = "tnum"; setSingleLine(true)
            setAutoSizeTextTypeUniformWithConfiguration(12, 17, 1, android.util.TypedValue.COMPLEX_UNIT_SP)
        }
        val note = textView(position.asset.symbol, 11, muted).apply { gravity = Gravity.END; maxLines = 3 }
        val row = LinearLayout(this).apply {
            gravity = Gravity.CENTER_VERTICAL; minimumHeight = dp(84)
            setPadding(0, dp(15), 0, dp(15)); background = touchBackground(Color.TRANSPARENT, 12)
            isFocusable = true; isClickable = true; filterTouchesWhenObscured = true
            val icon = FrameLayout(this@MainActivity)
            icon.addView(AssetGlyph(this@MainActivity, position.asset.symbol), FrameLayout.LayoutParams(dp(40), dp(40)))
            val badge = FrameLayout(this@MainActivity).apply {
                background = rounded(this@MainActivity.background, 10)
                addView(AssetGlyph(this@MainActivity, position.network.glyph), FrameLayout.LayoutParams(dp(14), dp(14), Gravity.CENTER))
            }
            icon.addView(badge, FrameLayout.LayoutParams(dp(18), dp(18), Gravity.END or Gravity.BOTTOM))
            addView(icon, LinearLayout.LayoutParams(dp(44), dp(44)))
            val name = LinearLayout(this@MainActivity).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(12), 0, dp(8), 0) }
            name.addView(textView(position.asset.symbol, 16, ink, true))
            name.addView(textView(position.network.name, 12, muted), rowParams(5))
            addView(name, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            val value = LinearLayout(this@MainActivity).apply { orientation = LinearLayout.VERTICAL; gravity = Gravity.END }
            value.addView(amount, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(26)))
            value.addView(note, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(4) })
            addView(value, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.1f))
            setOnClickListener { if (!busy && !authPending) { try { action() } catch (error: Exception) { showError(error) } } }
        }
        page.addView(row, rowParams(0))
        val divider = View(this).apply { setBackgroundColor(line) }
        page.addView(divider, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(1)))
        return AssetRowViews(amount, note, row, divider)
    }

    private fun updateAssetRow(position: PortfolioAsset, views: AssetRowViews) {
        val snapshot = balanceCache[position.network.chainId]
        val amount = snapshot?.balances?.get(position.asset.symbol)
        val usd = WalletPortfolio.usdValue(position, amount, usdQuotes, System.currentTimeMillis())
        val shown = if (balancesHidden) "••••" else usd.amount?.let(UsdPrices::formatUsd) ?: "—"
        views.amount.updateText(shown)
        val quantity = if (balancesHidden) "•••• ${position.asset.symbol}" else amount?.let { "${Amounts.display(it, position.asset.decimals)} ${position.asset.symbol}" } ?: "Balance unavailable"
        val note = when {
            amount == null -> ""
            usd.freshness == PriceFreshness.UNAVAILABLE -> "\nUSD price unavailable"
            usd.freshness == PriceFreshness.STALE || syncState[position.network.chainId] != "live" -> "\nLast checked"
            else -> ""
        }
        views.note.updateText(quantity + note)
        views.row.contentDescription = "${position.label}. USD estimate $shown. ${views.note.text}. Open asset."
    }

    private fun assetChoiceRow(symbol: String, action: () -> Unit) {
        val row = LinearLayout(this).apply {
            gravity = Gravity.CENTER_VERTICAL; minimumHeight = dp(86)
            setPadding(0, dp(17), 0, dp(17)); background = touchBackground(Color.TRANSPARENT, 12)
            isClickable = true; isFocusable = true; filterTouchesWhenObscured = true
            contentDescription = "$symbol. ${assetName(symbol)}. Choose asset."
            addView(AssetGlyph(this@MainActivity, symbol), LinearLayout.LayoutParams(dp(42), dp(42)))
            val name = LinearLayout(this@MainActivity).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(14), 0, dp(10), 0) }
            name.addView(textView(symbol, 17, ink, true))
            name.addView(textView(assetName(symbol), 13, muted), rowParams(6))
            addView(name, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            addView(WalletGlyph(this@MainActivity, "chevron", muted), LinearLayout.LayoutParams(dp(18), dp(18)))
            setOnClickListener { if (!busy && !authPending) { try { action() } catch (error: Exception) { showError(error) } } }
        }
        page.addView(row, rowParams(0))
        page.addView(View(this).apply { setBackgroundColor(line) }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(1)))
    }

    private fun assetPicker(symbol: String, action: String) {
        val context = transferViewRequest()
        paragraph("Asset", 12, muted)
        val row = LinearLayout(this).apply {
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(15), dp(14), dp(15), dp(14)); background = touchBackground(surface, 18, line)
            isFocusable = true; filterTouchesWhenObscured = true; contentDescription = "Asset: $symbol. Change asset."
            addView(AssetGlyph(this@MainActivity, symbol), LinearLayout.LayoutParams(dp(36), dp(36)))
            val name = LinearLayout(this@MainActivity).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(12), 0, 0, 0) }
            name.label(symbol, 16, true); name.label(assetName(symbol), 12, false, muted)
            addView(name, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            addView(WalletGlyph(this@MainActivity, "down", muted), LinearLayout.LayoutParams(dp(17), dp(17)))
            setOnClickListener { if (!busy && !authPending && transferViewCurrent(context)) renderAssetSelection(action) }
        }
        page.addView(row, rowParams(7))
    }

    private fun networkPicker(symbol: String, action: String, selected: WalletNetwork?) {
        val context = transferViewRequest()
        paragraph("Network", 12, muted).apply { setPadding(0, dp(5), 0, 0) }
        val row = LinearLayout(this).apply {
            gravity = Gravity.CENTER_VERTICAL; minimumHeight = dp(58)
            setPadding(dp(16), dp(14), dp(16), dp(14)); background = touchBackground(surface, 18, line)
            isClickable = true; isFocusable = true; filterTouchesWhenObscured = true
            contentDescription = "Network: ${selected?.name ?: "not selected"}. Choose a network for $symbol."
            if (selected != null) addView(AssetGlyph(this@MainActivity, selected.glyph),
                LinearLayout.LayoutParams(dp(25), dp(25)).apply { marginEnd = dp(12) })
            addView(textView(selected?.name ?: "Select network", 16, if (selected == null) muted else ink, selected != null),
                LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            addView(WalletGlyph(this@MainActivity, "down", muted), LinearLayout.LayoutParams(dp(17), dp(17)))
        }
        row.setOnClickListener {
            if (!busy && !authPending && transferViewCurrent(context)) showNetworkPicker(symbol, action, selected, context)
        }
        page.addView(row, rowParams(7))
    }

    private fun showNetworkPicker(symbol: String, action: String, selected: WalletNetwork?, context: WalletUiRequest) {
        if (!transferViewCurrent(context) || busy || authPending) return
        dismissNetworkDropdown()
        val networks = AssetSelection.networksFor(repository!!.networks, symbol)
        val sheet = WithdrawalSheet(this, withdrawalPalette(), "Choose network",
            if (action == "send") "Match the network used by the receiving wallet."
            else "Match the network used by the wallet or exchange sending your deposit.")
        networks.forEach { network ->
            val chosen = network.chainId == selected?.chainId
            val balance = balanceCache[network.chainId]?.balances?.get(symbol)
            val amount = balance?.let { "${Amounts.display(it, network.asset(symbol).decimals)} $symbol" }
            val row = LinearLayout(this).apply {
                gravity = Gravity.CENTER_VERTICAL; minimumHeight = dp(92)
                setPadding(dp(16), dp(15), dp(16), dp(15)); background = touchBackground(if (chosen) elevated else surface, 18, if (chosen) null else line)
                isClickable = true; isFocusable = true; isSelected = chosen; filterTouchesWhenObscured = true; isSaveEnabled = false
                addView(AssetGlyph(this@MainActivity, network.glyph), LinearLayout.LayoutParams(dp(34), dp(34)).apply { marginEnd = dp(14) })
                val labels = LinearLayout(this@MainActivity).apply {
                    orientation = LinearLayout.VERTICAL
                    label(network.name, 17, true)
                    label(amount?.let { "${if (syncState[network.chainId] == "live") "Available" else "Last checked"} · $it" } ?: "Balance unavailable", 12, false, muted)
                }
                addView(labels, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(12) })
                val choice = FrameLayout(this@MainActivity).apply {
                    background = rounded(if (chosen) ink else surface, 12, if (chosen) null else line)
                    if (chosen) addView(WalletGlyph(this@MainActivity, "check", surface), FrameLayout.LayoutParams(dp(15), dp(15), Gravity.CENTER))
                }
                addView(choice, LinearLayout.LayoutParams(dp(23), dp(23)))
                contentDescription = "${network.name}. ${amount ?: "Balance unavailable"}.${if (chosen) " Selected." else " Choose network."}"
                setOnClickListener {
                    if (transferViewCurrent(context) && !busy && !authPending) {
                        if (chosen) sheet.dismiss()
                        else try { selectTransferNetwork(symbol, action, network.chainId) } catch (error: Exception) { showError(error) }
                    }
                }
            }
            sheet.body.addView(row, rowParams(10))
        }
        sheet.body.apply {
            gap(12)
            label("Fees are paid in the selected network’s native asset.", 12, false, muted)
            smallButton("Cancel") { sheet.dismiss() }
        }
        sheet.setOnDismissListener { if (networkDropdown === sheet) networkDropdown = null }
        networkDropdown = sheet
        sheet.show()
    }

    private fun dismissNetworkDropdown() {
        networkDropdown?.dismiss()
        networkDropdown = null
    }

    private fun linkButton(text: String, action: () -> Unit) {
        page.addView(textView(text, 13, ink, true).apply {
            gravity = Gravity.CENTER; minHeight = dp(48); isClickable = true; isFocusable = true
            background = touchBackground(Color.TRANSPARENT, 12); setPadding(dp(12), dp(10), dp(12), dp(10))
            filterTouchesWhenObscured = true
            setOnClickListener { if (!busy && !authPending) { try { action() } catch (error: Exception) { showError(error) } } }
        }, rowParams(6))
    }

    private fun newPage(title: String, subtitle: String, tabs: Boolean = false) {
        if (screen != "private_wallet") {
            privateWorkspace?.setForeground(false)
            if (screen !in setOf("locked", "opening")) privateResumePending = false
        }
        dismissNetworkDropdown()
        invalidateWithdrawalForm()
        visibleDataUpdate = null
        reviewControlsUpdate = null
        familyReview = null
        pageScroll = null
        clearSensitiveViews()
        recoveryContent = null
        pageSequence++
        val shell = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setBackgroundColor(this@MainActivity.background); isSaveEnabled = false }
        pageShell = shell
        val header = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL; setPadding(dp(20), dp(10), dp(14), dp(8))
            minimumHeight = dp(74)
        }
        val onboarding = screen in setOf("welcome", "locked", "restore", "wallets", "wallet_transition")
        if (tabs) {
            val mark = FrameLayout(this).apply {
                background = rounded(elevated, 20)
                addView(ZeroBrand.mark(this@MainActivity, ink, 31f), FrameLayout.LayoutParams(dp(30), dp(36), Gravity.CENTER))
            }
            header.addView(mark, LinearLayout.LayoutParams(dp(40), dp(40)).apply { marginEnd = dp(12) })
            val identity = LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL; isClickable = true; isFocusable = true; filterTouchesWhenObscured = true
                contentDescription = if (screen == "profile") "Profile" else "$walletName. Open profile."
                setOnClickListener { if (!busy && !authPending) renderProfile() }
            }
            identity.addView(textView(if (screen == "profile") "Profile" else walletName, 17, ink, true).apply { maxLines = 1; ellipsize = TextUtils.TruncateAt.END })
            header.addView(identity, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            if (screen != "profile") header.addView(iconControl("profile", "Profile") { renderProfile() }, LinearLayout.LayoutParams(dp(44), dp(44)))
        } else if (screen in setOf("welcome", "locked", "wallets", "wallet_transition")) {
            header.addView(ZeroBrand.wordmark(this, ink, 31f).apply { gravity = Gravity.START or Gravity.CENTER_VERTICAL; setPadding(0, 0, 0, 0) },
                LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            header.addView(textView("SELF CUSTODY", 9, muted).apply { letterSpacing = 0.1f })
        } else {
            header.addView(iconControl("back", "Back") {
                when (screen) {
                    "asset_picker" -> returnFromAssetPicker()
                    "restore" -> { clearSensitiveViews(); if (savedWallets.isEmpty()) renderWelcome() else renderAddWallet() }
                    "add_wallet" -> renderSignedOut()
                    "fee_preview" -> renderSend()
                    "review" -> cancelReviewForEditing()
                    "track" -> renderReceive()
                    "reveal" -> { clearSensitiveViews(); renderSecurity() }
                    "security" -> renderProfile()
                    "private_wallet" -> { privateResumePending = false; renderSecurity() }
                "tron_connection" -> renderSecurity()
                    "app_lock" -> renderSecurity()
                    "set_pin" -> renderAppLock()
                    else -> { review = null; renderHome() }
                }
            }, LinearLayout.LayoutParams(dp(44), dp(44)).apply { marginStart = dp(-10) })
            val labels = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
            labels.addView(textView(if (onboarding) "Restore wallet" else title, 18, ink, true))
            if (subtitle.isNotBlank() && subtitle.uppercase() in setOf("BASE", "ETHEREUM")) labels.addView(textView(subtitle.lowercase().replaceFirstChar { it.uppercase() }, 12, muted), rowParams(4))
            header.addView(labels, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginStart = dp(5) })
        }
        shell.addView(header, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        val scroll = ScrollView(this).apply { isFillViewport = true; isSaveEnabled = false; clipToPadding = false }
        pageScroll = scroll
        page = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(24), dp(6), dp(24), dp(28)); isSaveEnabled = false }
        scroll.addView(page)
        shell.addView(scroll, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        shell.setOnApplyWindowInsetsListener { view, insets ->
            val system = insets.getInsets(android.view.WindowInsets.Type.systemBars() or android.view.WindowInsets.Type.displayCutout())
            val keyboard = insets.getInsets(android.view.WindowInsets.Type.ime())
            view.setPadding(system.left, system.top, system.right, maxOf(system.bottom, keyboard.bottom))
            insets
        }
        if (tabs) {
            shell.addView(View(this).apply { setBackgroundColor(line) }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(1)))
            val bar = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL; setPadding(dp(24), dp(6), dp(24), dp(6)); setBackgroundColor(this@MainActivity.background) }
            val entries = listOf("Wallet" to "home", "Activity" to "history", "Profile" to "profile")
            for ((label, target) in entries) bar.addView(LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL; gravity = Gravity.CENTER; minimumHeight = dp(56)
                background = touchBackground(Color.TRANSPARENT, 14); isFocusable = true
                addView(WalletGlyph(this@MainActivity, when (target) { "home" -> "wallet"; "profile" -> "profile"; else -> "history" }, if (screen == target) ink else muted), LinearLayout.LayoutParams(dp(22), dp(22)))
                addView(textView(label, 11, if (screen == target) ink else muted, screen == target), LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(6) })
                filterTouchesWhenObscured = true; contentDescription = "$label${if (screen == target) ", selected" else ""}"
                setOnClickListener {
                    if (busy || authPending || !unlocked) return@setOnClickListener
                    if (screen == target) return@setOnClickListener
                    when (target) { "home" -> renderHome(); "profile" -> renderProfile(); else -> openHistory() }
                }
            }, LinearLayout.LayoutParams(0, dp(60), 1f))
            shell.addView(bar)
        }
        showPageContent(shell)
        if (onboarding) paragraph(title, 31, ink).apply { typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL); setPadding(0, dp(34), 0, dp(4)); letterSpacing = -0.025f }
        status = textView("", 13, muted).apply {
            visibility = View.GONE; accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
            background = rounded(surface, 12); setPadding(dp(14), dp(12), dp(14), dp(12))
        }
        page.addView(status, rowParams())
    }

    private fun sectionTitle(text: String) {
        paragraph(text, 18, ink).apply { typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL); setPadding(0, dp(16), 0, dp(6)) }
    }

    private fun actionRow(first: String, firstAction: () -> Unit, second: String, secondAction: () -> Unit, secondary: Boolean = false) {
        val row = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        for ((index, item) in listOf(first to firstAction, second to secondAction).withIndex()) {
            row.addView(Button(this).apply {
                text = item.first; isAllCaps = false; textSize = 15f; minHeight = dp(52); setTextColor(if (!secondary && index == 0) this@MainActivity.background else ink)
                background = touchBackground(if (!secondary && index == 0) ink else surface, 27, if (!secondary && index == 0) null else line)
                setPadding(dp(12), dp(14), dp(12), dp(14))
                filterTouchesWhenObscured = true
                setOnClickListener { if (!busy && !authPending) { try { item.second() } catch (error: Exception) { showError(error) } } }
            }, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { if (index == 0) marginEnd = dp(5) else marginStart = dp(5) })
        }
        page.addView(row, rowParams())
    }

    private fun paragraph(text: String, size: Int = 16, color: Int = ink, monospace: Boolean = false, parent: LinearLayout = page): TextView {
        val view = textView(text, size, color).apply {
            if (monospace) typeface = Typeface.MONOSPACE
        }
        parent.addView(view, rowParams())
        return view
    }

    private fun button(text: String, secondary: Boolean = false, allowWhileBusy: Boolean = false, parent: LinearLayout = page, action: () -> Unit) {
        val view = Button(this).apply {
            this.text = text; textSize = 16f; isAllCaps = false; minHeight = dp(54)
            setTextColor(if (secondary) ink else this@MainActivity.background)
            typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
            background = touchBackground(if (secondary) surface else ink, 28, if (secondary) line else null)
            setPadding(dp(16), dp(10), dp(16), dp(10))
            filterTouchesWhenObscured = true
            setOnClickListener { if (allowWhileBusy || (!busy && !authPending)) { try { action() } catch (error: Exception) { showError(error) } } }
            isSaveEnabled = false
        }
        parent.addView(view, rowParams())
    }

    private fun field(hint: String, secret: Boolean = false, multiline: Boolean = false, decimal: Boolean = false): EditText {
        val label = paragraph(hint, 13, muted)
        return EditText(this).apply {
            id = View.generateViewId(); label.labelFor = id
            this.hint = if (decimal) "0.00" else if (secret) "" else "0x…"
            textSize = 17f
            minHeight = dp(if (multiline) 144 else 56)
            setPadding(dp(14), dp(12), dp(14), dp(12))
            setTextColor(ink)
            setHintTextColor(muted)
            background = rounded(surface, 14, line)
            inputType = when {
                decimal -> InputType.TYPE_CLASS_NUMBER or InputType.TYPE_NUMBER_FLAG_DECIMAL
                secret -> InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD or (if (multiline) InputType.TYPE_TEXT_FLAG_MULTI_LINE else 0)
                else -> InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS or InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
            }
            // setInputType can change TextView flags; apply multiline gravity afterwards.
            if (multiline) gravity = Gravity.TOP or Gravity.START
            imeOptions = EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING or (if (multiline) EditorInfo.IME_ACTION_NONE else EditorInfo.IME_ACTION_DONE)
            importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
            isSaveEnabled = false
            filterTouchesWhenObscured = true
            page.addView(this, rowParams())
        }
    }

    private fun card(parent: LinearLayout = page, content: LinearLayout.() -> Unit) {
        val container = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL; setPadding(dp(18), dp(17), dp(18), dp(17)); background = rounded(surface, 18, line)
            isSaveEnabled = false
            content()
        }
        parent.addView(container, rowParams())
    }

    private fun LinearLayout.label(text: String, size: Int, bold: Boolean = false, color: Int = ink, monospace: Boolean = false) {
        addView(TextView(this@MainActivity).apply {
            this.text = text; textSize = size.toFloat(); setTextColor(color); setPadding(0, dp(3), 0, dp(3)); setLineSpacing(dp(2).toFloat(), 1f)
            typeface = if (monospace) Typeface.MONOSPACE else Typeface.create(if (bold) "sans-serif-medium" else "sans-serif", Typeface.NORMAL)
            isSaveEnabled = false
        }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
    }
    private fun LinearLayout.infoRow(label: String, value: String, valueColor: Int = ink) {
        val row = LinearLayout(this@MainActivity).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL; setPadding(0, dp(8), 0, dp(8)) }
        row.addView(textView(label, 13, muted), LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        row.addView(textView(value, 14, valueColor, true).apply { gravity = Gravity.END }, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.2f))
        addView(row, rowParams(0))
    }
    private fun LinearLayout.gap(height: Int) { addView(View(this@MainActivity), LinearLayout.LayoutParams(1, dp(height))) }
    private fun LinearLayout.smallButton(text: String, action: () -> Unit) {
        addView(Button(this@MainActivity).apply {
            this.text = text; isAllCaps = false; textSize = 14f; setTextColor(ink); minHeight = dp(48)
            background = touchBackground(elevated, 24, line)
            filterTouchesWhenObscured = true; isSaveEnabled = false
            setOnClickListener { if (!busy && !authPending) { try { action() } catch (error: Exception) { showError(error) } } }
        }, rowParams())
    }
}
