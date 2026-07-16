# Build dependencies

- Rust 1.75+ (rustup)
- Node 20+, pnpm
- Windows: WebView2 runtime (preinstalled on Win10/11), MSVC build tools
- Linux: webkit2gtk, libgtk-3, libayatana-appindicator3-dev
- macOS: Xcode command-line tools

# First-time setup

```bash
pnpm install
```

# Development (hot reload)

```bash
pnpm tauri dev
```

Starts Vite dev server on http://localhost:1420 and the Rust backend.

# Production build

```bash
pnpm tauri build
```

Outputs installers + portable binaries under
`src-tauri/target/release/bundle/`.

# Decoder binaries

PulseScope does not bundle GPL decoder binaries by default. Install them
separately and PulseScope will find them on `$PATH`, or point **Settings**
at the absolute paths.

Recommended Windows install (MSYS2):

```bash
pacman -S mingw-w64-x86_64-rtl-sdr mingw-w64-x86_64-multimon-ng \
          mingw-w64-x86_64-direwolf mingw-w64-x86_64-rtl-433 \
          mingw-w64-x86_64-dumpvdl2 mingw-w64-x86_64-acarsdec
```

dsd-neo and rs41mod are not in MSYS2 — clone the upstream repos and build
per their READMEs.

# Tests

```bash
cargo test --lib
pnpm check
```
