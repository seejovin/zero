# zerø

An Android non-custodial wallet for receiving, holding and sending supported mainnet assets. The Android app stores wallet secrets and signs locally; the repository also retains the earlier web client and the private gas sponsorship service.

## Current release candidate: 0.13.0

- **App:** zerø, with a native opening transition and matching launcher icon. The existing `app.localwallet.mainnet` package, wallet storage and app-signing identity are preserved for an in-place update.
- **Public wallet:** nine supported networks and the retained asset-first Deposit/Send flow. Balances, network fees, recovery and transaction signing remain local-client workflows.
- **Private gas abstraction:** the local RAILGUN V2 engine, Groth16 prover, Android bridge, encrypted recovery, OHTTP transport and restricted operator service are implemented. **Production private sending is disabled pending the release qualifications below.**
- **Build:** [Android CI](.github/workflows/android.yml) builds ARM64 and x86-64 native libraries, runs release JVM tests and lint, and produces an aligned unsigned APK. The original distribution key stays outside GitHub. [Release evidence](docs/RELEASE_0.13.0.md) records what has actually run.
- **Hosting:** the public wallet does not require a company backend. The private service requires operated infrastructure, production pins and a separately operated privacy relay. Follow [the Tencent deployment guide](docs/TENCENT_DEPLOYMENT.md) for the supplied deployment files and remaining provisioning work.

The private qualification target is **RAILGUN V2 on Arbitrum One with native USDC**. The principal and private recipient are represented by encrypted notes and proofs; the operator sees its own fee and request timing. This flow requires already shielded funds and a private `0zk` recipient. Public shielding, unshielding and public-wallet transactions remain outside that privacy claim.

The previous implementation's offline checks passed genuine one-input and eleven-input proofs, encrypted recipient and fee-note checks, native journal recovery and the packaged JavaScript MessagePort flow. They do not establish operation on Android or a currently deployed mainnet verifier. Production deployment qualification, device testing, independent relay operation, production secret provisioning and funded end-to-end testing remain explicit release requirements. See [PRIVATE_GAS_UPGRADE.md](docs/PRIVATE_GAS_UPGRADE.md) and [VERIFICATION.md](docs/VERIFICATION.md).

## Build and deployment documentation

- [Android build and device guidance](android/README.md)
- [Latest release and delivered APK status](docs/RELEASE_0.13.0.md)
- [Signed APK identity](docs/APK_MANIFEST.json)
- [Private engine build and proof verification](privacy-engine/README.md)
- [Operator service](gas-service/README.md)
- [Tencent deployment](docs/TENCENT_DEPLOYMENT.md)
- [Network and asset matrix](docs/NETWORK_EXPANSION.md)

Wallet recovery phrases, user vaults, app-signing credentials and operator treasury keys do not belong in this repository. The immutable protocol/domain identifiers retain their established values; renaming the display does not change a user's keys or recovery format.

## Earlier implementation and protocol notes

The sections below preserve the earlier source/runtime history. Current build and delivery status is recorded in the release evidence linked above; historical version numbers are not claims of a newly delivered APK.

## Retained 0.10.0 network functionality

**A non-custodial wallet for real mainnet assets.** The Android 0.10.0 source supports nine reviewed networks: Ethereum, Base, OP Mainnet, Arbitrum One, Avalanche C-Chain, Polygon PoS, BNB Smart Chain, Solana and TRON. Wallet creation and transfer signing remain local. The deployed web 0.6.0 runtime remains limited to Ethereum and Base. See [NETWORK_EXPANSION.md](docs/NETWORK_EXPANSION.md) for the exact asset matrix, derivation paths, fee semantics and limitations.

**Historical 0.10.0 status:** that version was a source candidate because the Android toolchain could not be downloaded in the earlier workspace. At that checkpoint, 0.8.0 was the last verified APK. Current binary status and checks are recorded in [RELEASE_0.13.0.md](docs/RELEASE_0.13.0.md) and `docs/APK_MANIFEST.json`.

This is an engineering build with security controls and automated verification, not an independently audited production wallet. See [VERIFICATION.md](docs/VERIFICATION.md) for exact tests, build results and validation gaps. No real funds were transferred during development.

## Start using it

The existing hosted web wallet keeps its original private deployment. Its URL and deployment identifier are intentionally not part of this public source distribution. Use the previously supplied URL for that installation, or build the standalone web client below for your own HTTPS origin.

The project source includes both clients. Android 0.10.0 adds the five remaining requested networks to the native app only; the hosted web client remains at 0.6.0 and was not deployed by this update.

### Android

**Retained public wallet:** **DEPOSIT** and **SEND** each start with one entry per supported asset. The next network choice lists only supported native representations. Send retains **Asset → Address → Network → Amount**, Paste, exact Max, inline fees and a final review. Deposit requires a network before displaying the correct address and QR. Solana and TRON derive distinct addresses from the same recovery phrase; EVM networks share the existing EVM address. Network changes do not bridge assets. All supported choices remain available even when home balances are zero or hidden.

One portfolio displays positive holdings across the nine Android networks, with a USD estimate from Coinbase's public spot-price API. Each position names its network, such as **USDC (Base)** and **USDC (Arbitrum One)**. Observed zero balances are omitted; unavailable balance or price data is never silently replaced with zero. Optional **Hide small balances** is off by default and uses a user-set USD threshold, initially $1. It changes only home-row visibility, not portfolio value, history or transfer availability. Holdings without a fresh valuation remain visible. The two primary actions remain **DEPOSIT** and **SEND**.

The Profile page contains the wallet name, public address and Settings entry. Renaming, display preferences, app lock and recovery controls are inside Settings; there is no account nickname to manage. Creation opens the saved wallet immediately and shows a confirmation pointing to **Profile → Settings → Recovery phrase**. Backup is no longer an onboarding gate. Words are concealed until an explicit reveal. Switching apps returns to the same recovery page with the words concealed; an enabled app lock still applies. The multiline import field starts at the top of its text box.

The app keeps the 0.5.1 local journal path fix, existing signing identity and encrypted wallet data. The native app connects directly to approved public RPC providers; it does not require your own hosted site, server or domain. TRON uses TronGrid; a user-owned access key can be stored, encrypted locally, under **Profile → Settings → TRON connection**. Unauthenticated access can be throttled or rejected.

The earlier signed `local-wallet-0.8.0-mainnet.apk` supported Android 11 or newer, ARM64 phones and x86-64 devices; it did not include the new networks. The current 0.13.0 source targets the same devices and package, with version code 16. A release APK must pass the full native/Android build, tests, lint, signature and alignment gates before being supplied as an update. New wallets do not require an app PIN or device screen lock. Optional app protection is in **Profile → Settings**: Off, a fixed 6-digit app PIN, or biometrics/device screen lock. Existing protected wallets retain their authentication until explicitly changed. Updates must use the existing signing identity, without uninstalling or clearing data.

1. Create a wallet or restore English BIP39 recovery words.
2. Open **Profile → Settings → Recovery phrase** when ready to record the newly generated 24 words. Copy and screenshots are allowed while the words are explicitly revealed. There is no mandatory word quiz. A clipboard or image copy can expose the wallet through history, photo backups or sync. The operator receives no recovery phrase or private key and cannot recover lost keys.
3. Tap **DEPOSIT**, choose an asset, then choose its network from the dropdown. Match that exact network when withdrawing from an exchange or sending from another wallet. The QR and receiving screen retain that network.
4. Obtain the selected network’s gas asset: ETH, SOL, TRX, AVAX, POL or BNB. A stablecoin does not pay the network fee in this version.
5. In **SEND**, enter or paste the recipient, select the network, then enter an amount or use **Max**. Exact integer units are retained: six decimals for the supported stablecoins/TRX, nine for SOL, eighteen for EVM native assets. Token Max uses the full spendable token balance; native Max subtracts the disclosed fee reserve. Solana may require associated-token-account rent; TRON quotes bandwidth, energy and activation costs. Review shows the exact recipient amount and fee reserve. Confirm using the selected optional app lock. Submission and final confirmation remain separate states.

Android Keystore encrypts the vault key in every mode. The device-authentication mode binds each use to an Android authentication prompt. The recovery words are the portable backup. Clearing application data, uninstalling, losing the device or invalidating its Keystore key can require recovery from those words.

### Web

Version 0.6.0 uses a warm off-white interface, quiet typography, a single six-position asset list and two primary actions. There is no global network switch. Choosing Deposit or Send opens an asset-and-network picker; the selected network remains explicit through the QR, fee estimate and transfer review. Wallet and account names are editable from Profile and stored as local display labels separately from signing identity.

The large **Total USDC** value is the exact USDC quantity across Base and Ethereum. It is not a combined USD valuation of ETH, EURC and USDC, nor an amount that can all be spent on one network. Both network observations are required; unavailable data is never treated as zero, and saved observations are identified. Balances and transfer rows retain their individual networks.

Background reads update retained balance views without rebuilding the screen, moving scroll or replacing form fields. A short original opening transition plays once; the web version respects reduced-motion preferences. Profile → Settings retains optional app lock and recovery controls. Interrupted recovery verification resumes the already-saved wallet. No trading, swaps, staking, earn products, bridges or dApp connections are added.

The Rust signing policy, entropy-vault format, account derivation, payment journal and network transports are preserved. Versioned native/web protection metadata supports optional app lock and atomic migration of existing wallets. Version 0.3.2's hosted relay correction is retained: Worker requests use `redirect: "manual"` and reject redirects without following them. Approved direct providers remain available for read-only queries after specific relay availability failures; signing and submission do not use this fallback.

Open the delivered HTTPS wallet URL in a current browser with WebAssembly, Web Workers, Web Crypto and IndexedDB strict durability. The site is private to its owner by default. Site sign-in controls access to the page; it is not wallet custody or recovery.

Create or import a wallet without choosing a password. A random vault credential is encrypted under a non-extractable WebCrypto key stored in IndexedDB. Profile → Settings optionally adds a **6-digit app PIN**. Browser biometrics is explicitly unavailable in this release; Android provides device authentication. Existing password-protected web wallets retain their old password until an authenticated settings change re-encrypts the same account. App lock is separate from a BIP39 passphrase. Clearing browser data, changing the site origin or using a different browser does not carry over the vault or history; recovery words remain necessary.

The hosted update preserves the existing origin and local vault. Reload the page normally; do not clear browser data. The supplied web bundle can also be served from your own HTTPS origin, or localhost for local development. Do not open `index.html` directly with a `file:` URL. No backend service is required for wallet secrets or signing. The hosted site has a restricted same-origin RPC relay; the standalone bundle uses fixed public RPC endpoints directly.

## Networks and assets

| Network | Identity | Supported assets | Fees paid in |
| --- | --- | --- | --- |
| Ethereum | EVM 1 | ETH, USDC, EURC, USDT | ETH |
| Base | EVM 8453 | ETH, USDC, EURC | ETH |
| OP Mainnet | EVM 10 | ETH, USDC | ETH |
| Arbitrum One | EVM 42161 | ETH, USDC | ETH |
| Avalanche C-Chain | EVM 43114 | AVAX, USDC, EURC, USDT | AVAX |
| Polygon PoS | EVM 137 | POL, native USDC | POL |
| BNB Smart Chain | EVM 56 | BNB | BNB |
| Solana | Fixed mainnet genesis | SOL, USDC, EURC, USDT | SOL |
| TRON | Mainnet 0x2b6653dc | TRX, USDT-TRC20 | TRX |

Canonical token identities are pinned to [Circle USDC](https://developers.circle.com/stablecoins/usdc-contract-addresses), [Circle EURC](https://developers.circle.com/stablecoins/eurc-contract-addresses), and [Tether’s protocol registry](https://tether.to/en/supported-protocols/), checked 2026-11-06. BNB Smart Chain’s Binance-Peg stablecoins are not silently treated as issuer-native tokens. The Android registry contains 24 asset/network positions grouped into nine asset choices. See [the expansion specification](docs/NETWORK_EXPANSION.md).

## What is implemented

- Local 24-word wallet creation; recovery of valid 12/15/18/21/24-word English BIP39 phrases using the first account and an empty BIP39 passphrase.
- Encrypted entropy vault and address verification after decryption. Android creation verifies its durable manifest before reporting success; its backup state remains advisory and is not silently marked confirmed.
- A unified portfolio with 24 supported asset/network positions in Android source, and six in the delivered web runtime; Android lists positive holdings and optionally hides small balances. Deposit/Send retain explicit asset-and-network selection, a network-labelled receiving address and locally generated QR.
- Device-local wallet names, independent of addresses, derivation, vaults and payment records. Android has no account nickname in the UI; web 0.6.0 retains its prior two-label presentation.
- Exact integer balances for supported assets from identified chain observations; unavailable or stale data is not reported as a fresh zero balance.
- Android-only USD estimates from fixed Coinbase public spot-price URLs for supported symbols, when the pair is available. No Coinbase login or API key is needed; the requests contain currency pairs, not wallet addresses or holdings. Exact decimal multiplication is used for valuation and filtering; rounded UI values never enter transactions. Public RPC providers still see public address queries, and network providers receive ordinary connection metadata.
- Automatic balance reads, visible connection/freshness states and preserved last successful observations when a read fails.
- Read-only transfer-cost estimates before review, with the selected native fee currency. EVM supports conservative unfunded previews. Solana/TRON preparation currently requires affordable funds; a gas-deposit shortcut is available when the fee cannot yet be quoted.
- Restricted EVM native/ERC-20, Solana SOL/classic-SPL and TRON TRX/USDT transfers, preflight checks, immutable review, explicit final approval and fresh selected-lock authentication when enabled.
- Durable intent and signed-byte storage before broadcast, explicit uncertain outcomes and identical-byte rebroadcast.
- Receipt checks, expected token Transfer-event checks, fee accounting, inclusion/finality and reorganization corrections.
- Local outgoing activity plus checked incoming transfers for the supported native assets and canonical tokens. Paste a transaction ID and choose its network to verify a specific deposit. Each family has the history-coverage limits described below.

## Incoming activity and history coverage

Solana/TRON use bounded recent-ID discovery followed by core-validated transaction/block evidence; earlier transfers can be tracked by ID. The following EVM coverage description remains applicable to the seven EVM networks. Full limits are in [NETWORK_EXPANSION.md](docs/NETWORK_EXPANSION.md).

Balances come from chain state and do not depend on transaction-history discovery. EVM activity discovery scans up to the latest 8,000 blocks for supported canonical-token Transfer events, with at most 24 candidate transactions validated per refresh. On Base this covers roughly 4.4 hours at a two-second block interval; Ethereum covers a longer period. Subsequent scans overlap the saved cursor. This is bounded recent history, not a complete index from account creation.

Direct EVM-native-asset discovery inspects only the latest eight full blocks. Older direct deposits can be verified by transaction hash. Internal EVM-native transfers require a trace/indexing service and are not inferred from balance changes. The UI explicitly identifies partial or unavailable history. A missing history entry never proves that a payment failed.

Each displayed chain transfer is reconstructed from matching transaction, receipt, supported-contract log and canonical-block evidence by the shared Rust validator. Cached projections are checked against their saved evidence. Reorganizations append corrections and retain earlier observations. RPC responses remain provider claims, not cryptographic consensus proofs.

The hosted relay optionally supports a server-held Blockscout API key for candidate-hash discovery. No key or paid history service is configured in the delivered build. Recent scans and transaction-hash checks work without it.

## Fee handling

Ethereum's execution fee cap is `gas limit × maxFeePerGas`. Actual execution fees use receipt gas used and effective gas price.

Base and OP Mainnet also have L1 data fees and operator fees. The wallet queries the fixed GasPriceOracle on the selected network, includes those estimates and a local buffer in the review, rechecks prerequisites before signing, and accounts for all components from the receipt and canonical-block oracle evidence. **These total estimates and buffers are not enforceable on-chain maxima.** See the [Base fee documentation](https://docs.base.org/specifications/transactions/network-fees) and [OP Mainnet fee documentation](https://docs.optimism.io/op-stack/transactions/fees).

Arbitrum One's `eth_estimateGas` includes L1 posting costs converted to L2 gas. Its receipt fee is `gasUsed × effectiveGasPrice`; the app does not add an OP-style L1/operator fee again. An unavailable Arbitrum ETH simulation never becomes an assumed 21,000-gas total. The existing 300,000-gas policy ceiling remains: a transfer requiring more is blocked, including one made expensive by posting costs. See [Arbitrum gas estimation](https://docs.arbitrum.io/arbitrum-essentials/how-to-estimate-gas).

The original ETH networks limit gas to 21,000–300,000, fee per gas to 500 gwei, and execution reserve to 0.05 ETH. New networks use explicit native-currency policies described in NETWORK_EXPANSION.md. Additional review-budget limits are enforced in the core. A review expires after five minutes. Already signed transactions do not expire when the review expires. Fee previews never reserve a nonce, sign, or submit. A conservative preview does not bypass the successful-simulation and sufficient-funds requirements for actual review/signing.

## Transaction uncertainty

A timeout does not prove failure. The app saves signed bytes before submission, so a restart can recover the exact hash and query its receipt. It does not automatically sign a new transaction, change the nonce, or duplicate a payment to resolve a timeout. An explicit retry resubmits the identical signed bytes.

Only an unsigned draft can be cancelled locally. One unresolved outgoing payment is allowed per network. A payment is not marked complete merely because a provider accepts its submission. Transaction execution, block inclusion and provider-reported finality remain distinct.

## Build from source

### Shared core

Install Rustup, Node/npm and Python 3. The project pins Rust 1.99.1 and exact dependencies in lockfiles.

```bash
cargo test --locked --all-targets
npm ci --ignore-scripts --no-fund --no-audit
npm run test:differential
./scripts/verify-bridge.sh
```

### Web

```bash
./scripts/build-wasm.sh
cd web
npm ci --ignore-scripts --no-fund --no-audit
npm run typecheck
npm test
npm run test:wasm
npm run build
```

Serve `web/dist/` as static content over HTTPS. Build/test dependencies are separate from the locally bundled browser runtime. No CDN scripts or analytics are included.

Build the embedded-asset Worker and its restricted same-origin relay from the repository root:

```bash
npm run build
npm run test:relay
./web/node_modules/.bin/tsc -p worker/tsconfig.json
node --test scripts/hosting-manifest.test.mjs
node scripts/verify-hosted.mjs
```

The hosted build selects metadata in this order:

1. An explicitly supplied `ZERO_HOSTING_MANIFEST` JSON file. Relative paths resolve from the repository root.
2. An existing local `.openai/hosting.json`, preserving that installation's deployment identifier and settings unchanged.
3. [`config/hosting.example.json`](config/hosting.example.json), an identifier-free Worker template for a clean checkout.

An invalid existing manifest or a missing explicit file fails the build; it is never replaced silently with the template. `.openai/` is local deployment configuration and is excluded from the public source. The checked-in template has no Site identifier, credentials, database or object-storage binding. It permits building and inspecting the Worker without selecting or publishing a deployment.

For an already configured target outside this checkout, use the same explicit manifest for the build and its verification:

```bash
export ZERO_HOSTING_MANIFEST=/absolute/path/to/your/hosting.json
npm run build
node scripts/verify-hosted.mjs
```

Use the real existing `project_id` when targeting an existing ChatGPT Site; preserve the current origin when updating a wallet installation. A static manifest is rejected because this artifact includes the RPC Worker. Creating a hosting project and deploying the artifact are separate operations; these commands do neither. The host verification writes a sanitized result to `build/verification/` without publishing the deployment identifier.

Also run the native Cloudflare runtime gate against the built artifact using the pinned official `workerd@1.20261104.1` binary:

```bash
node worker/runtime-tests/verify.mjs /absolute/path/to/workerd /absolute/path/to/local-wallet/dist/server/index.js
```

This gate uses controlled outbound HTTP responses after real Worker Request/fetch validation. It catches runtime incompatibilities that Node and browser tests cannot establish. It is separate from live-provider availability and authenticated production checks.

The hosted artifact is `dist/server/index.js` with `dist/.openai/hosting.json`. Its relay accepts a fixed method/contract/network allowlist, verifies provider chain IDs, limits payload sizes and read failover, and makes exactly one attempt for a signed submission. A failed submission never causes automatic direct fallback. No seed, vault, password, private key, or signing API is sent to the server.

### Android

Install JDK 17, Android SDK platform 35/build-tools 35.0.0, NDK 27.2.12479018, and Gradle 8.11.1. Set `JAVA_HOME` and `ANDROID_HOME` to your local installations, then run:

```bash
./scripts/build-android.sh
```

See [android/README.md](android/README.md) for prerequisites and signing. Build Rust libraries and generated Kotlin bindings from the same source revision. The supplied APK has a unique development signing certificate. Rebuilds for distribution must use your controlled signing identity; app signing keys and wallet recovery keys are separate.

## Security boundaries

- **Android:** a random vault key is always wrapped by Android Keystore. Optional device authentication binds each use; optional 6-digit PIN adds a derived-key envelope and durable attempt limits. Recovery-phrase screens allow screenshots at the user’s request; other screens retain `FLAG_SECURE`. Secret views are cleared when the app backgrounds. Wallet files remain excluded from app backup/device transfer.
- **Web:** new vaults use a random 256-bit credential encrypted with a non-extractable WebCrypto key. Optional 6-digit PIN adds an Argon2id (64 MiB, 3 passes) AES-GCM inner envelope and persisted attempt limits. Vault and wrapper changes commit atomically. Secret operations run in temporary workers. App-lock-off permits anyone using the browser profile to access the wallet; non-extractability is not hardware isolation.
- **Shared core:** AES-256-GCM vault encryption, standard BIP39/BIP32/SLIP-0010, secp256k1 and Ed25519 libraries, checked integer arithmetic and a restricted signing API.
- **Persistence:** SQLite on Android; strict-durability IndexedDB transactions on web. Signed bytes must commit before submission. Storage deletion still loses local history.
- **Network:** public HTTPS RPC providers supply chain observations. Chain ID and canonical-block checks detect inconsistencies; they are not consensus proofs and do not remove provider trust.

A compromised OS, malicious application update, malicious same-origin web code or browser extension can defeat a software wallet. Rust and a worker do not create an independent trusted display. Browser hosting and browser code remain part of the wallet's security boundary. See [SECURITY.md](docs/SECURITY.md).

## Outside this version

No iOS, testnet, arbitrary token import, token approvals, permit, arbitrary message signing, dApp connection, smart accounts, delegation, hardware signers, swaps, bridges, staking, NFTs, cloud seed backup or custodial recovery. No automatic fee replacement, full historical account index or non-default derivation-path discovery.

No merchant liabilities, payout ledger, chargebacks or bank/acquirer reconciliation. Applicable correctness controls are mapped in [REQUIREMENTS.md](docs/REQUIREMENTS.md). Third-party licensing is in [THIRD_PARTY.md](docs/THIRD_PARTY.md). No distribution license for the original application has been selected by the owner.
