package app.localwallet.mainnet

import android.app.Dialog
import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.os.Build
import android.text.InputFilter
import android.text.InputType
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.Window
import android.view.WindowManager
import android.view.inputmethod.EditorInfo
import android.widget.Button
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView

internal data class WithdrawalPalette(
    val background: Int, val surface: Int, val elevated: Int, val border: Int,
    val ink: Int, val muted: Int, val accent: Int, val warning: Int
)

/** Native retained views. Network and price responses update text, never rebuild the form. */
internal class WithdrawalFormViews(
    context: Context, private val colors: WithdrawalPalette,
    symbol: String, assetName: String, network: WalletNetwork?
) : LinearLayout(context) {
    private val ui = WithdrawalViewFactory(context, colors)
    val assetSelector = ui.selector(symbol, assetName, AssetGlyph(context, symbol))
    val recipient = ui.input("Enter the receiving address", decimal = false).apply {
        minLines = 2; maxLines = 3; setSingleLine(false)
        gravity = Gravity.TOP or Gravity.START
        typeface = Typeface.MONOSPACE; textSize = 15f
        filters = arrayOf(InputFilter.LengthFilter(256))
        setPadding(ui.dp(16), ui.dp(16), ui.dp(10), ui.dp(16))
        contentDescription = "Recipient address"
    }
    val paste = ui.textAction("Paste")
    val networkSelector = ui.selector(network?.name ?: "Select network", null,
        network?.let { AssetGlyph(context, it.glyph) })
    val amount = ui.input("0.00", decimal = true).apply {
        textSize = 30f; minHeight = ui.dp(62); setSingleLine(false); setHorizontallyScrolling(false)
        gravity = Gravity.TOP or Gravity.START
        typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
        setPadding(ui.dp(16), ui.dp(14), ui.dp(8), ui.dp(14))
        contentDescription = "Amount to send in $symbol"
    }
    val maximum = ui.textAction("Max")
    val available = ui.text("", 12, colors.muted).apply { gravity = Gravity.END }
    val amountUsd = ui.text("", 13, colors.muted)
    val message = ui.text("", 13, colors.muted).apply {
        visibility = View.GONE
        accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
    }
    val estimateAction = ui.textAction("Refresh fee").apply { visibility = View.GONE }
    val depositFees = ui.textAction("Deposit ${network?.nativeSymbol ?: "gas asset"} for fees").apply { visibility = View.GONE }

    init {
        orientation = VERTICAL; isSaveEnabled = false
        label("Asset", assetSelector)
        addView(assetSelector, ui.match(7))
        assetSelector.contentDescription = "Asset: $symbol. Change asset."

        label("Address", recipient, 23)
        val addressBox = LinearLayout(context).apply {
            gravity = Gravity.TOP; background = ui.surface(18)
            addView(recipient, LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            addView(paste, LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ui.dp(48)).apply {
                topMargin = ui.dp(8); marginEnd = ui.dp(6)
            })
        }
        addView(addressBox, ui.match(7))

        label("Network", networkSelector, 23)
        addView(networkSelector, ui.match(7))
        networkSelector.contentDescription = "Network: ${network?.name ?: "not selected"}. Choose a network."
        addView(ui.text("Use the same network as the receiving wallet.", 12, colors.muted), ui.match(9))

        label("Amount", amount, 23)
        val amountBox = LinearLayout(context).apply {
            orientation = VERTICAL; background = ui.surface(18)
            addView(LinearLayout(context).apply {
                gravity = Gravity.CENTER_VERTICAL; setPadding(ui.dp(16), ui.dp(2), ui.dp(4), 0)
                addView(ui.text(symbol, 13, colors.muted, true), LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
                addView(maximum, LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ui.dp(40)))
            })
            // Full width and wrapping preserve every native decimal, including a long ETH Max.
            addView(amount, ui.match())
        }
        addView(amountBox, ui.match(7))
        addView(available, ui.match(10))
        addView(amountUsd, ui.match(7))
        val feeHelp = when {
            network == null -> "Choose the receiving network before checking the amount and fee."
            symbol == network.nativeSymbol -> "Max leaves a ${network.nativeSymbol} reserve for network fees."
            else -> "The recipient gets the full token amount. Fees are paid separately in ${network.nativeSymbol}."
        }
        addView(ui.text(feeHelp, 12, colors.muted), ui.match(13))
        addView(message, ui.match(17))
        val support = LinearLayout(context).apply {
            gravity = Gravity.START or Gravity.CENTER_VERTICAL
            addView(estimateAction, LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ui.dp(44)))
            addView(depositFees, LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ui.dp(44)).apply { marginStart = ui.dp(4) })
        }
        addView(support, ui.match(3))
    }

    private fun label(value: String, target: View, top: Int = 4) {
        if (target.id == View.NO_ID) target.id = View.generateViewId()
        addView(ui.text(value, 12, colors.muted).apply { labelFor = target.id }, ui.match(top))
    }

    fun editable(enabled: Boolean, networkChosen: Boolean) {
        listOf(assetSelector, recipient, paste, networkSelector).forEach { it.isEnabled = enabled }
        amount.isEnabled = enabled && networkChosen
        maximum.isEnabled = enabled && networkChosen
        maximum.alpha = if (maximum.isEnabled) 1f else 0.4f
        amount.alpha = if (amount.isEnabled) 1f else 0.45f
        estimateAction.isEnabled = enabled
        depositFees.isEnabled = enabled
    }

    fun message(value: String, warning: Boolean = false) {
        if (message.text.toString() != value) message.text = value
        message.setTextColor(if (warning) colors.warning else colors.muted)
        message.visibility = if (value.isEmpty()) View.GONE else View.VISIBLE
    }
}

/** Fixed below the scrollable form, so the reviewed action remains above system/IME insets. */
internal class WithdrawalDock(context: Context, private val colors: WithdrawalPalette, feeLabel: String = "Network fee") : LinearLayout(context) {
    private val ui = WithdrawalViewFactory(context, colors)
    val received = ui.text("—", 19, colors.ink, true).apply {
        gravity = Gravity.END; fontFeatureSettings = "tnum"
        setTextIsSelectable(true)
    }
    val fee = ui.text("Choose a network", 12, colors.muted).apply {
        gravity = Gravity.END; minHeight = ui.dp(28)
        isFocusable = true; isClickable = true; filterTouchesWhenObscured = true
    }
    val primary = Button(context).apply {
        text = "Review transfer"; textSize = 16f; isAllCaps = false
        minHeight = ui.dp(54); setPadding(ui.dp(16), ui.dp(13), ui.dp(16), ui.dp(13))
        typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
        filterTouchesWhenObscured = true; isSaveEnabled = false
    }

    init {
        orientation = VERTICAL; isSaveEnabled = false
        setPadding(ui.dp(24), ui.dp(15), ui.dp(24), ui.dp(16))
        setBackgroundColor(colors.surface)
        addRow("Recipient receives", received)
        addRow(feeLabel, fee, 6)
        addView(primary, ui.match(11))
        button("Review transfer", false)
    }

    private fun addRow(label: String, value: TextView, top: Int = 0) {
        addView(LinearLayout(context).apply {
            gravity = Gravity.CENTER_VERTICAL
            addView(ui.text(label, 12, colors.muted), LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 0.9f).apply { marginEnd = ui.dp(12) })
            addView(value, LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.5f))
        }, ui.match(top))
    }

    fun button(label: String, enabled: Boolean) {
        if (primary.text.toString() != label) primary.text = label
        primary.isEnabled = enabled
        primary.setTextColor(if (enabled) colors.surface else colors.muted)
        primary.background = ui.touch(if (enabled) colors.ink else colors.elevated, 28)
    }
}

/** No WebView or external UI dependency; sheets keep the wallet's screen and touch protection. */
internal class WithdrawalSheet(context: Context, colors: WithdrawalPalette, title: String, description: String? = null) : Dialog(context) {
    private val ui = WithdrawalViewFactory(context, colors)
    val body = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL; isSaveEnabled = false }
    init {
        requestWindowFeature(Window.FEATURE_NO_TITLE)
        setCanceledOnTouchOutside(true)
        val shell = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL; setPadding(ui.dp(24), ui.dp(11), ui.dp(24), ui.dp(20))
            background = GradientDrawable().apply {
                setColor(colors.surface)
                val radius = ui.dp(28).toFloat()
                cornerRadii = floatArrayOf(radius, radius, radius, radius, 0f, 0f, 0f, 0f)
            }
        }
        shell.addView(View(context).apply { background = ui.rounded(colors.border, 3) }, LinearLayout.LayoutParams(ui.dp(34), ui.dp(4)).apply {
            gravity = Gravity.CENTER_HORIZONTAL; bottomMargin = ui.dp(23)
        })
        shell.addView(ui.text(title, 24, colors.ink, true))
        if (description != null) shell.addView(ui.text(description, 14, colors.muted), ui.match(12))
        val scroll = ScrollView(context).apply { isSaveEnabled = false; clipToPadding = false; addView(body) }
        shell.addView(scroll, ui.match(17))
        setContentView(shell)
        window?.apply {
            setDecorFitsSystemWindows(false)
            setBackgroundDrawableResource(android.R.color.transparent)
            addFlags(WindowManager.LayoutParams.FLAG_SECURE or WindowManager.LayoutParams.FLAG_DIM_BEHIND)
            if (Build.VERSION.SDK_INT >= 31) setHideOverlayWindows(true)
            setDimAmount(0.28f); setGravity(Gravity.BOTTOM)
            setLayout(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
        }
        shell.setOnApplyWindowInsetsListener { view, insets ->
            val bars = insets.getInsets(android.view.WindowInsets.Type.systemBars() or android.view.WindowInsets.Type.displayCutout())
            view.setPadding(ui.dp(24) + bars.left, ui.dp(11), ui.dp(24) + bars.right, ui.dp(20) + bars.bottom)
            insets
        }
    }

    override fun show() {
        super.show()
        window?.setLayout(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
    }

    override fun dispatchTouchEvent(event: MotionEvent): Boolean {
        if (event.flags and (MotionEvent.FLAG_WINDOW_IS_OBSCURED or MotionEvent.FLAG_WINDOW_IS_PARTIALLY_OBSCURED) != 0) return false
        return super.dispatchTouchEvent(event)
    }
}

private class WithdrawalViewFactory(private val context: Context, private val colors: WithdrawalPalette) {
    fun dp(value: Int) = (value * context.resources.displayMetrics.density).toInt()
    fun match(top: Int = 0) = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(top) }
    fun rounded(color: Int, radius: Int, border: Int? = null) = GradientDrawable().apply {
        setColor(color); cornerRadius = dp(radius).toFloat(); border?.let { setStroke(dp(1), it) }
    }
    fun surface(radius: Int) = rounded(colors.surface, radius, colors.border)
    fun touch(color: Int, radius: Int, border: Int? = null) = RippleDrawable(ColorStateList.valueOf(Color.argb(22, 80, 110, 80)), rounded(color, radius, border), rounded(Color.WHITE, radius))
    fun text(value: String, size: Int, color: Int, bold: Boolean = false) = TextView(context).apply {
        text = value; textSize = size.toFloat(); setTextColor(color); includeFontPadding = false; isSaveEnabled = false
        typeface = Typeface.create(if (bold) "sans-serif-medium" else "sans-serif", Typeface.NORMAL)
        setLineSpacing(dp(3).toFloat(), 1f)
    }
    fun textAction(label: String) = text(label, 13, colors.accent, true).apply {
        gravity = Gravity.CENTER; minHeight = dp(44); setPadding(dp(12), 0, dp(12), 0)
        background = touch(Color.TRANSPARENT, 22)
        isClickable = true; isFocusable = true; filterTouchesWhenObscured = true
    }
    fun input(hint: String, decimal: Boolean) = EditText(context).apply {
        this.hint = hint; setTextColor(colors.ink); setHintTextColor(colors.muted)
        background = null; isSaveEnabled = false; filterTouchesWhenObscured = true
        inputType = if (decimal) InputType.TYPE_CLASS_NUMBER or InputType.TYPE_NUMBER_FLAG_DECIMAL
            else InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS or InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD or InputType.TYPE_TEXT_FLAG_MULTI_LINE
        imeOptions = EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING or EditorInfo.IME_ACTION_DONE
        importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
        if (decimal) filters = arrayOf(InputFilter.LengthFilter(96))
    }
    fun selector(title: String, subtitle: String?, icon: View?): LinearLayout = LinearLayout(context).apply {
        gravity = Gravity.CENTER_VERTICAL; minimumHeight = dp(if (subtitle == null) 59 else 73)
        setPadding(dp(16), dp(13), dp(16), dp(13)); background = touch(colors.surface, 18, colors.border)
        isClickable = true; isFocusable = true; filterTouchesWhenObscured = true; isSaveEnabled = false
        if (icon != null) addView(icon, LinearLayout.LayoutParams(dp(if (subtitle == null) 26 else 37), dp(if (subtitle == null) 26 else 37)).apply { marginEnd = dp(12) })
        val labels = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            addView(text(title, 16, colors.ink, true))
            if (subtitle != null) addView(text(subtitle, 12, colors.muted), match(5))
        }
        addView(labels, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        addView(WalletGlyph(context, "down", colors.muted), LinearLayout.LayoutParams(dp(16), dp(16)).apply { marginStart = dp(10) })
    }
}
