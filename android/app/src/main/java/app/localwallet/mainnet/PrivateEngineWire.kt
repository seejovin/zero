package app.localwallet.mainnet

import org.json.JSONObject
import java.security.SecureRandom

/** Versioned messages for the isolated bundled engine. Never accepts executable JavaScript. */
internal enum class PrivateEngineCommand(val wire: String, val timeoutMillis: Long) {
    INITIALIZE("initialize", 60_000), SYNC("sync", 600_000), BALANCE("balance", 30_000),
    PREPARE("prepare", 120_000), PROVE("prove", 600_000), VERIFY("verify", 120_000), OBSERVE("observe", 120_000),
    RESTORE_RESERVATIONS("restore_reservations", 120_000),
    RELEASE_CANCELLED_RESERVATIONS("release_cancelled_reservations", 120_000),
    RECOVERY_CUTOFF("recovery_cutoff", 120_000), RECOVERY_SCAN("recovery_scan", 600_000),
    RECOVERY_PREPARE("recovery_prepare", 120_000),
    EXPORT_STATE("export_state", 120_000), IMPORT_STATE("import_state", 120_000),
    CANCEL("cancel", 30_000), CLOSE("close", 10_000)
}

internal object PrivateIdentifiers {
    private val random = SecureRandom()
    fun create(): String {
        val bytes = ByteArray(32)
        do { random.nextBytes(bytes) } while (bytes.all { it == 0.toByte() })
        return try { "0x" + bytes.joinToString("") { "%02x".format(it.toInt() and 255) } }
            finally { bytes.fill(0) }
    }
    fun require(value: String): String {
        if (!Regex("0x[0-9a-f]{64}").matches(value) || value.drop(2).all { it == '0' }) {
            throw PrivatePaymentFailure(PrivateFailure.INVALID_INPUT)
        }
        return value
    }
}

internal sealed class PrivateEngineEvent {
    class Ready : PrivateEngineEvent()
    class Result(val requestId: String, val result: JSONObject) : PrivateEngineEvent() {
        override fun toString() = "PrivateEngineEvent.Result([local data redacted])"
    }
    class Failure(val requestId: String, val failure: PrivateFailure) : PrivateEngineEvent()
    class Progress(val requestId: String, val phase: String, val percent: Int) : PrivateEngineEvent()
    class Port(val requestId: String, val portId: String, val port: String, val payload: JSONObject) : PrivateEngineEvent() {
        override fun toString() = "PrivateEngineEvent.Port([local data redacted])"
    }
}

internal object PrivateEngineWire {
    const val VERSION = 1
    const val MAX_MESSAGE_CHARS = 2_097_152
    const val PROTOCOL = "railgun-v2-candidate"
    const val SDK_VERSION = "local-wallet-engine-1"
    const val ENGINE_VERSION = "9.9.1"
    private val phases = setOf("initializing", "syncing", "preparing", "proving", "verifying", "reconciling", "exporting", "importing")
    private val ports = setOf("rpc", "storage_read", "storage_write", "artifact_read")

    fun connect(sessionId: String): String = base("connect", sessionId).toString()

    fun request(sessionId: String, requestId: String, command: PrivateEngineCommand, payload: JSONObject): String =
        base("request", sessionId).put("request_id", PrivateIdentifiers.require(requestId))
            .put("command", command.wire).put("payload", payload).toString().also(::bound)

    fun portResult(sessionId: String, requestId: String, portId: String, result: JSONObject): String =
        base("port_result", sessionId).put("request_id", PrivateIdentifiers.require(requestId))
            .put("port_request_id", PrivateIdentifiers.require(portId)).put("result", result).toString().also(::bound)

    fun portError(sessionId: String, requestId: String, portId: String): String =
        base("port_error", sessionId).put("request_id", PrivateIdentifiers.require(requestId))
            .put("port_request_id", PrivateIdentifiers.require(portId)).put("code", "UNAVAILABLE").toString()

    fun parse(text: String, expectedSessionId: String): PrivateEngineEvent {
        bound(text)
        val value = RpcJson.parse(text, maxChars = MAX_MESSAGE_CHARS, maxNodes = 70_000)
        return parseObject(value, expectedSessionId)
    }

    /** Kept separate so strict closed-schema behavior is tested without an Android runtime stub. */
    internal fun parseObject(value: JSONObject, expectedSessionId: String): PrivateEngineEvent {
        number(value, "schema_version", VERSION.toLong(), VERSION.toLong())
        if (string(value, "session_id") != expectedSessionId) invalid()
        return when (string(value, "kind")) {
            "ready" -> {
                fields(value, "schema_version", "session_id", "kind", "protocol", "sdk_version", "engine_version")
                if (string(value, "protocol") != PROTOCOL || string(value, "sdk_version") != SDK_VERSION ||
                    string(value, "engine_version") != ENGINE_VERSION) invalid()
                PrivateEngineEvent.Ready()
            }
            "result" -> {
                fields(value, "schema_version", "session_id", "kind", "request_id", "result")
                PrivateEngineEvent.Result(id(value, "request_id"), value.getJSONObject("result"))
            }
            "error" -> {
                fields(value, "schema_version", "session_id", "kind", "request_id", "code")
                val reason = when (string(value, "code")) {
                    "INVALID_INPUT" -> PrivateFailure.INVALID_INPUT
                    "SYNC_INCOMPLETE" -> PrivateFailure.SYNC_INCOMPLETE
                    "INSUFFICIENT_FUNDS" -> PrivateFailure.INSUFFICIENT_FUNDS
                    "QUOTE_EXPIRED" -> PrivateFailure.QUOTE_EXPIRED
                    "CONSOLIDATION_REQUIRED" -> PrivateFailure.CONSOLIDATION_REQUIRED
                    "STORAGE" -> PrivateFailure.STORAGE
                    "BACKUP_REQUIRED" -> PrivateFailure.BACKUP_REQUIRED
                    "RECOVERY_WAITING_FOR_FINALITY" -> PrivateFailure.RECOVERY_WAITING_FOR_FINALITY
                    "RECOVERY_CUTOFF_REORG" -> PrivateFailure.RECOVERY_CUTOFF_REORG
                    "RECOVERY_CAPACITY_EXCEEDED" -> PrivateFailure.RECOVERY_CAPACITY
                    "RECOVERY_FEE_EXCEEDS_BALANCE" -> PrivateFailure.RECOVERY_FEE_EXCEEDS_BALANCE
                    "RECOVERY_NEEDS_RECONCILIATION", "RECOVERY_NOTE_MISMATCH", "RECOVERY_SELF_TRANSFER_ONLY" -> PrivateFailure.RECONCILIATION
                    "RECOVERY_REVIEW_IN_PROGRESS" -> PrivateFailure.BUSY
                    "PROOF_INVALID", "PROTOCOL_MISMATCH" -> PrivateFailure.PROOF_INVALID
                    "CANCELLED" -> PrivateFailure.LOCAL_CANCELLED
                    "UNAVAILABLE", "TRANSPORT_UNAVAILABLE" -> PrivateFailure.UNAVAILABLE
                    "LOCAL_ENGINE" -> PrivateFailure.LOCAL_ENGINE
                    else -> invalid()
                }
                PrivateEngineEvent.Failure(id(value, "request_id"), reason)
            }
            "progress" -> {
                fields(value, "schema_version", "session_id", "kind", "request_id", "phase", "percent")
                val phase = string(value, "phase")
                val percent = number(value, "percent", 0, 100).toInt()
                if (phase !in phases) invalid()
                PrivateEngineEvent.Progress(id(value, "request_id"), phase, percent)
            }
            "port_request" -> {
                fields(value, "schema_version", "session_id", "kind", "request_id", "port_request_id", "port", "payload")
                val port = string(value, "port")
                if (port !in ports) invalid()
                PrivateEngineEvent.Port(id(value, "request_id"), id(value, "port_request_id"), port, value.getJSONObject("payload"))
            }
            else -> invalid()
        }
    }

    fun fields(value: JSONObject, vararg expected: String) {
        if (value.keys().asSequence().toSet() != expected.toSet()) invalid()
    }
    fun string(value: JSONObject, field: String): String = (value.get(field) as? String) ?: invalid()
    fun boolean(value: JSONObject, field: String): Boolean = (value.get(field) as? Boolean) ?: invalid()
    fun number(value: JSONObject, field: String, minimum: Long, maximum: Long): Long {
        val raw = value.get(field)
        if (raw !is Number || !Regex("0|[1-9][0-9]*").matches(raw.toString())) invalid()
        return raw.toString().toLongOrNull()?.takeIf { it in minimum..maximum } ?: invalid()
    }
    private fun id(value: JSONObject, field: String): String = PrivateIdentifiers.require(string(value, field))
    private fun base(kind: String, sessionId: String) = JSONObject().put("schema_version", VERSION)
        .put("kind", kind).put("session_id", PrivateIdentifiers.require(sessionId))
    private fun bound(text: String) { if (text.length !in 2..MAX_MESSAGE_CHARS) invalid() }
    private fun invalid(): Nothing = throw PrivatePaymentFailure(PrivateFailure.LOCAL_ENGINE)
}
