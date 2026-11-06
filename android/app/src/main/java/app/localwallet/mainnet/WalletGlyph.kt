package app.localwallet.mainnet

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Typeface
import android.view.View
import kotlin.math.min

/** Locally drawn interface icons. No network images or fonts enter the wallet. */
internal class WalletGlyph @JvmOverloads constructor(context: Context, private val name: String = "wallet", private val tint: Int = Color.WHITE) : View(context) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val path = Path()

    init {
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
        isSaveEnabled = false
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val side = min(width, height).toFloat()
        canvas.save()
        canvas.translate((width - side) / 2, (height - side) / 2)
        canvas.scale(side / 24f, side / 24f)
        paint.color = tint
        paint.style = Paint.Style.STROKE
        paint.strokeWidth = 1.65f
        paint.strokeCap = Paint.Cap.ROUND
        paint.strokeJoin = Paint.Join.ROUND
        fun line(vararg values: Float) {
            path.reset()
            path.moveTo(values[0], values[1])
            var index = 2
            while (index < values.size) { path.lineTo(values[index], values[index + 1]); index += 2 }
            canvas.drawPath(path, paint)
        }
        when (name) {
            "send" -> { line(5f, 19f, 19f, 5f); line(5f, 5f, 19f, 5f, 19f, 19f) }
            "receive" -> { line(19f, 5f, 5f, 19f); line(5f, 5f, 5f, 19f, 19f, 19f) }
            "back" -> { line(19f, 12f, 5f, 12f); line(11f, 6f, 5f, 12f, 11f, 18f) }
            "chevron" -> line(9f, 5f, 16f, 12f, 9f, 19f)
            "down" -> line(6f, 9f, 12f, 15f, 18f, 9f)
            "copy" -> { canvas.drawRoundRect(8f, 8f, 20f, 21f, 2f, 2f, paint); line(15f, 8f, 15f, 3f, 3f, 3f, 3f, 15f, 8f, 15f) }
            "edit" -> { line(4f, 20f, 5f, 14f, 16f, 3f, 21f, 8f, 10f, 19f, 4f, 20f); line(13f, 6f, 18f, 11f) }
            "settings" -> { line(4f, 7f, 20f, 7f); line(4f, 17f, 20f, 17f); line(9f, 4f, 9f, 10f); line(15f, 14f, 15f, 20f) }
            "profile" -> { canvas.drawCircle(12f, 8f, 4f, paint); canvas.drawArc(4f, 13f, 20f, 29f, 180f, 180f, false, paint) }
            "refresh" -> { canvas.drawArc(4f, 4f, 20f, 20f, 215f, 270f, false, paint); line(3f, 4f, 3f, 10f, 9f, 10f) }
            "history" -> { canvas.drawCircle(12f, 12f, 9f, paint); line(12f, 7f, 12f, 12f, 16f, 14f) }
            "check" -> line(5f, 12f, 10f, 17f, 20f, 6f)
            "lock" -> { canvas.drawRoundRect(5f, 10f, 19f, 21f, 2f, 2f, paint); canvas.drawArc(8f, 2f, 16f, 16f, 180f, 180f, false, paint); line(12f, 14f, 12f, 17f) }
            "eye" -> {
                path.reset(); path.moveTo(2f, 12f); path.quadTo(12f, -1f, 22f, 12f); path.quadTo(12f, 25f, 2f, 12f)
                canvas.drawPath(path, paint); canvas.drawCircle(12f, 12f, 3f, paint)
            }
            "hidden" -> { canvas.drawArc(3f, 6f, 21f, 18f, 5f, 170f, false, paint); line(4f, 3f, 20f, 21f) }
            "shield" -> {
                path.reset(); path.moveTo(12f, 2f); path.lineTo(21f, 6f); path.lineTo(21f, 12f); path.quadTo(21f, 18f, 12f, 22f); path.quadTo(3f, 18f, 3f, 12f); path.lineTo(3f, 6f); path.close()
                canvas.drawPath(path, paint); line(8f, 12f, 11f, 15f, 16f, 9f)
            }
            else -> {
                canvas.drawRoundRect(3f, 5f, 21f, 21f, 3f, 3f, paint)
                line(5f, 5f, 5f, 2f, 18f, 2f, 18f, 5f)
                canvas.drawRoundRect(14f, 11f, 21f, 17f, 1f, 1f, paint)
                paint.style = Paint.Style.FILL
                canvas.drawCircle(17f, 14f, 0.8f, paint)
            }
        }
        canvas.restore()
    }
}

/** Asset artwork stays bundled with the app and never depends on remote token metadata. */
internal class AssetGlyph @JvmOverloads constructor(context: Context, private val symbol: String = "ETH") : View(context) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val path = Path()
    private val tokenTypeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)

    init { importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO; isSaveEnabled = false }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val side = min(width, height).toFloat()
        canvas.save()
        canvas.translate((width - side) / 2, (height - side) / 2)
        canvas.scale(side / 48f, side / 48f)
        paint.style = Paint.Style.FILL
        paint.color = when (symbol) {
            "ETH" -> Color.rgb(37, 46, 63)
            "BASE" -> Color.rgb(0, 82, 255)
            "OP" -> Color.rgb(255, 4, 32)
            "ARB" -> Color.rgb(33, 49, 71)
            "SOL" -> Color.rgb(48, 38, 69)
            "TRX" -> Color.rgb(210, 39, 49)
            "USDT" -> Color.rgb(38, 161, 123)
            "AVAX" -> Color.rgb(212, 62, 58)
            "POL" -> Color.rgb(116, 65, 197)
            "BNB" -> Color.rgb(176, 133, 0)
            else -> Color.rgb(39, 117, 202)
        }
        canvas.drawCircle(24f, 24f, 24f, paint)
        if (symbol == "ETH") {
            paint.color = Color.rgb(190, 204, 231)
            path.reset(); path.moveTo(24f, 7f); path.lineTo(14f, 25f); path.lineTo(24f, 31f); path.lineTo(34f, 25f); path.close()
            canvas.drawPath(path, paint)
            paint.color = Color.rgb(128, 150, 187)
            path.reset(); path.moveTo(14f, 28f); path.lineTo(24f, 41f); path.lineTo(34f, 28f); path.lineTo(24f, 34f); path.close()
            canvas.drawPath(path, paint)
        } else if (symbol == "BASE") {
            paint.color = Color.WHITE
            canvas.drawRect(11f, 22.2f, 37f, 25.8f, paint)
        } else if (symbol in setOf("OP", "ARB", "SOL", "TRX", "AVAX", "POL", "BNB", "USDT")) {
            paint.color = Color.WHITE
            paint.textAlign = Paint.Align.CENTER
            paint.textSize = when (symbol) { "OP" -> 21f; "USDT" -> 24f; "AVAX" -> 14f; else -> 17f }
            paint.typeface = tokenTypeface
            val baseline = 24f - (paint.ascent() + paint.descent()) / 2f
            canvas.drawText(if (symbol == "USDT") "₮" else symbol, 24f, baseline, paint)
        } else {
            paint.color = Color.WHITE
            paint.textAlign = Paint.Align.CENTER
            paint.textSize = 28f
            paint.typeface = tokenTypeface
            canvas.drawText(if (symbol == "USDC") "$" else "€", 24f, 33.5f, paint)
            if (symbol == "USDC") {
                paint.style = Paint.Style.STROKE
                paint.strokeWidth = 1.4f
                paint.strokeCap = Paint.Cap.ROUND
                canvas.drawArc(6.5f, 6.5f, 41.5f, 41.5f, 108f, 144f, false, paint)
                canvas.drawArc(6.5f, 6.5f, 41.5f, 41.5f, -72f, 144f, false, paint)
            }
        }
        canvas.restore()
    }
}
