package app.localwallet.mainnet

import android.annotation.SuppressLint
import android.content.Context
import android.net.Uri
import android.net.http.SslError
import android.os.Handler
import android.os.Looper
import android.view.View
import android.webkit.ClientCertRequest
import android.webkit.ConsoleMessage
import android.webkit.GeolocationPermissions
import android.webkit.HttpAuthHandler
import android.webkit.JsResult
import android.webkit.JsPromptResult
import android.webkit.PermissionRequest
import android.webkit.RenderProcessGoneDetail
import android.webkit.SslErrorHandler
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebMessage
import android.webkit.WebMessagePort
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import org.json.JSONObject
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

internal interface PrivateEngineRuntime : AutoCloseable {
    /** Called on a serialized worker, never the Android UI thread. */
    fun call(command: PrivateEngineCommand, payload: JSONObject, progress: (String, Int) -> Unit = { _, _ -> }): JSONObject
    fun setForeground(foreground: Boolean)
    /** Stops the local runtime; does not cancel any released blockchain operation or journal entry. */
    fun cancelLocalWork()
}

internal interface PrivateEnginePorts {
    /** Strict port schemas are implemented by native code. There is no URL/fetch/sign/submit port. */
    fun request(port: String, payload: JSONObject): JSONObject
}

/**
 * Dedicated offscreen, local-only JavaScript/WASM host. Native views remain the UI. One transferred
 * MessagePort is the entire bridge; no Java object is exposed to JavaScript and no user input is
 * interpolated into executable source. Startup and every message are tied to one random session.
 */
internal class PrivateWebViewEngineRuntime private constructor(
    private val context: Context,
    private val assets: PrivateEngineAssets,
    private val ports: PrivateEnginePorts
) : PrivateEngineRuntime {
    private class Pending(val future: CompletableFuture<JSONObject>, val progress: (String, Int) -> Unit)
    private val main = Handler(Looper.getMainLooper())
    private val portWork = Executors.newSingleThreadExecutor()
    private val sessionId = PrivateIdentifiers.create()
    private val started = CompletableFuture<Unit>()
    private val closed = AtomicBoolean(false)
    private val pending = ConcurrentHashMap<String, Pending>()
    private val pendingPorts = ConcurrentHashMap.newKeySet<String>()
    private val portCount = AtomicInteger()
    @Volatile private var foreground = true
    private var webView: WebView? = null
    private var nativePort: WebMessagePort? = null
    private var connected = false

    companion object {
        fun open(context: Context, assets: PrivateEngineAssets, ports: PrivateEnginePorts): PrivateWebViewEngineRuntime {
            PrivatePaymentRelease.requireAvailable()
            if (Looper.myLooper() == Looper.getMainLooper()) throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE)
            assets.verifyAll()
            return PrivateWebViewEngineRuntime(context.applicationContext, assets, ports).also { runtime ->
                runtime.main.post { runtime.createView() }
                try { runtime.started.get(30, TimeUnit.SECONDS) }
                catch (_: Exception) { runtime.close(); throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE) }
            }
        }
    }

    override fun call(command: PrivateEngineCommand, payload: JSONObject, progress: (String, Int) -> Unit): JSONObject {
        if (Looper.myLooper() == Looper.getMainLooper() || closed.get() || !started.isDone) {
            throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE)
        }
        if (!foreground && command in setOf(PrivateEngineCommand.INITIALIZE, PrivateEngineCommand.PREPARE,
                PrivateEngineCommand.PROVE, PrivateEngineCommand.IMPORT_STATE, PrivateEngineCommand.RECOVERY_CUTOFF,
                PrivateEngineCommand.RECOVERY_PREPARE)) {
            throw PrivatePaymentFailure(PrivateFailure.FOREGROUND_REQUIRED)
        }
        val id = PrivateIdentifiers.create()
        val message = PrivateEngineWire.request(sessionId, id, command, payload)
        val waiting = Pending(CompletableFuture(), progress)
        synchronized(pending) {
            if (pending.isNotEmpty() || closed.get()) throw PrivatePaymentFailure(PrivateFailure.BUSY)
            pending[id] = waiting
        }
        main.post {
            if (closed.get()) waiting.future.completeExceptionally(PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE))
            else try { nativePort?.postMessage(WebMessage(message)) ?: failRuntime() }
                catch (_: Exception) { failRuntime() }
        }
        try { return waiting.future.get(command.timeoutMillis, TimeUnit.MILLISECONDS) }
        catch (error: java.util.concurrent.ExecutionException) {
            throw (error.cause as? PrivatePaymentFailure) ?: PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE)
        } catch (_: Exception) {
            close()
            throw PrivatePaymentFailure(PrivateFailure.LOCAL_CANCELLED)
        } finally { pending.remove(id) }
    }

    override fun setForeground(foreground: Boolean) {
        this.foreground = foreground
        main.post { if (!closed.get()) { if (foreground) webView?.onResume() else webView?.onPause() } }
    }

    override fun cancelLocalWork() { closeWith(PrivateFailure.LOCAL_CANCELLED) }

    @SuppressLint("SetJavaScriptEnabled")
    private fun createView() {
        if (closed.get()) return
        try {
            WebView.setWebContentsDebuggingEnabled(false)
            val view = WebView(context)
            webView = view
            view.importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
            view.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
            view.isSaveEnabled = false
            view.isFocusable = false
            view.isClickable = false
            view.settings.apply {
                javaScriptEnabled = true
                javaScriptCanOpenWindowsAutomatically = false
                setSupportMultipleWindows(false)
                blockNetworkLoads = true
                blockNetworkImage = true
                allowFileAccess = false
                allowContentAccess = false
                allowFileAccessFromFileURLs = false
                allowUniversalAccessFromFileURLs = false
                domStorageEnabled = false
                databaseEnabled = false
                cacheMode = WebSettings.LOAD_NO_CACHE
                mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
                setGeolocationEnabled(false)
                mediaPlaybackRequiresUserGesture = true
                safeBrowsingEnabled = true
            }
            // This is the application's only WebView; public wallet UI remains native. The engine
            // has no web login or persistent cookie store, including first-party cookies.
            android.webkit.CookieManager.getInstance().setAcceptCookie(false)
            android.webkit.CookieManager.getInstance().setAcceptThirdPartyCookies(view, false)
            view.webChromeClient = object : WebChromeClient() {
                override fun onConsoleMessage(consoleMessage: ConsoleMessage?): Boolean = true
                override fun onPermissionRequest(request: PermissionRequest) { request.deny() }
                override fun onGeolocationPermissionsShowPrompt(origin: String?, callback: GeolocationPermissions.Callback?) { callback?.invoke(origin, false, false) }
                override fun onJsAlert(view: WebView?, url: String?, message: String?, result: JsResult?): Boolean { result?.cancel(); return true }
                override fun onJsConfirm(view: WebView?, url: String?, message: String?, result: JsResult?): Boolean { result?.cancel(); return true }
                override fun onJsPrompt(view: WebView?, url: String?, message: String?, defaultValue: String?, result: JsPromptResult?): Boolean { result?.cancel(); return true }
                override fun onShowFileChooser(webView: WebView?, filePathCallback: ValueCallback<Array<Uri>>?, fileChooserParams: FileChooserParams?): Boolean {
                    filePathCallback?.onReceiveValue(null); return true
                }
            }
            view.webViewClient = object : WebViewClient() {
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean = true
                override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse {
                    if (request.method != "GET") return assets.response("denied:")
                    return assets.response(request.url.toString())
                }
                override fun onPageFinished(view: WebView, url: String) {
                    if (url != PrivateEngineAssets.DOCUMENT || connected || closed.get()) { failRuntime(); return }
                    connected = true
                    val pair = view.createWebMessageChannel()
                    nativePort = pair[0]
                    pair[0].setWebMessageCallback(object : WebMessagePort.WebMessageCallback() {
                        override fun onMessage(port: WebMessagePort, message: WebMessage) { receive(message) }
                    }, main)
                    view.postWebMessage(WebMessage(PrivateEngineWire.connect(sessionId), arrayOf(pair[1])), Uri.parse(PrivateEngineAssets.ORIGIN))
                }
                override fun onReceivedSslError(view: WebView?, handler: SslErrorHandler, error: SslError?) { handler.cancel(); failRuntime() }
                override fun onReceivedHttpAuthRequest(view: WebView?, handler: HttpAuthHandler, host: String?, realm: String?) { handler.cancel(); failRuntime() }
                override fun onReceivedClientCertRequest(view: WebView?, request: ClientCertRequest) { request.cancel(); failRuntime() }
                override fun onReceivedError(view: WebView?, request: WebResourceRequest?, error: WebResourceError?) { failRuntime() }
                override fun onRenderProcessGone(view: WebView?, detail: RenderProcessGoneDetail?): Boolean { failRuntime(); return true }
            }
            // The HTML itself is immutable hash-verified package data, never user-derived markup.
            // It must carry the same CSP as intercepted resources because loadDataWithBaseURL has
            // no HTTP headers. Reject a bundle which omits that policy before loading any keys.
            val document = assets.document()
            if (!document.contains("http-equiv=\"Content-Security-Policy\"") || !document.contains(PrivateEngineAssets.CSP)) {
                throw PrivatePaymentFailure(PrivateFailure.UNAVAILABLE)
            }
            view.loadDataWithBaseURL(PrivateEngineAssets.DOCUMENT, document, "text/html", "UTF-8", null)
        } catch (_: Exception) { failRuntime() }
    }

    private fun receive(message: WebMessage) {
        if (closed.get() || message.ports?.isNotEmpty() == true) { failRuntime(); return }
        try {
            val event = PrivateEngineWire.parse(message.data ?: throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE), sessionId)
            when (event) {
                is PrivateEngineEvent.Ready -> {
                    if (started.isDone) failRuntime() else started.complete(Unit)
                }
                is PrivateEngineEvent.Result -> {
                    val active = pending[event.requestId] ?: throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE)
                    if (pendingPorts.isNotEmpty() || !active.future.complete(event.result)) failRuntime()
                }
                is PrivateEngineEvent.Failure -> {
                    val active = pending[event.requestId] ?: throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE)
                    if (!active.future.completeExceptionally(PrivatePaymentFailure(event.failure))) failRuntime()
                }
                is PrivateEngineEvent.Progress -> {
                    val active = pending[event.requestId] ?: throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE)
                    active.progress(event.phase, event.percent)
                }
                is PrivateEngineEvent.Port -> dispatchPort(event)
            }
        } catch (_: Exception) { failRuntime() }
    }

    private fun dispatchPort(request: PrivateEngineEvent.Port) {
        if (!pending.containsKey(request.requestId) || !pendingPorts.add(request.portId) || portCount.incrementAndGet() > 8) {
            failRuntime(); return
        }
        portWork.execute {
            val response = try {
                if (closed.get()) throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE)
                PrivateEngineWire.portResult(sessionId, request.requestId, request.portId, ports.request(request.port, request.payload))
            } catch (_: Exception) { PrivateEngineWire.portError(sessionId, request.requestId, request.portId) }
            main.post {
                pendingPorts.remove(request.portId)
                portCount.decrementAndGet()
                if (!closed.get() && pending.containsKey(request.requestId)) try { nativePort?.postMessage(WebMessage(response)) }
                    catch (_: Exception) { failRuntime() }
            }
        }
    }

    private fun failRuntime() { closeWith(PrivateFailure.LOCAL_ENGINE) }
    private fun closeWith(reason: PrivateFailure) {
        if (!closed.compareAndSet(false, true)) return
        started.completeExceptionally(PrivatePaymentFailure(reason))
        pending.values.forEach { it.future.completeExceptionally(PrivatePaymentFailure(reason)) }
        pending.clear()
        portWork.shutdownNow()
        main.post {
            runCatching { nativePort?.close() }
            nativePort = null
            webView?.let { view ->
                runCatching { view.stopLoading(); view.clearHistory(); view.clearCache(true); view.destroy() }
            }
            webView = null
        }
    }
    override fun close() { closeWith(PrivateFailure.LOCAL_ENGINE) }
}
