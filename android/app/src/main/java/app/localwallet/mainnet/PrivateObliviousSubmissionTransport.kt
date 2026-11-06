package app.localwallet.mainnet

import uniffi.wallet_core.PrivateOhttpGateway
import uniffi.wallet_core.PrivateOhttpRequest
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.net.CookieHandler
import java.net.ResponseCache
import java.net.SocketTimeoutException
import java.net.URL
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import javax.net.ssl.HttpsURLConnection

/**
 * Shared release material, never user-specific configuration. Different hostnames alone do not
 * establish independent ownership; a release review must verify relay/gateway independence too.
 */
internal class PrivateRelayConfiguration internal constructor(
    val relay: URL,
    val gatewayAuthority: String,
    gatewayKey: ByteArray,
    keyDigest: ByteArray
) {
    private val encodedKey = gatewayKey.copyOf()
    private val trustedDigest = keyDigest.copyOf()
    init {
        if (relay.protocol != "https" || relay.userInfo != null || relay.query != null || relay.ref != null ||
            relay.host.isBlank() || relay.port !in setOf(-1, 443) || relay.path.isBlank() ||
            relay.host.equals(gatewayAuthority.substringBefore(':'), ignoreCase = true) ||
            encodedKey.size !in 1..16_384 || trustedDigest.size != 32) {
            throw PrivatePaymentFailure(PrivateFailure.UNAVAILABLE)
        }
    }

    fun nativeGateway(): PrivateOhttpGateway = PrivateOhttpGateway(gatewayAuthority, encodedKey.copyOf(), trustedDigest.copyOf())

    companion object {
        /** No approved relay, gateway key or independent deployment is present in this build. */
        fun reviewedRelease(): PrivateRelayConfiguration = throw PrivatePaymentFailure(PrivateFailure.UNAVAILABLE)
    }
}

/**
 * Concrete OHTTP transport over an independent HTTPS relay. Only a durable released envelope may
 * enter; the Rust bridge enforces closed-schema/digest binding and encrypts with the pinned gateway.
 * There is no mnemonic, viewing key, recipient or principal argument, and no cleartext fallback.
 * The local service implements this closed gateway contract. A production relay deployment and
 * reviewed release configuration remain unavailable in this build.
 */
internal class PrivateObliviousSubmissionTransport internal constructor(
    private val configuration: PrivateRelayConfiguration,
    private val open: (URL) -> HttpsURLConnection = { it.openConnection() as HttpsURLConnection }
) : PrivateSubmissionTransport, PrivateFeeScheduleSource, PrivatePoolReadTransport {
    companion object {
        private const val MAX_ENVELOPE_BYTES = 540_672
        private const val REQUEST_MILLIS = 20_000L
        private val deadlines = Executors.newSingleThreadScheduledExecutor { task ->
            Thread(task, "private-transport-deadline").apply { isDaemon = true }
        }

        fun create(): PrivateObliviousSubmissionTransport {
            PrivatePaymentRelease.requireAvailable()
            return PrivateObliviousSubmissionTransport(PrivateRelayConfiguration.reviewedRelease())
        }
    }

    override fun submit(operation: PrivateReleasedOperation): PrivateSubmissionAcknowledgement {
        val bytes = operation.transportBytes()
        val envelope = try { String(bytes, Charsets.UTF_8) } finally { bytes.fill(0) }
        val body = RpcJson.parse(exchange(8_192, setOf(200, 202)) { it.sealSubmission(envelope) }, maxChars = 8_192, maxNodes = 10)
        PrivateEngineWire.fields(body, "operation_digest", "accepted")
        if (PrivateEngineWire.string(body, "operation_digest") != operation.operationDigest) {
            throw PrivatePaymentFailure(PrivateFailure.INVALID_ACKNOWLEDGEMENT)
        }
        return PrivateSubmissionAcknowledgement(operation.operationDigest, PrivateEngineWire.boolean(body, "accepted"))
    }

    /** Only a fixed asset/network fee publication is requested; no wallet fields exist in it. */
    override fun packet(): String = exchange(16_384, setOf(200)) { it.sealFeeSchedule() }

    /** Advisory service state is never passed to the journal as canonical receipt evidence. */
    fun advisoryStatus(operationDigest: String): JSONObject {
        PrivateIdentifiers.require(operationDigest)
        val body = RpcJson.parse(exchange(8_192, setOf(200)) { it.sealStatus(operationDigest) }, maxChars = 8_192, maxNodes = 20)
        PrivateEngineWire.fields(body, "schema_version", "operation_digest", "state", "transaction_hash")
        PrivateEngineWire.number(body, "schema_version", 1, 1)
        if (PrivateEngineWire.string(body, "operation_digest") != operationDigest || PrivateEngineWire.string(body, "state") !in
            setOf("not_found", "admitted", "signed", "unknown", "pending", "included", "included_revert", "finalized", "finalized_revert", "retry_exhausted", "reorged", "incident")) {
            throw PrivatePaymentFailure(PrivateFailure.RECONCILIATION)
        }
        if (!body.isNull("transaction_hash")) PrivateIdentifiers.require(PrivateEngineWire.string(body, "transaction_hash"))
        return body
    }

    override fun read(request: JSONObject): JSONObject {
        val checked = PrivatePoolRequestPolicy.check(request)
        val body = RpcJson.parse(exchange(524_288, setOf(200)) { it.sealPoolRequest(checked.toString()) }, maxChars = 524_288)
        PrivatePoolRequestPolicy.checkResponse(checked, body)
        return body
    }

    private fun exchange(maximum: Int, statuses: Set<Int>, seal: (PrivateOhttpGateway) -> PrivateOhttpRequest): String {
        PrivateNativeRelease.requireQualified()
        // Global cookies/cache can record traffic. Streaming disables automatic HTTP auth/redirect
        // retries. Each explicit retry encrypts the same durable operation in a fresh OHTTP context.
        if (CookieHandler.getDefault() != null || ResponseCache.getDefault() != null) throw PrivatePaymentFailure(PrivateFailure.UNAVAILABLE)
        try {
            return configuration.nativeGateway().use { gateway ->
                seal(gateway).use { request ->
                    val outbound = request.ciphertext()
                    val received = try { post(outbound) } finally { outbound.fill(0) }
                    val decoded = try { request.openResponse(200.toUShort(), "message/ohttp-res", received) }
                        finally { received.fill(0) }
                    val wrapper = RpcJson.parse(decoded, maxChars = 1_048_576)
                    PrivateEngineWire.fields(wrapper, "status", "body_json")
                    if (PrivateEngineWire.number(wrapper, "status", 100, 599).toInt() !in statuses) throw PrivatePaymentFailure(PrivateFailure.UNKNOWN_OUTCOME)
                    PrivateEngineWire.string(wrapper, "body_json").also { if (it.length > maximum) throw PrivatePaymentFailure(PrivateFailure.UNKNOWN_OUTCOME) }
                }
            }
        } catch (error: PrivatePaymentFailure) { throw error }
        catch (_: Exception) { throw PrivatePaymentFailure(PrivateFailure.UNKNOWN_OUTCOME) }
    }

    private fun post(ciphertext: ByteArray): ByteArray {
        if (ciphertext.isEmpty() || ciphertext.size > MAX_ENVELOPE_BYTES) throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
        val endpoint = configuration.relay
        val connection = open(endpoint)
        val started = System.nanoTime()
        // HttpsURLConnection has no separate write timeout. Closing the connection at the overall
        // deadline bounds a stalled upload as well; an interrupted submission remains unknown.
        val deadline = deadlines.schedule({ connection.disconnect() }, REQUEST_MILLIS, TimeUnit.MILLISECONDS)
        fun remaining(): Int {
            val millis = REQUEST_MILLIS - (System.nanoTime() - started) / 1_000_000L
            if (millis <= 0) throw SocketTimeoutException()
            return millis.coerceAtMost(8_000L).toInt().coerceAtLeast(1)
        }
        try {
            if (connection.url != endpoint) throw PrivatePaymentFailure(PrivateFailure.UNAVAILABLE)
            connection.requestMethod = "POST"
            connection.connectTimeout = remaining()
            connection.readTimeout = remaining()
            connection.instanceFollowRedirects = false
            connection.useCaches = false
            connection.doInput = true
            connection.doOutput = true
            connection.setFixedLengthStreamingMode(ciphertext.size)
            connection.setRequestProperty("Content-Type", "message/ohttp-req")
            connection.setRequestProperty("Accept", "message/ohttp-res")
            connection.setRequestProperty("Accept-Encoding", "identity")
            connection.setRequestProperty("Cache-Control", "no-store")
            connection.setRequestProperty("User-Agent", "LocalWallet-Privacy/1")
            connection.setRequestProperty("Cookie", "")
            connection.setRequestProperty("Cookie2", "")
            connection.setRequestProperty("Authorization", "")
            connection.outputStream.use { it.write(ciphertext) }
            connection.readTimeout = remaining()
            if (connection.responseCode != 200 || connection.contentType != "message/ohttp-res" ||
                connection.contentEncoding?.takeIf { it.isNotBlank() } !in setOf(null, "identity") ||
                connection.contentLengthLong > MAX_ENVELOPE_BYTES || connection.getHeaderField("Set-Cookie") != null) {
                throw PrivatePaymentFailure(PrivateFailure.UNKNOWN_OUTCOME)
            }
            val bounded = ByteArrayOutputStream()
            connection.inputStream.use { input ->
                val buffer = ByteArray(8_192)
                try {
                    while (true) {
                        connection.readTimeout = remaining()
                        val count = input.read(buffer)
                        if (count < 0) break
                        if (bounded.size() + count > MAX_ENVELOPE_BYTES) throw PrivatePaymentFailure(PrivateFailure.UNKNOWN_OUTCOME)
                        bounded.write(buffer, 0, count)
                    }
                } finally { buffer.fill(0) }
            }
            remaining()
            return bounded.toByteArray().also { if (it.isEmpty()) throw PrivatePaymentFailure(PrivateFailure.UNKNOWN_OUTCOME) }
        } finally { deadline.cancel(false); connection.disconnect() }
    }
}
