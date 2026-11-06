#!/usr/bin/env bash
set -euo pipefail
wallet_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$wallet_root"
./scripts/build-wasm.sh
npm --prefix web ci --ignore-scripts --no-fund --no-audit
npm --prefix web run typecheck
npm run build
./web/node_modules/.bin/tsc -p worker/tsconfig.json
node scripts/verify-hosted.mjs
