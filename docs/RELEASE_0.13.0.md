# zerø 0.13.0 release evidence

## Status

The recovered source is being integrated into `seejovin/zero`. Android version code is 16, the version is `0.13.0-mainnet`, and the app label is **zerø**. The package remains `app.localwallet.mainnet`.

The ARM64 and x86-64 native libraries, Kotlin app, release JVM tests, lint and unsigned APK assembly have completed successfully. Signing and the final APK manifest are the remaining packaging steps at this source checkpoint. `APK_MANIFEST.json` still describes 0.8.0 until the new signed binary is verified.

## Included engineering

- zerø display name, launcher artwork and native opening transition, with reduced-motion/accessibility handling and no network dependency.
- Existing vault, account derivation, app ID and encrypted storage preserved.
- Nine-network public-wallet source and 0.12.0 private engine/service implementation carried forward.
- GitHub Actions Android compilation, native libraries, JVM tests, lint and unsigned aligned APK workflow.
- Separate local signing with the existing certificate, rather than storing the distribution key in the repository.
- Tencent operator/gateway and independently operated relay deployment material.

## Private service release boundary

Production private sending remains disabled. The compiled release gates and default unqualified manifest remain in place. Offline proof fixtures do not qualify a deployed contract or a funded service.

Remaining operational requirements include current deployment/verifier qualification, reviewed production configuration, a truly independently operated relay, secret provisioning, a bounded funded treasury, Android/device qualification, and end-to-end transfer/recovery evidence. The supplied cloud files do not supply or fabricate these values.

## Verification executed for this release

| Check | Result |
| --- | --- |
| Rust workspace tests | 256 passed; 0 failed |
| Private-engine JavaScript tests | 40 passed; 0 failed |
| Android release JVM tests | 270 passed; 0 failed; 0 skipped |
| Android release lint | Completed; 0 errors |
| Native libraries | ARM64 and x86-64 compiled from the current source |
| Android release assembly | Successful; signing checked separately |

Build integration corrected the private-engine asset task declaration, three hidden Android directory-open API uses, and four Android 11-incompatible Solana counter conversions. Directory sync retains no-follow, descriptor type validation, fsync and close. Solana counter narrowing rejects overflow while token quantities retain their full unsigned range. Stale network fixtures now cover the expanded registry. The common fee-publication deadline and historical pre-USDC scanner behavior have regression coverage.

No physical Android device installation, on-device proving, funded private transfer or recovery was executed. The attempted desktop Chromium qualification was blocked by the environment before application assertions ran.

## Earlier evidence

The previous exact results are retained in `android/verification/v0120-verification.json` and the historical verification report. They cover 251 Rust tests, 40 JavaScript engine tests, real Groth16 proof/ciphertext checks, native bridge checks and packaged-runtime fixtures. They were executed on the earlier source and are identified as historical until rerun for this release.

## Signing continuity

The required distribution certificate SHA-256 is:

`da1632404a817505713e77527ef97543fc6c1028148c401462b96683e2392687`

The signing workflow must fail if it cannot prove this identity. No debug key or new signing identity substitutes for the retained certificate.
