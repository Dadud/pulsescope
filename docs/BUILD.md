# Supported build and release matrix

| OS / package | Architecture | Status |
| --- | --- | --- |
| Windows 10 22H2, Windows 11 | x86-64 | MSI and NSIS CI targets; acceptance pending |
| Ubuntu 24.04 LTS | x86-64, ARM64 | Headless CI targets; acceptance pending |
| Ubuntu 24.04 desktop | x86-64 | DEB and AppImage CI targets; acceptance pending |
| Debian 12 container | x86-64, ARM64 | Docker CI targets; acceptance pending |

Other distributions, Windows ARM64, macOS, and 32-bit systems are unsupported
for 1.0. A build target is not a completeness claim; the acceptance matrix is.

## SDR drivers

Hardware release binaries use SoapySDR. Operators supply the appropriate Soapy
module. The 1.0 gate requires one named device/module to pass discovery,
acquisition, retune, teardown, and reconnect; that row is currently pending. Mock
mode (`--no-default-features --features mock-source`) is portable CI coverage.
Drivers not exposed through SoapySDR are unsupported.

## Build dependencies

- Rust 1.75+ (rustup)
- Node 20+, pnpm
- Windows: WebView2 runtime (preinstalled on Win10/11), MSVC build tools
- Linux: webkit2gtk, libgtk-3, libayatana-appindicator3-dev

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

CI builds Windows MSI/NSIS, Ubuntu DEB/AppImage, x86-64 and ARM64 headless
binaries, and Debian Docker images. The headless smoke uses a data path containing
whitespace and non-ASCII text. Upgrade, rollback, interactive NSIS lifecycle, and
data preservation remain pending until clean release runners record evidence.

## Release metadata

The release workflow generates a CycloneDX Rust SBOM, pnpm dependency inventory,
and consolidated Rust/third-party license bundle. Publication fails if metadata
generation or any required acceptance check fails.
