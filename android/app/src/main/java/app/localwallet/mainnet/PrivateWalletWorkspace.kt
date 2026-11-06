package app.localwallet.mainnet

import android.app.Activity
import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipboardManager
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import android.widget.Button
import android.widget.EditText
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.MultiFormatWriter
import uniffi.wallet_core.PrivateRecoveryBackup
import java.math.BigInteger

internal interface PrivateWorkspaceHost {
    fun canAct(): Boolean
    fun <T> work(label: String, task: () -> T, discard: (T) -> Unit = {}, complete: (Result<T>) -> Unit)
    fun <T> authorized(label: String, task: (VaultManifest, ByteArray, () -> Boolean) -> T, discard: (T) -> Unit = {}, complete: (Result<T>) -> Unit)
    fun exportBackup(value: PrivateRecoveryBackup)
    fun chooseBackup()
    fun usd(units: BigInteger): String?
    fun settings()
}

/** Minimal native private-wallet screens. No public asset is silently shielded or rerouted. */
internal class PrivateWalletWorkspace(private val activity: Activity, private val host: PrivateWorkspaceHost) : AutoCloseable {
    val view = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL; isSaveEnabled = false }
    private val ink = Color.rgb(23, 42, 36)
    private val muted = Color.rgb(104, 115, 108)
    private val border = Color.rgb(223, 228, 219)
    private var session: PrivateWalletSession? = null
    private var route = "home"
    private var networkChosen = false
    private var recipient = ""
    private var amount = ""
    private var maximum = false
    private var lastError: String? = null
    @Volatile private var foreground = true

    fun render() {
        view.removeAllViews()
        view.visibility = if (foreground) View.VISIBLE else View.INVISIBLE
        if (route == "privacy") { privacy(); return }
        val current = session
        if (current == null) {
            text("Private transfers", 27, true)
            text("A separate private address. Your public wallet stays available.", 15, false, muted)
            val availability = PrivatePaymentRelease.availability()
            if (!availability.available) {
                text(availability.summary, 18, true)
                text(availability.explanation, 14, false, muted)
            }
            button("Open private wallet", enabled = availability.available) { open() }
            button("Restore encrypted backup", secondary = true, enabled = availability.available) { host.chooseBackup() }
            button("Privacy details", secondary = true) { route = "privacy"; render() }
            button("Back to settings", secondary = true) { host.settings() }
            error()
            return
        }
        when (route) {
            "deposit" -> deposit(current)
            "send" -> send(current)
            "review" -> review(current)
            "payment" -> payment(current)
            "recovery" -> recovery(current)
            "history" -> history(current)
            else -> home(current)
        }
        error()
    }

    fun open() {
        if (!host.canAct()) return
        lastError = null
        host.authorized("Opening private wallet…", { manifest, key, valid ->
            PrivateWalletSession.open(activity.applicationContext, manifest, key, valid)
        }, discard = { it.close() }) { result ->
            result.fold({ opened -> session?.close(); session = opened; route = "home"; render(); refresh(opened) }, ::failed)
        }
    }

    fun imported() { session?.close(); session = null; route = "home"; lastError = null; open() }
    fun refreshVisible() { if (route == "home" && foreground) render() }

    private fun home(current: PrivateWalletSession) {
        text("Private wallet", 27, true)
        text("USDC · Arbitrum", 13, false, muted)
        val balance = current.balance
        val verified = balance?.takeIf { it.scanComplete }
        text(verified?.let { host.usd(it.units) ?: "${Amounts.display(it.units, 6)} USDC" } ?: "—", 40, true)
        text(verified?.let { "${Amounts.display(it.units, 6)} USDC" } ?: "Private balance not verified yet", 14, false, muted)
        if (current.recovery.quarantined || current.recovery.incident) {
            text("Recovery verification required", 17, true)
            text("Your private history is protected from conflicting transfers. Complete recovery verification before sending.", 14, false, muted)
        }
        val row = LinearLayout(activity).apply { gravity = Gravity.CENTER_VERTICAL }
        action(row, "DEPOSIT", false) { route = "deposit"; networkChosen = false; render() }
        action(row, "SEND", true, enabled = current.canSend) { route = "send"; networkChosen = false; render() }
        view.addView(row, margins(24))
        if (verified?.units == BigInteger.ZERO) text("No private assets yet", 15, false, muted)
        button("Refresh private balance", secondary = true) { refresh(current) }
        val pending = current.coordinator.snapshot()
        if (pending.intentId != null && pending.stage !in setOf(PrivatePaymentStage.EMPTY, PrivatePaymentStage.CANCELLED)) {
            button("View private transfer", secondary = true) { route = if (pending.stage == PrivatePaymentStage.REVIEW) "review" else "payment"; render() }
        }
        if (current.payments.isNotEmpty()) button("Private activity", secondary = true) { route = "history"; render() }
        button("Recovery and encrypted backup", secondary = true) { route = "recovery"; render() }
        button("Back to settings", secondary = true) { host.settings() }
    }

    private fun selection(title: String, done: () -> Unit) {
        text(title, 27, true)
        text("Asset", 12, false, muted)
        button("USDC  ·  USD Coin", secondary = true) {
            AlertDialog.Builder(activity).setTitle("Choose asset").setItems(arrayOf("USDC")) { _, _ -> networkChosen = false; done() }.show()
        }
        text("Network", 12, false, muted)
        button(if (networkChosen) "Arbitrum" else "Choose network", secondary = true) {
            AlertDialog.Builder(activity).setTitle("Choose network").setItems(arrayOf("Arbitrum")) { _, _ -> networkChosen = true; done() }.show()
        }
    }

    private fun deposit(current: PrivateWalletSession) {
        selection("Deposit") { render() }
        if (networkChosen) {
            text("Receive a private transfer", 19, true)
            val address = current.privateAddress
            val matrix = MultiFormatWriter().encode(address, BarcodeFormat.QR_CODE, 600, 600, mapOf(EncodeHintType.MARGIN to 4))
            val pixels = IntArray(matrix.width * matrix.height) { index -> if (matrix[index % matrix.width, index / matrix.width]) Color.BLACK else Color.WHITE }
            val bitmap = Bitmap.createBitmap(pixels, matrix.width, matrix.height, Bitmap.Config.ARGB_8888)
            val image = ImageView(activity).apply { setImageBitmap(bitmap); contentDescription = "Private USDC receiving address on Arbitrum" }
            view.addView(image, LinearLayout.LayoutParams(dp(236), dp(236)).apply { gravity = Gravity.CENTER_HORIZONTAL; topMargin = dp(20) })
            text(address, 13, false, muted, monospace = true).setTextIsSelectable(true)
            button("Copy private address") {
                activity.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Private receiving address", address))
            }
            text("Use a compatible private wallet on Arbitrum. An exchange or ordinary public address cannot send directly to this private address.", 14, false, muted)
            text("Public-to-private funding is a separate on-chain action. This screen does not move your public assets.", 13, false, muted)
        }
        button("Back", secondary = true) { route = "home"; render() }
    }

    private fun send(current: PrivateWalletSession) {
        selection("Send") { render() }
        if (networkChosen) {
            text("Private destination", 12, false, muted)
            val destination = input("0zk…", recipient, decimal = false)
            destination.addTextChangedListener(watcher { recipient = it })
            text("Amount", 12, false, muted)
            val quantity = input(if (maximum) "Max" else "0.00", if (maximum) "" else amount, decimal = true)
            quantity.addTextChangedListener(watcher { amount = it; maximum = false })
            button(if (maximum) "Max selected" else "Max", secondary = true) { maximum = true; amount = ""; render() }
            button("Review transfer", enabled = current.canSend) {
                try {
                    val draft = PrivateTransferDraft.create(current.configuration.asset, destination.text.toString(), quantity.text.toString(), maximum)
                    host.work("Checking private balance and transfer cost…", { current.coordinator.prepare(draft) }) { outcome ->
                        outcome.fold({ state ->
                            if (state.stage == PrivatePaymentStage.REVIEW) { route = "review"; lastError = null; render() }
                            else failed(PrivatePaymentFailure(state.failure ?: PrivateFailure.LOCAL_ENGINE))
                        }, ::failed)
                    }
                } catch (error: Exception) { failed(error) }
            }
        }
        button("Back", secondary = true) { route = "home"; render() }
    }

    private fun review(current: PrivateWalletSession) {
        val approved = current.coordinator.currentReview()
        if (approved == null) { route = "payment"; payment(current); return }
        text("Review transfer", 27, true)
        text("${Amounts.display(approved.recipientUnits, 6)} USDC", 32, true)
        text("Private transfer on Arbitrum", 14, false, muted)
        text("To", 12, false, muted)
        text(approved.draft.recipient, 13, false, ink, true)
        detail("Transfer amount", "${Amounts.display(approved.recipientUnits, 6)} USDC")
        detail("Network cost", "${Amounts.display(approved.plan.feeUnits, 6)} USDC")
        detail("Total", "${Amounts.display(approved.recipientUnits + approved.plan.feeUnits, 6)} USDC")
        text("The service pays the native gas. The reviewed fee is paid privately in USDC.", 13, false, muted)
        button("Confirm and send", enabled = current.canSend) {
            host.authorized("Preparing the private proof…", { _, _, valid ->
                if (!valid() || !current.canSend) throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED)
                val stamp = current.coordinator.authorizationStamp()
                val prepared = current.coordinator.approveAndProve(approved)
                if (prepared.stage == PrivatePaymentStage.READY && valid() && current.coordinator.authorizationCurrent(stamp)) {
                    current.coordinator.submitExisting()
                } else prepared
                current.reloadLocalRecords()
                current.coordinator.snapshot()
            }) { outcome -> outcome.fold({ route = "payment"; render() }, ::failed) }
        }
        button("Edit", secondary = true) {
            host.work("Discarding the unsubmitted review…", { current.coordinator.cancelUnreleased().also { current.reloadLocalRecords() } }) { outcome ->
                outcome.fold({ route = "send"; render() }, ::failed)
            }
        }
    }

    private fun payment(current: PrivateWalletSession) {
        val state = current.coordinator.snapshot()
        text("Private transfer", 27, true)
        val label = when (state.stage) {
            PrivatePaymentStage.FINALIZED -> "Transfer confirmed"
            PrivatePaymentStage.INCLUDED -> "Waiting for finality"
            PrivatePaymentStage.READY -> "Ready to send"
            PrivatePaymentStage.CANCELLED -> "Not submitted"
            PrivatePaymentStage.SUPERSEDED -> "Replaced by recovery"
            PrivatePaymentStage.PROVING -> "Preparing private proof"
            PrivatePaymentStage.FAILED -> "Transaction reverted"
            PrivatePaymentStage.CONFLICT -> "Recovery review required"
            PrivatePaymentStage.APPROVED_UNFINISHED -> "Preparation interrupted"
            else -> "Transfer outcome pending"
        }
        text(label, 22, true)
        current.payments.firstOrNull { it.entry.intentId == state.intentId }?.let { payment ->
            text("${Amounts.display(payment.amountUnits, 6)} USDC", 26, true)
            text(payment.recipient, 12, false, muted, monospace = true).setTextIsSelectable(true)
        }
        text(state.failure?.publicMessage ?: "The saved operation is checked against the blockchain. A service response alone does not confirm delivery.", 14, false, muted)
        if (state.intentId != null && state.stage !in setOf(PrivatePaymentStage.CANCELLED, PrivatePaymentStage.SUPERSEDED)) {
            val saved = current.payments.firstOrNull { it.entry.intentId == state.intentId }
            button("Reload saved transfer", secondary = true) {
                host.work("Reading saved private history…", { current.selectPayment(state.intentId) }) { outcome -> outcome.fold({ render() }, ::failed) }
            }
            if (saved?.entry?.operationDigest != null) button("Check status", secondary = true) {
                host.work("Checking this saved private transfer…", {
                    current.coordinator.restore(state.intentId)
                    current.coordinator.reconcile().also { current.reloadLocalRecords() }
                }) { outcome -> outcome.fold({ render() }, ::failed) }
            }
            val retryExhausted = current.engine.advisoryState(saved?.entry?.operationDigest) == "retry_exhausted"
            if (retryExhausted) text(PrivateFailure.RETRY_EXHAUSTED.publicMessage, 14, false, muted)
            if (state.stage in setOf(PrivatePaymentStage.READY, PrivatePaymentStage.UNKNOWN, PrivatePaymentStage.AWAITING_CONFIRMATION, PrivatePaymentStage.FAILED)) {
                button(if (state.stage == PrivatePaymentStage.READY) "Send saved transfer" else "Retry the same transfer", enabled = !current.recovery.quarantined && !retryExhausted) {
                    host.authorized("Checking the saved proof before submission…", { _, _, valid ->
                        if (!valid()) throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED)
                        current.coordinator.restore(state.intentId)
                        current.coordinator.submitExisting().also { current.reloadLocalRecords() }
                    }) { outcome -> outcome.fold({ render() }, ::failed) }
                }
            }
            if (state.stage in setOf(PrivatePaymentStage.READY, PrivatePaymentStage.APPROVED_UNFINISHED) &&
                saved?.entry?.envelopeBackedUp != true && saved?.entry?.releaseHistoryIncomplete != true) {
                button("Cancel local preparation", secondary = true) {
                    host.work("Cancelling local preparation…", { current.coordinator.cancelUnreleased().also { current.reloadLocalRecords() } }) { outcome -> outcome.fold({ render() }, ::failed) }
                }
            }
        }
        button("Back to wallet", secondary = true) { route = "home"; render() }
    }

    private fun history(current: PrivateWalletSession) {
        text("Private activity", 27, true)
        text("Saved on this device", 13, false, muted)
        current.payments.asReversed().forEach { payment ->
            val label = "${Amounts.display(payment.amountUnits, 6)} USDC · ${payment.entry.phase.name.lowercase().replace('_', ' ')}"
            button(label, secondary = true) {
                host.work("Reading saved private transfer…", { current.selectPayment(payment.entry.intentId) }) { outcome ->
                    outcome.fold({ route = "payment"; render() }, ::failed)
                }
            }
        }
        button("Back to wallet", secondary = true) { route = "home"; render() }
    }

    private fun recovery(current: PrivateWalletSession) {
        text("Recovery and backup", 27, true)
        text(if (current.recovery.quarantined) "Recovery verification is required" else "Your private history is saved on this device", 19, true)
        text("Your recovery words recover keys. An encrypted backup also preserves private transfer instructions and uncertain outcomes.", 14, false, muted)
        val recovery = current.recoveryActions
        val workspace = recovery.workspace
        if (current.recovery.incident) {
            text("A conflicting chain observation needs review. Sending remains locked and the saved history has been preserved.", 14, false, muted)
        } else if (current.recovery.quarantined) {
            text("An older backup cannot prove that no later proof was released. Recovery checks a saved blockchain checkpoint and, where needed, moves old private notes to fresh notes in your own wallet.", 14, false, muted)
            text("Stop private spending on other devices before beginning recovery.", 14, true)
            val proposed = recovery.proposedPlan
            when {
                proposed != null -> recoveryPlan(current, proposed)
                workspace?.planJson == null -> button("Verify recovered history") {
                    host.work("Verifying private history through a finalized checkpoint…", { recovery.preparePlan() }) { outcome -> outcome.fold({ render() }, ::failed) }
                }
                else -> recoveryProgress(current, workspace)
            }
        }
        button("Export encrypted backup", secondary = true) {
            host.authorized("Encrypting private transfer history…", { manifest, key, valid ->
                if (!valid()) throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED)
                current.exportBackup(manifest, key).also { current.reloadLocalRecords() }
            }, discard = { it.encryptedBytes.fill(0) }) { outcome -> outcome.fold(host::exportBackup, ::failed) }
        }
        button("Restore encrypted backup", secondary = true) { host.chooseBackup() }
        button("Privacy details", secondary = true) { route = "privacy"; render() }
        text("Restoring uses a new isolated local journal and starts in recovery quarantine. It does not overwrite the old journal or automatically resend transfers.", 13, false, muted)
        button("Back to wallet", secondary = true) { route = "home"; render() }
    }

    private fun privacy() {
        text("Private-transfer privacy", 27, true)
        text("Your keys stay on your device", 19, true)
        text("The gas service does not receive your recovery words, spending keys, private destination or transfer amount. Private transfer proofs are prepared locally.", 14, false, muted)
        text("What the service retains", 19, true)
        text("The service keeps opaque proofs, spent-note markers, fees and gas transaction records to prevent duplicate payments and resolve uncertain outcomes. This is not a claim that the service stores no data.", 14, false, muted)
        text("Network privacy", 19, true)
        text("An independent relay separates your network connection from the gas service. The relay can still observe connection timing. Public transfers and public-to-private funding remain visible on the blockchain.", 14, false, muted)
        if (!PrivatePaymentRelease.availability().available) {
            text("Private sending is not activated in this build. These privacy protections require a verified deployment before use.", 14, false, muted)
        }
        button("Back", secondary = true) { route = if (session == null) "home" else "recovery"; render() }
    }

    private fun recoveryPlan(current: PrivateWalletSession, proposed: PrivateRecoveryReview) {
        if (proposed.noteCount == 0) {
            text("No old private notes found", 19, true)
            text("The complete finalized scan reached the saved recovery checkpoint. A fresh check must still pass before sending is unlocked.", 14, false, muted)
        } else {
            detail("Recovered private value", "${Amounts.display(proposed.inputUnits, 6)} USDC")
            detail("Maximum total recovery fees", "${Amounts.display(proposed.maximumFeeUnits, 6)} USDC")
            text("This is a conservative fee cap. Each self-transfer has an exact amount and fee for you to review before it is sent. The cap cannot be exceeded.", 13, false, muted)
        }
        button(if (proposed.noteCount == 0) "Finish verification" else "Approve recovery plan") {
            host.authorized("Saving the recovery decision…", { _, _, valid ->
                if (!valid()) throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED)
                current.recoveryActions.approvePlan(proposed)
                if (proposed.noteCount == 0 && valid()) current.recoveryActions.finish()
                current.reloadLocalRecords()
            }) { outcome -> outcome.fold({ lastError = null; render() }, ::failed) }
        }
    }

    private fun recoveryProgress(current: PrivateWalletSession, workspace: PrivateRecoveryWorkspace) {
        val recovery = current.recoveryActions
        val review = recovery.currentBatchReview()
        val state = recovery.currentBatch()
        if (review != null) {
            text("Review recovery transfer", 20, true)
            detail("To your own private wallet", "${Amounts.display(review.recipientUnits, 6)} USDC")
            detail("Exact transfer fee", "${Amounts.display(review.plan.feeUnits, 6)} USDC")
            text(current.privateAddress, 12, false, muted, monospace = true)
            text("This batch replaces old private notes. It does not pay another person.", 13, false, muted)
            button("Confirm recovery transfer") {
                host.authorized("Preparing the recovery proof…", { _, _, valid ->
                    if (!valid()) throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED)
                    recovery.approveBatchAndSubmit(valid).also { current.reloadLocalRecords() }
                }) { outcome -> outcome.fold({ render() }, ::failed) }
            }
            button("Discard this review", secondary = true) {
                host.work("Discarding the unsubmitted recovery review…", { recovery.cancelUnreleasedBatch() }) { outcome -> outcome.fold({ render() }, ::failed) }
            }
            return
        }
        val active = workspace.batches.firstOrNull { it.phase !in setOf(PrivateJournalPhase.FINALIZED, PrivateJournalPhase.CANCELLED, PrivateJournalPhase.SUPERSEDED) }
        if (active != null) {
            text(when (active.phase) {
                PrivateJournalPhase.APPROVED -> "Recovery preparation interrupted"
                PrivateJournalPhase.READY -> "Recovery transfer saved"
                PrivateJournalPhase.FAILED -> "Recovery transaction reverted"
                else -> "Waiting for recovery confirmation"
            }, 19, true)
            text(state?.failure?.publicMessage ?: "The same saved proof stays tracked until its exact canonical outcome is verified.", 14, false, muted)
            if (active.phase == PrivateJournalPhase.FAILED) {
                text("A reverted outer transaction does not invalidate the saved private proof. A retry uses the same proof after its canonical finalized outcome is checked again.", 14, false, muted)
            }
            if (active.operationDigest != null) button("Check recovery status", secondary = true) {
                host.work("Checking the saved recovery transfer…", {
                    if (recovery.currentBatch()?.intentId != active.intentId) recovery.resumeBatch(active.intentId)
                    recovery.reconcileBatch().also { current.reloadLocalRecords() }
                }) { outcome -> outcome.fold({ render() }, ::failed) }
            }
            val exhausted = current.engine.advisoryState(active.operationDigest) == "retry_exhausted"
            if (exhausted) text(PrivateFailure.RETRY_EXHAUSTED.publicMessage, 14, false, muted)
            if (active.phase in setOf(PrivateJournalPhase.READY, PrivateJournalPhase.RELEASED, PrivateJournalPhase.UNKNOWN, PrivateJournalPhase.FAILED)) {
                button(if (active.releaseCount == 0L) "Send saved recovery transfer" else "Retry the same recovery transfer", enabled = !exhausted) {
                    host.authorized("Checking the saved recovery proof…", { _, _, valid ->
                        if (recovery.currentBatch()?.intentId != active.intentId) recovery.resumeBatch(active.intentId)
                        recovery.retryBatch(valid).also { current.reloadLocalRecords() }
                    }) { outcome -> outcome.fold({ render() }, ::failed) }
                }
            }
            if (active.releaseCount == 0L && !active.releaseHistoryIncomplete && !active.envelopeBackedUp && active.phase in setOf(PrivateJournalPhase.APPROVED, PrivateJournalPhase.READY)) {
                button("Cancel local preparation", secondary = true) {
                    host.work("Cancelling unsubmitted recovery preparation…", {
                        if (recovery.currentBatch()?.intentId != active.intentId) recovery.resumeBatch(active.intentId)
                        recovery.cancelUnreleasedBatch().also { current.reloadLocalRecords() }
                    }) { outcome -> outcome.fold({ render() }, ::failed) }
                }
            }
        } else if (workspace.remaining.length() > 0) {
            button("Review next recovery transfer") {
                host.work("Preparing the next exact self-transfer…", { recovery.prepareNextBatch() }) { outcome -> outcome.fold({ render() }, ::failed) }
            }
        } else {
            button("Verify recovery complete") {
                host.work("Checking final receipts and rescanning private notes…", {
                    recovery.finish().also { current.reloadLocalRecords() }
                }) { outcome -> outcome.fold({ lastError = null; refresh(current) }, ::failed) }
            }
        }
    }

    private fun refresh(current: PrivateWalletSession) {
        host.work("Scanning the private pool locally…", { current.refresh() }) { outcome -> outcome.fold({ render() }, ::failed) }
    }
    /** Called only after fresh native app-lock authorization, on the serialized worker. */
    fun authorizedForeground(stillAuthorized: () -> Boolean) {
        if (!stillAuthorized()) throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED)
        foreground = true; session?.setForeground(true)
        if (!stillAuthorized()) {
            foreground = false; session?.setForeground(false)
            throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED)
        }
    }
    fun setForeground(value: Boolean) { foreground = value; session?.setForeground(value); view.visibility = if (value) View.VISIBLE else View.INVISIBLE }
    private fun failed(error: Throwable) { lastError = (error as? PrivatePaymentFailure)?.failure?.publicMessage ?: "The private wallet could not complete this step safely. Its saved records have been preserved."; render() }
    private fun error() { lastError?.let { text(it, 14, false, Color.rgb(138, 86, 28)) } }
    private fun dp(value: Int) = (value * activity.resources.displayMetrics.density).toInt()
    private fun margins(top: Int = 12) = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(top) }
    private fun text(value: String, size: Int, bold: Boolean = false, color: Int = ink, monospace: Boolean = false): TextView = TextView(activity).apply {
        text = value; textSize = size.toFloat(); setTextColor(color); isSaveEnabled = false
        typeface = if (monospace) Typeface.MONOSPACE else if (bold) Typeface.create("sans-serif", Typeface.BOLD) else Typeface.create("sans-serif", Typeface.NORMAL)
        setLineSpacing(dp(3).toFloat(), 1f)
        view.addView(this, margins(if (size >= 27) 18 else 12))
    }
    private fun button(label: String, secondary: Boolean = false, enabled: Boolean = true, action: () -> Unit) {
        val button = Button(activity).apply {
            text = label; isAllCaps = false; textSize = 15f; minHeight = dp(52); isSaveEnabled = false
            setTextColor(if (secondary) ink else Color.WHITE); isEnabled = enabled; alpha = if (enabled) 1f else 0.45f
            background = GradientDrawable().apply { cornerRadius = dp(18).toFloat(); setColor(if (secondary) Color.WHITE else ink); if (secondary) setStroke(dp(1), border) }
            filterTouchesWhenObscured = true
            setOnClickListener { if (host.canAct()) action() }
        }
        view.addView(button, margins())
    }
    private fun action(parent: LinearLayout, label: String, primary: Boolean, enabled: Boolean = true, action: () -> Unit) {
        val button = Button(activity).apply {
            text = label; textSize = 13f; letterSpacing = 0.05f; minHeight = dp(58); isSaveEnabled = false
            setTextColor(if (primary) Color.WHITE else ink); isEnabled = enabled; alpha = if (enabled) 1f else 0.45f
            background = GradientDrawable().apply { cornerRadius = dp(28).toFloat(); setColor(if (primary) ink else Color.rgb(238, 243, 232)) }
            filterTouchesWhenObscured = true; setOnClickListener { if (host.canAct()) action() }
        }
        parent.addView(button, LinearLayout.LayoutParams(0, dp(58), 1f).apply { marginEnd = dp(6) })
    }
    private fun input(hint: String, value: String, decimal: Boolean): EditText = EditText(activity).apply {
        this.hint = hint; setText(value); textSize = 16f; setTextColor(ink); setHintTextColor(muted)
        inputType = if (decimal) InputType.TYPE_CLASS_NUMBER or InputType.TYPE_NUMBER_FLAG_DECIMAL else InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        gravity = Gravity.TOP or Gravity.START; isSaveEnabled = false; importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
        imeOptions = EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
        setPadding(dp(16), dp(16), dp(16), dp(16)); minHeight = dp(58); maxLines = if (decimal) 1 else 4
        background = GradientDrawable().apply { cornerRadius = dp(16).toFloat(); setColor(Color.WHITE); setStroke(dp(1), border) }
        view.addView(this, margins())
    }
    private fun watcher(change: (String) -> Unit) = object : TextWatcher {
        override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
        override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) { change(s?.toString().orEmpty()) }
        override fun afterTextChanged(s: Editable?) {}
    }
    private fun detail(title: String, value: String) { text(title, 12, false, muted); text(value, 17, true) }
    override fun close() {
        session?.close(); session = null; recipient = ""; amount = ""
        activity.runOnUiThread { view.removeAllViews() }
    }
}
