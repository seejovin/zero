"""Release checks shared by the unsigned packager and local signing command."""

import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import struct
import subprocess
import zipfile


PACKAGE = "app.localwallet.mainnet"
VERSION_NAME = "0.13.0-mainnet"
VERSION_CODE = 16
CERTIFICATE_SHA256 = "da1632404a817505713e77527ef97543fc6c1028148c401462b96683e2392687"
ABIS = {"arm64-v8a": 183, "x86_64": 62}


def sha256(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def run(command):
    result = subprocess.run([str(item) for item in command], capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f"{Path(str(command[0])).name} failed:\n{result.stdout}{result.stderr}")
    return result.stdout + result.stderr


def build_tools(sdk):
    path = Path(sdk).resolve() / "build-tools" / "35.0.0"
    for tool in ("aapt", "zipalign", "apksigner"):
        if not (path / tool).is_file():
            raise ValueError(f"Missing Android build tool: {path / tool}")
    return path


def check_elf(data, machine):
    if len(data) < 64 or data[:7] != b"\x7fELF\x02\x01\x01":
        raise ValueError("Native library is not a little-endian 64-bit ELF")
    if struct.unpack_from("<H", data, 18)[0] != machine:
        raise ValueError("Native library architecture does not match its ABI folder")
    offset = struct.unpack_from("<Q", data, 32)[0]
    entry_size, count = struct.unpack_from("<HH", data, 54)
    if entry_size != 56 or count < 1 or offset + count * entry_size > len(data):
        raise ValueError("Invalid ELF program header table")
    alignments = []
    for index in range(count):
        segment = struct.unpack_from("<IIQQQQQQ", data, offset + index * entry_size)
        if segment[0] == 1:  # PT_LOAD
            alignment = segment[7]
            if alignment < 16384 or alignment & (alignment - 1):
                raise ValueError("Native library is not aligned for 16 KiB memory pages")
            if (segment[2] - segment[3]) % alignment:
                raise ValueError("Native library has inconsistent ELF load alignment")
            alignments.append(alignment)
    if not alignments:
        raise ValueError("Native library has no loadable segments")
    return alignments


def inspect_apk(apk, tools):
    apk = Path(apk).resolve()
    badging = run([tools / "aapt", "dump", "badging", apk])
    package = re.search(r"^package: name='([^']+)' versionCode='([^']+)' versionName='([^']+)'", badging, re.M)
    if not package or package.groups() != (PACKAGE, str(VERSION_CODE), VERSION_NAME):
        raise ValueError("APK package or version does not match this release")
    label = re.search(r"^application-label:'([^']+)'", badging, re.M)
    if not label or label.group(1) != "zerø":
        raise ValueError("APK display name is not zerø")
    if "application-debuggable" in badging:
        raise ValueError("A debuggable APK cannot be distributed as this release")
    if not re.search(r"^sdkVersion:'30'$", badging, re.M):
        raise ValueError("Unexpected minimum Android API")
    if not re.search(r"^targetSdkVersion:'35'$", badging, re.M):
        raise ValueError("Unexpected target Android API")
    run([tools / "zipalign", "-c", "-P", "16", "4", apk])
    native = {}
    with zipfile.ZipFile(apk) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError("APK contains duplicate ZIP entries")
        for name in names:
            if name.startswith("lib/") and name.endswith(".so"):
                parts = PurePosixPath(name).parts
                if len(parts) != 3 or parts[1] not in ABIS:
                    raise ValueError(f"Unexpected native library: {name}")
                native[name] = check_elf(archive.read(name), ABIS[parts[1]])
        for abi in ABIS:
            if f"lib/{abi}/libwallet_core.so" not in native:
                raise ValueError(f"Missing freshly compiled wallet core for {abi}")
        prefix = "assets/private-engine/"
        manifest_bytes = archive.read(prefix + "manifest.json")
        manifest = json.loads(manifest_bytes)
        if manifest.get("schema_version") != 1 or manifest.get("entry") != "index.html":
            raise ValueError("Invalid packaged private-engine asset manifest")
        checked = set()
        for entry in manifest["files"]:
            name = entry["path"]
            parts = PurePosixPath(name).parts
            if not parts or name.startswith("/") or any(p in (".", "..") for p in parts) or name in checked:
                raise ValueError("Invalid or duplicate private-engine asset path")
            checked.add(name)
            payload = archive.read(prefix + name)
            if len(payload) != entry["bytes"] or hashlib.sha256(payload).hexdigest() != entry["sha256"]:
                raise ValueError(f"Private-engine asset hash mismatch: {name}")
        if not {"index.html", "engine.js", "third-party-notices.txt"}.issubset(checked):
            raise ValueError("Packaged private engine is incomplete")
    return {
        "apk": apk.name,
        "bytes": apk.stat().st_size,
        "sha256": sha256(apk),
        "package_id": PACKAGE,
        "display_name": "zerø",
        "version_code": VERSION_CODE,
        "version_name": VERSION_NAME,
        "debuggable": False,
        "minimum_android_api": 30,
        "target_android_api": 35,
        "architectures": sorted(ABIS),
        "zip_alignment_bytes": 16384,
        "native_load_alignments": native,
        "private_engine_asset_manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(),
        "private_engine_checked_assets": len(checked),
        "private_transfer_qualification": "not qualified for production",
    }, badging


def verify_certificate(apk, tools):
    evidence = run([tools / "apksigner", "verify", "--verbose", "--print-certs", apk])
    certificates = re.findall(r"^Signer #\d+ certificate SHA-256 digest: ([0-9a-fA-F]+)$", evidence, re.M)
    if [value.lower() for value in certificates] != [CERTIFICATE_SHA256]:
        raise ValueError("APK signing certificate does not match the existing installed wallet")
    return evidence
