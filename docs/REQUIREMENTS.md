# Scope and requirements coverage — Local Wallet 0.3

## Current product

The product is an Android application and a web application with a shared Rust cryptographic/validation core. It enables real **Ethereum mainnet (1)** and **Base mainnet (8453)** for ETH, Circle USDC and Circle EURC. Other networks and tokens are rejected. The same standard recovery secret derives the same address on both supported networks; network selection changes assets, balances, nonce scope, fees and journal context. Switching networks does not bridge funds.

These are implemented capabilities and source-level controls, not a claim of independent audit, physical device validation or a completed funded mainnet exercise. [VERIFICATION.md](VERIFICATION.md) records actual checks and limitations; [SECURITY.md](SECURITY.md) explains the trust model.

## User capabilities

| Capability | Version 0.3 implementation |
| --- | --- |
| Create | Locally generate 256-bit entropy and 24 English BIP39 recovery words; encrypt the entropy in an AES-256-GCM vault |
| Restore | Validate standard English BIP39 phrase/checksum; derive first Ethereum account with empty BIP39 passphrase |
| Receive | Show the account address and selected network/asset; observe actual on-chain token/native balances |
| Hold | Signing authority remains with the recovery secret; no custodial liability or server balance |
| Send | Exact ETH or supported ERC-20 transfer; fixed network/token registry; explicit immutable review; fresh authentication; restricted Rust signing |
| Android protection | Random vault key wrapped with an authentication-per-operation Android Keystore key; no-backup app storage; native SQLite journal |
| Browser protection | Password-derived Argon2id vault key; crypto in temporary WASM worker; strict-durability IndexedDB journal; hosting/browser trust remains |
| Mainnet selection | Ethereum and Base enabled; mainnet chain ID is bound into every signature and local record |
| History | Local outgoing payments plus verified direct ETH and allowlisted-token activity; bounded recent discovery and manual transaction-hash lookup; coverage remains explicitly partial |
| Fee preview | Read-only cost estimate before durable preparation, including labeled conservative estimates for unfunded drafts; complete preparation/signing still require simulation and affordability |
| Availability | Nonblocking balance updates preserve a clearly marked saved observation after read failure; unavailable is distinct from zero |
| Interrupted submission | Reopen saved intent/signature, query receipts, preserve uncertainty and retry only identical bytes when eligible |
| Recovery portability | Words recreate the standard account; local journal history and device authentication settings are not recovered from words |

USDC and EURC use six decimal places; ETH uses eighteen. The core registry contains Circle's network-specific native contract addresses. No remote token metadata can expand signing permissions. ETH on the selected network is required for gas, including stablecoin transfers.

## Adaptation of the requested processing controls

| Requested control | Non-custodial implementation or exclusion |
| --- | --- |
| Processing APIs and provider connectors | Fixed HTTPS Ethereum/Base JSON-RPC; hosted web uses a restricted same-origin relay, static web and Android use direct endpoints; no card, bank or acquirer connector |
| Routing and orchestration | Explicit network selection; one unresolved outgoing payment per account/network; no automatic route, swap or bridge |
| Provider authentication/rotation | Public RPC endpoints require no embedded provider secret; vault credentials are local, unrelated to provider custody |
| Rate limits, outages and API errors | Bounded requests/responses and timeouts; failed prerequisite reads block sends; uncertain broadcast is retained for reconciliation |
| Payment-method selection | ETH, USDC or EURC on a selected supported mainnet |
| Durable state transitions | Native SQLite transactions; browser strict-durability IndexedDB transactions; validated current projection and event history |
| Idempotency and duplicates | Intent identity, immutable preparation/signature, chain/account/nonce reservation, atomic concurrent writes |
| Safe retries | Reads may fail over only to fixed, chain-verified providers. Broadcast has one path/attempt; explicit retry reconciles and submits exactly the saved signed transaction and hash |
| Uncertain outcomes | Unknown/pending submission is separate from successful/failed execution and included/finalized chain state |
| Delayed, duplicated, missing or reordered notifications | No webhook dependency; receipt polling, idempotent evidence matching, monotonic submission/finality and explicit conflicts |
| Cancellation races | Only an unsigned draft can be cancelled; its nonce reservation is released atomically. A signed payment cannot be cancelled by erasing local data |
| Partial completion | Status, expected ERC-20 event, fee charge and inclusion/finality are separate; success without the expected transfer remains unresolved |
| Reversal/refund semantics | No automatic financial reversal, refund API or chargeback. A deliberate return of assets is a new reviewed transaction |
| Timeouts and degraded/inconsistent providers | Preserve signed bytes and existing evidence; no inferred failure, invented zero balance, replacement nonce or automatic duplicate transfer |
| Audit and controlled replay | Immutable evidence through the application API; explicit reorg correction; exports omit secrets and raw signed bytes |
| Intent/authorization/execution/finality separation | Preparation → reviewed authorization → durable signature → submission → receipt execution/inclusion → provider-observed finality |
| Merchant reporting and operations | Excluded; this is a personal non-custodial wallet with local payment evidence |

A timeout does not prove failure. A successful `eth_sendRawTransaction` response is an acknowledgement, not proof of execution, confirmation or finality. Receipt identity and expected token transfer must be checked, and absence of a receipt is unresolved unless supported by further evidence. Reconciliation never creates a new signed payment to make uncertainty disappear.

## Financial records and reconciliation

| Requested accounting/correctness control | Application here |
| --- | --- |
| Authoritative orders/executions/positions/balances/obligations | No orders, trading positions or customer obligations. Blockchain observations supply balances and transfer evidence; local intents supply the user's recorded instruction |
| Financial traceability | Outgoing: intent → immutable preparation/approval digest → signed bytes/hash → receipt/canonical block/finality. Incoming: candidate hash → matched transaction/receipt/canonical block → verified token/native event → retained observation |
| Durable history and corrections | Append observations and reorg corrections; preserve previous evidence; reject conflicting receipt/snapshot replacement |
| Double-entry accounting | Excluded: no custodied customer balance, internal money movement, merchant payable or liability ledger exists |
| Gross amount | Exact positive asset units stored as integer strings; no floating-point financial arithmetic |
| Ethereum fee | Receipt gas used × effective gas price; execution cap from signed gas limit × maximum fee per gas |
| Base fee | Execution charge + actual L1 fee + actual operator fee; three components retained separately |
| Base review estimate/reserve | Estimate and two-times-additional-fee budgeting buffer are bound into review. They are not protocol-enforced total fee caps |
| Charges above fee estimate | Preserve actual payment outcome and charge; flag additional-fee discrepancy instead of discarding receipt or inventing failure |
| Commissions/reserves/chargebacks/merchant settlement adjustments | Excluded; not provided by this wallet |
| Reconciliation source | Supported blockchain RPC observations tied to blocks; no bank, acquirer, broker, exchange or clearing statement |
| Missing/duplicated/delayed/mismatched observations | Idempotent matching, immutable identity checks, explicit unresolved state or error, durable corrective event where justified |
| Balance adjustments | No manual-credit or balance-setting API. Display comes from validated network/account/block observations, not local debit/credit arithmetic |
| Legal ownership versus local records | Possessing local history does not establish legal title. Signing control, token balance, transfer execution and finality are kept distinct |

The journal is an evidence store, not a consensus verifier. Providers can lie or withhold data. The clients and hosted relay use fixed read-provider failover with a chain check before accepting another endpoint. This improves availability but is not an independently validated provider quorum or light client. A receipt's finality derives from that provider's finalized-block observation. This limitation applies to both platforms.

## Core invariants

- Only canonical type-2 ETH transfers or exact ERC-20 `transfer(address,uint256)` calls to the selected chain's fixed supported token contract.
- No generic signer, caller-supplied arbitrary calldata, approvals, delegate calls or hidden contract interactions.
- Review expires after five minutes; reconstruct and compare all signed terms and fee-review metadata before signing.
- Persist the exact signature and hash before broadcast; no transaction may leave the wallet merely because an individual database request succeeded.
- Unresolved signed payment keeps its nonce reservation. Cancellation only applies before signature.
- Stablecoin receipt must contain the exact expected transfer event from the reviewed token contract; failed execution still retains actual fees.
- Prove the old inclusion noncanonical at its own stored height before a reorg correction. Preserve canonical old evidence when a new provider result conflicts.
- A contradiction of previously finalized evidence stops automatic recovery/rebroadcast and remains visible for investigation.
- Account/network identity is checked on stored records. The browser v0.2 vault/payment database is preserved through an additive activity-store migration; Android incoming observations use a new file. Old pre-mainnet native journal schemas remain rejected/preserved rather than repurposed for mainnet.

## Explicit exclusions and practical limitations

- No iOS deliverable, TRON, TRX, USDT, testnet mode, custom tokens, NFTs, swaps, bridges, staking or dApp connections.
- No smart accounts, EIP-7702 delegation, multisig, external hardware signer, transaction queue, fee replacement or on-chain cancellation.
- No BIP39 passphrase, account-index selection, imported raw private key or automatic discovery of additional accounts.
- No complete historical index or internal ETH tracing. The web scans up to 8,000 recent token-log blocks and eight recent full blocks for direct ETH; Android scans recent supported-token recipient logs. Manual hashes verify older direct ETH or supported-token transactions. Unavailable/partial coverage must not be presented as an empty authoritative account statement.
- No cloud seed backup, hosted recovery or server-side key service. Losing both recovery words and usable local access loses signing access.
- Browser hosting, device/OS, display/input and dependency compromise remain material risks. An encrypted vault and shared Rust code cannot remove them.
- Issuer freezing, contract upgrades, token availability and Base/Ethereum network behavior are outside wallet control.
- Automated tests, compiled artifacts and read-only live mainnet checks do not substitute for physical-device testing, funded send/receive lifecycle validation or independent security review.
