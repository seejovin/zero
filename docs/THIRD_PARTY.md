# Third-party components

The application uses established cryptographic and systems libraries. It does not implement an elliptic curve, ECDSA, Ed25519, BIP39 word list, BIP32 derivation algorithm, operating-system random generator or authenticated-encryption primitive from scratch. The wallet-specific code restricts what may be signed, constructs and validates supported EVM, Solana and TRON transfers, and maintains durable transaction evidence.

## Selected direct components

### 0.12.0 private gas source additions

The native privacy transport uses pinned `ohttp 0.8.0` / `bhttp 0.8.0` with the Rust HPKE backend. New native key derivation and fee verification use established HMAC/HKDF/SHA-2 and Ed25519 libraries. The service adds pinned reqwest0.13.3, rustls0.23.46/rustls-webpki0.103.16 and Tokio1.53.3, bounded HTTP handling and existing Rust cryptographic/SQLite components; no customer secrets are accepted by its API.

The Android computational engine now bundles the narrow actual SDK, not the old full-wallet qualification wrapper:

| Component | Version | Declared license | Role |
| --- | --- | --- | --- |
| `@railgun-community/engine` | 9.9.1 | MIT | Protocol addresses, keys, notes, transactions and witness formatting |
| `@railgun-community/circomlibjs` | 0.0.8, vendored metadata | GPL-3.0 | Upstream cryptographic primitives; source hashes unchanged |
| `snarkjs` | 0.7.6 | GPL-3.0 | Actual Groth16 proving, verification and key extraction |
| `ethers` | 6.17.0 | MIT | Canonical ABI and Ethereum encodings |
| `@noble/hashes` | 1.8.0 | MIT | Narrow offline SHA-2 compatibility |
| `browserify-aes` | 1.2.0 | MIT | Required SDK AES compatibility |

The earlier 89-entry SDK advisory report is retained historically. The current narrowed and pinned runtime graph has zero known advisory entries in `privacy-engine/qualification-audit-current.json`. Removing unused `web3` metadata from circomlib eliminated its unused legacy chains; required `web3-utils` remains. The package's actual cryptographic source is hash-verified against `vendor/circomlibjs/UPSTREAM.json`. A GPL COPYING file is included because the upstream npm package omitted that file. Source modification is not hidden, and the fork is not represented as an untouched upstream npm package.

`privacy-engine/scripts/third-party-notices.mjs` inventories packages reached by the actual esbuild graph, preserves available upstream license/notice text and embeds `third-party-notices.txt` as a hash-pinned APK asset. The accompanying inventory explicitly lists upstream packages whose full notice text is absent. This is not a complete native/JVM distribution-notice bundle and does not establish GPL corresponding-source compliance or the license of the combined application. Resolve those distribution requirements before publishing a binary; the source project does not claim proprietary redistribution clearance.

Public proving artifacts are qualified from the immutable upstream CID and SHA-256 catalog. Their provenance, exact hashes and verification-key derivation are documented in `privacy-engine/config/ARTIFACT_PROVENANCE.md`. Qualifying hashes does not grant a license or certify the deployed contract.

### Retained wallet components

| Component | Pinned version | Purpose | Declared package license |
| --- | --- | --- | --- |
| [aes-gcm](https://github.com/RustCrypto/AEADs) | 0.10.3 | Authenticated vault encryption | Apache-2.0 OR MIT |
| [bip39](https://github.com/rust-bitcoin/rust-bip39/) | 2.2.2 | Mnemonic validation and entropy/seed conversion | CC0-1.0 |
| [bip32](https://github.com/iqlusioninc/crates/tree/main/bip32) | 0.6.0 | Standard hierarchical derivation | Apache-2.0 OR MIT |
| [k256](https://github.com/RustCrypto/elliptic-curves) | 0.14.0 | secp256k1 signing, verification and recovery | Apache-2.0 OR MIT |
| [getrandom](https://github.com/rust-random/getrandom) | 0.3.4 direct | Operating-system randomness | MIT OR Apache-2.0 |
| [zeroize](https://github.com/RustCrypto/utils) | 1.9.1 | Best-effort secret buffer clearing | Apache-2.0 OR MIT |
| [sha3](https://github.com/RustCrypto/hashes) | 0.10.9 | Ethereum Keccak hashing | MIT OR Apache-2.0 |
| [sha2](https://github.com/RustCrypto/hashes) | 0.10.9 direct | SHA-256 transaction and address checks | MIT OR Apache-2.0 |
| [ed25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek/tree/main/ed25519-dalek) | 2.2.0 | Solana Ed25519 signing and verification | BSD-3-Clause |
| [slip10_ed25519](https://gitlab.com/westonian/slip10-ed25519-rust-crate) | 0.1.3 | Hardened Solana derivation | MIT OR Apache-2.0 |
| [bs58](https://github.com/Nullus157/bs58-rs) | 0.5.1 | Base58 and Base58Check | MIT/Apache-2.0 |
| [base64](https://github.com/marshallpierce/rust-base64) | 0.22.1 | Solana wire encoding | MIT OR Apache-2.0 |
| [prost](https://github.com/tokio-rs/prost) | 0.14.4 | TRON restricted protobuf serialization | Apache-2.0 |
| [solana-pubkey / solana-instruction / solana-message / solana-hash](https://github.com/anza-xyz/solana-sdk) | 3.0.0 / 3.0.0 / 3.0.1 / 3.0.0 direct | Solana canonical key, instruction and message types | Apache-2.0 |
| [primitive-types](https://github.com/paritytech/parity-common) | 0.13.1 | Exact public U256 quantities | MIT OR Apache-2.0 |
| [rlp](https://github.com/paritytech/parity-common) | 0.6.1 | Ethereum serialization container | MIT OR Apache-2.0 |
| [rusqlite](https://github.com/rusqlite/rusqlite) | 0.41.1 | SQLite transaction journal | MIT |
| [serde](https://github.com/serde-rs/serde) / [serde_json](https://github.com/serde-rs/json) | 1.0.229 / 1.0.154 | Typed serialization and validation | MIT OR Apache-2.0 |
| [UniFFI](https://github.com/mozilla/uniffi-rs) | 0.32.2 | Generated Kotlin/native language boundary | MPL-2.0 |
| [argon2](https://github.com/RustCrypto/password-hashes) | 0.6.0 | Browser password key derivation | MIT OR Apache-2.0 |
| [wasm-bindgen](https://github.com/wasm-bindgen/wasm-bindgen) | 0.2.130 | Rust/browser boundary | MIT OR Apache-2.0 |
| [JNA](https://github.com/java-native-access/jna) | 5.17.0 | Android native library bridge | LGPL-2.1-or-later OR Apache-2.0 |
| [ZXing](https://github.com/zxing/zxing) | 3.5.3 | Android local QR generation | Apache-2.0 |
| [qrcode](https://github.com/soldair/node-qrcode) | 1.5.4 | Browser local QR generation | MIT |
| [ethers](https://github.com/ethers-io/ethers.js) | 6.17.0 | Independent development-only Ethereum reference | MIT |

The patched transitive arithmetic library is `crypto-bigint 0.7.10`. Additional versions of `getrandom` occur through dependencies and are recorded in the lockfile. The complete Cargo resolution, package metadata and npm lockfile are available for inspection; dependency features determine what is actually built for each target.

`dependency-inventory.json` records package/version/license/repository metadata from the locked Cargo application/test graph. The host `bindgen` feature additionally uses the UniFFI generator's template/parser dependencies, including the maintenance notice documented in `VERIFICATION.md`. This inventory is a provenance aid, not a claim that every resolved platform-specific package is linked into the app.

The optional offline Solana differential checker uses `@solana/web3.js 1.98.4` (MIT), `@solana/spl-token 0.4.15` (Apache-2.0), and `ed25519-hd-key 2.0.0` (MIT), installed outside the application. These are development references and are not shipped in the Android runtime. Its exact setup is recorded in `scripts/check-solana.mjs` and the 0.10.0 verification log.

Generated bindings and external components retain their applicable upstream terms. The source repository contains the original wallet source and lockfiles; matching bindings and third-party binaries are produced by the build. The Android APK and browser bundle contain their compiled runtime dependencies. Review upstream notices when making a distribution. The original application's release license is left to the project owner.

The 0.10.0 source update does not yet include a compiled-runtime license/copyright notice bundle in Android assets. That bundle remains a binary-distribution gate, including BSD notices for the Dalek packages and the ISC notice for hmac-sha512. Package metadata and this table do not replace the applicable upstream text.


## Build tooling

Android builds use JDK 17, AGP 8.9.1, Kotlin 2.1.20, Gradle 8.11.1, SDK 35 and NDK 27.2.12479018. Web builds use pinned TypeScript and esbuild. Lockfiles and Gradle verification metadata document the exact resolution. These compiler/build dependencies are not a wallet custody service.
