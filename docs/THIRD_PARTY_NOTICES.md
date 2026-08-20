# Third-Party Notices

PulseScope incorporates the following third-party components. Each is listed
with its upstream project and license. The PulseScope core application code
is licensed under the MIT license (see `../LICENSE`).

## Decoder sidecars

PulseScope does **not** link any GPL code into the main binary. Each decoder
below is invoked as a separate child process (stdin/stdout or localhost
socket); this subprocess boundary is the project's GPL containment line.

| Component  | Purpose                                  | Upstream                                  | License            |
|------------|------------------------------------------|-------------------------------------------|--------------------|
| multimon-ng| Paging, tones, EAS, AFSK                 | https://github.com/EliasOenal/multimon-ng | GPL-2.0-or-later   |
| rtl_433    | ISM-band sensors and utility meters      | https://github.com/merbanan/rtl_433       | GPL-2.0-or-later   |
| direwolf   | APRS / AX.25 decode                      | https://github.com/wb2osz/direwolf        | GPL-2.0            |
| nrsc5      | HD Radio / NRSC-5 decode                 | https://github.com/theori-io/nrsc5        | GPL-3.0            |
| rs41mod    | RS41 radiosonde decode                   | https://github.com/rs1729/RS              | GPL-3.0            |
| SatDump    | GOES HRIT/LRIT and multi-satellite decode | https://github.com/SatDump/SatDump        | GPL-3.0            |
| dump978    | UAT 978 MHz ADS-B decode                 | https://github.com/mutability/dump978     | GPL-2.0            |
| dumpvdl2   | VDL Mode 2 decode                        | https://github.com/szpajder/dumpvdl2      | GPL-3.0            |
| acarsdec   | ACARS decode                             | https://github.com/TLeconte/acarsdec      | GPL-2.0-or-later   |
| dsd-neo    | P25 / DMR / NXDN digital voice decode    | https://github.com/arancormonk/dsd-neo    | GPL-3.0-or-later   |
| rtl_tcp    | RTL-SDR device access (IQ server)        | https://github.com/librtlsdr/librtlsdr    | GPL-2.0-or-later   |

These binaries are **not** redistributed with PulseScope by default. Users
install them separately (e.g. via MSYS2, PothosSDR, or the upstream release
pages) and PulseScope invokes whichever it finds on `$PATH` or via configured
paths in **Settings**. Per GPL §3(a), corresponding source for each is
available at the upstream URL.

PulseScope implements its own client adapters for rtl_tcp, Airspy SpyServer 2.x / SDR++ framing, PulseScope PSIQ `raw_udp`, and KA9Q radiod RTP/s16le UDP. Those adapters do not link or redistribute SpyServer, ka9q-radio, or librtlsdr. KiwiSDR is not implemented.

## SDR device abstraction

| Component | Purpose                       | Upstream                            | License            |
|-----------|-------------------------------|-------------------------------------|--------------------|
| SoapySDR  | SDR device abstraction layer  | https://github.com/pothosware/SoapySDR | Boost Software 1.0 |

SoapySDR is linked dynamically and provides RTL-SDR, Airspy, HackRF,
bladeRF, SDRPlay, and PlutoSDR access through per-vendor modules. It is
**not** GPL — the Boost License is permissive and compatible with the MIT
core.

## Application frameworks

| Component | Purpose                    | Upstream                       | License        |
|-----------|----------------------------|--------------------------------|----------------|
| Rust      | Core language + toolchain  | https://www.rust-lang.org      | Apache-2.0/MIT |
| Tauri     | Desktop app shell + IPC    | https://tauri.app              | Apache-2.0/MIT |
| Svelte    | Frontend UI framework      | https://svelte.dev             | MIT            |
| axum      | HTTP/WebSocket server      | https://github.com/tokio-rs/axum | MIT          |
| tokio     | Async runtime              | https://tokio.rs               | MIT            |
| rusqlite  | SQLite bindings            | https://github.com/rusqlite/rusqlite | MIT       |
| rustfft   | FFT primitives             | https://github.com/ejmahler/RustFFT | MIT        |
| realfft   | Real-input FFT wrapper     | https://github.com/HEnquist/realfft | MIT        |

A full per-binary dependency manifest is generated at build time in
`Cargo.lock`.

## License summary

- PulseScope core (this repository): **MIT**
- Sidecar decoders: **GPL-2.0+ / GPL-3.0** (separate processes, not linked)
- SoapySDR device layer: **Boost Software License 1.0** (permissive)
- All other Rust/Svelte dependencies: permissive (MIT / Apache-2.0 / BSD)
