#!/usr/bin/env bash
set -euo pipefail
WALLET_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$WALLET_ROOT"
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
npm ci --ignore-scripts --no-fund --no-audit
npm run test:differential
./scripts/verify-bridge.sh
if command -v cargo-audit >/dev/null 2>&1; then
  cargo audit
else
  printf '%s\n' 'Dependency audit not run: install cargo-audit, then run cargo audit.' >&2
  exit 1
fi
npm audit

