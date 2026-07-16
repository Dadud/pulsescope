# PulseScope

**Open-source desktop SDR scanner.** Wideband receiver + multi-VFO scanner UI that
orchestrates the open-source decoder ecosystem (rtl_433, dumpvdl2, acarsdec,
dsd-neo, direwolf, multimon-ng, rs41mod, …) through a local WebSocket API.

PulseScope is a **clean-room reimplementation** of the architecture popularised by
closed-source SDR scanners. It exists because the entire DSP / decoder floor
underneath those tools is already open-source; only the orchestration glue is
proprietary. This project rebuilds the glue.

## Status

Scaffold — API surface and schemas are drafted to mirror the proven ergonomics
of the category, but the scanner core, sidecar launcher, and UI are under
active development. Nothing runnable end-to-end yet.

## What it is (and isn't)

PulseScope is **not** a fork, decompilation, or derivative of any proprietary
binary. It is a new project written from scratch using public SDR protocol
specifications, public upstream decoder sources, and standard Web/Rust stacks.
No code, assets, or wordmarks from any other scanner product appear here.

The decoder binaries (rtl_433, dumpvdl2, etc.) are **GPL sidecar processes** —
PulseScope spawns them and parses their textual output; it does not link them.
The PulseScope core is licensed MIT.

## Stack

- **Rust** (1.75+) + **Tauri 2** — backend, native window, IPC
- **Svelte 5 / SvelteKit** — frontend (spectrum, VFO tiles, message log)
- **SQLite** — scan history, signal events, talkgroups, decoded messages
- **SoapySDR** — device abstraction (RTL-SDR, Airspy, HackRF, bladeRF, SDRPlay,
  PlutoSDR) via the in-tree `soapysdr-sys` bindings; no GPL linkage in core

## Getting started

Requirements: Rust 1.75+, Node 20+, pnpm, an SDR dongle (or a Mock source for
offline testing), and the decoder binaries on your `$PATH` (or point the config
at them on the **Settings** page).

```bash
pnpm install
pnpm tauri dev      # builds Rust + launches webview
```

## Layout

```
pulsescope/
├─ src-tauri/        Rust backend (Tauri + HTTP/WS + scanner core + sidecars)
│  ├─ src/
│  │  ├─ main.rs     entry point + Tauri setup
│  │  ├─ api/        HTTP/WS endpoints (~150, see docs/API.md)
│  │  ├─ scanner/    FFT VFO bank, squelch, scan-range engine (clean-room DSP)
│  │  ├─ device/     SoapySDR device layer
│  │  ├─ sidecar/    process launcher for GPL decoder binaries
│  │  └─ db/         SQLite schema + migrations
│  ├─ migrations/    schema SQL
│  └─ Cargo.toml
├─ ui/               Svelte 5 + SvelteKit frontend
│  └─ src/
│     ├─ lib/        components (spectrum, vfo_tile, talkgroup_panel, ...)
│     └─ routes/     pages (main scanner, settings, cases, trunking, ...)
├─ docs/
│  ├─ API.md         HTTP/WS endpoint contract
│  ├─ SCHEMA.md      SQLite schema
│  └─ PRESETS.md     scan-range preset table (75 defaults, HF→5.8 GHz)
└─ assets/           icons, wordmark (original art)
```

## License

MIT (see `LICENSE`). GPL decoder binaries are separate processes invoked at
runtime, never linked into the PulseScope binary — the same containment
boundary documented in `THIRD_PARTY_NOTICES.md`.

## Acknowledgements

PulseScope is built on the shoulders of the SDR open-source community. The
decoder ecosystem — `rtl_433`, `dumpvdl2`, `acarsdec`, `dsd-neo`,
`direwolf`, `multimon-ng`, `rs41mod`, `nrsc5`, `dump978`, `rtl_tcp`,
`librtlsdr` — is the actual hard part and is the work of dozens of upstream
maintainers. Key frameworks: Rust, Tauri, Svelte, SoapySDR.
