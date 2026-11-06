# Local Wallet web 0.5.0

A functional, non-custodial software wallet for **Ethereum mainnet (chain 1)** and **Base mainnet (chain 8453)**. It receives, holds and sends native ETH and the reviewed Circle-issued USDC and EURC contracts. There is no testnet mode and no custody backend. Mainnet transaction construction and submission are enabled.

This is an implemented development release, **not an independently audited wallet**. Automated tests use public fixture keys, local databases and mocked RPC responses. They do not move real funds or prove that every live RPC method is available from every deployment region.

## Build and serve

The repository root owns the shared Rust core and its WASM build:

```sh
./scripts/build-wasm.sh
cd web
npm ci --ignore-scripts
npm run typecheck
npm test
npm run test:wasm
npm run build
```

`npm run build` copies the **actual release Rust WASM** from `../build/wasm`, bundles the TypeScript UI and worker with pinned esbuild, and writes `web/dist`. The root `scripts/build-web.sh` also stages the output for the Site. Serve the resulting files over HTTPS or a local development origin accepted as a secure context. Opening `index.html` directly with `file://` is unsupported.

The standalone output is static. The root `npm run build` additionally embeds it in a Worker with a restricted same-origin RPC relay for the existing hosted Site. No JavaScript, font, image or cryptographic library is loaded from a third-party CDN. Standalone connections use approved mainnet RPC endpoints directly; hosted connections use the relay, which can fail over public reads between fixed providers. `_headers` contains recommended deployment headers for static hosting; the host must actually apply them. The Worker applies its security headers directly. A restrictive CSP also appears in the HTML, and the app refuses to operate in an iframe. `build-info.json` records SHA-256 hashes of the built JS, CSS and WASM. The HTML pins the app and stylesheet with SRI. These checks detect accidental asset mismatches; they do not protect against an attacker able to replace the entire deployed application.

## Use

1. Choose **Create a wallet** and save the 24 recovery words. No app passcode is required. The encrypted vault and non-extractable device protection key commit together before words are displayed; interruption resumes the same wallet. Confirm three randomly selected words to complete onboarding.
2. Alternatively, use **I have recovery words**. Restore accepts standard English BIP39 word counts. This version uses the first Ethereum account at `m/44'/60'/0'/0/0` and an empty optional BIP39 passphrase. It does not discover other derivation paths or account indexes.
3. Select **Base** or **Ethereum** in the header. The address is the same account on both networks, but balances, nonces and local history are separate. Selecting a network does not bridge funds.
4. **Receive** displays the full public address, selected network, token contract where relevant, and an EIP-681 QR request that includes the chain ID. Share the public address and the exact network. Never share the recovery words.
5. Balances refresh automatically every 20 seconds while unlocked, or through **Refresh**. ETH, USDC and EURC are read at one canonical block context. These are RPC observations, not amounts inferred from local transaction history. Failed reads preserve the last saved observation with a stale label.
6. **Send** first shows a read-only **Cost to transfer** estimate and any ETH shortfall. Choosing **Review transfer** creates a durable unsigned preparation only after chain, balance, nonce, contract, account-code and transfer-simulation checks. Review the full recipient, asset, contract, amount, chain and fees. Confirm the transfer and enter the selected 6-digit app PIN or legacy password if app lock is enabled. Refreshes do not replace recipient, amount or credential fields.
7. **Activity** combines local outgoing intents with receipt-validated incoming and external-wallet activity. Recent USDC/EURC discovery scans at most 8,000 blocks and 24 candidates; direct ETH fallback examines eight blocks. Use **Check transaction** with a public transaction hash for older or missing direct deposits. Internal ETH transfers and a complete historical index are outside this build. Partial coverage is explicit and does not mean a deposit failed.
8. Reconcile outgoing outcomes using receipts. A timeout does not imply failure. A retry sends the exact saved bytes and the same nonce; it never signs a new payment. A failed relay submission never falls back automatically to a second send path.

USDC and EURC transfers need ETH for gas **on the selected network**. There is no paymaster or fee sponsor. There is no fiat price feed, so the app never guesses a fiat value for ETH or fees.

Profile → Settings contains **App lock** (Off / 6-digit app PIN) and recovery controls. Browser biometrics is explicitly unavailable in this release. Existing wallet passwords are labelled separately from the 6-digit PIN.

## Key boundary

- The shared Rust core generates entropy, implements the restricted transfer policy using cryptographic libraries, derives the account, encrypts the entropy vault with AES-256-GCM, constructs type-2 transactions, signs, and independently inspects the result.
- New wallets use a random 256-bit core credential, wrapped by a non-extractable WebCrypto AES-GCM key in IndexedDB. An optional exactly-6-digit PIN adds an inner AES-GCM envelope derived with **Argon2id, 64 MiB, 3 passes, one lane** and fresh salt. Wrong attempts are reserved durably before decrypting; delays persist across reload. Existing password wallets remain unchanged until explicit authenticated migration rekeys the same seed under a random credential.
- Each operation runs in a fresh Web Worker. A secret operation derives the key, authenticates the vault address against its saved public address, performs the restricted operation, clears byte arrays and terminates the worker. No unlocked key is cached.
- The recovery screen allows selection, copying and screenshots. It warns about clipboard history, cloud photo sync and other copies. Hiding the tab or two minutes of recovery-screen inactivity clears secret UI state and terminates workers. A selected PIN/legacy lock authenticates protected actions; Off needs no app credential. Cross-tab operation checks bind to the saved protection policy, with BroadcastChannel used only for prompt UI cancellation.
- JavaScript strings cannot be reliably overwritten; browser and OS memory protection remain trusted. Worker termination and zeroization limit exposure, but do not defend against a compromised browser, extension, running device or served application.
- The app stores an encrypted vault, its public account metadata, signed transaction bytes, local intent records and public chain observations in IndexedDB. Plaintext recovery words and core credentials are not persisted. A non-extractable CryptoKey is persisted through browser structured cloning, together with its authenticated wrapper. This is not a hardware-storage guarantee; a compromised browser/profile remains a trust boundary.

Browser storage is origin-specific and can be cleared or evicted. The app requests persistence, but a browser may refuse. Changing the web origin does not migrate its vault or history. Recovery words restore control of the assets; they do not restore deleted local transaction records. Avoid sending concurrently from other wallet software with the same recovery words.

## Transaction correctness

The browser uses the same Rust `validate_prepared_json`, `validate_signed_for_prepared` and `validate_receipt` functions as the native integration. Its IndexedDB journal supplies browser-specific storage:

- An atomic unique nonce reservation is scoped by chain and account; a second unresolved intent is rejected. Only an unsigned cancellation releases that reservation. Signed bytes and historical events are retained.
- Writes request `durability: "strict"`, require browser support for that hint, and wait for the transaction's **complete** event. A successful individual request is not a durable commit. Signing is followed by commit and validated readback **before** the network receives the bytes. Strict durability is a browser hint, not a guarantee against user deletion, hardware loss or browser bugs.
- All monetary quantities use Rust integers or JavaScript `BigInt`. Decimal input is strict; floating-point conversion is not used for value or fees.
- RPC responses are bounded to 1 MiB, 32 levels and 50,000 JSON nodes. Duplicate object keys, including escaped duplicates, are rejected before JSON normalization. Requests use HTTPS, no referrer and no redirects. Direct provider requests omit cookies. Same-origin relay requests retain the private Site session; upstream requests never forward those cookies. A wrong chain or inconsistent result fails closed.
- A receipt is tied to the signed hash, sender, actual transaction destination, canonical block, execution status and gas limits. A successful token receipt must contain exactly the expected token transfer. Inclusion, execution success and finality are distinct.
- Raw receipt evidence, canonical hash and observed finality height are saved. Every loaded receipt is revalidated through Rust and compared to its stored projection. Evidence conflicts cannot silently overwrite history. A delayed response cannot downgrade an already finalized observation.
- The receipt's canonical block is rechecked after the finalized-block lookup. A conflicting new receipt cannot invalidate a still-canonical earlier inclusion. A missing receipt is not proof of failure or a reorganization. Reorg corrections append evidence and preserve signed bytes. Loss of previously observed finality blocks further sending for investigation.
- Balances are appended as block-scoped observations. Repeated older observations never become the latest balance, and different amounts for one block hash are rejected. Latest projections must match a saved observation.

### Base fees

The execution fee cap is `gas_limit × max_fee_per_gas`. It does **not** cap Base's additional L1 data or operator charges. Before review, the client calls the fixed Base GasPriceOracle at `0x420000000000000000000000000000000000000F`:

- `getL1FeeUpperBound(1024)` for an L1 estimate that covers the core's accepted maximum transaction byte size.
- `getOperatorFee(gas_limit)` for the operator estimate.

The reviewed budget is the execution cap plus twice the estimated additional fees. The network is rechecked before signing; a quote outside that budget requires a fresh review. The budget is **not an on-chain cap** on the additional charges.

Actual fee accounting includes execution + L1 + operator fees. The receipt must supply its L1 fee. The operator fee is queried at the receipt's canonical block hash and cross-checked against a receipt operator field if one is present. Missing fee evidence is not silently treated as zero. If actual additional fees exceed an estimate, the actual fees and payment outcome are retained.

## Verification and limits

The 0.3.2 update corrects server-side redirect handling for Cloudflare Workers and adds a real-workerd release gate. It also permits fixed-provider read-only fallback after explicit relay availability statuses, with shared bounded cooldown. It does not automatically retry or reroute transaction submission. See the repository verification report for the production failure evidence, runtime reproduction and verification limits.

The 0.3.1 hotfix corrects a native browser `fetch` receiver error affecting RPC and history requests. Node fetch and injected arrow-function mocks masked the failure in the 0.3 checks. The added tests reject an unrelated receiver, exercise the default native-fetch path, and model the Fetch Standard's Origin behavior for the real relay admission checks. Browser checks are recorded separately from Node/in-process checks in `docs/VERIFICATION.md` at the repository root.

Tests cover concurrent intent reservation, chain separation, preserved signed bytes, commit aborts, uncertain submissions, duplicate prevention, exact-byte retries, exact amounts, hostile RPC JSON, wrong chains, Base fee calculations, receipt corruption, gas-charged reverts, missing token transfer logs, delayed finality observations, conflicting inclusion evidence and reorganization races. Integration tests exercise the actual release WASM, public BIP39 fixtures, password-derived vaults, generated 24-word phrases and both mainnet transaction formats.

The public fixture phrases in `tests/` are deliberately public. Never fund them. Runtime tests are not a security audit, a formal proof, a physical browser-storage durability test, or a live-value transfer test. RPC providers are trusted for chain observations; this version does not run a light client, obtain independent consensus proofs, or use a provider quorum. Provider outages, CORS limitations or rate limits block affected reads/sends without changing the transaction into a false failure.

This version excludes arbitrary signing, dApp connections, approvals, swapping, bridging, delegated/contract senders, account abstraction, nonce replacement, automatic cancellation, a full historical/internal-transfer index and cloud recovery. Those exclusions keep the operation set small; they are not claims that a software wallet is secure against a compromised host.

## Primary technical references

- Circle native USDC contracts: https://developers.circle.com/stablecoins/usdc-contract-addresses
- Circle native EURC contracts: https://developers.circle.com/stablecoins/eurc-contract-addresses
- Base RPC/sequencer configuration: https://docs.base.org/get-started/connect-to-base
- Base fee oracle and total fee components: https://docs.base.org/specifications/transactions/network-fees
- IndexedDB transaction durability semantics: https://www.w3.org/TR/IndexedDB-3/

The deployed build still needs independent security review and real-device/browser operational validation before being entrusted with meaningful funds.
