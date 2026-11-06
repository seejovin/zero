package app.localwallet.mainnet

import uniffi.wallet_core.privateCapabilities

/** Independent compiled native release gate. It cannot be enabled by a UI or service response. */
internal object PrivateNativeRelease {
    fun requireQualified() {
        PrivatePaymentRelease.requireAvailable()
        val capabilities = try { RpcJson.parse(privateCapabilities(), maxChars = 8_192, maxNodes = 50) }
            catch (_: Exception) { throw PrivatePaymentFailure(PrivateFailure.UNAVAILABLE) }
        PrivateEngineWire.fields(capabilities, "schema_version", "enabled", "protocol", "candidate_chain_id", "candidate_asset",
            "journal_schema", "ohttp_client_library", "proof_verification", "android_prover", "deployed_privacy_transport", "release_review", "public_fallback")
        PrivateEngineWire.number(capabilities, "schema_version", 1, 1)
        PrivateEngineWire.number(capabilities, "journal_schema", 2, 2)
        PrivateEngineWire.number(capabilities, "candidate_chain_id", 42161, 42161)
        if (PrivateEngineWire.string(capabilities, "protocol") != PrivateEngineWire.PROTOCOL ||
            PrivateEngineWire.string(capabilities, "candidate_asset") != "USDC" ||
            PrivateEngineWire.boolean(capabilities, "public_fallback") ||
            listOf("enabled", "ohttp_client_library", "proof_verification", "android_prover", "deployed_privacy_transport", "release_review")
                .any { !PrivateEngineWire.boolean(capabilities, it) }) {
            throw PrivatePaymentFailure(PrivateFailure.UNAVAILABLE)
        }
    }
}
