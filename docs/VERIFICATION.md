# Verification report — Android 0.12.0 source candidate / web 0.6.0

## Android 0.12.0 — implemented private gas abstraction

The actual local SDK, prover, Android bridge and recovery source, OHTTP relay/gateway, provider verifier, restricted signer and durable operator workflow are implemented. [PRIVATE_GAS_UPGRADE.md](PRIVATE_GAS_UPGRADE.md) describes the architecture and limits. The machine-readable results and associated logs are in `android/verification/v0120-verification.json`.

### Completed checks

| Check | Result |
| --- | --- |
| Rust workspace tests | **251 passed**, including59 service cases |
| JavaScript engine tests | **40 passed** |
| Rust formatting, all-target check and strict Clippy | Passed |
| Genuine Groth16 proofs | One-input and eleven-input proofs generated and verified |
| Packaged engine and MessagePort flow | Passed, including lost acknowledgement and incorrect receipt rejection |
| Public-wallet differential checks | 36 ethers vectors;5 Solana vectors |
| Native bridge | 18 EVM and7 Solana/TRON cases;17 private rejection checks |
| Proving parameters | All11 supported sets authenticated and their verification keys derived |
| Dependency audit | Zero vulnerability/unsoundness entries;2 maintenance notices retained |
| Android private test source | 63 declarations; **not executed** |
| New signed APK / funded mainnet transfer | **Neither produced nor performed** |

The positive protocol tests generate and verify genuine Groth16 proofs with authenticated upstream parameters. They use public fixture keys, synthetic unfunded Merkle trees and controlled chain observations. Tests cover the minimum and maximum supported input counts, actual native approval and encrypted-journal reopening, exact provider-fee decryption, hidden-sender output, invalid public inputs and wrong fee keys. All eleven configured parameter sets are hash-qualified; their verification keys are derived from authenticated proving keys. Current deployed-verifier correspondence is still unverified.

The built JavaScript bundle is also executed through the actual transferred MessagePort protocol in an isolated Node VM. It scans an encrypted note, proves and verifies a payment, persists encrypted reservations, and resolves a lost acknowledgement by scanning the pool and checking exact calldata and output logs. Missing receipts preserve uncertainty; wrong calldata and missing outputs are rejected. This is a bundled-runtime integration test, **not** a Chromium CSP or Android WebView test.

The Rust workspace, strict Clippy, native FFI bridge and existing public-wallet differential checks are executed separately. The operator service has real HPKE/OHTTP relay-to-gateway integration and a real Node subprocess proof/fee-verification test. Its production admission and treasury-release gates remain closed. No funded transaction was broadcast and no real customer seed was used.

The narrowed SDK graph has **zero known advisory entries in the recorded npm audit**. The final Rust audit also has zero vulnerability/unsoundness entries after pinning rustls0.23.46, rustls-webpki0.103.16 and Tokio1.53.3. Maintenance notices for basic-toml and bincode are retained; the former is part of host binding generation and the latter remains in the existing Solana dependency graph. The prior 89-entry qualification audit is retained as historical evidence. Vendored circomlib cryptographic source is hash-checked against its upstream package; only unused dependency metadata was changed. An advisory scan and internal source review are not an independent security or cryptographic audit.

### Unexecuted and blocked release checks

- Android compilation, JVM/Gradle suites, lint, ARM64/x86-64 libraries, same-certificate signing, install/upgrade and physical-device operation. The automatic approval rejection of Android build-tool downloads remains in force; no alternative download route was used.
- Actual browser/WebView CSP, renderer isolation, memory pressure and lifecycle behavior. Chromium launch was blocked by the environment and the requested escalation was rejected.
- Current deployed bytecode/proxy/admin/verifier qualification. The attempted read-only Arbitrum RPC POSTs returned HTTP 403; an RPC failure was not treated as zero state or successful validation.
- Operated HTTPS infrastructure, a genuinely independent relay, production secret provisioning, funded treasury, live fee schedule and end-to-end mainnet receipt/finality checks.
- Independent security review and complete binary-distribution licensing/corresponding-source review, including GPL-3.0 components.

The 63 private-feature Kotlin test declarations are source coverage, **not passing Android tests**. Exact current host counts are in the JSON evidence rather than inferred from declared test functions. `docs/APK_MANIFEST.json` continues to describe the verified 0.8.0 binary. The deployed web remains 0.6.0.

## Historical Android 0.11.0 foundation

The earlier 0.11.0 source supplied the native journal, OHTTP client and coordinator/service boundaries, but had no integrated local prover or service adapters and had unresolved SDK advisories. Its original logs remain under `android/verification/v0110-*`. The 0.12.0 implementation and evidence above supersede those limitations; it does not retroactively turn the earlier source into a verified binary.

## Historical Android 0.10.0 source candidate

**2026-11-06 UTC:** the Android/Rust source candidate contains nine mainnets. **No 0.10.0 APK has been built or signed.** The latest delivered APK remains 0.8.0, and the deployed web runtime remains 0.6.0. This report distinguishes source and host checks from actual Android compilation, device operation and funded-network validation. It is not an independent security audit.

## Android 0.10.0 — remaining requested networks

### Implemented changes

- Added Solana, TRON, Avalanche C-Chain, Polygon PoS and BNB Smart Chain to the Ethereum/Base/OP/Arbitrum source. The fixed registry contains 24 asset/network positions grouped into nine asset choices. Canonical USDT is supported on Ethereum, Avalanche, Solana and TRON. BNB Smart Chain supports native BNB; Binance-Peg stablecoins are not represented as issuer-native tokens. The exact matrix and token identities are in [NETWORK_EXPANSION.md](NETWORK_EXPANSION.md).
- Added separate restricted Solana Ed25519 and TRON secp256k1/protobuf engines. Existing vaults, recovery words and EVM addresses remain intact; family public addresses derive from the authenticated seed at explicit paths. Negative local registry IDs never become EVM chain IDs.
- Integrated asset-first Deposit/Send, family-specific addresses and QR, exact units and Max, native fee currencies, Solana token-account rent, TRON bandwidth/energy/activation, USD-price requests, incoming discovery and receipt verification. A per-user TronGrid credential can be encrypted locally under a separate Keystore key. No provider signing backend or shared API secret is introduced.
- Added separate address/network-bound Solana/TRON journals. Signed bytes commit before submission; a crash/timeout cannot imply non-submission. Identical-byte retries retain the original transaction identity. Signed expiry and a null provider status remain unresolved and retain the reservation. Contradictory finality/activity evidence, concurrent duplicate intents, older balance observations and cost discrepancies are explicitly handled.
- New EVM journals use schema 4. Existing schema-2/3 journals remain readable with their original schema and immutable bytes. The Android candidate is version code 13 / 0.10.0-mainnet. This update does not publish or rebuild the web app.

### Completed host checks

| Check | Executed result | Evidence under `android/verification/` |
| --- | --- | --- |
| Rust workspace tests | **124 passed**, no failures or ignored cases | `v0100-rust-tests.log` |
| Rust formatting, type check and all-target Clippy | **Passed**, Clippy warnings denied | `v0100-format.log`, `v0100-check.log`, `v0100-clippy.log` |
| Independent ethers comparison | **36 deterministic transactions passed** across seven EVM networks | `v0100-ethers.log` |
| Independent Solana JS comparison | **5 public offline vectors passed** | `v0100-solana-differential.log` |
| Host release library and actual Kotlin/Python bindings | **Built/generated** from the changed Rust source | `v0100-host-build.log`, `v0100-bindings.log` |
| Actual EVM UniFFI bridge | **18 asset/network cases passed** across seven EVM networks | `v0100-bridge.log` |
| Actual Solana/TRON UniFFI bridge | **7 cases passed**; 7 journals, 14 signed/unknown-state reopens, 25 rejection checks | `v0100-family-bridge.log` |

The 124 Rust cases comprise 7 core unit, 8 activity, 19 crypto, 18 family-journal, 21 existing-journal, 21 EVM mainnet, 11 Solana and 19 TRON cases. They include exact atomic amounts, approved wire/signature reconstruction, unsupported token/chain rejection, resource costs, successful/failed receipts, canonical inclusion and finality, caught failed Solana inner calls, strict duplicate JSON handling, unknown outcomes, same-byte recovery, contradictory observations, stale snapshots and journal identity/schema constraints.

The ethers comparison independently derives the public account and verifies amount units, fee arithmetic, unsigned encoding, signature, recovered sender and transaction hash. The Solana comparison uses `@solana/web3.js 1.98.4`, `@solana/spl-token 0.4.15` and `ed25519-hd-key 2.0.0` to reconstruct key derivation, ATAs, account permissions, instruction bytes, messages, wire bytes and Ed25519 signatures. Its five cases include SOL, USDC with ATA creation, USDC with two sources, EURC and USDT. TRON has Rust policy and actual-FFI verification; this report does not claim an independent third-party TRON transaction-codec comparison.

Every signature in these checks uses a deliberately public development phrase and offline fixtures. The Solana/TRON fixtures use synthetic block references; the EVM fixtures use real mainnet chain IDs with synthetic balances and nonce assumptions. No user's recovery phrase was accessed, no user payment was signed and no transaction was broadcast. Kotlin generation is not Kotlin compilation. Optional ktlint formatting was unavailable; the generator warning is preserved in its log. Android unit-test sources were expanded, but none of the Android tests were executed for this candidate.

### Remaining release gate

The existing automatic approval review rejected Android SDK/NDK metadata/packages, the Gradle distribution and a JDK archive because the official destinations were outside the allowed network hosts. The missing tools cannot be replaced by a successful host Rust build. No rejected download was retried by another route. The exact prior rejection evidence is retained in `v090-build-environment.json` and referenced by `v0100-build-environment.json`.

The following remain **unrun** for 0.10.0:

- ARM64 and x86-64 Android native compilation and matching generated-binding integration.
- Android Kotlin/Java compilation, Gradle release unit tests, lint and APK assembly.
- APK alignment, signing with the existing distribution certificate, signature/manifest inspection and upgrade installation.
- Bundling the applicable third-party runtime license/copyright notices for binary distribution; the current dependency inventory is provenance metadata, not that notice bundle.
- Physical-device/emulator UI, Keystore, background/restart and application-opening checks.
- Live RPC/provider access, Coinbase pricing, mainnet balance/receipt checks and funded-send validation on each new network.

`docs/APK_MANIFEST.json` still describes the verified 0.8.0 APK. No old native library has been labelled a new release, and the private app-signing backup remains unopened. The exact source/build evidence is consolidated in `android/verification/v0100-verification.json`. The remaining device checklist is [DEVICE_TESTS.md](../android/DEVICE_TESTS.md).

### Product and evidence limits

- Public endpoints may reject or throttle requests. TRON production access generally needs a user-owned TronGrid API key; unauthenticated access is not assumed to work. Availability must be checked on the actual Android network.
- Solana/TRON preparation currently enforces affordability before returning the full fee quote. An unfunded send can show an unavailable estimate and a funding error; a native-gas Deposit shortcut is available. A separate unfunded fee preview remains a UX improvement.
- Incoming discovery is bounded. Balances are checked separately. Missing history or a null receipt never establishes that money was not received or sent.
- Provider evidence is checked for identity, consistency and expected transfer effects. It is not independent cryptographic consensus verification, a provider quorum or an external security audit.

## Historical Android 0.9.0 — OP Mainnet and Arbitrum One source candidate


### Implemented changes

- Added exactly OP Mainnet (10) and Arbitrum One (42161), with ETH and Circle's canonical native USDC. Ethereum/Base and their EURC support remain. No unsupported EURC, arbitrary ticker, bridged USDC.e or additional chain is admitted.
- Kept the asset-first Deposit/Send flow. Network labels, QR, addresses, balance snapshots, USD holdings, drafts, Max, fee review, activity and explorer context use the selected chain. Canonical registry matching and explicit native/token branches prevent a missing token contract from becoming an ETH payment.
- Added an explicit fee profile: Base/OP require separate L1 and operator accounting; Arbitrum's estimate and actual gas charge already include posting costs. Arbitrum cannot use the old unfunded direct-ETH 21,000-gas fallback. The existing gas/fee policy bounds remain, including rejection above 300,000 gas.
- Preserved the vault format, derivation, optional app protection, exact integer payment arithmetic, immutable approval, durable signed bytes, nonce handling and same-byte retry. Existing per-network schema-2 Ethereum/Base journals keep their schema and records. New journals use schema 3; a later downgrade to the older app would not read those newly created journals.
- Android candidate metadata is code 12 / 0.9.0-mainnet. No signed artifact is claimed from that metadata. Web was not rebuilt or deployed.

### Completed host checks

- **71/71 Rust tests passed** across all targets, with no failures or ignored cases. Tests include unsupported-chain/token cases, exact units, wrong-chain signed data, OP fees, Arbitrum single fee accounting, uncertain outcome and nonce retention, receipt/reorg/finality evidence, and byte-preserving schema-2 prepared/signed/event/balance compatibility. Evidence: `android/verification/v090-rust-tests.log`.
- **Rust formatting and Clippy passed**, with warnings denied for Clippy. Evidence: `v090-rust-format.log` and `v090-rust-clippy.log` in the same directory.
- **20 deterministic transactions independently matched ethers** across all four networks: account derivation, amount units, fee arithmetic, unsigned encoding, deterministic signatures, recovered sender and transaction hash. These are public unfunded offline fixtures, not live transfer simulations. Evidence: `android/verification/v090-differential.log`.
- The changed Rust core built as a **host release library**, and Kotlin/Python UniFFI bindings were regenerated from it. **10/10 real host-FFI payment cases passed**, covering every supported asset/network pair, vault authentication with a public fixture, fee validation, durable per-network journal reopening, exact signed-byte preservation and idempotent receipts. Evidence: `android/verification/v090-host-build-bindings.log`, `v090-host-ffi.log` and the consolidated `v090-verification.json`. Kotlin generation succeeded; optional ktlint formatting was unavailable. Host FFI success does not establish Kotlin compilation, Android ABI or Android runtime behavior.
- Root integration review and separate Rust/Android source cross-reviews found no blocking chain, asset, fee, storage or approval mismatch. These are internal development reviews. `git diff --check` passed.
- Android test source contains **169 cases across 19 suites**, including 15 new cases. **Those Kotlin/JVM tests have not been executed for 0.9.0.** Source counts are not passing-test counts.

### Blocked Android release gate

The workspace was reset and no Android SDK/NDK or Gradle cache remained. Automatic approval review rejected official Android metadata, Gradle distribution and JDK archive downloads because their destinations were outside the environment's permitted network hosts. No rejected request was routed through a mirror, changed proxy, indirect downloader or disabled security check. The local Java compiler module and pinned Rust toolchain were available/restored, but they cannot replace the Android SDK, NDK and Gradle.

The precise missing prerequisites and rejection reasons are in `android/verification/v090-build-environment.json`. Consequently, these steps remain **unrun**:

- ARM64 and x86-64 Android native compilation from the new Rust source.
- Android Kotlin/Java compilation, release unit tests and lint.
- APK assembly, alignment, signing and signature/manifest inspection.
- Installation, cold start, upgrade and UI/Keystore/lifecycle validation on an Android device or emulator.

No old native library was reused to manufacture a new APK. `docs/APK_MANIFEST.json` still identifies the last verified **0.8.0** artifact. No 0.9.0 artifact replaces it.

### Network and product limits

Official Circle, Optimism and Arbitrum documentation was reviewed for identities, endpoints and fee semantics. **No new live RPC, Coinbase, funded-send or user's-deposit check is claimed for this candidate.** Historical live checks below remain historical. The remaining read-only/device scenarios are in `android/DEVICE_TESTS.md`.

Automatic token activity discovery remains bounded to 8,000 blocks, which covers less elapsed time on a faster chain. Balances are checked separately; direct ETH is balance-visible and manually trackable by hash. Provider-reported finalized blocks are not independent consensus proofs, and missing finality is not replaced with `latest`. Public provider availability and the complete Android app flow must be checked before this candidate is distributed.

## Android 0.8.0 — withdrawal form and transaction coordination

### Implemented changes

- Send follows the supplied recording's useful form order: Asset, Address with explicit Paste, Network and Amount with exact Max. The warm off-white native form uses a network bottom sheet, retained input views, inline fee feedback and a fixed recipient/fee/Review footer. The prior separate fee-preview page is removed. Deposit still lists assets once and requires a network before exposing its address/QR.
- An address entered before the first network choice survives that choice. Subsequent changes restore only the wallet/chain/asset draft. Invalid clipboard input does not erase the recipient. Long exact quantities wrap rather than being rounded or ellipsized. The Activity explicitly owns system-bar, cutout and keyboard insets; its sheets retain secure-window and obscured-touch protection.
- A dedicated read-only quote worker keeps input editing responsive. One request can run and one latest request can wait. Immutable request identity, revisions, page/wallet/foreground checks, single-use completions and a 30-second monotonic deadline reject stale or replayed estimates. Only preview and Max use that worker; durable repository operations remain serialized. A quote neither reserves a nonce nor authorizes a signature.
- FeePreview now carries the asset balance from the same pinned observation as its fee calculation. The form prefers that verified current quantity and labels cached fallback as Last checked. The quote validates amount, asset/network/recipient, balance sufficiency, fee arithmetic and Max evidence. ETH fees remain separate from the USDC/EURC recipient amount, and fresh Coinbase USD estimates remain display-only.
- Review claims the quote once and performs fresh durable preparation. The final screen shows exact amount, complete recipient, explicit network and an honestly labelled fee reserve. Sender, token contract, execution limit, gas, nonce and Base data/operator estimates are available in its details sheet. Confirmation independently revalidates the immutable prepared transaction before existing local signing, durable signed-record commit and broadcast.
- Edit, toolbar Back and hardware Back cancel the exact unsigned intent before reopening the draft. Cancellation is idempotent; an uncertain local cancellation keeps Confirm blocked until resolved. A key-open failure refreshes the review controls after the worker releases its busy state. Background preparation remains recoverable through Activity, without restoring approval authority.
- Result screens distinguish submitted, saved-for-submission and unconfirmed outcomes. An interrupted signing/submission path directs the user to the saved Activity record rather than automatically creating another payment. Existing reconciliation and retries preserve the exact original signed bytes, nonce and transaction hash.
- This Android-only change retains the vault format, optional app lock, key derivation, Rust signing policy, supported mainnet assets and package identity. No web runtime is deployed.

### Completed checks

- **154 JVM tests passed in 18 suites**, with zero failures, errors or skips. Fourteen new cases comprise twelve withdrawal-flow cases and two real repository/Rust-journal cancellation cases. They exercise stale and A → B → A responses, replay, expiry and clock rollback, malformed/wrong-context fee data, exact Max evidence, fresh asset balance changes, insufficient funds, single preparation claims, wrong-network cancellation, repeated cancellation and re-preparation after cancellation. Existing exact-amount, price, vault, foreground, network and journal suites also pass.
- **Release build and lint passed.** Lint reports zero errors and nine warnings: one pinned dependency update notice, two constructor notices for views instantiated programmatically, and six text/localization notices. Compiler deprecation notices and the existing MaxTransfer data-class future-language warning remain recorded. Evidence: `android/verification/v080-final-release-build.log` and `v080-verification.json`. A wrapper distribution-download failure happened before compilation; the same installed Gradle 8.11.1 ran the successful offline gate. Its environment failure is preserved separately in `v080-wrapper-environment.log`.
- **Signed APK verified:** package `app.localwallet.mainnet`, code 11 / 0.8.0-mainnet, non-debuggable, valid v3 signature with the existing certificate, and 16 KiB ZIP/ELF alignment. All four native libraries are byte-identical to 0.7.1. The APK is 10,205,240 bytes with SHA-256 `5096819858a6065a61917a167e7e1118c96bcc609df9c2211d06081c13b3c3f7`. `docs/APK_MANIFEST.json` records the same artifact.
- Root integration review and separate source reviews checked the form, quote isolation, fresh balances, immutable review, exact cancellation, authorization/error callbacks and result semantics. The reviews found and resolved first-network draft loss, invalid-Paste clearing, queued-quote loading state, inset ownership and key-open recovery issues. They are internal development reviews, not an independent security audit.

### Verification boundaries

The tests execute production Kotlin policy/repository code and the existing Rust bridge on the host. They do not render Android views. No physical phone or emulator was available to validate the new form, bottom sheets, fixed footer, keyboard positioning, enlarged fonts, API 30/34 versus API 35+ insets or OEM authentication timing. The outstanding scenarios are in `android/DEVICE_TESTS.md`. No funded transfer was signed or broadcast, and no new live RPC or Coinbase result is claimed for 0.8.0; historical live checks below remain historical. This release does not establish production security or an independent audit.

## Android 0.7.1 — asset first, then network dropdown

### Implemented changes

- Deposit and Send list USDC, EURC and ETH once each. The next screen exposes a Network dropdown derived from that canonical asset's supported positions. Home actions begin without a network selected and do not expose QR/address actions or transfer fields until the user chooses one. Contextual holding shortcuts retain their explicit network, which remains editable.
- The selector validates chain, network name, native/token identity, decimals and the fixed Circle token contracts. It rejects duplicate or unsupported entries and does not merge arbitrary ticker matches. A native signing-core registry cross-check prevents the Kotlin identity list from drifting unnoticed.
- Switching networks saves the old wallet/chain/asset draft before loading the new one, clears Max/fees/review state and rebuilds the page. Page, wallet, epoch and foreground checks reject late callbacks and stale dropdown actions. QR, warning, copy/share, balance, fee estimate and final review remain network-bound.
- Public route version 2 preserves an unselected network across app switching or Activity recreation. Version 1 explicit-network routes remain readable. Prepared transactions retain their immutable network and durable intent; leaving a prepared review exposes it in Activity for explicit cancellation.
- The supported assets/networks, vault formats, local data, core cryptography, USD pricing, Max arithmetic and transaction repository are retained. This Android-only update leaves web at 0.6.0 and does not deploy it.

### Completed checks

- **140 JVM tests passed**, with zero failures, errors or skips. Eight new asset-selection cases cover canonical grouping, per-asset supported networks, native registry agreement, unknown or unchosen networks, wrong contracts/decimals, duplicates, stable ordering and independence from display filtering. Three additional route tests cover explicit unselected-state restoration, invalid omissions and legacy route compatibility. Existing chain-scoped draft, foreground callback, exact Max, vault, pricing and repository suites also passed.
- **Release build and lint passed.** Lint reports zero errors and five warnings: one pinned dependency update notice and four translation-resource warnings. Evidence: `android/verification/v071-final-release-build.log` and `v071-verification.json`.
- **Signed APK verified:** package `app.localwallet.mainnet`, code 10 / 0.7.1-mainnet, non-debuggable, with the existing signing certificate and 16 KiB ZIP/native ELF alignment. All bundled native libraries are byte-identical to 0.7.0. The APK is 10,139,704 bytes with SHA-256 `3f713f0799e65178e3ca788a96ddd8d7558c7f7602d678ac4b7c3586c31af3f9`. `docs/APK_MANIFEST.json` records the same artifact.
- Root integration review and a separate source review checked network selection, scoped drafts, stale callbacks, QR/copy/share consistency and immutable prepared transactions. No blocking defect was found.

### Verification boundaries

The tests execute production Kotlin policy and the existing Rust bridge on the host; they do not exercise Android views. No physical phone or emulator was available to verify the dropdown's rendering, touch behavior, keyboard/insets or OEM lifecycle timing. Those checks are listed in `android/DEVICE_TESTS.md`. No mainnet transaction was signed or broadcast by these checks. The earlier Coinbase and RPC checks below are historical evidence, not new live-network results for 0.7.1. This remains an engineering release without an independent security audit.

## Android 0.7.0 — USD valuation, exact Max and immediate creation

### Implemented changes

- Coinbase public spot prices value the entire unfiltered portfolio and each asset/network row in USD. Precise decimal arithmetic is separate from transaction units. Complete, partial known-value and unavailable totals are distinct; zero or a stablecoin peg is never fabricated for missing evidence.
- Hide small balances uses an address-scoped USD threshold, initially $1 and off by default. The user can enter $0.01–$1,000,000 with up to two decimal places. Comparison is strictly below the unrounded value. Unknown/stale prices and stale positive balances remain visible. Totals and Deposit/Send selectors remain unfiltered.
- Max uses the full USDC/EURC token balance and exact ETH units minus the reviewed reserve. Recipient-dependent ETH gas is simulated to a stable amount, with bounded failure. Fresh preparation recomputes Max before the final immutable review. Quotes never create an intent, access keys or submit a payment. ETH residue caused by actual fees being below the reserve is not decimal truncation.
- Creation persists and reads back the encrypted manifest before opening the wallet and showing the recovery/custody message. Startup and sending no longer require a word quiz or confirmed-backup flag. Existing metadata, vaults and selected app-lock policy remain intact; unconfirmed backups are not silently marked confirmed.
- Recovery returns to the same public route after an app switch or recreation, with words concealed and the selected unlock policy respected. No phrase, PIN or review authority enters saved route state. Prior prepared reviews return to Activity, where the existing unsigned intent can be cancelled explicitly.
- Asynchronous Max/fee/revalidation callbacks bind to wallet, page, epoch and foreground generation. A pause invalidates them even without onStop. Key workers capture their authorization generation after the device-auth callback and check it before/after unwrap and through signing/submission; a later pause/resume cannot revive the operation.
- Public pricing uses fixed HTTPS endpoints, bounded strict payloads, normal certificate/hostname verification and exactly one timeout-only retry. HTTP, TLS and parser failures are not retried. The price worker/cache does not block transaction work or upload addresses/holdings.

### Completed checks

- **129 JVM tests passed**, with zero failures, errors or skips. Coverage includes exact USD totals/filter boundaries, missing/stale/future quotes, malformed transport responses, bounded retry, preference migration, every native decimal of Max, changed recipient/asset/nonce prerequisites, inconsistent pinned balances, durable preparation, new-wallet readback faults, legacy lock preservation and public-route invalidation.
- **Release build and lint passed.** Lint has zero errors and 5 warnings: one pinned dependency update notice and four translation-resource warnings. Final evidence: `android/verification/v070-final-release-build.log` and `v070-verification.json`.
- **Signed APK verification passed:** package `app.localwallet.mainnet`, code 9 / 0.7.0-mainnet, non-debuggable, existing signing certificate retained, and 16 KiB ZIP/native ELF alignment verified. All bundled native libraries are byte-identical to 0.6.1. The APK is 10,123,320 bytes with SHA-256 `2ab98c9099d7545637d23a231f51432b5407aa7269be684f84062551edcc0551`. `docs/APK_MANIFEST.json` records the same artifact.
- **Live production price client passed** at 2026-11-05T16:39:26.146548909Z: the compiled production client/transport/parser returned HTTP 200 and valid USD quotes for USDC, EURC and ETH in one refresh. Earlier direct-DNS/default-JDK trust failures and partial cold-connection failures are retained in `android/verification/v070-coinbase-live.json`. The final host run used the executor-provided proxy and trusted CA bundle with ordinary certificate and hostname checks. Those environment settings are not shipped in Android. The observed cold timeout prompted the single public-read retry and its regression tests.
- Independent source reviews checked pricing/signing separation, creation readback and lock preservation, exact Max, and foreground/approval boundaries. No real transfer was signed or broadcast, and no user recovery phrase was handled.

### Verification boundaries

No physical Android phone or emulator is available here. The tests execute production Kotlin policy/network/storage code and the Rust bridge with controlled fixtures; they do not validate Android views, actual Keystore/BiometricPrompt callbacks, OEM app-switch timing, installation or screenshot controls. The live price check is a host-JVM network check, not an Android-device check. Device scenarios are recorded in `android/DEVICE_TESTS.md`. This remains an engineering release without an independent security audit. Web source/runtime remains at 0.6.0 and was not deployed by this Android-only update.

## Android 0.6.1 — holdings, profile and concealed recovery

### Implemented changes

- The multiline recovery import field explicitly sets top/start gravity after input-type setup.
- Home omits observed zero holdings. Positive holdings remain visible by default, including exactly 0.01 USDC. Unknown amounts are not treated as zero; all-zero, stale-zero, unavailable and filtered-small empty states have distinct wording. Existing rows and dividers update in place.
- An address-scoped **Hide small balances** preference defaults to off. Enabling it hides positions below 1 USDC, 1 EURC or 0.0001 ETH using integer units; amounts at the thresholds remain visible. Total USDC, history, drafts and the complete supported-asset selectors are unaffected.
- Profile contains one wallet-name/public-address identity area and Settings. Wallet rename, display, app-lock, recovery and export controls live inside Settings. The wallet-only metadata path reads/writes the prior wallet-name key without depending on or rewriting legacy account-name data.
- Recovery screens start concealed in Settings, new-wallet backup and unfinished-backup resume. Only an explicit reveal request obtains the words. Completion is checked against the issued request, wallet, page, epoch and foreground state; a rejected matching request is consumed without cancelling a newer one.
- Retained words and backup-challenge answers use erasable character buffers. Moving to verification retains only the three selected words and clears the full retained phrase. Hide, navigation and ordinary backgrounding clear sensitive views and buffers and restore secure-window protection. If device authentication pauses the Activity, returning keeps the phrase concealed and requires a fresh foreground Reveal confirmation before vault-key unwrapping or phrase decryption; the deferred authorization is bound to the original request and expires. Copy/screenshots remain available after reveal.
- The package remains `app.localwallet.mainnet`, version code 8 / 0.6.1-mainnet. Rust signing policy, vault formats, derivation, fixed supported assets, optional app-lock behavior and the 0.5.1 journal-path correction are retained. This update does not change or publish the web client.

### Completed checks

- **79/79 Android release JVM tests passed**, with no failures, errors or skipped tests. This includes the previous 54 tests plus six holdings tests, three filter-preference tests, three wallet-only naming tests and thirteen recovery-session/continuation tests. The continuation tests use the actual production gate with an operation counter to establish that no operation runs automatically after paused authentication, confirmation is one-use, and cancellation, expiry, foreground/context mismatch and subsequent pause prevent execution.
- **Release compilation and APK assembly passed.** Android lint reported **zero errors and eight warnings** (one dependency-version advisory and seven localization warnings). The only remaining Kotlin compiler warning is the existing API-30-compatible `onBackPressed` callback. Evidence is retained under `android/verification/v061-*`.
- **Signed APK verification passed:** package `app.localwallet.mainnet`, code 8 / 0.6.1-mainnet, non-debuggable, the same signing certificate as previous releases, and 16 KiB ZIP/native ELF alignment. All native libraries are byte-identical to 0.6.0. The APK is 10,074,168 bytes with SHA-256 `bb236014e9447046ea041db410b80e10de7b43c90e22b7c3c63fd2e9e8b04ce9`. Evidence: `android/verification/v061-verification.json` and `docs/APK_MANIFEST.json`.
- **Focused source review** checked home-only filtering, unchanged totals/asset selectors, preservation of legacy wallet-name storage, explicit recovery entry and pre-decryption authorization in the actual MainActivity. Native rendering and platform callbacks remain outside these source/JVM checks.

### Verification boundaries

The JVM tests exercise production Kotlin helpers and existing vault/repository/transport integrations with controlled test fixtures. They do not execute Android views or hardware Keystore/BiometricPrompt. No emulator, AVD or connected phone is available in this environment. The import-field geometry, keyboard/insets, screen transitions and physical screenshot/task-switcher behavior therefore remain device checks in `android/DEVICE_TESTS.md`. No user recovery phrase was handled and no transaction was signed or broadcast on mainnet during this update.

Platform references used for the recovery policy: Android's [`Activity` lifecycle](https://developer.android.com/reference/android/app/Activity#onPause()) does not make every pause a reliable indication of why the app lost interaction. Authorization expiry uses [`SystemClock.elapsedRealtime()`](https://developer.android.com/reference/android/os/SystemClock#elapsedRealtime()) so device sleep counts toward the deadline; the Handler timer is cleanup rather than the authority for accepting a request.

## Version 0.6.0 — unified portfolio and stable presentation

### Implemented changes

- Base and Ethereum appear together in one six-position portfolio. Asset selectors, activity records and transfer details retain chain identity; there is no global network switch.
- DEPOSIT and SEND are the only primary wallet actions. The large Total USDC is an exact USDC quantity, requires both network observations and identifies cached/stale data. It is not an ETH/EURC/USDC fiat valuation or a single-network spendable balance.
- Wallet/account names are device-local, address-scoped display metadata. Saving labels does not change vaults, derivation, signing fields or journal identity. Name validation rejects control/hidden-format characters and limits names to 32 Unicode code points.
- Background refreshes patch retained UI elements instead of reconstructing the portfolio, forms or deposit QR. A short opening transition is limited to app opening and respects motion preferences.
- Rust signing policy, recovery derivation, asset registry, encrypted vault formats, optional app lock and the Android 0.5.1 journal-path fix are retained.

### Validation scope

- **Web: 97 tests passed.** Includes 17 new helper/controller tests for exact multi-network totals, missing/duplicate coverage, display-name validation/persistence, late network replies, draft/focus preservation, lock/reopen cancellation, explicit deposit QR network binding, chain-aware activity/pending behavior, frozen transfer signing context and literal name rendering. The controller harness executes actual `main.ts`/`ui.ts` with in-memory DOM and mocked external boundaries. Evidence: `docs/evidence/web-v060-tests.txt`.
- **TypeScript and web/Worker builds passed.** Hosted verification checked strict headers, four asset integrity hashes, excluded preview fixtures and the unchanged Rust WASM hash. The native `workerd` artifact gate passed chain reads and redirect rejection with controlled outbound fixtures. Relay tests passed **18/18**.
- **Browser layout/interaction review passed** on desktop, 390px and 320px preview frames, including long names, large exact amounts, partial-network unavailability, renaming, network-labelled Deposit, visible transfer costs and retaining edited amount/focus through a two-second refresh cycle. Evidence: `docs/evidence/ui-v060-review.json`.
- **Android: 54 JVM tests passed**, including 12 new portfolio, explicit-chain/session/draft and display-name regressions. Release assembly passed. Lint reports zero errors and seven warnings. Package `app.localwallet.mainnet` is version code 7 / 0.6.0-mainnet, not debuggable, uses the same signing certificate as 0.5.1 and passes 16 KiB ZIP/native ELF alignment. All bundled native libraries are byte-identical to 0.5.1. Evidence: `android/verification/v060-verification.json`.
- **Focused source review** found no release-blocking cross-network or stale-session issue in either controller. Android balance reads remain sequential, so a slow provider for one chain can delay the next chain's read without mixing their data.

Browser layout review uses an isolated synthetic fixture with no real vault, recovery phrase, signing or broadcast access. Controller regression tests inject synthetic providers and wallet state; they do not prove live-provider availability. Physical Android installation, keyboard/inset behavior, biometric prompts and the user's reported glitch on their phone require device validation. No real transfer is performed by this release's checks.

## Android 0.5.1 — local journal path hotfix

The reported Android screens show `WalletException.Storage` alongside a generic Base connection warning. **A confirmed local storage defect was reproduced; the screenshots do not establish a missing hosting service.** The native app connects to its approved HTTPS RPC providers directly. It needs network access, but does not require a separately hosted wallet site, custom server or domain to read balances.

### Reproduced failure and coverage gap

- Opening a journal through a symbolic-link parent fails with the unchanged 0.5.0 Rust core. The underlying SQLite result is `SQLITE_CANTOPEN_SYMLINK` (extended code `1550`). The failure was independently reproduced through the actual compiled Kotlin `WalletRepository` and native library. Reproduction evidence: `docs/evidence/android-v051-parent-path-reproduction.txt`.
- Resolving the trusted parent to its canonical path, while retaining the same journal filename, permits read/write/reopen. A symbolic link at the database file itself remains rejected. These checks use disposable test directories and fixture values, without inspecting the user's wallet or device.
- SQLite documents that [`SQLITE_OPEN_NOFOLLOW` disallows symbolic links in the database filename](https://www.sqlite.org/c3ref/open.html). Android's [AOSP Zygote implementation](https://android.googlesource.com/platform/frameworks/base/+/3468cd94064c/core/jni/com_android_internal_os_Zygote.cpp) creates `/data/user/0` as a symbolic link to `/data/data` in the app's mount namespace. This provides a concrete Android mechanism for the reproduced failure. **The user's actual filesystem path has not been observed, so this does not prove the exact cause on that phone.**
- Earlier host/JVM storage tests used ordinary temporary directories without an aliased parent. They covered vault restart and protection behavior but missed this path condition. Their passing results and successful APK compilation did not establish that the journal would open in the Android app's actual filesystem namespace.

### Hotfix scope and remaining verification

Android 0.5.1, version code `6`, normalizes the trusted Android-provided parent before opening wallet storage, explicitly rejects symbolic links at the wallet directory and database file, and retains the core's `NOFOLLOW` protection. It separates storage failures from network failures using stage-specific error codes. The Rust core and web version 0.5.0 remain unchanged. Existing wallet data is retained; the correction does not require clearing app data, regenerating a wallet or provisioning a server.

### Final build verification

- All **42 Android JVM tests passed**, including eight new storage-path, retained-record, identity, corruption and error-classification regressions.
- Release assembly passed. Lint reported **zero errors**, with seven nonblocking localization/dependency warnings.
- The signed `local-wallet-0.5.1-mainnet.apk` uses package `app.localwallet.mainnet`, version code `6`, the same signing certificate as 0.5.0, and is not debuggable. Signature verification and 16 KiB ZIP/native ELF alignment checks passed. Native Rust and JNA library bytes are unchanged from 0.5.0.
- APK SHA-256: `6434cc9dfedb1a101ce1e489a75a9582f418d9cb5ee4f54d1c63bf8c2dfca95b`.
- Public evidence is retained in `android/verification/v051-verification.json`, `android/verification/v051-path-reproduction.json` and the matching build/APK verification files.

Installation and journal/balance behavior on the user's phone remain unverified. Physical Android Keystore, biometric and screenshot checks also remain outstanding. No real transfer or user recovery phrase/private-key handling is part of this diagnosis. The earlier release sections below are historical evidence, not new 0.5.1 verification results.

## Version 0.5.0 — light interface, resumable backup and optional app lock

This release replaces the dark interface with white and off-white surfaces, moves lock controls into **Profile → Settings**, and removes the requirement to choose an app passcode when creating or importing a new wallet. A new app PIN is explicitly **six digits**. Existing password-protected or device-authentication-protected wallets retain their protection until the user authorizes a settings change. Biometrics are an Android option; the web interface identifies them as unavailable in this web version.

### Recovery setup and local protection

- New wallet creation persists the encrypted wallet and its protection metadata before displaying recovery words. Reopening an unfinished backup offers **Continue backup** and recovers the same saved account and phrase; it does not create or overwrite a wallet. Backup confirmation remains a separate durable state transition.
- App lock is optional, while local vault encryption remains enabled. With app lock off, someone who can use the browser profile or app can open that wallet and approve transfers. Browser WebCrypto non-extractability is not a claim of Android Keystore hardware isolation.
- Recovery screens offer copying and permit screenshots as requested. The UI explains that clipboard history, photos and cloud backup can retain these copies. Words are removed from the displayed app when it is backgrounded; the physical Android screenshot and task-switcher behavior has not been verified on a phone.
- Sensitive web operations bind their authorization to the saved vault and protection policy. The engine awaits policy checks after transfer revalidation, after signing, after durable journal readback and after the final chain check. A lock change in another tab can therefore stop signing or submission without relying solely on a BroadcastChannel notification. Retry retains the original signed bytes and nonce.

The reported device message was generic, and the original exception on the user's device was **not reproduced**. Resumable setup and persistence/protection failure cases have been implemented and tested, but those checks do not establish the exact cause of that device-specific exception or prove its resolution on that phone.

### Checks completed for this release

- **Web tests: 80/80 passed**, including the retained transaction/network/storage tests, new protection and restart coverage, and four asynchronous authorization regressions. Evidence: `docs/evidence/web-v050-tests.txt`.
- **Hosted build and artifact verification passed for 0.5.0.** `npm run build` and `node scripts/verify-hosted.mjs` passed, including all four asset-integrity hashes, the unchanged Rust WASM and exclusion of isolated preview fixtures from the production artifact.
- **Hosted relay tests: 18/18 passed.** Evidence: `docs/evidence/relay-v050-tests.txt`.
- **Android JVM tests: 34/34 passed**: 20 retained tests plus 14 tests using the real Rust vault for restart, protection and fault scenarios. These are host/JVM results, not physical-device Keystore or biometric results. Evidence is retained under `android/verification/`.
- **Android release assembly passed; lint reported zero errors.** This establishes compilation and static checks, not successful installation or native behavior on the user's phone.
- **The signed Android APK was verified:** version code `5`, `debuggable=false`, the same signing certificate as the preceding release and 16 KiB alignment. The output is `local-wallet-0.5.0-mainnet.apk`; native verification is recorded in `android/verification/v05-verification.json` and `docs/evidence/apk-v050-*.txt`.
- **Browser presentation checks** covered desktop and 320px views of Profile, Settings, PIN and recovery layouts using the isolated synthetic fixture. The fixture has no wallet execution code and uses invalid `sampleXX` placeholders instead of recovery words. Review found word wrapping in the three-column mobile recovery grid; the correction uses two columns at widths up to 440px and prevents individual words from splitting. The final 320px visual recheck confirmed the two-column layout and intact words.
- No funded mainnet transfer was performed, and no user recovery phrase or private key was handled. The fixture and automated tests are not evidence of a funded send/receive cycle.

Publication and live-chain read outcomes are not claimed by this section's results. **Physical Android installation, Keystore/biometric flows and screenshot behavior remain unverified.** An independent security audit remains outstanding. The versioned sections below preserve historical results; checks labelled for an earlier release must not be read as new 0.5.0 runs.

## Version 0.4.0 — send and receive interface

This release changes the wallet presentation and navigation. It retains the original Site identity, vault and derivation formats, Rust signing/validation core, network transports and durable transaction model. It implements the compact account/balance/action layout of a storage wallet while retaining the Local Wallet identity; it does not claim equivalent audit coverage or reproduce OKX/Coinbase's proprietary implementation.

- Web transaction, storage, amount, activity and transport tests: **60 passed**, `docs/evidence/web-v040-tests.txt`.
- Hosted relay tests: **18 passed**, `docs/evidence/relay-v040-tests.txt`.
- Web TypeScript and the final hosted build passed. The artifact checks verified all four asset-integrity hashes, the embedded WASM, restricted routes/security headers, the preserved Site identity and exclusion of preview fixtures; `docs/evidence/hosted-artifact-v040.json`.
- Android JVM tests: **20 passed**. Release assembly succeeded and lint reported **zero errors**; `android/verification/v04-verification.json` records the checks and remaining warnings.
- The APK uses package `app.localwallet.mainnet`, version code `4` and version name `0.4.0-mainnet`, with `debuggable=false`. Its signature matches the preceding release's certificate. APK ZIP and native-library ELF alignment passed at 16 KiB; `docs/evidence/apk-v040-summary.txt` and the accompanying evidence files.
- Browser presentation checks covered desktop, 390px and 320px layouts, Send/review/Receive/settings/activity, network and receive-asset changes, hidden balances across refresh, and unavailable/last-saved states. A discovered navigation-scroll issue was fixed: changing screens starts at the top, while background updates preserve scroll and entered form values. These checks use the labelled isolated fixture and do not establish a funded outgoing transaction.
- The unchanged shared WASM has SHA-256 `d8a7ff53c6cf2301ffed8d230ee6de2c7d54fd424b0c869714648a76f9daa6e1`.
- Displayed amounts remain exact token quantities. No simulated portfolio price or combined fiat balance is inserted into the production UI.
- The isolated presentation preview uses explicitly labelled synthetic values, imports no wallet execution code, and is excluded from the deployed bundle. It must not be mistaken for a live balance or funded-send test.

Physical Android Keystore/biometric testing and a funded outgoing mainnet transaction remain outside this release's automated verification. The older sections below retain the evidence and limitations recorded at each preceding release; the final 0.3.2 owner-authenticated read-only production check subsequently succeeded for the supplied incoming 0.01-USDC transaction.

## Web 0.3.2 hosted runtime correction

The user's v0.3.1 screenshot still showed unavailable balances. Native production logs then showed eight POST requests to `/api/rpc` returning HTTP 503, with matching Origin and same-origin fetch-site headers, in 1–4 milliseconds. The request-admission checks passed; the relay failed before obtaining upstream data. The sanitized sample is in `docs/evidence/production-v031-rpc-failure.json`.

The server used `redirect: "error"` for outbound requests. Cloudflare Workers does not implement that option and throws a TypeError when constructing the request. This differs from browser and Node fetch. The exact deployed v0.3.1 Worker bundle was run in official `workerd@1.20261104.1`; it reproduced HTTP 503 in 3 milliseconds with zero requests reaching the controlled upstream. See `docs/evidence/workerd-v031-reproduction.log` and [Cloudflare's Request reference](https://developers.cloudflare.com/workers/runtime-apis/request/).

The server now uses `redirect: "manual"` and rejects every non-2xx response, explicitly classifying all 3xx before body parsing. It does not follow Location or send credentials to a redirect destination. Read retries remain limited to approved providers; transaction submission remains exactly one attempt. Sanitized diagnostics record fixed failure categories, chain, provider hostname, operation and timing, excluding addresses, hashes, request/response bodies, credentials and raw exception messages.

The browser has a separate read-only fallback for explicit relay availability statuses 408, 429, 502, 503 and 504. It uses only the pre-approved direct RPC endpoints, validates their chain, and retains canonical-block and evidence checks. Authorization failures, malformed evidence, wrong-chain relay data, JSON-RPC semantic errors, caller cancellation and ambiguous fetch exceptions do not authorize that fallback. Per-chain relay cooldown and a shared recovery probe avoid repeated outage requests. Broadcast ignores this read-routing state and never automatically changes path.

The reusable gate in `worker/runtime-tests/` executes the exact built Worker under real workerd with controlled outbound responses. This tests runtime request construction, both supported chains and redirect rejection without replacing native fetch. A live internet check from the local workerd process was attempted but failed because this environment could not resolve the provider hostnames; it is not claimed as a live or authenticated production pass.

Native browser UI checks also exercised the actual production NetworkClient and Rust WASM against live Base mainnet, with a controlled local relay returning HTTP 503. The approved direct read fallback returned 10,000 USDC units (0.01 USDC), and the Rust validator confirmed the externally supplied incoming transfer as finalized according to the provider. A read-only 0.01-USDC transfer simulation returned execution and Base L1 data fee estimates, and correctly reported insufficient ETH for gas. These checks used no vault, secrets, signature or broadcast; personal account and transaction identifiers are not included in this repository. They prove the browser fallback and public-chain reads, not authenticated production relay connectivity.

The protected `/diagnostics.html` page provides the final production check through the same deployed NetworkClient and local Rust validator. It accepts only a public address and optional transaction hash, starts with empty inputs, displays the successful relay/direct transport, and never opens or modifies a wallet vault or transaction journal. It has the same HTTPS/top-level requirements, restricted CSP and asset integrity checks as the wallet. At packaging time, normal browser sign-in and this deployed-page check remain the final verification step.

## Web 0.3.1 browser networking correction

The 0.3 browser transport stored native `fetch` as a class property and invoked it with the transport object as its receiver. A real browser reproduced `TypeError: Failed to execute 'fetch' on 'Window': Illegal invocation` before any HTTP request. Node's fetch implementation and the injected arrow-function transports used in earlier acceptance checks did not enforce that receiver requirement. Those earlier checks therefore did not establish that the deployed browser could read balances.

The transport now invokes the injected fetch function through a receiver-free closure for both RPC and activity requests. Relay requests use explicit CORS mode with the fixed relative URL, same-origin credentials, no-referrer and redirect rejection. Strict server Origin and method checks remain intact. The CORS change also avoids the Fetch Standard's null-Origin behavior for non-CORS POST plus no-referrer; null Origin was modeled in a regression test, not observed in the managed HTTP preview.

Confirmed with browser UI actions against the actual production `NetworkClient`, its default native browser fetch and public mainnet providers:

- Base chain identification and pinned-block balances succeeded. The externally supplied receiving address held 10,000 USDC units, equal to 0.01 USDC, with zero ETH and EURC at the checked block. Personal address and transaction identifiers are excluded from the source and deployment.
- Ethereum chain identification succeeded.
- The actual relay-mode client passed a local relay-shaped fixture enforcing Origin, content type, chain, method and fetch-site checks. The server observed the correct Origin, no Referer and a same-origin nonsecret test cookie.
- These browser checks did not create a vault, request recovery words, sign, submit funds or bypass the private site's authentication.

The authenticated production wallet flow remains unverified from this environment. Production page/asset logs and a successful publication are not substitutes for that flow. Automatic approval review rejected a separate machine-authenticated private-production diagnostic; no result from that diagnostic is claimed. The safer browser-native and permitted hosting-log checks were used instead.

## Completed checks

| Check | Result | Evidence and meaning |
| --- | --- | --- |
| Rust tests | **63 passed** | Prior 56 plus 7 adversarial incoming-activity validation tests; `docs/evidence/core-v03-tests.txt` |
| Property checks | **Passed within crypto suite** | Exact amount round trips, supported signed-payment reconstruction and malformed input behavior; deterministic fixtures plus generated cases |
| Native and WASM Clippy | **Passed with warnings denied** | `docs/evidence/core-v03-native-clippy.txt` and `core-v03-wasm-clippy.txt` |
| Rust formatting | **Passed** | `docs/evidence/core-v02-fmt.txt` |
| Independent ethers comparison | **12 vectors passed** | Both networks, ETH/USDC/EURC, normal and calldata-floor boundary cases; exact derivation, units, unsigned serialization, signed bytes, recovered sender and hashes |
| Generated Python/UniFFI bridge | **Passed** | Restored and authenticated a public fixture, signed both mainnets, persisted/reopened Ethereum records, reconciled actual fees and rejected a testnet |
| Release WASM runtime | **Passed** | Actual compiled module, wallet generation/restoration, standard public account, independent Argon2 vector, both-network signatures and receipt validation |
| Web TypeScript | **Passed for 0.3.2** | `docs/evidence/web-v032-typecheck.txt`; hosted Worker typecheck also passed |
| Web unit/adversarial tests | **60 passed for 0.3.2** | Includes controlled read fallback, wrong-chain/auth/malformed-response rejection, concurrent recovery probes, cancellation and response-generation races, plus native-fetch receiver and Origin admission regressions; `docs/evidence/web-v032-tests.txt`. |
| Web WASM integration tests | **3 passed** | Actual release WASM in `web/evidence/wasm-tests.log` |
| Web static and hosted Worker build | **Passed for 0.3.2** | Embedded wallet/diagnostic assets, unchanged valid Rust WASM, four SRI checks, restricted routes/security headers and preserved Site identity; `docs/evidence/hosted-artifact-v032.json` |
| Relay adversarial tests | **18 passed for 0.3.2** | Includes manual redirect rejection, no forwarded credentials and sanitized diagnostics, in addition to wrong-chain providers, bounded responses, allowlists, semantic reverts, failover, cooldown and single-attempt broadcast; `docs/evidence/relay-v032-tests.txt` |
| Native Cloudflare runtime | **Passed for final 0.3.2 artifact** | Exact built bundle under official workerd, both chains, RPC/index redirect rejection and credential containment; `docs/evidence/workerd-v032-final-artifact.txt`. Controlled outbound fixture, not a live network or authentication claim. |
| Android ARM64 and x86-64 Rust libraries | **Compiled** | NDK 27.2.12479018, API 30; 16 KiB ELF LOAD alignment |
| Kotlin/Android compilation | **Passed** | AGP 8.9.1, Kotlin 2.1.20, JDK 17, SDK 35; real generated UniFFI Kotlin and JNA |
| Android JVM tests | **20 passed** | Includes the real generated Kotlin-to-Rust bridge, incoming 0.01-USDC receipt validation, zero-ETH fee previews, provider failures, canonical read fallback and single-attempt submission; `android/verification/` |
| Android release lint | **No errors** | Remaining warnings concern English-only UI strings; lint caught an API 30 incompatibility that was fixed |
| APK assembly | **Passed** | Real release APK with debuggable=false; APK Signature Scheme v3 verified; non-debuggable manifest and 16 KiB ZIP alignment checked in `docs/evidence/apk-*.txt` |
| Cargo dependency audit | **0 known vulnerabilities** | One maintenance warning for host-only `basic-toml`, detailed below |
| npm dependency audit | **0 known vulnerabilities** | Root independent-reference graph and web graph checked; web report in `web/evidence/npm-audit.json` |
| Read-only mainnet balances and fees | **Passed on Ethereum and Base** | Actual NetworkClient, in-process production relay and release registry; pinned-block ETH/USDC/EURC balances and simulated USDC fee quotes, including Base additional fees; `docs/evidence/mainnet-live-reads-v03.json` |
| Read-only mainnet incoming USDC | **Passed on Ethereum and Base** | Real public transaction/receipt/canonical-block observations validated by the actual WASM; `docs/evidence/mainnet-live-activity-v03.json`. These public fixtures are not the user's deposit. |
| Desktop and mobile presentation | **Reviewed in browser** | Actual shared presentation components/CSS, isolated synthetic fixture, desktop and 390px phone widths, expanded fee breakdown. No wallet/seed/storage/signing code in the preview. This is presentation QA, not authenticated wallet end-to-end testing. |
| Managed HTTP preview | **Secure-origin rejection retained** | The real wallet still refuses insecure HTTP and iframes. A separate non-wallet UI fixture was used for presentation review. |
| Private HTTPS deployment | **Existing origin retained** | Hosted release uses the same owner-private Site and local database identity. The native deployment result is the publication authority. |
| Native browser networking | **Passed for 0.3.2 fallback** | Actual NetworkClient/default native fetch and Rust WASM, live Base balance/received-transfer/fee checks through a controlled local relay outage; see correction details above. Earlier 0.3.1 checks also covered Ethereum chain identity. This is distinct from signing or authenticated production end-to-end testing. |
| Authenticated production wallet flow | **Final check pending at packaging** | The normal sign-in gate and owner-private audience are preserved. `/diagnostics.html` can verify deployed public-data reads without importing a wallet. No machine-authenticated result is claimed. |

Tests do not broadcast transactions. Public development phrases in tests have no secrecy and must never be funded. The supplied applications do not preload those phrases or any account. Wallet creation occurs when the user runs the client.

## Important failure cases exercised

- Wrong network and chain-specific token confusion; Ethereum and Base nonce 0 remain separate.
- Invalid addresses/checksums, decimal overflow/precision, malformed/canonical encoding, signatures and modified approval context.
- Altered Base fee estimates invalidate review even when unsigned transaction bytes are unchanged.
- Missing Base fee data, Ethereum receipts carrying Base fees, excessive execution charges, and actual Base additional fees exceeding estimates/reserves.
- Exact signed bytes must match the approved preparation. Conflicting duplicate writes are rejected.
- Durability failure after an IndexedDB request succeeds but before its transaction commits produces **zero broadcasts**.
- Broadcast timeout stays uncertain; retry preserves the same bytes, nonce and hash.
- Delayed uncertainty cannot downgrade an already acknowledged pending state.
- Correct and incorrect ERC-20 Transfer evidence, reverted transactions and gas charges.
- Canonical-block/finality races, explicit reorg correction, and contradiction of previously finalized evidence.
- Corrupted saved receipt projections and balance snapshot linkage; another chain/account cannot masquerade as the active wallet.
- Old native schema v1 data is preserved and rejected rather than silently repurposed for a mainnet wallet.
- Browser v0.2 database migration preserves exact encrypted vault, signed bytes, nonce reservations and historical events while adding activity stores.
- Missing/invalid incoming evidence, unrelated transaction hashes, ambiguous status, same-block event additions/removals and oversized aggregate activity evidence fail closed.
- Automatic refresh preserves transfer/password fields and ignores responses belonging to another session or network. Manual lookup is serialized with background discovery.

## Implementation review fixes

The implementation was reviewed within this development task. This is not an external audit. Review and compilation led to fixes for:

- Clearing the complete Argon2 working memory allocation, rather than relying only on a dependency feature flag.
- Using a previous receipt's own height to establish a reorg; a contradictory new receipt cannot erase still-canonical evidence.
- Preserving finality contradictions as incidents rather than automatically repairing them into a spendable state.
- Rejecting duplicate RPC JSON keys before normalization can erase evidence of ambiguity.
- Revalidating stored browser receipt evidence and checking latest balances against their original snapshot records.
- Preserving an existing vault if setup commits and a subsequent operation fails or the UI locks.
- Binding Android authentication callbacks to operation epochs/cancellation identity and preventing queued work from signing/submitting after lock.
- Wiping Android's transient vault key immediately after signing, before journal/network work.
- Replacing an Android API 31-only integer conversion with a bounded API 30-compatible conversion.
- Removing raw signed bytes from audit exports while preserving public transaction traceability.

## Dependency findings

The Cargo audit database reported 1,403 advisories and commit `965bf9ad083b58e6340d65829d85217b18be9290` (last updated 2026-11-03). The locked graph had **zero known vulnerabilities**.

`basic-toml 0.1.10` remains an **unmaintained** dependency in the host UniFFI binding generator (`RUSTSEC-2026-0374`). It is not in the native application or WASM runtime graph. No ignore rule suppresses the finding. Generation operates on local project metadata. The generator should be updated when its upstream dependency is replaced.

The patched signing stack uses `bip32 0.6.0`, `k256 0.14.0` and `crypto-bigint 0.7.10`; the earlier vulnerable arithmetic dependency was not reintroduced. A clean scan is limited to published advisories and does not establish absence of vulnerabilities. Maven dependencies have checksum verification metadata; an independent Maven vulnerability scan is not claimed.

## Not established by these checks

- **Physical Android behavior:** installation and Keystore/biometric/PIN flows on a real device, lifecycle/overlay protection on vendor variants, device backup/transfer behavior, and hardware protection level.
- **A funded mainnet send/receive cycle:** none was performed. v0.3 read-only mainnet balance, fee and incoming-receipt checks succeeded through the production relay implementation running in-process. They do not prove the user's actual deposit status, an authenticated deployed-browser flow or every provider/deployment region. Earlier environment-specific 403 probe results are retained as historical v0.2 evidence and are superseded for these successful v0.3 read paths.
- **Consensus verification:** this is a fixed-provider RPC wallet, not a light client. A consistently dishonest provider can present false chain observations. Repeated canonical reads do not create an independent quorum.
- **Independent audit, penetration test or release-process audit:** internal review and tests cannot substitute for these.
- **Full account-history reconstruction:** balances are observed from blockchain state independently of discovery. Activity combines local outgoing intents with bounded recent token/direct-ETH evidence and hash lookups. No paid historical index is configured; old/internal ETH activity can be absent. Restoring words does not recover deleted audit records.
- **Protection against malicious software delivery or a compromised host:** Android app updates, web origin code, browsers/extensions and device compromise remain material trust boundaries.

Mainnet is enabled as requested. The remaining acceptance work is documented in [DEVICE_TESTS.md](DEVICE_TESTS.md) and the platform-specific notes. It is not a hidden testnet restriction or a claim that these checks have already been performed.
