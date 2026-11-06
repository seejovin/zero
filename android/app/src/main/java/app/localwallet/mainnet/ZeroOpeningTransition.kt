package app.localwallet.mainnet

import android.animation.ValueAnimator
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Typeface
import android.os.Build
import android.os.SystemClock
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityManager
import android.view.animation.PathInterpolator
import android.widget.FrameLayout
import android.widget.TextView

/** Local typography shared by the opening screen and native headers. */
internal object ZeroBrand {
    fun wordmark(context: Context, color: Int, size: Float = 80f): TextView =
        TextView(context).apply {
            text = context.getString(R.string.app_name)
            contentDescription = text
            setTextColor(color)
            // A logo keeps fixed optical proportions; interactive copy still follows font scale.
            setTextSize(TypedValue.COMPLEX_UNIT_DIP, size)
            typeface = Typeface.create("serif", Typeface.NORMAL)
            letterSpacing = -0.075f
            includeFontPadding = false
            gravity = Gravity.CENTER
            setSingleLine(true)
            isSaveEnabled = false
            // Keep the slashed final letter inside the text view's optical bounds.
            val inset = (8 * resources.displayMetrics.density).toInt()
            setPadding(inset, 0, inset, 0)
        }

    fun mark(context: Context, color: Int, size: Float): TextView =
        wordmark(context, color, size).apply {
            text = "ø"
            letterSpacing = 0f
            setPadding(0, 0, 0, 0)
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }

    fun animationsEnabled(context: Context): Boolean =
        ValueAnimator.areAnimatorsEnabled() &&
            context.getSystemService(AccessibilityManager::class.java)?.isTouchExplorationEnabled != true

    @Suppress("DEPRECATION")
    fun releaseLabel(context: Context): String {
        val info = if (Build.VERSION.SDK_INT >= 33) {
            context.packageManager.getPackageInfo(context.packageName, PackageManager.PackageInfoFlags.of(0))
        } else {
            context.packageManager.getPackageInfo(context.packageName, 0)
        }
        return context.getString(R.string.release_label, context.getString(R.string.app_name), info.versionName.orEmpty())
    }
}

/**
 * A finite, presentation-only cover. Storage and authentication initialize beneath it.
 * It never waits for an RPC, owns a wallet state transition, or reappears on app resume.
 */
internal class ZeroOpeningTransition(
    context: Context,
    backgroundColor: Int,
    inkColor: Int,
    private val onFinished: () -> Unit,
) : FrameLayout(context) {
    private val wordmark = ZeroBrand.wordmark(context, inkColor)
    private val startedAt = SystemClock.elapsedRealtime()
    private var exitRequested = false
    private var finished = false
    private val arrive = PathInterpolator(0.22f, 1f, 0.36f, 1f)
    private val depart = PathInterpolator(0.4f, 0f, 0.2f, 1f)
    // Wall-clock deadline, independent of system animator scaling and wallet initialization.
    private val finishCover = Runnable { finishImmediately() }

    private val startEntry = Runnable {
        if (!finished) {
            wordmark.animate()
                .alpha(1f).translationY(0f).scaleX(1f).scaleY(1f)
                .setStartDelay(60).setDuration(420).setInterpolator(arrive).start()
        }
    }

    private val startExit = Runnable {
        if (!finished) {
            if (!ZeroBrand.animationsEnabled(context)) finishImmediately()
            else {
                wordmark.animate().cancel()
                wordmark.animate()
                    .alpha(0f).translationY(-dp(3f)).scaleX(0.982f).scaleY(0.982f)
                    .setStartDelay(0).setDuration(300).setInterpolator(depart).start()
                animate().alpha(0f).setStartDelay(0).setDuration(360)
                    .setInterpolator(depart).withEndAction { finishImmediately() }.start()
            }
        }
    }

    init {
        setBackgroundColor(backgroundColor)
        isSaveEnabled = false
        isClickable = true
        filterTouchesWhenObscured = true
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        wordmark.alpha = 0f
        wordmark.translationY = dp(6f)
        wordmark.scaleX = 0.986f
        wordmark.scaleY = 0.986f
        addView(wordmark, LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT,
            ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.CENTER))
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        postOnAnimation(startEntry)
        postDelayed(finishCover, 1_500L)
    }

    /** Once ready, hold only long enough to complete the brief wordmark introduction. */
    fun contentReady() {
        if (finished || exitRequested) return
        exitRequested = true
        if (!ZeroBrand.animationsEnabled(context)) finishImmediately()
        else postDelayed(startExit, (850L - (SystemClock.elapsedRealtime() - startedAt)).coerceAtLeast(0L))
    }

    /** Errors, authentication, backgrounding and destruction always take priority. */
    fun finishImmediately() {
        if (finished) return
        finished = true
        stopAnimations()
        (parent as? ViewGroup)?.removeView(this)
        onFinished()
    }

    private fun stopAnimations() {
        removeCallbacks(startEntry)
        removeCallbacks(startExit)
        removeCallbacks(finishCover)
        wordmark.animate().withEndAction(null).cancel()
        animate().withEndAction(null).cancel()
    }

    override fun onDetachedFromWindow() {
        // Window teardown already owns child removal; do not mutate its detach traversal.
        if (!finished) {
            finished = true
            stopAnimations()
            onFinished()
        }
        super.onDetachedFromWindow()
    }

    private fun dp(value: Float) = value * resources.displayMetrics.density
}
