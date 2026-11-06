#!/usr/bin/env python3
"""Package a checked, aligned unsigned APK; signing credentials stay local."""

import argparse
import json
from pathlib import Path
import re
import shutil

from android_release import CERTIFICATE_SHA256, VERSION_NAME, build_tools, inspect_apk, run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", required=True, type=Path)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--apk", type=Path, default=Path("android/app/build/outputs/apk/release/app-release-unsigned.apk"))
    parser.add_argument("--output", type=Path, default=Path("build/android-release"))
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9a-f]{40}", args.source_commit):
        parser.error("--source-commit must be the complete reviewed Git commit SHA")
    tools = build_tools(args.sdk)
    args.output.mkdir(parents=True, exist_ok=True)
    apk = args.output.resolve() / f"zero-{VERSION_NAME}-unsigned.apk"
    run([tools / "zipalign", "-P", "16", "-f", "4", args.apk.resolve(), apk])
    report, badging = inspect_apk(apk, tools)
    report.update(source_commit=args.source_commit, signed=False, required_certificate_sha256=CERTIFICATE_SHA256)
    (args.output / "unsigned-apk-manifest.json").write_text(json.dumps(report, indent=2) + "\n")
    (args.output / "apk-badging.txt").write_text(badging)
    (args.output / "SHA256SUMS.txt").write_text(f"{report['sha256']}  {apk.name}\n")
    for name in ("android_release.py", "sign-android-release.py"):
        shutil.copy2(Path(__file__).with_name(name), args.output / name)
    (args.output / "SIGNING.md").write_text(
        "# Complete the zerø Android release\n\n"
        "This APK is unsigned and cannot be installed yet. Use the original private app-signing backup. "
        "Do not upload the keystore or its password to GitHub. The signer verifies the APK, checks the existing "
        "certificate and writes the installable APK only after verification succeeds.\n\n"
        "```bash\npython3 sign-android-release.py \\\n"
        "  --sdk /absolute/path/to/android-sdk \\\n"
        f"  --input {apk.name} \\\n"
        "  --keystore /private/path/local-wallet-development.p12 \\\n"
        "  --ks-pass-file /private/path/keystore-password.txt \\\n"
        "  --output signed-release\n```\n\n"
        "The optional `--key-alias` selects an alias when a keystore contains more than one. "
        "The keystore password is also the key password unless `--key-pass-file` is provided.\n\n"
        "Install the resulting signed APK over the existing app. Do not uninstall or clear wallet data. "
        "Private sending remains unavailable until its separate production qualification is complete.\n"
    )
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
