#!/usr/bin/env bash
set -uo pipefail
export PATH=/usr/local/cargo/bin:$PATH
cd /build
rustup component add clippy rustfmt >/dev/null 2>&1 || true
status=0
echo "----- cargo test lora:: -----"
cargo test --manifest-path src-tauri/Cargo.toml \
  --no-default-features --features headless,mock-source --lib \
  lora:: 2>&1 | tail -80 || status=1
echo "----- cargo test decoder_fixtures::tests::canonical -----"
cargo test --manifest-path src-tauri/Cargo.toml \
  --no-default-features --features headless,mock-source --lib \
  decoder_fixtures::tests::canonical 2>&1 | tail -80 || status=1
echo "----- gate status ${status} -----"
exit "${status}"
