# Local private engine and provider verifier

This workspace implements the actual offline RAILGUN V2 engine used by the Android source candidate: private address derivation, encrypted-note discovery, membership witnesses, exact private transfers, real Groth16 proving/verification, encrypted cache, receipt observation and recovery self-transfers. The provider module verifies the exact proof and opens only its own fee note.

**Production private transfers remain disabled.** The default bundle contains no approved deployment or circuit manifest. A supplied manifest does not override the Kotlin/native/operator release gates. Android compilation, JVM tests, lint and APK assembly pass for 0.13.0. Device qualification, current deployed-verifier correspondence, independent transport operation and security review are still required. See [the upgrade report](../docs/PRIVATE_GAS_UPGRADE.md).

## Components

| Source | Responsibility |
| --- | --- |
| `protocol.mjs` | Narrow pinned SDK primitives, protocol address/key/note behavior; no full wallet network wrapper |
| `local-engine.mjs` | Complete bounded pool scan, exact selection/Max, approval-bound proof flow, reservations and receipt observation |
| `compact-forest.mjs` | Active public tree plus retained closed-tree membership paths; strict rollover and snapshot checks |
| `merkle-forest.mjs` | Full reference tree used by protocol comparisons and compact-tree machinery |
| `local-prover.mjs` / `local-verifier.mjs` | Real snarkjs proving, exact public-signal checks and mandatory verification against pinned artifacts |
| `recovery-engine.mjs` | Cutoff/finalized rescans and native-plan-bound self-sweep preparation |
| `encrypted-state.mjs` | AES-GCM encrypted state with wallet/domain identity and bounded chunked persistence |
| `bridge-runtime.mjs` | Transferred MessagePort protocol with typed native ports, session binding and command serialization |
| `candidate-calldata.mjs` | Exact canonical ABI and envelope binding; rejects unshield/adapt/native-value routes |
| `provider-verifier.mjs` | Actual proof verification followed by exact provider-fee opening; never a generic decryptor |
| `scripts/provider-verifier.mjs` | Bounded single-request offline subprocess entry point |

The browser bundle disables direct SDK networking and optional dynamically initialized WASM accelerators, uses WebCrypto randomness and the pinned pure-JS/Noble fallbacks, and does not use the SDK's global witness/proof cache. The native host supplies fixed pool queries, artifact chunks and encrypted storage only. No secret or private recipient is sent to an analytics service.

## Install and routine verification

From this directory, with the pinned Node toolchain:

```sh
npm ci --ignore-scripts
npm audit --json
npm run inspect
npm test
npm run build
npm run test:bundle
```

`inspect` checks the real SDK interface and the vendored cryptographic source hashes. The current audit records zero known advisory entries; `qualification-audit.json` preserves the earlier failed qualification. The vendored circomlib source is unchanged. Its metadata excludes unused legacy `web3` dependencies; required `web3-utils` remains.

`test:bundle` executes the actual built code in an isolated Node VM with transferred MessagePorts and string-code generation disabled. It is not an Android or browser-CSP test. `test:browser` is a separate Chromium test that could not be run in the current environment; do not treat its presence as a passing result.

## Authenticate public proving artifacts

From the repository root:

```sh
python3 scripts/fetch-private-artifacts.py \
  --output build/protocol-artifacts/upstream \
  --shapes 01x03,02x03,03x03,04x03,05x03,06x03,07x03,08x03,09x03,10x03,11x03
cd privacy-engine
node scripts/qualify-artifacts.mjs \
  ../build/protocol-artifacts/upstream \
  ../build/protocol-artifacts/qualified-all \
  01x03,02x03,03x03,04x03,05x03,06x03,07x03,08x03,09x03,10x03,11x03
```

The downloader accepts only fixed HTTPS gateways, a fixed immutable CID and supported circuit shapes. It rejects redirects, bounds downloads and never replaces an existing file. Downloaded bytes alone are untrusted.

Qualification authenticates decompressed WASM/zkey bytes against the pinned upstream catalog, derives the verification key from the authenticated proving key, compares a supplied published key and writes immutable output files. It does not approve a deployment or create a trusted setup. See [ARTIFACT_PROVENANCE.md](config/ARTIFACT_PROVENANCE.md).

## Positive native/protocol and packaged-runtime tests

Build the native bridge first, from the repository root:

```sh
./scripts/verify-bridge.sh
cd privacy-engine
npm run test:protocol -- ../build/protocol-artifacts/qualified-all ../build/bridge-check 01x03
npm run test:protocol -- ../build/protocol-artifacts/qualified-all ../build/bridge-check 11x03
npm run test:bundle:protocol -- ../build/protocol-artifacts/qualified-all ../build/bridge-check
```

These tests use deliberately public fixture keys, actual native-derived protocol keys, native-sealed approval/envelope digests and actual Groth16 proofs. They verify the recipient and provider fee ciphertexts, exact integer Max, altered public-input rejection and encrypted native journal restart. The bundled test also scans an encrypted note through the MessagePort bridge, persists reservations and recovers a lost acknowledgement from exact pool/receipt evidence. Missing receipts, wrong calldata and missing output logs are rejected appropriately.

All chain observations are controlled offline fixtures. No synthetic Merkle root is asserted to exist on mainnet. No funded transaction, production phrase or Android runtime is used. The first/maximum-input proofs are genuine; that does not constitute deployed-verifier approval.

## Android packaging and resource limits

`android/app` runs the locked build during `preBuild`. The normal build generates a disabled engine with no circuit parameters. A reviewed release manifest supplies domain, fee recipient, deployment block and per-file circuit path/SHA-256/size; the build independently requires WASM/zkey hashes to match the pinned upstream catalog. A separate native release qualification remains mandatory.

The engine asset manifest lists only hash-verified resources. Artifact reads are chunked, individual files are limited to 256 MiB and the package to 512 MiB. All eleven qualified circuit sets currently total 272,781,169 uncompressed bytes. Device memory, APK size, proving time, historical synchronization and actual WebView CSP behavior must be measured before release.

The build also generates `third-party-notices.txt` and an inventory from the actual esbuild input graph. snarkjs and circomlibjs declare GPL-3.0; the engine declares MIT. Notices are not a substitute for a complete distribution/corresponding-source review. Missing upstream full-notice text is listed explicitly, never invented.

## Trust and recovery limits

Pool evidence is normalized and checked against two RPC providers, canonical headers, roots, exact finalized checkpoints and complete rollover-block receipts. This is not a consensus light client. Unsupported/oversize history fails explicitly; it never becomes an empty wallet.

Private history backups are authenticated and seed-bound. A restored wallet cannot assume an old released proof no longer exists. Native quarantine, complete cutoff scans and explicitly approved self-sweep receipts are required before unresolved inputs become ordinarily spendable. Tiny notes that cannot cover the recovery fee remain explicit unresolved dust rather than being discarded.

The operator can observe opaque requests, fees and timing; it cannot derive private principal or recipient from a correctly qualified proof and ciphertext without the corresponding secret information. OHTTP does not remove timing correlation or protect against colluding relay/gateway operators. Public shielding/unshielding and ordinary public-wallet RPC traffic remain outside that confidentiality claim.
