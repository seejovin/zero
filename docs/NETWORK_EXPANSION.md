# Android 0.10.0 network expansion

Status: implemented source candidate, not a compiled or distributed APK. The latest verified APK is 0.8.0/code11; candidate metadata is 0.10.0/code13. See VERIFICATION.md for executed checks and remaining gates. This work does not deploy or expand the web runtime.

## Supported mainnet matrix

| Network | Assets | Native fee asset | Native decimals |
| --- | --- | --- | ---: |
| Ethereum, EVM1 | ETH, USDC, EURC, USDT | ETH | 18 |
| Base, EVM8453 | ETH, USDC, EURC | ETH | 18 |
| OP Mainnet, EVM10 | ETH, USDC | ETH | 18 |
| Arbitrum One, EVM42161 | ETH, USDC | ETH | 18 |
| Avalanche C-Chain, EVM43114 | AVAX, USDC, EURC, USDT | AVAX | 18 |
| Polygon PoS, EVM137 | POL, native USDC | POL | 18 |
| BNB Smart Chain, EVM56 | BNB | BNB | 18 |
| Solana mainnet | SOL, USDC, EURC, USDT | SOL | 9 |
| TRON mainnet | TRX, USDT-TRC20 | TRX | 6 |

All included stablecoins have six decimals. The 24 asset/network positions appear as nine asset choices, with a separate explicit Network dropdown. Assets on different chains cannot be spent as one balance; switching networks does not bridge funds. Binance-Peg stablecoins, bridged USDC.e, arbitrary imported tokens and unsupported EURC deployments are excluded.

## New canonical token identities

| Network | Asset | Contract or mint |
| --- | --- | --- |
| Ethereum | USDT | `0xdAC17F958D2ee523a2206206994597C13D831ec7` |
| Avalanche C-Chain | USDC | `0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E` |
| Avalanche C-Chain | EURC | `0xC891EB4cbdEFf6e073e859e987815Ed1505c2ACD` |
| Avalanche C-Chain | USDT | `0x9702230a8ea53601f5cd2dc00fdbc13d4df4a8c7` |
| Polygon PoS | USDC | `0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359` |
| Solana | USDC | `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v` |
| Solana | EURC | `HzwqbKZw8HxMN6bF2yFZNrht3c2iXXzpKcFu7uBEDKtr` |
| Solana | USDT | `Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB` |
| TRON | USDT | `TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t` |

Existing Ethereum/Base/OP/Arbitrum canonical contracts remain pinned in core/src/crypto.rs and Android AssetSelection.kt. The full new matrix was checked against the issuer sources on 2026-11-06: [Circle USDC](https://developers.circle.com/stablecoins/usdc-contract-addresses), [Circle EURC](https://developers.circle.com/stablecoins/eurc-contract-addresses), [Tether protocols](https://tether.to/en/supported-protocols/).

## Recovery and addresses

| Family | Derivation | Address |
| --- | --- | --- |
| Seven EVM networks | BIP-32 secp256k1, `m/44'/60'/0'/0/0` | Existing `0x…` address |
| Solana | SLIP-0010 Ed25519, `m/44'/501'/0'/0'` | Base58 public key |
| TRON | BIP-32 secp256k1, `m/44'/195'/0'/0/0` | Base58Check `T…` address |

All derive from the existing authenticated BIP-39 seed with an empty extra passphrase. No recovery phrase is regenerated, no seed is exported to Kotlin and no wallet-key backend is introduced. The Android manifest’s EVM address remains its stable local wallet identifier; family receiving addresses are derived during authenticated opening. A compatible recovery wallet must select the matching family/path. Local UI IDs -101/-195 are private registry keys, never EVM or protocol chain IDs.

## Fees and Max

- EVM token Max preserves every uint256 base unit. Native Max reserves the network’s actual gas currency. Avalanche uses AVAX; Polygon uses POL; BNB uses BNB. BNB’s explicit gas-price quote uses a London-compatible type-2 envelope with zero base fee and matched caps.
- Ethereum’s older canonical USDT transfer accepts an empty return value during simulation. No other token gains that exception, and receipt validation still requires the canonical token’s expected Transfer evidence.
- Solana uses canonical legacy messages, Ed25519 signatures, System transfers, fixed classic-SPL TransferChecked instructions and required ATA creation. Fee quotation includes base/priority fees and required token-account rent. Token Max can combine up to eight verified spendable token accounts; an unsupported fragmented/frozen balance is blocked rather than truncated. Native Max retains a fee reserve. Accounts are not automatically closed to reclaim rent.
- TRON builds exact TransferContract or official-USDT TriggerSmartContract protobuf locally. Review includes activation, bandwidth and energy. Ordinary free/staked bandwidth is evaluated under the protocol’s full-coverage rule; energy fee_limit does not include bandwidth/activation. Native Max reserves those separate costs. Available resource subsidies affect estimates while a conservative burn reserve protects against resource consumption before inclusion.
- Local spending ceilings are policy choices, not consensus limits: original EVM networks0.05ETH; Avalanche0.5AVAX; Polygon1POL; BNB0.01BNB. Solana/TRON have separate bounded protocol policies. Higher-cost operations are blocked for a new reviewed policy decision.
- A Solana/TRON review currently requires sufficient native funds before the full preparation returns. Unlike the existing EVM preview, an unfunded family transfer can show an unavailable fee estimate plus its funding error. A native-asset Deposit shortcut remains available; splitting read-only fee quotation from affordability is a remaining UX improvement.
- USD values use Coinbase public spot quotes when available. No hardcoded stablecoin peg or fabricated zero price is used. Display rounding never changes transfer units.

## Durable transfer sequence

1. Fetch bounded mainnet observations and validate recipient, token identity, integer units and required resources.
2. Quote without signing authority. A single running quote and latest pending request prevent queued stale typing requests.
3. Revalidate and persist the exact unsigned review. Show network, source, complete recipient, exact amount, cost and reserve.
4. Revalidate that same payload again, apply the selected app lock, and sign through the restricted Rust boundary.
5. Zeroize owned key buffers and commit exact signed bytes before making a single broadcast attempt.
6. Record submitted/unknown separately from included/finalized. Reconcile receipts and canonical-block evidence. An explicit retry reuses only the committed bytes and hash.

Solana blockhash/lastValidBlockHeight and TRON TAPOS/expiration are bound to the review. Neither is silently refreshed after signing. Expiry and a null RPC status cannot prove a payment never executed; an expired unknown transaction remains reserved until reliable evidence establishes its outcome. A local cancel is allowed only for an unsigned review.

Family journals bind one network and address, use SQLite WAL/FULL and append-only events, and rebuild projections from verified evidence. Conflicting finality, same-block balance discrepancies and contradictory deposit evidence remain visible/blocked rather than overwritten. These controls do not make local storage tamper-proof against full device compromise.

## Provider access and history limits

Solana checks fixed mainnet genesis `5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d`; TRON checks mainnet `0x2b6653dc`. Read fallbacks must pass identity checks. TLS, provider availability and truthful chain evidence remain trusted; this is not a light client or a provider quorum.

TRON’s fixed origin is `https://api.trongrid.io`. Production requests should include a user-owned TronGrid key; unauthenticated requests may be limited or rejected. Configure it in Profile → Settings → TRON connection. It is stored using a separate Android Keystore AES-GCM key and an atomic no-backup file, never embedded in source or logged. Requests use the TRON-PRO-API-KEY header and reject redirects. An API key does not grant wallet signing authority. See [access-key documentation](https://developers.tron.network/reference/select-network) and [rate limits](https://developers.tron.network/reference/rate-limits).

Recent Solana/TRON discovery returns at most20 candidate IDs per pass. Every candidate requires core-validated transaction/receipt/block evidence; indexer rows alone do not create credits. Solana also searches supported token accounts. TRON supports direct TRX and official-USDT Transfer logs, including valid contract payout logs. Older transfers can be tracked explicitly by ID. EVM discovery retains its bounded scan and manual native-deposit verification. The local history is not a complete address index.

## Release gate

No new mainnet broadcast, funded-account test, Android runtime test, APK assembly, signature check or device validation is claimed. The environment’s earlier automatic approval review rejected Android SDK/NDK, Gradle and JDK archive download destinations. No workaround route was used. The source includes scripts/build-android.sh and android/DEVICE_TESTS.md for the remaining concrete gates. Existing distribution signing credentials must stay private and the same certificate must be used for an in-place upgrade.

The APK must also include its compiled-runtime third-party license/copyright notices. The updated dependency inventory records provenance; it is not a substitute for that distribution notice bundle.
