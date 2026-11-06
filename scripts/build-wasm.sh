#!/usr/bin/env bash
set -euo pipefail
wallet_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$wallet_root"
export PATH="$HOME/.cargo/bin:$PATH"
rustup target add wasm32-unknown-unknown
if ! command -v wasm-bindgen >/dev/null || [[ "$(wasm-bindgen --version)" != "wasm-bindgen 0.2.130" ]]; then
  cargo install wasm-bindgen-cli --version 0.2.130 --locked
fi
RUSTFLAGS='--cfg getrandom_backend="wasm_js"' cargo build --locked -p wallet_core --lib --release --target wasm32-unknown-unknown --no-default-features --features wasm
mkdir -p build/wasm
wasm-bindgen --target web --out-dir build/wasm --out-name wallet_core target/wasm32-unknown-unknown/release/wallet_core.wasm
