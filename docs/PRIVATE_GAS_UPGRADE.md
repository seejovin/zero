# Private gas abstraction — 0.13.0 engineering release

## Result and release status

The local private-note engine, genuine proof generation and verification, Android engine host and native UI integration, encrypted recovery workflow, OHTTP relay/gateway, provider fee verification, restricted treasury signer and durable operator accounting are implemented in source. Positive protocol tests use real upstream Groth16 circuits and public unfunded fixtures. The packaged JavaScript engine also passes the transferred-MessagePort proof and receipt flow.

**Production private sending remains disabled.** Android compilation, 270 JVM tests, release lint and APK assembly now pass for 0.13.0; signing evidence is recorded separately in the release manifest. No service has been deployed and no funded transaction was broadcast. Current deployed verifier/code/proxy qualification, independent relay operation, device validation and independent security review remain release requirements. There is no environment-variable, configuration or Settings bypass for the compiled release checks.

Android metadata is version code 16 / `0.13.0-mainnet`, displayed as **zerø**. The public wallet's recovery format, addresses and ordinary transfer paths remain in place. The release bundle is unqualified and contains no proving-parameter set. The web runtime was not deployed as part of this Android release.

## Precisely supported private operation

| Item | Candidate scope |
| --- | --- |
| Protocol | RAILGUN V2; fixed canonical `transact` ABI |
| Network | Arbitrum One, chain 42161 |
| Asset | Circle native USDC, `0xaf88d065e77c8cc2239327c5edb3a432268e5831` |
| Pool | `0xfa7093cdd9ee6932b4eb2c9e1cde7ce00b1fa4b9` |
| Recipient | Canonical 127-character `0zk` private address |
| Inputs | One to eleven spendable notes from one input tree |
| Outputs | Recipient, provider fee, and change or a zero-valued private padding note; locally shuffled |
| Fees | Shared signed USDC fee; operator pays Arbitrum native gas under an explicit cumulative cap |
| Excluded from this route | Unshielding, arbitrary external calls/adapters, nonzero native value, public-address transfers and automatic shielding |

These are candidate identities, not certification of the deployment's current code or administrative configuration. Ordinary USDC in an EVM account is not already a private note. Private Deposit displays a private receiving address for compatible shielded transfers; this upgrade does not automatically move public holdings into a pool. Existing public chains remain available through their existing public Deposit/Send flows.

## Architecture and data boundaries

### Android device

- The native vault derives only the protocol spending key, viewing key and independent encrypted-state key for the engine. It does not give JavaScript the BIP39 seed, recovery phrase or ordinary EVM signing key.
- The actual SDK derives the private address, decrypts incoming notes locally, constructs membership witnesses, selects exact integer amounts, generates the Groth16 proof and independently verifies it before release.
- The recipient, principal, note openings and witnesses stay in the local engine and encrypted local records. Hidden-sender notes have no sender address or memo disclosure. The provider fee is a separate encrypted output.
- Native journal approval precedes proving. The exact verified envelope is saved before release. An HTTP response, SDK return value or spent nullifier alone cannot mark a payment successful.

### Independent relay

The relay accepts only bounded OHTTP ciphertext and forwards it to one configured HTTPS gateway. It discards original client IP/Forwarded/User-Agent/cookie/authorization/request-ID headers, rejects redirects and accepts no user-selected destination URL. Its application has no per-user request log. It does see the network source, connection timing and ciphertext size.

The supplied relay executable is software, not evidence of organizational independence. Running it on another company-owned server would still leave the company able to observe both sides. HTTPS ingress, network infrastructure, access logs and retention must be configured and reviewed separately.

### Company gateway and broadcaster

The gateway decrypts a closed operation envelope, verifies the actual proof and its public inputs, and decrypts **only its own fee output** using the provider viewing key. It checks exact USDC amount, payee commitment and hidden-sender/no-memo encoding. The verifier has no customer viewing or spending key and is invoked as a pinned, bounded offline subprocess.

The company receives opaque operation digests, tree/root/nullifiers, commitments, ciphertexts, shared fee terms and timing. It retains the operational records required to prevent duplicate sponsorship and reconcile native expenses: admitted envelopes, reservations, outer signed bytes/hashes, attempts, fees and receipts. **This is not a zero-data-retention service.** Those operational fields are intended not to disclose the private destination or principal; a valid proof and ciphertext qualification are essential to that property.

The restricted signer can reconstruct only the admitted zero-value call to the fixed pool on the fixed chain. There is no arbitrary-message signer, caller-selected destination or treasury-transfer endpoint. It independently verifies the signed transaction's sender, chain, nonce, target, access list, calldata, canonical encoding and fees before durable release.

### Remaining privacy assumptions

- OHTTP relies on authenticating the gateway's key/authority and using a genuinely independent relay. A key fetched beside its own hash is not authenticated configuration.
- Relay/gateway collusion, global timing/volume observation, public pool entry/exit and other external information can permit correlation. This implementation does not promise anonymity against a global observer.
- The ordinary public-wallet RPC paths remain public-address queries to external providers. The new private path has no silent public-RPC fallback.
- The app disables WebView usage metrics. Android's platform privacy documentation states that this opt-out does not disable consented WebView crash reports, which may include sanitized stack memory. There is no claim of zero platform telemetry or guaranteed secret erasure in managed-runtime strings. See the [official WebView privacy documentation](https://developer.android.com/develop/ui/views/layout/webapps/webview-privacy).

## Complete scan and encrypted state

The engine reads only approved full-pool event types through a closed RPC bridge. It never asks the company for notes belonging to an address. Ranges are aligned, bounded and ordered; the scan rejects gaps, overwrites, skipped initial trees and impossible tree transitions. Duplicate funded commitments are preserved as distinct `(tree, position)` notes. Nullifier reservations use `(tree, nullifier)`, not the commitment alone.

The current public tree remains available for appends. Closed trees retain their root/count and only the wallet's authenticated membership paths. This avoids indefinitely storing every historical public leaf in each snapshot. Restored paths are verified against their roots, and retained notes are re-derived and checked against their leaf and nullifier.

A final scan must exactly match the contract's active tree number, next leaf index and root at the same finalized block. Historical `rootHistory` membership alone is insufficient. At a tree-rollover block the scanner also checks the preceding block's checkpoint, compares the whole relevant block-log group with complete canonical receipts, and checks the following checkpoint. The service requires one receipt for every transaction in the agreed block, with no missing, duplicate or wrong-index receipt. Oversize or unsupported evidence causes an explicit incomplete scan.

Every range is checkpointed before its cursor is persisted, and a resumed cursor is checked again. A rejected omission can therefore be healed by a complete retry. Older unreleased SDK caches are rescanned under schema3 while retaining every reservation.

The service accepts up to64 strictly ordered, complete historical deployment profiles with exclusive block boundaries. Historical reads select the corresponding reviewed code/storage pins; current admission, quotes and treasury checks require the current profile. The example contains no invented mainnet upgrade boundaries.

Both RPC providers must agree on the normalized canonical evidence. These checks detect omitted/truncated history and inconsistent records within that trust model. **They are not cryptographic consensus or receipt-trie proofs.** Deployed implementation history and legacy event semantics still require qualification.

The SDK state is AES-256-GCM encrypted before crossing into storage, with wallet identity/domain authenticated as associated data. Native storage accepts bounded chunks, checks total size/hash, and atomically commits the ciphertext. Reservations survive cache reloads, reorg rewinds and process restart. Native journal claims are merged into the cache; cancellation cleanup cannot clear a newer intent's claim to the same input. Failed cleanup leaves sending blocked until reconciliation succeeds.

Full initial/recovery scans still scale with public history. Archive-provider access, synchronization time, memory and proving latency have not been measured on Android. The source does not represent an unscanned or failed balance as zero.

## Transfer correctness and gas accounting

1. A common fee schedule is signed with a pinned Ed25519 publisher key. It binds domain, fee recipient/commitment, exact USDC fee, validity epoch, gas limit and cumulative native cap. Its validity interval is at most 24 hours. Per-user quote identifiers are not generated.
2. The device selects notes and prepares the exact recipient amount, fee and change. Max uses all selected atomic units without decimal truncation. Wallet-wide Max refuses partial selection across trees or beyond the circuit input limit; it does not quietly send less.
3. Native approval binds the full intent. Proving uses the real pinned WASM/proving key and must pass independent verification against the pinned verification key and exact expected public signals.
4. Input reservations persist before proof bytes are returned. The journal commits the exact envelope before it can be transmitted. Once released or exposed through a history backup, a proof cannot be made harmless by a local cancel button or quote expiry.
5. The service atomically reserves nullifiers, native exposure and its nonce. It verifies root/unspent state, deployment/code/circuit pins, exact simulation and affordability against **all** pending treasury exposure. Signed bytes persist before any broadcast ticket is released.
6. A lost response leaves the operation unknown. Repeated submission reuses the same private envelope. The service reconciles first and re-releases only the exact saved outer bytes while an attempt is unknown; a timer does not authorize another send.
7. Up to three sequential outer attempts are permitted only after the previous outer transaction has a canonical finalized revert and the private inputs remain unspent. Every prior native cost is retained. The next attempt must fit `original native cap − prior finalized revert costs`; the private amount, ciphertexts, proof and provider fee do not change.
8. Successful receipt observation requires exact calldata, expected nullifier/output events and canonical block evidence. With no acknowledgement, the device can discover the transaction through its pool scan and apply those same checks. A null receipt preserves uncertainty.
9. A reverted transaction earns no provider fee but can consume native gas. Fee earnings, cumulative native expenses, pending exposure, inclusion and finality remain separate journal facts. Earlier receipts remain audited after later attempts. Contradictory finalized evidence halts further operator spending.

Arbitrum's selected fee model uses `gasUsed × effectiveGasPrice`; L1 posting cost is included in that gas count. The adapter rejects an extra OP-style fee component. Signed schedules rotate in the same append-only operator journal; new journals are not used to forget previous nonce/nullifier/budget claims.

## Recovery and backup

The recovery phrase restores protocol keys and can rediscover mined notes. It cannot prove that an older, still-executable payment proof was never released. Treating every currently unspent note as immediately available after restoration would permit conflicting payments.

The implemented workflow therefore includes:

- A seed-bound encrypted private-history backup, with authenticated wallet/domain identity and exact journal evidence. Export marks an existing READY proof as backup-exposed before emitting backup bytes. Export failure cannot make that proof safely cancellable again.
- Android destination readback: the selected backup document is reopened and its exact ciphertext is checked before success is shown. No recovery phrase is exported in that file.
- Imported history remains quarantined. An incomplete or absent backup never authorizes cancellation or a new payment from unresolved inputs.
- The native recovery journal durably captures a latest-block cutoff, then waits for that exact cutoff to become canonical/finalized. Empty scans do not skip the finality requirement.
- A complete local rescan identifies all pre-cutoff notes. The user explicitly approves a bounded total fee and sequential self-transfer batches to fresh notes controlled by the same private wallet. Recovery does not send value to an external beneficiary.
- Exact canonical final receipts plus a fresh scan are required before native quarantine ends. Contradictory historical finality remains an incident, including after a later successful retry or recovery batch.
- Opening an existing private journal authenticates its established seed binding against the currently unlocked native vault, without triggering a backup export or altering proof exposure.

A stale full-file rollback cannot be detected by an isolated local journal without an independent anchor. Imported histories are quarantined specifically because local files cannot establish absence of previously released proofs. Old unreleased schema-1 private journals are rejected and preserved; there is no migration that invents a missing input tree.

A pre-cutoff note in an old tree may be too small to cover the required recovery fee. Such dust is not silently dropped, and quarantine is not incorrectly cleared. A subsidized/zero-fee recovery policy would need separate reviewed engineering; this candidate reports `RECOVERY_FEE_EXCEEDS_BALANCE`.

## Android implementation

The Kotlin UI adds the private workspace under Profile → Settings, with private Deposit, Send, exact amount/fee review, local activity, encrypted backup and recovery. It preserves the existing off-white public wallet and asset-first network selection. Release capability is checked in Kotlin and native code before protocol keys or privacy transport can be used.

The offscreen WebView loads a signed-APK asset bundle at one synthetic HTTPS origin. Assets are hash checked. Navigation, direct network/file/content access, cookies, debugging and JavaScript console output are disabled. Communication uses a transferred MessagePort with closed schemas, session/request binding, bounded sizes and one active command. There is no `addJavascriptInterface`, dynamic executable interpolation, downloaded script or generic network port.

The whole BIP39 seed and ordinary account key stay native. The protocol-specific keys necessarily enter the local prover's address space. Closing the engine on lock is best-effort lifecycle containment, not hardware isolation. Backgrounding invalidates approvals; an uncertain already-released payment remains reserved and is reconciled after reopening. Private state/catalog files are scoped to the public wallet and stored separately from its existing vault and public journals.

## Dependencies, artifacts and evidence

The runtime uses the narrower pinned `@railgun-community/engine 9.9.1`, `snarkjs 0.7.6`, `ethers 6.17.0`, Noble hashes and the required AES compatibility library. It excludes the full wallet SDK's network/subgraph/broadcaster-discovery/global-proof-cache wrappers. The upstream circomlib cryptographic source is unchanged; vendored metadata removes unused old `web3` runtime dependencies. `inspect-sdk.mjs` verifies the cryptographic source hashes.

The recorded npm advisory count is zero. The final Cargo audit also has zero vulnerability/unsoundness entries after updating rustls0.23.46, rustls-webpki0.103.16 and Tokio1.53.3; two maintenance notices remain documented. These are known-advisory results, not proof of security. Original qualification evidence is retained. The build generates notices from its actual JavaScript dependency graph and includes them as a hash-pinned asset. GPL-3.0 dependencies and missing upstream notice details still require complete distribution/corresponding-source review.

All eleven `01x03` through `11x03` proving-parameter sets were downloaded from the immutable upstream CID, decompressed under size limits and checked against the pinned upstream SHA-256 catalog. Each verification key was derived from its authenticated proving key and matched against its published key. The total uncompressed artifacts are 272,781,169 bytes. Real positive proofs were generated for one and eleven inputs; the intermediate shapes were hash/key-qualified, not each exercised with a proof. No new trusted setup was invented.

Fresh evidence is in `android/verification/v0130-private-verification.json` (256 Rust tests and 40 JavaScript tests) and `android/verification/v0130-android-build.json` (270 JVM tests, compilation, lint and APK inspection). Earlier protocol/VM evidence remains explicitly historical in `v0120-verification.json`. Device, browser and live-chain results are not inferred from host tests. Small genuine-proof service fixtures are explicitly marked synthetic and unfunded. See [the engine guide](../privacy-engine/README.md) and [operator guide](../gas-service/README.md) for reproducible commands.

## Release requirements still outstanding

| Requirement | Current result |
| --- | --- |
| Source integration and offline proof/ciphertext/native-journal tests | Implemented and exercised |
| Current deployed contract/proxy/admin/verifier and historical event qualification | Unverified; attempted read-only RPC POSTs returned 403 |
| Independent relay and reviewed HTTPS ingress/retention | Executable source exists; infrastructure not provisioned |
| Production fee keys, gateway pin and bounded treasury configuration | No production secrets or funded configuration supplied |
| Android compile/JVM tests/lint/APK assembly | Passed for 0.13.0; 270 tests, zero lint errors and 13 warnings |
| Distribution signing | Exact certificate and source provenance recorded separately in release `APK_MANIFEST.json` |
| Device install/upgrade, memory/lifecycle/sync and private proving | Unverified; no emulator or physical-device result |
| Actual WebView/CSP runtime | Unrun; attempted Chromium launch was blocked |
| Funded mainnet transfer/recovery/fee accounting | Unrun; no funds spent |
| Independent security review and binary licensing/corresponding source | Outstanding |

These release facts are why private sending remains unavailable in the normal build. A successful source build or a proof fixture alone would not authorize removing the gates.
