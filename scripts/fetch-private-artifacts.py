#!/usr/bin/env python3
"""Download immutable public proving artifacts for subsequent hash qualification.

This downloads no wallet data, Android toolchain or executable installer. It
does not trust the downloaded verification key or enable private transfers.
Run privacy-engine/scripts/qualify-artifacts.mjs before using these bytes.
"""
import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import re
import tempfile
import urllib.request

CID = "QmUsmnK4PFc7zDp2cmC4wBZxYLjNyRgWfs5GNcJJ2uLcpU"
LIMIT = 256 * 1024 * 1024

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError("Artifact redirects are refused")

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--shapes", default="01x03")
    parser.add_argument("--gateway", choices=["https://ipfs.filebase.io", "https://ipfs-lb.com"],
                        default="https://ipfs.filebase.io")
    args = parser.parse_args()
    shapes = args.shapes.split(",")
    if not shapes or len(shapes) != len(set(shapes)) or any(not re.fullmatch(r"(0[1-9]|1[01])x03", s) for s in shapes):
        parser.error("Only distinct 01x03 through 11x03 circuit shapes are allowed")
    args.output.mkdir(parents=True, exist_ok=True)
    jobs = []
    for shape in shapes:
        jobs.extend([(f"{shape}.wasm.br", f"prover/snarkjs/{shape}.wasm.br"),
                     (f"{shape}.zkey.br", f"circuits/{shape}/zkey.br"),
                     (f"{shape}.vkey.json", f"circuits/{shape}/vkey.json")])

    def fetch(job):
        name, path = job
        target = args.output / name
        if target.exists():
            if not target.is_file() or not 0 < target.stat().st_size <= LIMIT:
                raise ValueError("Existing artifact is invalid")
            return {"file": name, "downloaded": False, "requires_qualification": True}
        opener = urllib.request.build_opener(NoRedirect())
        request = urllib.request.Request(args.gateway + "/ipfs/" + CID + "/" + path,
                                         headers={"Accept": "application/octet-stream, application/json"})
        descriptor, temporary_name = tempfile.mkstemp(prefix=name + ".", suffix=".partial", dir=args.output)
        temporary = Path(temporary_name)
        try:
            with os.fdopen(descriptor, "wb") as output, opener.open(request, timeout=40) as response:
                if response.status != 200:
                    raise ValueError("Artifact download failed")
                count = 0
                while True:
                    chunk = response.read(65_536)
                    if not chunk:
                        break
                    count += len(chunk)
                    if count > LIMIT:
                        raise ValueError("Artifact exceeds size limit")
                    output.write(chunk)
                if not count:
                    raise ValueError("Empty artifact")
                output.flush()
                os.fsync(output.fileno())
            # link() publishes the completed file atomically and refuses to
            # overwrite any file another process created while downloading.
            os.link(temporary, target)
            return {"file": name, "downloaded": True, "compressed_bytes": count, "requires_qualification": True}
        finally:
            temporary.unlink(missing_ok=True)

    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
        for result in executor.map(fetch, jobs):
            print(json.dumps(result), flush=True)

if __name__ == "__main__":
    main()
