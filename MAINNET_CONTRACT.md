# Local Wallet 0.2 — implementation contract

The user's current instruction authorizes Ethereum mainnet (chain 1), Base mainnet (chain 8453), Android APK and web. It supersedes the old iOS/Sepolia implementation contract. Mainnet is enabled; testing never spends real funds.

## Boundaries

- ETH, native Circle USDC and EURC on both networks. No custom contracts, approvals, arbitrary signing, bridges, swaps, delegated accounts, custody backend or external hardware signer.
- Shared Rust signing/validation core. Android uses Kotlin/UniFFI and SQLite; web uses a WASM worker and IndexedDB.
- BIP39 creation uses 256-bit OS/browser CSPRNG entropy, 24 English words, m/44'/60'/0'/0/0, empty BIP39 passphrase. Restore standard valid English word counts. No secret sent to RPC or a wallet server.
- Android Keystore wraps a random 32-byte vault key with an authentication-per-use AES key. Web derives its vault key using Argon2id (64 MiB, 3 iterations, 1 lane, 32 bytes) and a random 16-byte salt. The underlying entropy vault uses AES-256-GCM.

## Registry

`asset_registry()` returns `{networks:[{chain_id,network,rpc_url,broadcast_url,explorer_url,assets:[{symbol,decimals,contract}]}]}`.

| Network | Reads | Submission | USDC | EURC |
| --- | --- | --- | --- | --- |
| Ethereum, 1 | https://ethereum-rpc.publicnode.com | same | 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 | 0x1aBaEA1f7C830bD89Acc67eC4af516284b1bC33c |
| Base, 8453 | https://mainnet.base.org | https://mainnet-sequencer.base.org | 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913 | 0x60a3E35Cc302bFA44Cb288Bc5a4F316Fdb1adb42 |

USDC/EURC have 6 decimals; ETH has 18 and contract null. Network switching changes RPC, fee policy, nonce scope, balance and journal. It does not bridge funds. The same recovery seed derives the same address on both networks.

## Shared bridge

Existing JSON APIs remain; Kotlin uses generated camelCase, WASM snake_case. Vault key parameters are byte arrays. Export `generate_wallet`, `restore_wallet`, `reveal_mnemonic`, `wallet_address`, `prepare_payment`, `sign_payment`, `inspect_signed`, `asset_registry`, `format_amount`. WASM additionally exposes `derive_vault_key(password,salt)`; shared stateless validation exposes `validate_prepared_json`, `validate_signed_for_prepared` and `validate_receipt`. Native `WalletJournal` remains available with native-only SQLite.

Prepare request fields: `intent_id,chain_id,from,to,asset,amount,nonce,gas_limit,max_fee_per_gas,max_priority_fee_per_gas,l1_fee_estimate_wei,operator_fee_estimate_wei`. All monetary values are decimal strings; nonce/gas/chain IDs are bounded integers. Ethereum additional fees must be zero.

Prepared output includes normalized request, amount_units, max_fee_wei, estimated_total_fee_wei, fee_reserve_wei, unsigned_transaction, approval_digest, created_at, expires_at. Signed includes previous identity/amount/hash/raw fields plus estimated_total_fee_wei and fee_reserve_wei. Approval digest binds fee quotes as well as all transaction terms and expiry.

`max_fee_wei = gas_limit * max_fee_per_gas` is the EXECUTION fee cap only. `estimated_total_fee_wei = max_fee_wei + l1_fee_estimate_wei + operator_fee_estimate_wei`. `fee_reserve_wei = max_fee_wei + 2 * (l1_fee_estimate_wei + operator_fee_estimate_wei)` is a local budgeting buffer, never a guaranteed on-chain cap.

For Base, quote `getL1FeeUpperBound(1024)` and `getOperatorFee(gas_limit)` on fixed GasPriceOracle `0x420000000000000000000000000000000000000F`. Recheck immediately before signing; changed terms require another review. Signed transaction length must be within the quote's size bound. Base receipt fee is `gasUsed * effectiveGasPrice + l1Fee + operatorFee`; never discard an observed financial outcome because estimates were exceeded. If operatorFee is not included by RPC, query `getOperatorFee(gasUsed)` at the canonical receipt block. No silent zero when fee data is missing.

## Transaction invariants

- Check chain ID, bound RPC responses, reject nonempty sender code/delegated accounts, validate token code and preflight eth_call/estimateGas, use exact integers.
- Explicit address, chain, token contract, quantity and fee review. Sign only the immutable reviewed payload after fresh authentication; no generic signing bridge.
- Persist intent, raw signed bytes and hash durably before network submission. One unresolved signed outgoing payment per network. Concurrent tabs must share an atomic nonce/intent constraint.
- Timeout, dropped response or RPC error after signing produces unknown/unconfirmed outcome. Retry only identical signed bytes. Never automatically create another transaction or nonce to resolve uncertainty.
- Receipt must match hash, sender, destination, status, canonical block and expected ERC20 Transfer event. Read finalized head then recheck canonical inclusion block. Keep execution, inclusion and finality distinct; append corrections/reorg events rather than rewriting financial history.
- Real balances come from a consistent on-chain block observation. Outgoing local history is explicitly not a complete ledger of incoming transfers. No charges, capture, settlement, chargebacks, reserves or merchant payables in this wallet scope.

## Ownership during implementation

- Core agent: core/, Cargo files, WASM build script.
- Android agent: android/, scripts/build-android.sh.
- Web agent: web/ except root-owned .openai/hosting.json and Sites Git operations.
- Root: integration, toolchain, verification scripts, docs, packaging and publishing.

## Sources verified 2026-11-05

- https://developers.circle.com/stablecoins/usdc-contract-addresses
- https://developers.circle.com/stablecoins/eurc-contract-addresses
- https://docs.base.org/get-started/connect-to-base
- https://docs.base.org/specifications/transactions/network-fees
- https://developer.android.com/reference/android/security/keystore/KeyGenParameterSpec.Builder
- https://developer.android.com/identity/sign-in/biometric-auth
