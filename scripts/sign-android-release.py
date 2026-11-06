#!/usr/bin/env python3
"""Sign locally with the existing app key and refuse a changed certificate."""

import argparse
import json
import os
from pathlib import Path
import tempfile

from android_release import CERTIFICATE_SHA256, VERSION_NAME, build_tools, inspect_apk, run, sha256, verify_certificate


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", required=True, type=Path)
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--keystore", required=True, type=Path)
    parser.add_argument("--ks-pass-file", required=True, type=Path)
    parser.add_argument("--key-pass-file", type=Path)
    parser.add_argument("--key-alias")
    parser.add_argument("--output", type=Path, default=Path("signed-release"))
    args = parser.parse_args()
    tools = build_tools(args.sdk)
    inspect_apk(args.input, tools)
    metadata_path = args.input.resolve().parent / "unsigned-apk-manifest.json"
    unsigned = json.loads(metadata_path.read_text())
    if unsigned.get("sha256") != sha256(args.input) or unsigned.get("signed") is not False:
        raise ValueError("Unsigned APK does not match its build manifest")
    if unsigned.get("required_certificate_sha256") != CERTIFICATE_SHA256:
        raise ValueError("Build manifest has the wrong signing identity")
    for path in (args.keystore, args.ks_pass_file, args.key_pass_file or args.ks_pass_file):
        if not path.is_file():
            raise ValueError(f"Missing local signing input: {path}")
    args.output.mkdir(parents=True, exist_ok=True)
    destination = args.output.resolve() / f"zero-{VERSION_NAME}.apk"
    if destination.exists():
        raise ValueError("Output APK already exists; choose a new output directory")
    with tempfile.TemporaryDirectory(prefix="zero-sign-", dir=args.output) as temp:
        signed = Path(temp) / destination.name
        command = [tools / "apksigner", "sign", "--ks", args.keystore.resolve(),
                   "--ks-pass", "file:" + str(args.ks_pass_file.resolve()),
                   "--v1-signing-enabled", "false", "--v2-signing-enabled", "true",
                   "--v3-signing-enabled", "true", "--v4-signing-enabled", "false",
                   "--out", signed]
        # With no key password argument apksigner reuses the store password.
        # Passing the same file twice consumes a second line instead of reusing it.
        if args.key_pass_file:
            command.extend(["--key-pass", "file:" + str(args.key_pass_file.resolve())])
        if args.key_alias:
            command.extend(["--ks-key-alias", args.key_alias])
        command.append(args.input.resolve())
        run(command)
        signature = verify_certificate(signed, tools)
        report, badging = inspect_apk(signed, tools)
        report.update(source_commit=unsigned["source_commit"], signed=True,
                      certificate_sha256=CERTIFICATE_SHA256, same_certificate_as_v02=True)
        os.replace(signed, destination)
    (args.output / "APK_MANIFEST.json").write_text(json.dumps(report, indent=2) + "\n")
    (args.output / "apk-signature.txt").write_text(signature)
    (args.output / "apk-badging.txt").write_text(badging)
    (args.output / "SHA256SUMS.txt").write_text(f"{report['sha256']}  {destination.name}\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
