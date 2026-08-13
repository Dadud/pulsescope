# PulseScope

Current implementation and verification status is generated from the release contract: [docs/FEATURE_STATUS.md](docs/FEATURE_STATUS.md). Beta features are labeled with their remaining acceptance gate; only physically tested hardware is described as hardware verified.

Before first appliance startup, run `docker compose --profile preflight run --rm preflight`. Persistent configuration and calibration can be exported with `sh deploy/backup.sh`; `deploy/restore.sh` requires an explicit `RESTORE` confirmation and restarts only the PulseScope service.

**Open-source desktop and headless SDR scanner.** Wideband receiver + multi-VFO
scanner UI that orchestrates open-source decoders (rtl_433, dsd-fme, native
DSP, ...) through a local HTTP/WebSocket API. Runs as a Windows desktop app,
a headless Linux/Windows server, or inside Docker.

PulseScope is a clean-room reimplementation: it talks to public decoder sources
through well-defined sidecar boundaries, ships its own native Rust DSP
(APRS/AX.25, CTCSS, DCS, RDS, CW, DTMF), and exposes a documented HTTP/WS API.
Compare its feature coverage against the category leaders in
`NYXSCOPE_PARITY_MATRIX.md`.

## Status — verified working today

- ✅ **Desktop app** (Windows MSI/NSIS installers, Tauri 2 webview, embedded UI)
- ✅ **Headless server** (`--server` / `--api` flag, optional bearer auth, rustls
  TLS via `PULSESCOPE_TLS_CERT` / `PULSESCOPE_TLS_KEY`)
- ✅ **SDRplay RSP1B** end-to-end through SoapySDR (discovery, CF32 acquisition,
  retune, sample-rate rebuild, teardown, reconnect)
- ✅ **Native DSP decoders**: CTCSS (50 EIA tones), DCS, RDS (57 kHz BPSK),
  CW/Morse (ITU), DTMF (16 digits), **APRS/AX.25** (1200 baud AFSK with
  Goertzel mark/space, NRZ-I slicer, HDLC frame parser, full AX.25 UI frame
  parser)
- ✅ **dsd-fme sidecar** wired for digital voice: **P25 Phase 1/2, DMR,
  NXDN48/96, D-STAR, YSF, M17, ProVoice** (real `dsd-fme.exe` from lwvmobile
  build, runs against live RSP1B at 853 MHz)
- ✅ **rtl_433 sidecar** with valid u8-IQ transport and JSON parser
- ✅ **Spectrum + waterfall** UI, live mini-spectrum per VFO, signal-event
  persistence, SQLite WAL database
- ✅ **IQ recording** (CF32, real-verified 4M samples ≈ 32 MB)
- ✅ **IQ playback** (interleaved little-endian CF32 to EOF through the shared
  capture/scanner path)
- ✅ **UDP streaming** (PSAU audio + PSIQ IQ with magic/version headers)
- ✅ **79 default scan ranges** from AM Broadcast through GOES HRIT/LRIT
- ✅ **Docker multi-stage build** + systemd unit for headless deployment
- ✅ **35 unit tests passing**

See `NYXSCOPE_PARITY_MATRIX.md` for honest feature-by-feature classification
(what's wired, what's partial, what's blocked, what's deliberately unavailable).

## Quick start — server mode

```bash
# 1. build
cargo build --release --features soapysdr --manifest-path src-tauri/Cargo.toml

# 2. discover & connect RSP1B (or swap for any SoapySDR-compatible SDR)
./target/release/pulsescope.exe --server
# → bound to 127.0.0.1:8765

curl http://127.0.0.1:8765/health
# {"name":"pulsescope","status":"ok","version":"0.1.0"}

# 3. open the UI
xdg-open http://127.0.0.1:8765/        # Linux / WSL
start http://127.0.0.1:8765/           # Windows
```

With auth and TLS:
```bash
PULSESCOPE_AUTH_TOKEN=$(openssl rand -hex 24) \
PULSESCOPE_TLS_CERT=/etc/pulsescope/cert.pem \
PULSESCOPE_TLS_KEY=/etc/pulsescope/key.pem \
PULSESCOPE_BIND=0.0.0.0 \
./target/release/pulsescope.exe --server --port 8443
# → https://0.0.0.0:8443/  (UI + API)
# In the browser, append ?token=<your-token> once; it's cached in localStorage.
```

## Quick start — desktop app

```bash
pnpm install
pnpm tauri dev            # hot-reload development
pnpm tauri build          # produces MSI + NSIS installers in src-tauri/target/release/bundle/
```

## Quick start — Docker

```bash
docker build -t pulsescope .
docker run --rm -p 8765:8765 \
  -v /dev/bus/usb:/dev/bus/usb \
  --device-cgroup-rule='c 189:* rwm' \
  pulsescope
```

## Where decoders live

PulseScope expects decoders on `$PATH` or in `<exe-dir>/decoders/`. Currently
verified:

| Decoder      | Source / Binary                                  | Transport                          | Status              |
| ------------ | ------------------------------------------------ | ---------------------------------- | ------------------- |
| `rtl_433`    | PothosSDR 0.8.1                                  | Sidecar, u8 IQ via stdin/stdout    | WIRED, REAL-TESTED  |
| `rtl_adsb`   | PothosSDR                                        | (binary available, transport TBD)  | Available           |
| `dsd-fme`    | `/c/Users/Dadud/pulsescope/decoders/dsd-fme/`    | Sidecar, 48 kHz mono WAV via fs    | WIRED, REAL-TESTED  |
| `AIS-catcher`| jvde-github v0.70                                | (binary available, transport TBD)  | Installed, exclusive SDR |

Multimon-ng, direwolf, acarsdec, nrsc5, dumpvdl2, dump978 source tarballs are
checked into `decoders/` (GPL redistribution permitted by upstream authors) but
require a `cmake + ninja/make` build step on the host. The PulseScope daemon
finds them on `$PATH` once built; until then the corresponding endpoints
return explicit "decoder unavailable" responses.

## Layout

```
pulsescope/
├─ src-tauri/                Rust backend (server + scanner core + sidecars)
│  ├─ src/
│  │  ├─ main.rs            dual-mode entry: desktop or --server
│  │  ├─ api.rs             ~150 HTTP/WS routes, auth middleware, TLS bind
│  │  ├─ scanner.rs         capture/audio workers, VFO DSP, signal detection
│  │  ├─ demod.rs           native demodulators + tone decoders
│  │  ├─ aprs.rs            native AFSK 1200 APRS/AX.25 decoder
│  │  ├─ voice_decoder.rs   dsd-fme sidecar (digital voice)
│  │  ├─ sidecar.rs         rtl_433 sidecar (ISM sensors)
│  │  ├─ depend_manager.rs  cross-platform sidecar discovery
│  │  ├─ device.rs          SoapySDR hardware lifecycle
│  │  ├─ capture.rs         bounded IQ rings + CaptureWorker + playback
│  │  ├─ audio.rs           CPAL output + UDP PSAU streamer
│  │  └─ state.rs / db.rs   shared runtime state + SQLite WAL
│  ├─ migrations/001_init.sql
│  └─ Cargo.toml
├─ ui/                       Svelte 5 + SvelteKit (hash-router SPA)
│  └─ src/routes/           19 pages: scanner, trunking, deps, recording, ...
├─ decoders/                 (git-ignored) downloaded GPL sidecar binaries
├─ docs/
│  ├─ API.md                HTTP/WS endpoint contract
│  ├─ SCHEMA.md             SQLite schema
│  ├─ PRESETS.md            scan-range preset table
│  ├─ PARSERS.md            decoder transport recipes
│  └─ BUILD.md              platform build matrix
├─ Dockerfile
├─ contrib/systemd/pulsescope.service
└─ NYXSCOPE_PARITY_MATRIX.md  honest feature coverage comparison
```

## Configuration (env vars — server mode)

| Variable                | Default        | Notes                                        |
| ----------------------- | -------------- | -------------------------------------------- |
| `PULSESCOPE_BIND`       | `127.0.0.1`    | Listen addr. `0.0.0.0` for public           |
| `PULSESCOPE_PORT`       | `8765`         | HTTP listen port                             |
| `PULSESCOPE_AUTH_TOKEN` | _unset_        | If set, bearer token required on all `/...` |
| `PULSESCOPE_TLS_CERT`   | _unset_        | PEM cert bytes → enables rustls HTTPS        |
| `PULSESCOPE_TLS_KEY`    | _unset_        | PEM key bytes                                |
| `PULSESCOPE_API_ONLY`   | _unset_        | Skip UI static mount; API-only server        |
| `PULSESCOPE_UI_DIR`     | auto-detect    | Override static UI directory                 |
| `SOAPY_SDR_ROOT`        | _unset_        | PothosSDR install root for bindings          |
| `PULSESCOPE_SOAPY_UTIL` | auto-detect    | Path to `SoapySDRUtil.exe`                   |

## License

The PulseScope core is MIT (see `LICENSE`). Native decoders written here
(APRS/AX.25, CTCSS, DCS, RDS, CW, DTMF, voice decoder scaffold) are MIT.

Sidecar decoders invoked at runtime (rtl_433, dsd-fme, ...) are GPL and remain
separate processes. PulseScope communicates with them through stdin/stdout and
serialised JSON / WAV — it does not link them. See
`docs/THIRD_PARTY_NOTICES.md` for the full inventory.

## Acknowledgements

PulseScope is built on the shoulders of the SDR open-source community. The
decoder ecosystem is the actual hard part and is the work of dozens of
upstream maintainers — see `NYXSCOPE_PARITY_MATRIX.md` for the upstream
attribution of each protocol covered.
