# zerø 0.13.0 release evidence

## Status

The complete source is published in `seejovin/zero`. Android version code is 16, the version is `0.13.0-mainnet`, and the app label is **zerø**. The package remains `app.localwallet.mainnet`.

The installable **`zero-0.13.0-mainnet.apk`** has been built, signed with the original distribution certificate, and verified. It is supplied separately from the repository; signing keys remain private.

- Binary source commit: [`dbfda7a3dca7393dd7d626b5ffbf7b41f4f5e556`](https://github.com/seejovin/zero/commit/dbfda7a3dca7393dd7d626b5ffbf7b41f4f5e556).
- Size: **16,796,124 bytes**.
- SHA-256: `5652b48b7f8323f0918b0a69f0df740468a4338d192b9d61d8f9548394e92de9`.
- Exact binary metadata: [`APK_MANIFEST.json`](APK_MANIFEST.json).
- Install over the existing app; do not uninstall or clear wallet data.

Subsequent release-documentation or CI-only commits do not change this APK. No physical-device installation or upgrade was tested here.

## Included engineering

- zerø display name, launcher artwork and native opening transition, with reduced-motion/accessibility handling and no network dependency.
- Existing vault, account derivation, app ID and encrypted storage preserved.
- Nine-network public-wallet source and 0.12.0 private engine/service implementation carried forward.
- GitHub Actions Android compilation, native libraries, JVM tests, lint and unsigned aligned APK workflow.
- Separate local signing with the existing certificate, rather than storing the distribution key in the repository.
- Tencent operator/gateway and independently operated relay deployment material.

## Private service release boundary

Production private sending remains disabled. The compiled release gates and default unqualified manifest remain in place. Offline proof fixtures do not qualify a deployed contract or a funded service.

Remaining operational requirements include current deployment/verifier qualification, reviewed production configuration, a truly independently operated relay, secret provisioning, a bounded funded treasury, on-device qualification, and end-to-end transfer/recovery evidence. The supplied cloud files do not supply or fabricate these values.

## Verification executed for this release

| Check | Result |
| --- | --- |
| Rust workspace tests | 256 passed; 0 failed |
| Private-engine JavaScript tests | 40 passed; 0 failed |
| Android release JVM tests | 270 passed; 0 failed; 0 skipped |
| Android release lint | Completed; 0 errors, 13 warnings |
| Native libraries | ARM64 and x86-64 compiled from the current source |
| Android release assembly | Successful |
| APK signature | Verified against the original certificate |
| APK inspection | Version/name/package, both ABIs, 16 KiB ZIP/ELF alignment and packaged private-engine asset hashes verified |

Build integration corrected the private-engine asset task declaration, three hidden Android directory-open API uses, and four Android 11-incompatible Solana counter conversions. Directory sync retains no-follow, descriptor type validation, fsync and close. Solana counter narrowing rejects overflow while token quantities retain their full unsigned range. Stale network fixtures now cover the expanded registry. The common fee-publication deadline and historical pre-USDC scanner behavior have regression coverage.

No physical Android device installation, on-device proving, funded private transfer or recovery was executed. The attempted desktop Chromium qualification was blocked by the environment before application assertions ran.

## Clean GitHub build

[GitHub Actions run 42244968958](https://github.com/seejovin/zero/actions/runs/42244968958) completed successfully on source commit `0adb6bc027cb0b33f851eba0ab45e0e2ac58d859`. The clean runner passed Rust/native bridge checks, packaged private computation, Android compilation/tests/lint/assembly, unsigned APK alignment/inspection and artifact uploads.

The workflow copies the runner-provided Node executable with mode `0755` before verifier tests, retaining the verifier's rejection of writable executable files. This CI-only change does not alter the delivered APK's runtime source, identified above.

## Earlier evidence

The previous exact results are retained in `android/verification/v0120-verification.json` and the historical verification report. They cover 251 Rust tests, 40 JavaScript engine tests, real Groth16 proof/ciphertext checks, native bridge checks and packaged-runtime fixtures. They were executed on the earlier source and are identified as historical until rerun for this release.

## Signing continuity

The required distribution certificate SHA-256 is:

`da1632404a817505713e77527ef97543fc6c1028148c401462b96683e2392687`

The delivered APK matches this identity. The signing workflow fails if it cannot prove continuity. No debug key or new signing identity substitutes for the retained certificate. Signature evidence is in [`v0130-apk-signature.txt`](../android/verification/v0130-apk-signature.txt).
