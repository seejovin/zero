# zerø 0.14.0 — wallet switching and sign-out

## Behavior

- **Profile → Add wallet** opens Create wallet and Import wallet.
- **Profile → Wallets** lists saved wallets by name and public address, with an explicit Open action.
- **Sign out** is available with app lock on or off. It closes the active session and returns to saved wallets; it does not delete a vault, recovery phrase, wrapping key or transaction journal.
- A completed sign-out remains signed out after restarting the app. Selecting a wallet respects that wallet's own app-lock setting.
- Each added wallet has separate encrypted storage, wrapping keys and public transaction journals. Duplicate imports are rejected before a second vault is saved.

The same management controls are also available in Settings. New default wallet names are distinct; existing custom names are retained.

## Existing-wallet compatibility

The original `wallet-v1` directory stays in place. This release does not move or re-encrypt the existing vault during upgrade. New wallets use validated UUID directories under `wallets-v1`, with an atomic active-wallet selection file.

Wallet discovery includes a fully saved wallet even if selecting it failed afterward. Failed or unreadable entries are preserved and reported. Shared storage synchronization prevents catalog reads from racing atomic vault writes across Activity instances.

Switching clears account-specific cached balances, drafts, reviews, recovery views and pending document requests. Serialized cleanup retains already signed transaction records. Old callbacks and document-picker results cannot become operations on the next wallet.

## Release status

Source version is `0.14.0-mainnet`, Android version code `17`, with the existing `app.localwallet.mainnet` package. Android compilation, release assembly, all 296 JVM tests across 34 suites, and lint passed (zero errors, 13 warnings). The focused tests include 13 catalog/storage, eight routing/session-authority and five document-request regressions. The APK has been signed with the original distribution certificate and passed inspection again after signing.

The unsigned APK passed package/version/display-name checks, both 64-bit ABI checks, 16 KiB ELF/ZIP alignment and all declared private-engine asset hashes. The four native libraries match the previous signed 0.13.0 APK byte-for-byte; all 77 recorded Rust/private source hashes are unchanged. Evidence: `android/verification/v0140-android-build.json`, the matching lint reports and `v0140-tests/`.

Install the signed update over the existing app; do not uninstall or clear application data. Physical-device upgrade and UI instrumentation are separate from host JVM tests and have not been performed here.

The private gas-transfer production gate remains disabled; this release changes wallet management.

## Delivered APK

- File: `zero-0.14.0-mainnet.apk`.
- Source commit: [`39b140e83ef6be4e9a1975df8ea4f9a84dd3d2cf`](https://github.com/seejovin/zero/commit/39b140e83ef6be4e9a1975df8ea4f9a84dd3d2cf).
- Size: 16,861,660 bytes.
- SHA-256: `4e7d730ae5edadc7f46d86155ef6c963f2cc2362766edcd1ac04c1b7163159e7`.
- Original certificate SHA-256: `da1632404a817505713e77527ef97543fc6c1028148c401462b96683e2392687`.
- [Binary manifest](APK_MANIFEST.json) and [signature verification](evidence/v0140-apk-signature.txt).

The APK is supplied separately from this repository. No signing credential is published. Documentation-only follow-up commits do not change the runtime source identified above.
