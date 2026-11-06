# zerø for Android — 0.14.0 engineering release

**Implemented in source:** the actual local private-note engine and prover, an isolated bundled WebView host, native approval/journal binding, exact private Send/Deposit, encrypted history backup, quarantined restore and finalized self-sweep recovery. Private sponsorship uses the separate OHTTP relay/gateway and restricted gas service. Production private transfer remains disabled until deployment and device qualification are complete.

Metadata is code 17 / `0.14.0-mainnet`. Android compilation, all 296 release JVM tests, release lint (zero errors, 13 warnings), and APK assembly passed. Both 64-bit native libraries and all packaged private-engine resources were checked, including 16 KiB ELF/ZIP alignment. Signing and source provenance are recorded separately in the release `APK_MANIFEST.json`. Device installation, an upgrade preserving a real vault, and live private transfers remain unverified. See [RELEASE_0.14.0.md](../docs/RELEASE_0.14.0.md), [PRIVATE_GAS_UPGRADE.md](../docs/PRIVATE_GAS_UPGRADE.md), and `verification/v0140-android-build.json`.

The app displays **zerø** and a local opening transition. The package ID, encrypted vault, recovery derivation and storage identifiers remain unchanged. The transition respects disabled animations and screen-reader touch exploration, has a 1.5-second deadline, and yields immediately to authentication, errors or backgrounding.

The private engine is a packaged computational component. It has no browser UI, external navigation, direct network access or downloaded executable code. The native UI remains Kotlin; the host accepts only the pinned synthetic HTTPS origin, hash-verified APK resources and transferred MessagePort messages. `preBuild` runs the locked npm build. An optional `-PprivateProtocolManifest=/absolute/path/manifest.json` supplies qualified, hash-matched circuit artifacts; it cannot enable the independent native release gate. The artifact build and provenance instructions are in [the engine guide](../privacy-engine/README.md).

## Wallet management in 0.14.0

- **Profile → Add wallet:** create a new wallet or import a different recovery phrase. Each wallet has independent encrypted storage, wrapping keys and transaction journals.
- **Profile → Wallets:** select a saved wallet, identified by its name and public address. Each wallet keeps its own app-lock setting.
- **Profile → Sign out:** close the active wallet and persist the signed-out state. Saved wallets are retained; restarting does not automatically reopen them.
- These controls are also available in Settings, including when app lock is Off. Signing out is distinct from locking the current wallet.
- The existing `wallet-v1` directory is preserved. New wallets use separate UUID directories under `wallets-v1`. Duplicate imports are rejected, and switching invalidates old signing, recovery, private-workspace and document-picker callbacks.

## Retained public wallet and 0.10.0 network source

This is a native Kotlin application using restricted Rust signing engines through generated UniFFI bindings. The source supports Ethereum, Base, OP Mainnet, Arbitrum One, Avalanche C-Chain, Polygon PoS, BNB Smart Chain, Solana and TRON. The exact native assets and canonical stablecoins supported on each network are listed in [NETWORK_EXPANSION.md](../docs/NETWORK_EXPANSION.md). The public wallet contains no testnet switch, custodial server, dApp connector, token-approval flow, swaps, bridges or arbitrary message-signing endpoint. The separately gated private engine uses the restricted offline WebView described above.

The nine-network source is compiled into the 0.14.0 APK. The following version sections preserve historical changes; their earlier build limitations do not describe the current build result.

## Historical 0.10.0 remaining networks

- Added Solana, TRON, Avalanche C-Chain, Polygon PoS and BNB Smart Chain; retained Ethereum/Base/OP/Arbitrum. Exact supported assets are in [NETWORK_EXPANSION.md](../docs/NETWORK_EXPANSION.md).
- Added separate restricted Ed25519/Solana and secp256k1/protobuf/TRON signing engines. The authenticated entropy vault and EVM address are retained; Solana/TRON public addresses are derived only after opening that same vault.
- Asset-first selectors, native-currency fees, exact Max, USD quote requests, deposit QR, activity discovery and durable transaction handling are integrated into the native app.
- TRON provider credentials are optional local settings, encrypted with a distinct Android Keystore key. They cannot sign transfers. No shared provider secret is embedded in the APK.
- That source checkpoint used **0.10.0-mainnet / code 13** and did not produce an APK. Its implementation is carried into 0.13.0 with newly compiled libraries.
- Physical-device behavior and live provider operation still require qualification. Never reuse 0.8/0.9 native binaries for the current source.

## Historical 0.9.0 network expansion

- ETH and native USDC gain OP Mainnet and Arbitrum One. The asset picker still lists USDC, EURC and ETH only once; its network sheet is derived from the canonical registry. EURC remains limited to Base/Ethereum. Arbitrary tokens, bridged USDC.e and additional EVM chain IDs are not enabled.
- Deposit QR, copy/share context, balances, drafts, fees, Max, review, history and explorer links remain tied to the selected network. New networks share the existing first Ethereum-style address, without creating a seed or changing recovery derivation.
- The two supported OP Stack networks query their own GasPriceOracle and account separately for execution, L1 data and operator fees. Arbitrum estimates and receipts count posting costs inside its gas charge; no separate OP fee is added. Failed Arbitrum ETH simulation cannot fall back to a 21,000-gas total.
- Exact token Max remains the full integer token balance. ETH Max deducts the fee reserve using each network's fee model. The existing 300,000-gas and 0.05-ETH reserve policy limits remain; requests exceeding them fail closed.
- Old Ethereum/Base schema-2 journals remain readable and writable without changing their schema, prepared/signed bytes or event records. New journals use schema 3 and separate chain-specific files. The encrypted vault, key wrapping, recovery words, optional lock and wallet name are preserved.
- Candidate package metadata is **0.9.0-mainnet / code 12**, with the same package ID. Signing, Android unit tests, lint, assembly and physical-device checks remain release requirements; they are not implied by source review. Web remains at 0.6.0 and was not deployed.

## 0.8.0 single-form withdrawal (retained)

- The reference recording supplies the interaction order: choose an asset, then enter the recipient, network and amount on one form. The Android design uses warm off-white surfaces, restrained fields, a network bottom sheet and a fixed footer with the recipient amount, inline fees and one Review action. The separate fee-preview page is removed.
- Paste reads the clipboard only after the user's explicit tap. Generic network selection starts unchosen, and the first choice retains an address entered beforehand. Later network changes use the correct wallet/chain/asset draft. Amounts, Max and preparation remain bound to the selected canonical asset/network.
- Fee quotes are read-only, normalized to the exact recipient and integer amount, and bounded by a monotonic freshness deadline. One quote runs at a time; newer input replaces pending work. Changes, app backgrounding, and page transitions invalidate old results. Quote work does not block form editing or authorize a payment.
- USDC/EURC recipients receive the exact token amount; network fees are paid separately in ETH. ETH Max subtracts the fee reserve without decimal truncation. Coinbase USD equivalents are indicative and do not enter transaction arithmetic. Missing prices or fees are labelled as unavailable.
- Review performs fresh preparation and creates one durable unsigned intent. Final confirmation still revalidates the immutable transaction and uses the selected optional app lock. Edit and Back must durably cancel that exact unsigned intent before reopening the form; signed or uncertain submissions remain in Activity for reconciliation.
- Package version is **0.8.0-mainnet / code 11**. The existing signing certificate, wallet data and Rust signing policy are retained. Web remains at 0.6.0.

## 0.7.1 asset first, then network (historical)

- **DEPOSIT** and **SEND** show one entry per supported asset: USDC, EURC and ETH. The next screen provides a **Network** dropdown containing only that asset's supported networks. Asset choices do not depend on balances or the small-balance display filter.
- Home actions require an explicit network choice before showing a deposit QR/address or the transfer form. A shortcut from a specific holding keeps its already explicit network and exposes the same editable dropdown. Returning from another app preserves an unselected network as unselected.
- Changing asset or network invalidates Max, fee previews and pending UI read results. Each transfer draft remains scoped to its wallet, asset and chain. The receiving QR, copy/share action, token contract, available balance, fee estimate and final review use the selected network. Portfolio holdings remain network-specific; choosing a network does not bridge funds.
- Canonical choices are checked against the fixed supported asset identities and native signing-core registry. Tokens with a matching ticker but a different contract are rejected.
- Package version is **0.7.1-mainnet / code 10**. This update retains the previous vault, signing certificate and local data. Web remains at 0.6.0 and is not deployed.

## 0.7.0 USD values, exact Max and immediate onboarding (retained)

- Holdings are valued in USD using Coinbase's public `USDC-USD`, `EURC-USD` and `ETH-USD` spot endpoints. The independent price worker never receives a wallet address or a holding quantity, and does not block chain reads, signing or transfers. A public price GET retries once after a timeout; TLS, HTTP and malformed-response failures are not retried. Quotes use decimal strings, an explicit local receipt time and freshness state. This is an estimate, not an executable exchange quote or a guaranteed stablecoin peg.
- **Profile → Settings → Hide small balances** uses a configurable USD amount, initially **$1.00**, with hiding off by default. Threshold input accepts $0.01–$1,000,000 with up to two decimal places. Comparison uses exact unrounded valuations. Missing/stale prices or stale balances keep positive rows visible; hiding affects neither totals nor the complete Deposit/Send selectors.
- **Max** preserves integer units throughout. USDC/EURC Max selects all observed token units; ETH Max subtracts the reviewed network-fee reserve without truncating decimal digits. Max is refreshed before final review. The final reviewed amount remains fixed for signing. Actual fees can consume less than the reserve, leaving ETH behind; this is not decimal dust caused by rounding.
- New-wallet creation saves and reads back the encrypted manifest, then opens the wallet and shows a success message pointing to **Profile → Settings → Recovery phrase**. There is no forced backup challenge or app-lock setup. Existing incomplete-backup wallets open normally under their existing optional lock policy; their backup flag is not fabricated as confirmed.
- Returning from another app restores the allowlisted public page, including Recovery phrase. Plaintext words, PINs and payment approval are not retained for this purpose. Recovery words remain concealed until explicitly revealed again; an enabled lock still requires authentication. Copy and screenshots remain available after reveal.
- The operator receives no private key or recovery phrase, does not hold the funds and cannot recover lost keys. Encrypted wallet records are local. Public-chain providers can see public address requests, and Coinbase receives ordinary price-request metadata; the app does not claim that no data exists anywhere.
- Android package version is 0.7.0-mainnet / code 9. This update leaves the web client at 0.6.0 and does not deploy it.

## 0.6.1 holdings, profile and recovery privacy (historical)

- The home list shows known positive holdings. Observed zero balances do not produce asset rows. An empty list distinguishes verified zero holdings, unavailable observations and small balances hidden by the display filter. Cached positive holdings remain visible with their existing freshness label.
- **Profile → Settings → Hide small balances** is off by default. When enabled, it hides amounts below **1 USDC**, **1 EURC** or **0.0001 ETH**, using exact token units. Values equal to the threshold remain visible. The preference filters only home rows; it does not remove assets, modify Total USDC, change history or block sending/depositing. These are token thresholds, not USD price estimates.
- Profile contains the wallet name, public address and Settings entry. Settings contains wallet renaming and the existing display, lock, recovery and export controls. The hidden legacy account-name preference is retained for compatibility; saving a wallet name does not require or rewrite it.
- Opening Recovery phrase, creating a wallet or resuming an unfinished backup starts with concealed words. An explicit **Reveal recovery phrase** action decrypts and displays them using the current optional app-lock policy. **Hide** and lifecycle/navigation exits clear the displayed words and restore secure-window protection. Copy and screenshots are available only after reveal.
- Reveal results are bound to their screen, wallet and foreground session; late results cannot repopulate a hidden or unrelated page. If device authentication pauses the app, returning requires a fresh foreground Reveal confirmation before decrypting the phrase. The deferred authorization expires and is cleared by another background transition or navigation. Backup verification retains only its selected challenge words rather than the full phrase.
- The multiline import box uses top/start alignment. The existing off-white presentation and stable refresh bindings are retained.
- This is an Android-only update. Web remains at 0.6.0. Existing vault formats, derivation, transaction policy and native libraries are retained.

## 0.6.0 unified portfolio (historical; 0.6.1 filters the home list)

- The home page shows Base and Ethereum together, with a separate row for every supported asset/network pair. **DEPOSIT** and **SEND** open an explicit asset-and-network choice. There is no global network switch.
- **Total USDC** adds the exact integer USDC units from both chains. It is unavailable until both chains have an observation; a missing amount is never zero. ETH and EURC are not folded into a fabricated USD total. Cached totals retain a quiet “Last checked” label.
- The two networks refresh independently. Both balance reads run before slower activity discovery, and a failure on one chain preserves the other chain's last observation. The single transaction worker, per-network journals, review binding and exact fee checks remain in place.
- Polling updates existing text/status/asset views. It does not call `setContentView`, regenerate the receiving QR, rebuild a transfer form, or replay the opening transition. Activity content updates only when its display data changes and preserves the existing scroll view. Late responses are ignored after a lock, background transition, account change, or session change.
- Wallet and account names can be edited in Profile. They are cosmetic metadata in separate, address-scoped app-private preferences, committed together; no nickname enters the vault, derivation path, asset registry, transaction, or signature. Names use trimmed NFC text, 1–32 Unicode code points, and reject control/format/surrogate/line separator characters.
- A short original wordmark opening transitions into the app once. It does not impose an artificial wait and respects Android's disabled-animation setting.
- The supplied screen recording is a design reference. The refresh-rebuild issue was identified from code; no claim is made that the reference recording reproduced this wallet's device glitch.

## 0.5.1 local journal correction

- Android can expose the app-private directory through a framework-owned parent symlink. Passing that spelling directly to SQLite with `SQLITE_OPEN_NOFOLLOW` failed in 0.5.0 before the initial balance read; a later refresh also depended on saving the observation to that journal.
- The app now verifies that `wallet-v1` itself is a real directory, resolves only its trusted parent path, and appends the unchanged journal filename. The Rust database-leaf no-follow check, durable commit requirements, identity checks, and outgoing-payment gates are retained. No vault or database is reset, moved, or replaced by this correction.
- Local record failures show stage-specific codes such as `LOCAL_JOURNAL_OPEN` or `LOCAL_BALANCE_SAVE`. RPC failures show `RPC_UNAVAILABLE`; unexpected balance evidence shows `BALANCE_VERIFY`. Diagnostics omit paths, account addresses, and record contents.
- This Android app connects directly to fixed public HTTPS RPC providers. It does not require hosting the web app, registering a domain, or creating a cloud instance.
- The exact former path failure is reproduced with the actual compiled Kotlin repository and unchanged Rust library in a temporary Android-style symlink layout. The regression suite also verifies durable balance/history reopen, both app-directory and database-leaf symlink rejection, malformed journal preservation, and account/network/stale-record rejection. A physical-device confirmation remains necessary; no device filesystem was inspected remotely.

## Install the delivered APK

- Requires Android 11 / API 30 or newer and a 64-bit ARM or x86-64 device.
- New wallets do not require an app PIN or a secure screen lock. Configure optional **Off**, **6-digit app PIN**, or **Biometrics or device screen lock** under **Profile → Settings → App lock**. Existing wallets retain their prior device-authentication requirement until the owner explicitly changes it.
- Compare the APK SHA-256 and signing-certificate fingerprint with the delivery report before installing it through Android's package installer.
- The delivered package is a non-debuggable engineering release. It is not a Play Store publication or an independently audited wallet.
- Create a new wallet and open **Profile → Settings → Recovery phrase** to record the 24 words. Words are concealed until **Reveal recovery phrase** is tapped. Copy and screenshots are available while revealed. There is no required word quiz. Alternatively, restore a standard English BIP-39 phrase with an empty passphrase.
- In 0.13.0, choose the asset first and then its supported network before depositing or sending. The seven EVM networks share the existing receiving address; Solana and TRON each have a distinct address derived from the same recovery phrase. Balances and fees remain separate by network. Switching networks does not move assets.
- Receive only the canonical asset on the selected network, and hold that network's native fee asset before sending.

Derivation paths are `m/44'/60'/0'/0/0` for EVM, `m/44'/501'/0'/0'` for Solana, and `m/44'/195'/0'/0/0` for TRON. Existing BIP-39 passphrases, additional account indexes, and contract/delegated accounts are outside this version's scope.

## What changed in 0.5.0

- **White/off-white interface:** a warm off-white background, white cards, dark text and accessible green accents replace the dark theme. The compact Send/Receive layout, network-specific balances, asset detail screens, receive QR and fee review are retained.
- **Resume unfinished recovery verification:** a saved wallet with `backup_confirmed=false` opens **Finish your backup**, with its existing address and a **Continue recovery verification** action. It decrypts the already-saved wallet and shows the same recovery words. It never creates or overwrites a second wallet. Metadata loading and the startup destination are tested with real generated Rust vaults after disk reopen.
- **Optional app lock:** creation and import do not ask for a new code. **Profile → Settings → App lock** offers Off, exactly **6 numeric digits**, or Android's existing biometrics/device screen lock. Labels distinguish the app PIN from the phone's own PIN, password or pattern. Sensitive operations still follow an explicit user action and immutable payment review; an enabled lock requires fresh authentication for key access.
- **Existing wallet migration:** v1 metadata keeps mandatory device authentication. Only an explicit, authenticated Settings change rewraps the same random vault key in a v2 envelope. The Rust vault ciphertext, account address, recovery words and financial journal are not regenerated. The old wrapping key is retired only after the replacement manifest is durably saved and read back. A write with an uncertain outcome keeps both keys.
- **Recovery-word copy and screenshots:** Copy places the whole phrase on the clipboard with Android's sensitive-content flag and an app-owned expiry token. The app attempts to clear that exact clipboard entry after 60 seconds while its process is running, without erasing unrelated later clipboard data. This flag hides system previews; it does not make the clipboard confidential. Screenshots are allowed only while a recovery-word screen is visible. Backgrounding blanks the words and restores secure-window protection; Android 13+ recent-app screenshots are disabled independently. Clipboard managers, screenshot tools and cloud-photo backup may retain copies outside this app's control.
- **Profile and settings:** Profile is a secondary navigation destination containing Settings. App lock, recovery words and account information live there.

The user's earlier generic startup message did not include its underlying device exception, so no exact native-device cause is claimed. The revised implementation makes interrupted onboarding resumable, preserves existing storage on errors, and distinguishes common authentication and metadata failures. Physical lifecycle/Keystore behavior must still be tested on a device.

## Retained from 0.3.0

- **Clear connection states:** live, updating, stale, and unavailable are shown explicitly. A refresh failure preserves the last verified balance and does not imply a failed or uncertain payment. Read requests can use a fixed backup provider, with a chain-ID check on each provider. Broadcasts never automatically fan out to multiple providers.
- **Automatic refresh:** the app loads saved observations after unlock, then refreshes the selected network. It attempts another read every 30 seconds while the Wallet, Receive, Asset, or Activity screen is open and idle. Balances update independently of activity lookup failures.
- **Received activity:** native Circle token transfers are discovered through bounded Transfer-log scans and independently checked against transaction, receipt, and canonical-block data using the shared Rust validator. Exact quantities, including 0.01 USDC, are displayed without floating-point rounding. Incoming and outgoing activity is not inferred from balance differences.
- **Track a transaction:** paste a public transaction hash to distinguish pending, not found on the selected network, failed execution, no supported transfer for this address, and verified receipt. A connection failure is shown as inability to check, never proof that a deposit failed.
- **Transfer cost before authentication:** the Send screen has a read-only cost step. It displays estimated ETH fees, the fee reserve, ETH available, and any ETH shortfall even when the wallet has USDC but no gas funds. If simulation is unavailable, a conservative budget is labelled accordingly. A fresh real-balance simulation remains mandatory before the final signing review.

### Updating an existing installation

The 0.14.0 release advances to code 17 and must use the retained distribution certificate. The package ID stays `app.localwallet.mainnet`. **Do not uninstall or clear app data to update.** Existing vaults, backup metadata and app-lock settings retain their storage formats. Migration to a new lock mode occurs only after an explicit, authenticated change in Settings. Journals and `activity-v1.sqlite3` observations remain in their original location. Actual in-place device upgrade behavior has not been tested in this environment.

### Activity scope

The initial automatic token scan covers at most the latest 8,000 blocks, in windows of up to 2,000 blocks. Later scans use the last completed cursor with an overlap, while remaining bounded. At most 20 newly discovered receipts are verified per pass; unresolved candidates are retained and do not advance the completed cursor. Older token transfers and **direct ETH transfers** can be checked by transaction hash. Internal ETH transfers are not inferred and may appear only in the authoritative balance. This is a bounded wallet activity view, not a complete address index.

Received observations are append-only and include their transaction/receipt/block evidence. Cached projections are revalidated through the shared Rust validator before display. Reorganization corrections reference their immutable prior observation and canonical contradiction. A finalized-inclusion contradiction remains an incident. The app rejects same-block changes to the complete transfer set or financial fields; it never silently replaces an amount. Proof duplication is bounded to 4 MiB per receipt write. These consistency checks do not make locally stored data cryptographically tamper-proof or replace consensus verification.

## Build from source

Install JDK 17, the pinned Rust toolchain, Android SDK platform 35, build-tools 35.0.0, and NDK 27.2.12479018. Accept the Android SDK license through the official SDK tools. The checked-in Gradle wrapper uses 8.11.1 and a pinned distribution SHA-256.

From the repository root:

```bash
export JAVA_HOME=/absolute/path/to/jdk17
export ANDROID_SDK_ROOT=/absolute/path/to/android-sdk
./scripts/build-android.sh
```

The script generates Kotlin bindings from the exact host Rust library, builds the native core for `arm64-v8a` and `x86_64`, and runs the JVM tests, Android lint, and release assembly. It uses 16 KiB ELF segment alignment. The default output is:

```text
android/app/build/outputs/apk/release/app-release-unsigned.apk
```

Use `zipalign -P 16 -f 4` and `apksigner` from build-tools 35.0.0 to align and sign the APK using your own app-distribution key. Distribution signing material must remain outside the source tree. Do not substitute a public Android debug key. App-distribution keys and wallet recovery keys are different credentials.

`WALLET_GRADLE`, `WALLET_CARGO`, `WALLET_RUSTUP`, and `ANDROID_NDK_HOME` may override tool locations. The Gradle dependency-verification file records SHA-256 checksums of resolved dependencies; it does not constitute an independent source or security audit.

## Implemented protection

- **Local entropy and keys:** the Rust core generates 256 bits of operating-system randomness and a 24-word BIP-39 phrase. The local vault stores authenticated AES-GCM ciphertext, not plaintext recovery words.
- **Android-bound wrapping:** a non-exportable Android Keystore AES-256 key wraps a random 32-byte vault key. With app lock Off, the device key is available to this app without a prompt. Device-auth mode requires a fresh `BiometricPrompt.CryptoObject` for every key operation. In app-PIN mode, PBKDF2-HMAC-SHA256 with 600,000 iterations and a random 16-byte salt derives an AES-GCM PIN key; that PIN envelope is itself wrapped by Android Keystore. The decrypted vault key is not cached between operations and is overwritten before network submission begins. A six-digit PIN is a local convenience lock, not a high-entropy recovery secret.
- **Address integrity:** authenticated vault opening derives the account again and compares it to receiving metadata. Versioned wrapped-key associated data binds the account, Keystore alias, selected lock method and PIN salt. Old v1 associated data is kept exactly for compatibility.
- **Storage:** the encrypted vault and per-network SQLite journals reside in `noBackupFilesDir`. Cloud backup and device transfer are excluded. Atomic vault writes, file synchronization, private permissions, and the Rust durable journal protect against partial local writes.
- **Native review:** chain, beneficiary, asset, contract, exact decimal amount, nonce, and fee budget come from the immutable prepared transaction. The Rust signer reconstructs and verifies the approved payload. There is no generic arbitrary-calldata signing API.
- **Transaction sequence:** simulate and check balances/account state/nonce; show review; revalidate; apply the chosen local unlock method; sign; commit exact signed bytes; submit those committed bytes. Transport timeouts retain an uncertain state. Retries reconcile first and reuse the same saved bytes and hash.
- **Lifecycle:** explicit lock and leaving the app invalidate subsequent signing/submission checkpoints. A signature already created during a lock race is durably retained without a new submission. A network request already started cannot be recalled by locking the app. Authentication callbacks are bound to their original epoch and cancellation signal.
- **Screen and input controls:** screenshot protection remains on all screens except the explicitly visible recovery phrase. Word screens have Copy and allow screenshots as requested. Overlay-window suppression where supported, obscured-touch rejection, no saved form state and disabled autofill/personalized-learning hints on secret inputs remain. Backgrounding invalidates active operations, blanks words and protects the screen. Enabled app-lock modes require reauthentication; Off mode opens without an app-auth prompt. No signing key remains cached while the public wallet UI is open.
- **PIN retry protection:** each correctly formatted attempt is durably reserved before any envelope is decrypted. After five failures, retries start at 30 seconds and increase to one hour. The record survives normal restart; a successful authenticated unlock clears it. These app-local counters do not establish rollback resistance against a compromised OS or a full restore of private app data.
- **Network boundaries:** fixed HTTPS RPC endpoints and read-only fallback providers, no HTTP redirects or user CA certificates, explicit chain-ID checks, bounded responses and timeouts, duplicate-field rejection, canonical integer parsing, and integer-only financial arithmetic. Errors do not fabricate zero balances or successful payments.
- **Reconciliation:** validate receipt identity and status, expected ERC-20 Transfer evidence, canonical inclusion, observed finality, and actual fees. Recheck the canonical block after the finalized-head query. Reorganizations add corrections; a contradicted finality claim pauses automatic reconciliation.

## Network fee handling

Base and OP Mainnet have execution, L1 data, and operator fees. The review labels the execution fee limit separately from estimated extra fees. `GasPriceOracle.getL1FeeUpperBound(1024)` and `getOperatorFee(gasLimit)` on the selected network provide the additional estimates; a reserve equal to the execution limit plus twice the estimated additional fees is checked against ETH balance. That reserve is a budget, not an on-chain cap on the additional fees.

For these two OP Stack networks, the receipt's `l1Fee` is required evidence. The operator fee is queried at the canonical receipt block and compared with a receipt-provided total when available. The journal records execution + L1 + operator fees. An additional fee above the estimate does not erase an actual successful or failed transaction outcome.

Arbitrum One's gas estimate includes L1 posting costs. Its network fee is `gasUsed × effectiveGasPrice`, with no separately added OP-style L1/operator amount. A fallback budget is labelled conservative and cannot authorize review/signing; fresh successful simulation is required. Arbitrum transfers above the retained gas ceiling are blocked. These four EVM networks use ETH for fees. Avalanche, Polygon and BNB use AVAX, POL and BNB. Solana and TRON have separate fee and resource engines; see the expansion specification.

## Scope and unresolved verification

- JVM tests exercise exact decimal/ABI conversion, provider fallback and chain checks, canonical-read compatibility, one-attempt submission, zero-ETH fee previews, the actual Kotlin-to-Rust bridge, and vault restart/lock migration. New vault tests use real Rust ciphertext and JCA ciphers with controlled filesystem/key adapters; they cover interrupted recovery backup, v1 compatibility, PIN correctness/backoff, before/after-commit faults, cancellation and lock-mode tampering. They do not simulate hardware Keystore or Android process lifecycle. Android lint and release assembly are separate checks.
- This environment has no physical Android device. Keystore/BiometricPrompt behavior, screenshot/overlay controls, installation and cold start on real devices, process-kill recovery, OEM backup behavior, and the native UI have **not** been device-tested here. Follow `DEVICE_TESTS.md` before relying on a distribution build.
- Network calls are real mainnet RPC calls. Live operation of this APK on a physical Android device has **not** been demonstrated here. Shared-core and server-side read-only checks are reported separately in the root verification report. No funds were sent in Android testing.
- The wallet trusts the configured RPC's chain observations. It is not a light client and does not independently validate consensus or Ethereum proofs. Public RPC outages and rate limits can block sending and leave observations stale.
- The application has no automatic replacement/fee-bump or signed-payment cancellation. One unresolved outgoing intent per network blocks another. An uncertain payment must be reconciled; a history or finality conflict cannot be cleared by editing balances.
- The local activity view is not a complete address index. It combines locally created payment intents, bounded verified token discovery, and explicitly tracked transaction hashes; balances remain authoritative blockchain observations.
- The OS, installed keyboard, app distribution, and runtime must be trusted. Keystore protection does not make a compromised process or malicious OS safe. Kotlin/Java strings used for recovery-word entry/display cannot be reliably zeroized; their lifetime is minimized and no plaintext words are deliberately persisted or logged.
- Native stablecoin issuer controls, contract upgrades, freezes, and chain failures remain external risks.

## Primary references

- Coinbase public spot prices: <https://docs.cdp.coinbase.com/coinbase-app/track-apis/prices>

- Android auth-per-use keys: <https://developer.android.com/identity/sign-in/biometric-auth>
- Android Keystore: <https://developer.android.com/privacy-and-security/keystore>
- Android backup controls: <https://developer.android.com/privacy-and-security/risks/backup-best-practices>
- Circle USDC contracts: <https://developers.circle.com/stablecoins/usdc-contract-addresses>
- Circle EURC contracts: <https://developers.circle.com/stablecoins/eurc-contract-addresses>
- Base network connection: <https://docs.base.org/get-started/connect-to-base>
- Base network fees: <https://docs.base.org/specifications/transactions/network-fees>

- PublicNode Ethereum: <https://ethereum.publicnode.com/>
- PublicNode Base: <https://base.publicnode.com/>
- Alchemy public Ethereum RPC: <https://www.alchemy.com/rpc/ethereum>
- Token receive QR format: <https://eips.ethereum.org/EIPS/eip-681>

- Android clipboard handling: <https://developer.android.com/privacy-and-security/risks/secure-clipboard-handling>
- Android recent-app screenshot control: <https://developer.android.com/reference/android/app/Activity#setRecentsScreenshotEnabled(boolean)>
