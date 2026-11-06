#!/usr/bin/env bash
set -euo pipefail
WALLET_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$WALLET_ROOT"
case "$(uname -s)" in
  Darwin) WALLET_LIBRARY=libwallet_core.dylib ;;
  Linux) WALLET_LIBRARY=libwallet_core.so ;;
  *) printf '%s\n' 'This host bridge check supports macOS and Linux.' >&2; exit 1 ;;
esac
WALLET_BRIDGE_DIRECTORY="$WALLET_ROOT/build/bridge-check"
mkdir -p "$WALLET_BRIDGE_DIRECTORY"
cargo build --locked -p wallet_core
cargo run --locked -p wallet_core --features bindgen --bin uniffi-bindgen -- \
  generate --library "target/debug/$WALLET_LIBRARY" --language python \
  --out-dir "$WALLET_BRIDGE_DIRECTORY" --config core/uniffi.toml
cp "target/debug/$WALLET_LIBRARY" "$WALLET_BRIDGE_DIRECTORY/$WALLET_LIBRARY"
python3 scripts/check-bridge.py --bindings "$WALLET_BRIDGE_DIRECTORY"
cargo run --locked --quiet --example solana_differential_vectors > "$WALLET_BRIDGE_DIRECTORY/solana-vectors.json"
python3 scripts/check-family-bridge.py --bindings "$WALLET_BRIDGE_DIRECTORY" \
  --solana-vectors "$WALLET_BRIDGE_DIRECTORY/solana-vectors.json"
python3 scripts/check-private-bridge.py --bindings "$WALLET_BRIDGE_DIRECTORY"

