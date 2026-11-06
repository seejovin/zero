# OHTTP client boundary

This Rust library uses pinned `ohttp 0.8.0` and `bhttp 0.8.0` implementations for RFC 9458 HPKE encapsulation and binary HTTP. It does not implement new cryptography and opens no sockets.

- A gateway authority and key must be authenticated by a reviewed shared configuration. SHA-256 pin matching is not proof that an arbitrary fetched pin is trusted.
- Inner methods/paths/headers are fixed, request bodies are bounded and padded to 4096-byte blocks, and the native bridge accepts a closed public submission schema.
- Each encryption uses a fresh HPKE context. Responses must authenticate to the same request; the response context is consumed even when rejected.
- Redirect/plaintext fallback and arbitrary response headers are rejected. Response sizes are bounded before binary-HTTP parsing.

Host tests cover a real client/server HPKE round trip, key/authority rejection, fresh encapsulations of identical operations, cross-request/tampered response rejection and no cleartext/redirect fallback. These tests do not establish an independent relay deployment or resistance to collusion/timing correlation.

The wrapper is exposed to Android through `core/src/private_bridge.rs`; the Android transport remains disabled because there is no approved configuration or complete private engine. See [PRIVATE_GAS_UPGRADE.md](../docs/PRIVATE_GAS_UPGRADE.md).
