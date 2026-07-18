# Building and validating PulseScope

## Prerequisites

- Rust stable (installed with rustup), including the `rustfmt` and `clippy` components
- Node.js 20 and pnpm 10
- Docker with BuildKit for constructing the server image
- Windows: WebView2 runtime (preinstalled on Windows 10/11) and MSVC build tools
- macOS: Xcode command-line tools
- Linux (Ubuntu 24.04/Debian): install the same native development packages as CI:

```bash
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  pkg-config libglib2.0-dev libgtk-3-dev libwebkit2gtk-4.1-dev \
  libayatana-appindicator3-dev librsvg2-dev libasound2-dev \
  libsoapysdr-dev patchelf
```

These packages provide Tauri's GTK/WebKitGTK shell, GLib, CPAL's ALSA backend,
SoapySDR for the default desktop feature set, and `pkg-config` discovery.

## First-time setup

```bash
pnpm install --frozen-lockfile
rustup component add rustfmt clippy
```

## Exact local CI equivalents

Run these command groups from the repository root. Pull requests should require
all four named CI jobs in the repository's branch protection rules; a failed or
pending applicable job must block merging.

### Frontend validation

```bash
pnpm install --frozen-lockfile
pnpm check
pnpm build
git diff --check
```

### Headless Rust validation

Install the Linux packages above (SoapySDR is optional for this job), then run:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features
cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --all-targets -- -D warnings
git diff --check
```

### Desktop compilation (Linux)

Install all Linux packages above, including `libsoapysdr-dev`, then run:

```bash
cargo build --manifest-path src-tauri/Cargo.toml --all-targets
git diff --check
```

### Docker image construction

```bash
DOCKER_BUILDKIT=1 docker build --tag pulsescope:ci .
```

## Development and production builds

```bash
pnpm tauri dev
pnpm tauri build
```

The production command writes installers and portable binaries beneath
`src-tauri/target/release/bundle/`.

## Decoder binaries

PulseScope does not bundle GPL decoder binaries by default. Install them
separately and PulseScope will find them on `$PATH`, or set their absolute paths
in **Settings**.

Recommended Windows install (MSYS2):

```bash
pacman -S mingw-w64-x86_64-rtl-sdr mingw-w64-x86_64-multimon-ng \
  mingw-w64-x86_64-direwolf mingw-w64-x86_64-rtl-433 \
  mingw-w64-x86_64-dumpvdl2 mingw-w64-x86_64-acarsdec
```

`dsd-neo` and `rs41mod` are not in MSYS2; clone their upstream repositories and
follow their build instructions.
