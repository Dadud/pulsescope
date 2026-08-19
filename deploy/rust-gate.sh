#!/usr/bin/env bash
# Run the Rust half of the required gate inside the builder image, which is the
# only Linux toolchain available when developing from Windows. Mount the repo's
# docs/ at /build/docs so the API documentation contract tests can compile.
set -uo pipefail
export PATH=/usr/local/cargo/bin:$PATH
cd /build

rustup component add clippy rustfmt >/dev/null 2>&1 || true
status=0

echo "----- cargo fmt --check -----"
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check || status=1

echo "----- cargo clippy -----"
cargo clippy --manifest-path src-tauri/Cargo.toml \
  --no-default-features --features headless,mock-source \
  --all-targets -- -D warnings 2>&1 | tail -30 || status=1

echo "----- cargo test --lib -----"
cargo test --manifest-path src-tauri/Cargo.toml \
  --no-default-features --features headless,mock-source --lib 2>&1 | tail -30 || status=1

echo "----- gate status ${status} -----"
exit "${status}"
