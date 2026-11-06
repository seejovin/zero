# Private gas operator and opaque relay

**Implemented: runnable OHTTP gateway and relay, actual proof-verifier subprocess integration, restricted treasury signing, independent-provider RPC checks, and durable accounting. Production private sponsorship remains disabled pending deployment, circuit, transport and Android release qualification.**

The normal binary can run encrypted read-only service operations. It cannot admit a funded operation, load the treasury key, release a signed transaction or broadcast on mainnet. The core release gate and the separate service integration gate are compiled checks; no environment variable, configuration field, feature or HTTP request bypasses them. The private test harness exists only in `cfg(test)`.

The selected domain is the unreleased Arbitrum native-USDC / RAILGUN V2 candidate in `wallet_core::private_payment`. This work does not turn every supported wallet chain into a private gas network.

## Executables

Run from the repository root with the pinned toolchain. Node and the pinned `privacy-engine` dependencies are required for subprocess integration tests.

```sh
cargo test --locked --offline -p wallet_gas_service
cargo clippy --locked --offline -p wallet_gas_service --all-targets -- -D warnings
cargo run --locked --offline -p wallet_gas_service --bin wallet-gas-service -- capabilities
cargo run --locked --offline -p wallet_gas_service --bin wallet-gas-service -- check-runtime operator.json
cargo run --locked --offline -p wallet_gas_service --bin wallet-gas-service -- gateway-public-key operator.json
cargo run --locked --offline -p wallet_gas_service --bin wallet-gas-service -- serve operator.json
cargo run --locked --offline -p wallet_gas_service --bin wallet-ohttp-relay -- check-config relay.json
cargo run --locked --offline -p wallet_gas_service --bin wallet-ohttp-relay -- serve relay.json
```

| Command | Effect |
| --- | --- |
| `capabilities` | Reports implementations and unmet production gates. |
| `check-policy FILE` | Validates the closed policy schema, domain, exact integer amounts and bounds. |
| `check-runtime FILE` | Validates runtime configuration and the signed fee publication; does not contact RPC or load secrets. |
| `gateway-public-key CONFIG` | Reads only the configured HPKE seed and derives its public key configuration/pin. Does not open a journal, load a treasury key or contact RPC. |
| `inspect DATABASE` | Opens an existing journal read-only and verifies aggregate operator totals. Does not print operation IDs, addresses, ciphertexts or signed transactions. |
| `sign-fee-schedule POLICY PUBLIC_RECIPIENT_FILE PRIVATE_KEY_FILE` | Offline Ed25519 signing of the closed common fee schedule. It cannot sign an arbitrary message or blockchain transaction. Outputs only the signed public packet. |
| `serve CONFIG` | Runs the bounded loopback OHTTP gateway. Normal builds return encrypted `503` for private admission and fee availability. Status and configured pool reads are read-only. |
| `wallet-ohttp-relay serve CONFIG` | Runs the separate opaque relay. It does not read the operator configuration, journal, verifier, HPKE key or treasury key. |

`config.example.json` deliberately contains invalid placeholders, not usable public fixture keys. Populate it from reviewed operator and deployment configuration. `relay.example.json` supplies only a fixed gateway destination, loopback listener and optional exact browser origin.

A real deployment must place reviewed HTTPS ingress in front of each loopback listener. The relay must be controlled independently of the gateway operator. Running the two binaries on different company machines does not establish that independence. Access logs, load balancers, network telemetry, backup retention and TLS termination require their own verified configuration.

## Runtime components and boundaries

- **`gateway.rs`:** RFC 9458 OHTTP using the existing `ohttp` / `bhttp` libraries, not custom cryptography. It authenticates the inner destination and fixed paths, rejects unexpected fields/headers/trailers, uses zero padding and encrypts application errors. HTTP accepts only `POST /ohttp` ciphertext; there is no plaintext submission route.
- **`relay.rs`:** one fixed HTTPS gateway destination, no redirects, no original IP/Forwarded/User-Agent/cookies/authorization/request IDs forwarded, no request logging, fixed CORS origin, bounded framing and response sizes, four concurrent forwards and a circuit breaker. It handles ciphertext only and cannot decode wallet methods. No destination URL is accepted from a request.
- **`proof_process.rs`:** fixed absolute Node executable, script and operator-configuration paths, pinned SHA-256 digests, empty inherited environment, fixed argv, bounded stdin/stdout, no shell, no stderr collection, two concurrent executions and a 20-second child deadline. EOF is explicitly delivered before waiting for the result. Successful stdout is accepted only after successful child exit and exact field binding. The script lives at `privacy-engine/scripts/provider-verifier.mjs`.
- **`signer.rs` / `transaction.rs`:** a native EIP-1559 signer that reconstructs only an admitted zero-value call to the approved pool with exact verified calldata. Independent signature recovery checks sender, chain, target, nonce, canonical encoding, low-s signature, access list and fees. There is no request-selected destination or generic signing API.
- **`rpc.rs`:** two fixed independent-provider HTTPS endpoints, chain/deployment/proxy/circuit-key checks, canonical header agreement, fresh roots and unspent nullifiers, exact simulation, gas/nonce reads, pool history, and exact receipt reconciliation. A request cannot supply an RPC method, URL or customer-address filter.
- **`worker.rs`:** persists signed bytes before release, reconciles before any explicit replay, drives canonical receipt accounting, audits earlier attempts, and reserves every new outer attempt under the original cumulative exposure cap.
- **SQLite journal:** `STRICT` tables, WAL, `synchronous=FULL`, foreign keys, immutable rows, read snapshots and `BEGIN IMMEDIATE` reservation decisions. It is a local adapter, not a distributed PostgreSQL queue.

The gateway has 32 connection slots, four application-processing slots, a three-second HTTP header timeout, 55-second connection deadline and a 540,672-byte ciphertext limit. Blocking-work permits remain held even if the client disconnects. The relay has its own independent limits and 60-second connection deadline. Failure or timeout never becomes evidence that a payment failed.

### Separate key roles

| Key | Purpose | Where loaded |
| --- | --- | --- |
| Gateway HPKE seed | Decrypt OHTTP requests and encrypt corresponding responses | Gateway only |
| Fee-publisher Ed25519 key | Sign one common fee schedule | Offline publication command or separate release pipeline |
| Operator native spending key | Pay gas for policy-approved outer transactions | Treasury signer; normal gated CLI does not load it |
| Provider private viewing key | Decrypt only the provider's own fee output | Fixed offline verifier configuration |
| Customer spending/viewing/recovery keys | Control and recover customer funds | Not accepted by this service or relay |

Secret input supports owner-only files, explicitly named environment variables or stdin. File inputs reject symlinks and oversized data; the operator verifier configuration must be owner-only because it contains the provider viewing key. Secret values are never command-line arguments, serialized runtime fields, error messages or Debug output. Zeroization reduces memory retention; it is not an assertion that managed runtimes or compromised hosts cannot copy secrets.

## Proof and chain evidence

The actual provider verifier decodes the full canonical V2 `transact` call, verifies the real Groth16 proof against pinned artifacts, binds tree/root/nullifiers/commitments/ciphertexts/calldata, and decrypts the declared provider fee output. It requires the configured token and exact fee amount and rejects sender/memo disclosure in that fee note. Shape-valid JSON or a `VerifiedOperation` value alone does not establish proof validity or privacy.

`fixtures/genuine-proof/` contains a real 01x03 proof, matching verification key and public deterministic test fee-viewing key. Its tree is synthetic and unfunded. The positive subprocess test verifies the genuine proof and rejects a modified advertised fee even when the underlying proof remains valid. `provenance.json` records artifact hashes and limitations. **Do not fund those keys or broadcast those envelopes.** `fixtures/abi-case.json` is separately labelled as an invalid-proof ABI fixture, generated independently with the SDK's ethers Interface.

The chain adapter checks:

- Both RPC endpoints report chain 42161. Heads are reconciled at the lower shared height and the corresponding numeric block header must agree.
- Pool/token bytecode, additional implementation bytecode and configured proxy storage match independently reviewed deployment pins.
- The contract verification-key ABI bytes match the configured circuit pin; the root is known and each `(tree_number, nullifier)` is unspent before admission/release.
- Simulation is the exact approved zero-value pool call, gas fits the fixed limit, and the live native balance covers **all** pending operator exposure, not just one operation.
- A receipt matches the exact signed transaction, pool, chain, block header, execution status, expected nullifier/commitment/ciphertext events and fee output. A spent nullifier by itself is insufficient.
- Treasury accounting requires the operator's type-2 transaction. The separate client pool-receipt endpoint can return an exact direct pool transaction from another broadcaster so the wallet can reconcile a replay of its proof; that transaction cannot be charged to the operator ledger.

These are checks against pinned deployments and agreeing RPC providers. **They are not a consensus light client or cryptographic receipt-trie proof.** Provider independence, current canonical deployments and the chain's finalized-tag/L1 semantics remain qualification requirements.

Different hashes serve different roles: `verifier_digest`/artifact pins bind the published verification-key file, while `encoded_key_keccak256` binds the ABI-encoded result of the deployed contract's `getVerificationKey`. Do not substitute one for the other.

For Arbitrum Nitro, the selected adapter uses `gasUsed × effectiveGasPrice`. `gasUsedForL1` is part of that gas count, not an extra fee to add again. The adapter therefore requires `additional_native_fee_cap_wei: "0"`. See [Arbitrum's gas-estimation documentation](https://developer.arbitrum.io/arbitrum-essentials/how-to-estimate-gas).

## Closed wire protocol

All application messages are inside OHTTP. Outer success is not application acceptance; application acceptance is not inclusion or finality.

### Submission and status

Submission body is the strict core `PrivateEnvelope` **schema version 2**, including required `tree_number`. Unknown/duplicate fields and legacy v1 envelopes are rejected. Operation digest includes exact canonical calldata and the tree. Successful response is exactly:

```json
{"operation_digest":"0x…","accepted":true}
```

`accepted` means durable admission only. Gated builds return encrypted `503` with the same digest and `accepted:false`. The wallet must preserve uncertainty for transport failures or unrecognized responses.

Status request is `{schema_version:1,operation_digest}`. Status response is exactly `{schema_version:1,operation_digest,state,transaction_hash}`. `transaction_hash` is nullable. States are `not_found`, `admitted`, `signed`, `unknown`, `pending`, `included`, `included_revert`, `finalized`, `finalized_revert`, `retry_exhausted`, `reorged`, `incident`. Status is advisory; clients establish payment completion from canonical exact-proof execution evidence.

### Signed shared fee schedule

The packet is `{schema_version:1,schedule,key_id,signature}`. The schedule fields in signing order are `domain`, `schedule_id`, `fee_units`, `fee_recipient_address`, `fee_recipient_commitment`, `max_native_fee_wei`, `gas_limit`, `valid_from`, `valid_until`.

The Ed25519 signature covers UTF-8 `local-wallet/shared-fee-schedule/v1\0` followed by canonical typed schedule JSON. `key_id` is the SHA-256 of the pinned Ed25519 public key. The fee-recipient commitment is Keccak-256 of UTF-8 `local-wallet/provider-fee-recipient/v1\0` followed by the exact provider privacy address. Full address/checksum/network validation also occurs in the SDK. The key comes from reviewed release configuration; fetching a key alongside its own signature does not authenticate it.

Signed schedules rotate in the **same journal**, preserving global nonce/nullifier/reservation history. Rotation changes only schedule ID, fee and nonoverlapping validity epoch. The publisher key, recipient, domain, broadcaster and gas/spending bounds remain fixed. Old jobs retain their original schedule. Do not create independent journals for the same operator to simulate rotation.

### Fixed pool reads

`/v1/private/pool` accepts `{schema_version:1,chain_id:42161,kind}` with only the fields listed below. It is not a general RPC proxy.

| Kind | Additional request fields | Additional response fields |
| --- | --- | --- |
| `head` | None | `block` at agreed finalized height |
| `tip` | None | `block` at agreed fresh latest height |
| `block` | `number` | `block` |
| `checkpoint` | `number` at or below finalized height | `block,tree_number,next_leaf_index,merkle_root` |
| `block_logs` | `number` at or below finalized height | `block,logs` from complete quorum block receipts |
| `logs` | `from_block,to_block` | Same bounds, `block_to_hash,logs` |
| `root` | `tree_number,root` | Same tree/root, `known,block` |
| `nullifiers` | `tree_number,nullifiers` | Same array, `spent,block` |
| `receipt` | `transaction_hash` | `receipt,finalized_block,transaction_input` |

Every response also includes `schema_version`, `chain_id`, `kind`. Block is `{number,hash,parent_hash,timestamp}`. Logs use the standard camelCase RPC fields for address, topics, data, block/transaction hashes and indices, and `removed`.

Log ranges are at most 2,048 blocks, aligned powers of two, or an aligned tail ending at the current agreed finalized head. Requests cover the full fixed pool and approved event topics. Nullifier reads require exactly 16 unique hashes so clients can pad their set. A receipt is null or the closed object containing `transactionHash,blockHash,blockNumber,status,gasUsed,effectiveGasPrice,logs`; **`transaction_input` is a separate top-level field**, null exactly when the receipt is null.

A checkpoint reads the fixed pool's `treeNumber()`, `nextLeafIndex()` and `merkleRoot()` through both providers at the exact requested finalized block, then rechecks its canonical hash. Clients must match the final active tree, leaf count and root before declaring a pool sync complete. Membership of an older root in `rootHistory` alone does not establish that all later pool events were scanned.

At an output-tree rollover in block B, clients also need the exact checkpoint at B−1, all approved pool logs from `block_logs(B)`, and the checkpoint at B. A valid later active-tree root alone cannot reveal an omitted final event in an older tree. `block_logs` obtains the ordered transaction-hash list and **all** `eth_getBlockReceipts` entries from both providers. It requires one receipt per transaction at its exact index, matching block and transaction hashes, continuous global log indices, matching normalized receipts and a canonical block recheck. Only then does it filter the fixed pool and approved event topics. Foreign-contract logs, including valid zero-topic logs, are validated before filtering. No customer address or caller-selected RPC method/filter is accepted.

Complete-block reads allow at most 4,096 transactions and 8,192 total logs; existing 1 MiB upstream JSON and 500,000-byte response limits also apply. Unsupported, omitted, duplicated, misindexed, contradictory or oversized responses fail closed without partial results. The two-provider agreement remains a trust assumption, not a receipt-trie proof.

### Historical deployment pin epochs

`rpc.historical_deployments` is an optional array, defaulting to `[]`, containing at most **64 reviewed historical profiles**. Each profile contains an exclusive `until_block` boundary and the complete `pool_code_keccak256`, `token_code_keccak256`, `implementation_code_pins` and `proxy_storage_pins`. The first interval starts at `deployment_block`; every later interval starts at the preceding boundary. Boundaries must strictly increase. A caller cannot supply a starting block, override a profile, leave gaps or select pins through the pool API.

The top-level deployment pins apply at and after the last historical boundary, or from `deployment_block` when the array is empty. Every profile requires nonempty sets of at most eight implementation code pins and eight storage pins, canonical addresses/hashes/words, and no duplicate implementation addresses or storage slots. Historical reads check the selected complete profile against both RPC providers at the exact historical block. There is no fallback from a failing historical profile to current pins, and no acceptance of absent pool or implementation code.

If the supported token was deployed after the pool, a reviewed historical prefix may set `token_not_yet_deployed: true`. This field defaults to `false` in older profiles and is not accepted in the top-level current profile. An absence profile must pin `token_code_keccak256` to Keccak256 of empty bytes; both RPC providers must return exactly `0x` for the token at the requested historical block. Unexpected bytecode, provider disagreement, or a later return to an absence profile after a token-present profile fails. Pool, implementation and storage checks remain mandatory throughout. Current admission, quotes and treasury checks still require the current nonempty token code and reject historical profiles. The actual token deployment boundary and historical pool profiles must be independently qualified; the example supplies no guessed mainnet interval.

Current admission, gas quotes, initial treasury anchoring and pending-exposure checks require the **current** profile. A fresh agreeing latest head that falls inside a historical interval cannot authorize sponsorship. Existing historical treasury anchors and operation receipts can still be reconciled against their explicitly reviewed historical profile.

The example configuration includes deliberately invalid historical placeholders; it contains no real upgrade boundaries or code/storage pin values. Operators must independently qualify each epoch, including historical ABI compatibility and archive-provider availability. Providing this configuration mechanism does not qualify any deployed history. Missing, inaccurate or unsupported historical evidence remains a closed failure.

## Durable retries and accounting

All amounts use checked U256 integers encoded as canonical decimal strings. SQLite floats, signed-64-bit monetary sums and decimal truncation are not used.

One immutable private envelope owns its `(tree_number,nullifier)` claims for its lifetime. At most **three sequential outer attempts** are supported. Each has immutable signed bytes, its own unique operator nonce, release events and receipt evidence. The original private proof, recipient ciphertexts, principal, provider fee and cumulative native cap never change.

| Current state | Native accounting | Retry behavior |
| --- | --- | --- |
| Admitted/signed | Prior finalized revert costs retained; remaining original cap reserved | First release of the current saved attempt |
| Unknown/pending | Same reservation; no fee earned | Background reconciliation only. An explicit identical request can authorize another release of the exact saved bytes; no new nonce. |
| Included success/revert | Remaining cap reserved; no fee earned | Await canonical finality |
| Finalized success | All previous revert gas plus actual successful gas charged; exact original fee earned once | No new attempt |
| Finalized revert | All actual reverted gas retained; fee zero | Explicit identical resubmission can start the next attempt after fresh finality/unspent checks and atomic remaining-budget reservation |
| Retry exhausted | All actual reverted gas retained; fee zero | No new operator attempt; proof/nullifier claims remain retained |
| Canonical contradiction/incident | Conservative full exposure restored and service halted | No new spending; immutable evidence retained |

A new attempt requires both providers to confirm every preceding native charge belongs to a canonical finalized receipt, and the proof inputs remain unspent. Its maximum possible signed native cost must fit `original cap − prior finalized revert costs`. Global pending-count, pending-exposure, opening-balance, live-balance and lifetime-spending limits apply again. Concurrent retries cannot create multiple next attempts.

The common schedule deadline governs **new admission**. It does not revoke an admitted private proof, create a cancellation, or force a new fee. An already-admitted unchanged operation may explicitly retry after its epoch under its original fee and cumulative cap. A new operation under an expired schedule is rejected; the wallet must refresh its fee publication before creating a new intent.

There are at most three releases of identical bytes per outer attempt, with durable backoff. A timer does not authorize another unknown send. Reconciliation always runs first, and prior attempts remain auditable after later success. A finalized outer revert alone never authorizes the wallet to reuse its private notes for a different payment: someone could still publish the old proof. A canonical exact-proof transaction from another broadcaster can resolve that uncertainty through the client pool-read path.

## Persistence and data limits

The schema contains no customer identity, plaintext destination, principal, recovery phrase, private spending key or witness column. It **does** retain opaque envelopes, commitments/nullifiers, shared fee terms, admission/retry times, operation IDs, outer signed transaction bytes/hashes and receipt evidence. Do not describe it as storing no data. The relay's circuit state is transient aggregate health state, not a per-user record.

OHTTP separates network identity from request contents only with properly independent operation and authenticated key distribution. It does not erase public-chain timing, fee, traffic-volume or correlation information. The gateway observes public pool queries and submission timing; neither absence of HTTP logs nor proof validity is by itself a complete privacy guarantee.

Journal schema **4** adds tree-aware nullifiers, signed schedule history and per-attempt nonce claims. Earlier unreleased schemas are rejected and preserved; there is no migration that guesses a missing tree or silently drops economic history. New Unix journal files use owner-only permissions; foreign databases and symlinks are rejected. Append-only triggers and replay checks protect against mistakes, not a privileged host administrator. WAL durability relies on storage honoring the OS synchronization guarantees.

The current lifetime limits are 100,000 jobs, 10,000 events per job and 10,000 schedule epochs. Bounded queue selection prioritizes active jobs and reserves one finalized audit slot in multi-job batches. A one-slot batch alternates live work and terminal audits; skipped audit selections do not advance the audit cursor. The runtime uses batches of 32. These are defensive bounds, not a production throughput claim. No retention compaction, automatic top-up, fee bump, arbitrary balance adjustment, new private proof replacement or incident-unhalt endpoint exists.

## Qualification still required

Before replacing either economic release gate:

1. Independently review the exact deployed pool/token/proxy/circuit, encryption metadata, artifact provenance, public verification keys and permitted private-transfer ABI.
2. Complete real Android device/prover performance and recovery testing against the exact native bridge and reviewed artifacts. Host synthetic proofs do not establish device acceptance or canonical mainnet funding.
3. Review the complete gateway/verifier dependency tree and OS isolation. The fixed process pins cover configured files; deployment must protect the dependency tree and artifacts against replacement.
4. Configure genuinely independent RPC evidence sources and approve finalized-tag/L1 trust semantics, deployment code/storage pins, current treasury solvency and exclusive operator nonce control.
5. Operate an independent relay and gateway with reviewed HTTPS, key-pin distribution, no request telemetry, backup/retention controls and no public-RPC fallback.
6. Independently review the signer, cumulative retry accounting, crash recovery and operational incident handling; fund an isolated operator treasury only after release acceptance.

Tests exercise real OHTTP client/relay/gateway encryption, real signatures and loopback HTTP RPC, genuine and invalid proof verification, altered fee rejection, ambiguous-network outcomes, exact receipt/ciphertext matching, tree-scoped duplicates, concurrent reservations, multi-attempt caps, finality contradictions and read-only exports. They do not claim a funded mainnet private transfer or a completed external security audit.
