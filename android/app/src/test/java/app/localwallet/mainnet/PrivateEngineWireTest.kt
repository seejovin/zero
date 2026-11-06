package app.localwallet.mainnet

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.math.BigDecimal

/** Schema fixtures only. They do not represent SDK/prover/device qualification. */
class PrivateEngineWireTest {
    private val session = "0x" + "1".repeat(64)
    private val request = "0x" + "2".repeat(64)
    private fun message(kind: String) = JSONObject().put("schema_version", 1).put("session_id", session).put("kind", kind)
    private fun ready() = message("ready").put("protocol", PrivateEngineWire.PROTOCOL)
        .put("sdk_version", PrivateEngineWire.SDK_VERSION).put("engine_version", PrivateEngineWire.ENGINE_VERSION)

    @Test fun exactPinnedEngineHandshakeIsRequired() {
        assertTrue(PrivateEngineWire.parseObject(ready(), session) is PrivateEngineEvent.Ready)
        assertThrows(Exception::class.java) { PrivateEngineWire.parseObject(ready().put("engine_version", "9.9.2"), session) }
        assertThrows(Exception::class.java) { PrivateEngineWire.parseObject(ready().put("sdk_version", "wallet-wrapper"), session) }
    }

    @Test fun messagesFromAnotherSessionCannotReachTheEngineCaller() {
        assertThrows(Exception::class.java) { PrivateEngineWire.parseObject(ready().put("session_id", request), session) }
    }

    @Test fun closedSchemaRejectsExecutableFieldsAndUnrecognizedPorts() {
        assertThrows(Exception::class.java) { PrivateEngineWire.parseObject(ready().put("script", "fixture executable content"), session) }
        val port = message("port_request").put("request_id", request).put("port_request_id", "0x" + "3".repeat(64))
            .put("port", "fetch").put("payload", JSONObject().put("url", "https://example.invalid"))
        assertThrows(Exception::class.java) { PrivateEngineWire.parseObject(port, session) }
    }

    @Test fun progressMustHaveACompleteSafeIdentityAndBoundedInteger() {
        val progress = message("progress").put("request_id", request).put("phase", "proving").put("percent", 100)
        assertEquals(100, (PrivateEngineWire.parseObject(progress, session) as PrivateEngineEvent.Progress).percent)
        assertThrows(Exception::class.java) { PrivateEngineWire.parseObject(progress.put("percent", 101), session) }
        assertThrows(Exception::class.java) { PrivateEngineWire.parseObject(progress.put("percent", BigDecimal("0.5")), session) }
        assertThrows(Exception::class.java) { PrivateEngineWire.parseObject(progress.put("percent", "50"), session) }
    }

    @Test fun failuresExposeOnlyFixedSanitizedCodes() {
        val value = message("error").put("request_id", request).put("code", "RECOVERY_WAITING_FOR_FINALITY")
        val event = PrivateEngineWire.parseObject(value, session) as PrivateEngineEvent.Failure
        assertEquals(PrivateFailure.RECOVERY_WAITING_FOR_FINALITY, event.failure)
        assertThrows(Exception::class.java) { PrivateEngineWire.parseObject(value.put("details", "fixture secret"), session) }
        assertThrows(Exception::class.java) { PrivateEngineWire.parseObject(value.remove("details").let { value.put("code", "arbitrary remote reason") }, session) }
    }

    @Test fun aResponseCannotUseZeroOrMalformedRequestIdentity() {
        val value = message("result").put("request_id", "0x" + "0".repeat(64)).put("result", JSONObject())
        assertThrows(Exception::class.java) { PrivateEngineWire.parseObject(value, session) }
        assertThrows(Exception::class.java) { PrivateEngineWire.parseObject(value.put("request_id", "../../fixture"), session) }
    }

    @Test fun oversizedWireDataIsRejectedBeforeAnyJsonOrAndroidParser() {
        assertThrows(PrivatePaymentFailure::class.java) { PrivateEngineWire.parse("x".repeat(PrivateEngineWire.MAX_MESSAGE_CHARS + 1), session) }
    }

    @Test fun deviceLocalResultDebugTextDoesNotExposeItsPayload() {
        val result = message("result").put("request_id", request).put("result", JSONObject().put("private_fixture", "must remain local"))
        assertFalse(PrivateEngineWire.parseObject(result, session).toString().contains("must remain local"))
    }
}
