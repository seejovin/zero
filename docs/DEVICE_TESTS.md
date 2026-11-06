# Android and browser acceptance checks

These are remaining device and integration checks unless the verification report explicitly records a completed result. They use the implemented mainnet configuration, not a testnet build. Fault injection uses synthetic RPC evidence and public fixture keys offline. Any future real-value transfer requires a separately authorized sender, recipient and amount; this build did not spend funds.

The Android 0.12.0 source candidate includes the nine-network public wallet and the separately gated private-engine upgrade. Its APK build is currently blocked, and the additional private-engine, upgrade, network, fee and UI checks are in [Android device checks](../android/DEVICE_TESTS.md). The Ethereum/Base browser checks below remain relevant to the unchanged deployed web 0.6.0 runtime.

## Android installation and key protection

1. Install the signed APK on Android 11 or newer, including an ARM64 phone. Confirm its package ID, certificate fingerprint and file checksum against the release manifest.
2. Create a fresh unfunded wallet without an app PIN. App lock starts Off; a device screen lock is required only when enabling biometrics/device authentication. Verify no mandatory PIN step appears during creation.
3. Under Profile → Settings, test Off, exactly-6-digit app PIN, and biometrics/device screen lock. Verify every change authenticates the old mode and only commits the new one after successful wrapping. Cancel every prompt and confirm no obsolete signing or phrase reveal occurs. Device screen lock means the phone’s existing PIN/password/pattern, not the app PIN.
4. Confirm 24 generated recovery words, backup checking, and stable receiving address after restart. No phrase or private key may appear in logs, exported audit records or crash reporting. Recovery Copy is deliberate; verify sensitive clipboard marking and cleanup, and never claim clipboard-history removal is guaranteed.
5. Background the app during creation, phrase display, review and authentication. Verify secret UI clears and authentication cannot complete an obsolete operation after lock.
6. Verify recovery-word screenshots work only while the recovery screen is active. Other screens retain capture blocking; backgrounded secret views and task-switcher content must be cleared. Verify wallet-file exclusion from cloud and device-transfer backups. A user screenshot/photo backup is outside wallet-file backup controls.
7. Kill the process after vault-file commit but before backup completion. Reopen the same wallet; do not generate or overwrite it.
8. On a disposable installation, test key invalidation or data deletion. Recover from the offline words and verify the same first account on both chains. This restores signing access, not erased local intent history.

## Browser protection and durability

1. Use an HTTPS origin or localhost; verify a file URL/insecure context fails with an actionable message.
2. Create a fresh unfunded wallet without choosing a password. Quit/reload before and during the backup challenge; Continue backup must reveal the identical words/address. Then confirm the backup. Test older password-protected vault migration separately without deleting storage.
3. Enable a 6-digit app PIN in Profile → Settings. Verify exactly-six-digit input, wrong-PIN failure, persistent delays across restart and fresh selected-lock authentication before each signature or phrase reveal. Verify disabling/changing PIN requires current authentication, and another tab’s pending action rechecks policy. No derived key may be retained as session state.
4. Hide/background the page or reach its idle limit while secrets are displayed or a worker is signing. Confirm workers are terminated, secrets cleared and stale callbacks rejected.
5. Open two tabs for the same browser origin. Race preparation/signing and confirm only one active intent/nonce reservation succeeds on each network.
6. Inject IndexedDB aborts, quota errors and unavailable strict durability. No new raw transaction may reach the network before a completed durable commit.
7. Verify browser storage deletion and a different browser/origin require recovery. The site account/password is not wallet recovery.
8. Check CSP enforcement, absence of external scripts/CDNs and analytics, and correct operation of the locally bundled WASM and QR assets.

## Mainnet networking and receiving

1. Ethereum must report chain ID 1; Base must report 8453. Mismatched chain, redirect, duplicate JSON fields, oversized JSON and malformed quantities must block operations.
2. Verify receive screens show the full address and selected network. An Ethereum address can be the same on Base, but balances and fees must remain separate.
3. Compare known public-address balances with an independent provider at a specific block. Confirm exact 1 wei and 0.000001 stablecoin precision using fixtures.
4. Disconnect network access. Previously observed balances remain labelled stale/unavailable; no fabricated fresh zero appears.
5. Check fixed read failover: Base PublicNode then mainnet.base.org, Ethereum PublicNode then Alchemy public RPC. Base submissions use mainnet-sequencer.base.org, with one submission attempt. Hosted web uses the same bounded operations through its same-origin relay.
6. If separately authorized, receive a small amount of real USDC and ETH on each selected chain, then compare receipt and balance with an independent explorer. Do not import valuable recovery phrases into an unreviewed build for testing.

## Unified portfolio and visual stability — 0.6.0

1. Verify the home shows Base and Ethereum positions together, with explicit asset/network names and only DEPOSIT and SEND as its primary actions. There must be no global network switch.
2. Select USDC (Ethereum), then USDC (Base), through each action. Confirm the QR, token contract, fee source, ETH fee balance and final review match the chosen row. A late response from the other network must not overwrite the selected network's balance or quote.
3. Disconnect one network while the other remains available. The healthy network's rows remain usable; the missing network is not zero. Total USDC requires both observations and identifies saved data. ETH and EURC are not folded into an invented USD value.
4. While automatic updates complete, hold scroll halfway down the asset list, focus an action, edit a send amount and expand fee details. Check for flicker, scroll jumps, lost focus or replaced form inputs. Deposit QR must stay mounted across background refreshes.
5. Rename the wallet and account, close/reopen the app and verify the labels persist. Compare the full address before and after. Names must not affect the recovery phrase, derivation, encrypted vault, prepared transaction or prior journal records. Invalid or unavailable cosmetic metadata must not block opening the wallet.
6. Inspect 320px and 390px web widths and a physical Android phone with its keyboard open. Check long names, exact small balances, safe insets and no horizontal scrolling. The short opening animation must not replay on refresh; reduced-motion settings should suppress it.

## Transaction and crash correctness

1. Review full recipient, sender, network, token contract, amount, nonce and fees. Fee estimates must not be presented as guaranteed Base total caps.
2. Reject invalid checksums, zero addresses, excess precision, signed/exponent amounts, insufficient balance and unsupported or delegated sender accounts.
3. Expire a review or change its nonce, amount, recipient, chain or fee fields. Signing must require fresh preparation and approval.
4. Change nonce, pending delegation, gas requirement, balance or fee conditions externally between preparation and signing. Preflight must stop or require review again.
5. Fail authentication or signed-record persistence. Confirm no broadcast occurs.
6. Kill the process after signing, after durable commit, and after submission but before acknowledgement. Reopen without losing the hash or inventing a failure. Never automatically sign a replacement.
7. Deliver timeout, delayed response and duplicate response fixtures. Retry only the persisted bytes, with the same nonce/hash.
8. Apply failed receipts: no delivered amount, but actual gas fees remain recorded. For Base include execution, L1 data and operator components.
9. Supply successful receipts with incorrect contract, sender, recipient, amount or missing Transfer event. The payment must remain unresolved.
10. Exercise reorgs, missing receipts and conflicting observations. A provider simply returning a different receipt is not sufficient evidence to erase canonical inclusion. Corrections append evidence.
11. Reconcile inclusion and finality separately; reread the canonical inclusion block after the finality query. Preserve evidence when a previously finalized block appears noncanonical.
12. A Base fee exceeding the estimate must not erase a real successful payment. Record actual fees and flag the discrepancy.
13. Reopen stored journal records and balances after tampering/corruption fixtures. Invalid account, network, raw signed bytes, receipt evidence and snapshot linkage must be rejected.
14. Check audit export contains public transaction evidence but no wallet secrets or still-broadcastable raw transaction bytes.

## Release operations

The supplied build enables real mainnet signing. These acceptance checks, independent code review and controlled release signing are validation work still required before treating it as a production wallet. They are not a hidden mainnet enablement switch. Complete a deployment/rollback drill and protect the app-signing identity and web publishing access; compromised software delivery can defeat local encryption.
