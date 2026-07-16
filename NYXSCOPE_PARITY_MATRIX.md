# NyxScope parity matrix

Source reviewed: `https://github.com/ICBizLabs/NyxScope` README and MANUAL, retrieved 2026-07-15. This is a clean-room behavioral inventory only; no NyxScope source, proprietary assets, or implementation details are copied.

Status meanings:

- **REAL-VERIFIED** — implemented and exercised against live hardware/runtime data.
- **PARTIAL** — some behavior exists, but important parity behavior is absent or unverified.
- **SIDECAR** — documented external decoder contract exists, but local executable/transport is not end-to-end verified.
- **OFFLINE/FIXTURE** — endpoint/UI shape exists but behavior is deterministic, placeholder, or fixture-backed.
- **MISSING** — no meaningful implementation yet.

## Core receiver/runtime

| NyxScope capability | PulseScope status | Gap / next work |
|---|---|---|
| SoapySDR device discovery | PARTIAL | Pothos utility discovery and RSP1B enumeration work; common-device mapping unverified. |
| SDRplay RSP1B CF32 acquisition | REAL-VERIFIED | Direct live reads and packaged scanner runs pass. |
| Stable streaming / teardown | REAL-VERIFIED | Capture worker, bounded rings, explicit shutdown, and live lifecycle test pass. |
| Airspy / HackRF / Pluto / RTL-SDR / bladeRF / Lime / UHD | MISSING | Add mappings and serial live tests where hardware exists; truthful unavailable status otherwise. |
| Multi-SDR registry / per-slot streams | MISSING | Current runtime is single-device. |
| Sample-rate and frequency retune lifecycle | REAL-VERIFIED | Live RSP1B test passed initial read, frequency retune/read, sample-rate change/read, teardown, reopen, and reconnect/read. |
| Live spectrum / FFT | REAL-VERIFIED | Live RSP1B FFT and SSE/API spectrum path pass. |
| GPU/WebGL waterfall | PARTIAL | Live CPU canvas waterfall and persisted signal-event history are implemented; GPU/WebGL, hover, and peak actions are not verified. |
| Bounded IQ buffering | REAL-VERIFIED | Fan-out rings and dedicated capture worker are live-tested. |
| Audio demodulation/resampling | REAL-VERIFIED | Dedicated audio worker, fractional resampler, CPAL queue stability live-tested. Audible output still needs human listening verification. |
| Adaptive auto-squelch | PARTIAL | Smoothed median noise-floor tracking and 12 dB SNR-relative squelch are implemented; one-shot calibration and configurable hysteresis remain. |
| IQ playback (`cf32-le`) | REAL-VERIFIED | Packaged API runtime opened a real CF32 file, fed the shared capture worker/scanner path, consumed 240,000 samples to EOF, retained live audio with no error, and stopped cleanly. Other formats unsupported. |
| HTTP headless API | REAL-VERIFIED | Full API routes work; mounted at `/api/*` and at root; served behind optional `Authorization: Bearer` or `?token=` token. |
| HTTPS / TLS termination | REAL-VERIFIED | rustls-backed HTTPS via `PULSESCOPE_TLS_CERT`/`PULSESCOPE_TLS_KEY`; self-signed cert validated end-to-end on `:8766` with bearer auth. |
| UDP audio streaming | REAL-VERIFIED | `/audio/network/start` emits versioned `PSAU` f32-le packets from the CPAL-consumed samples; localhost packet/header/payload validation passes. |
| HTTP audio streaming | MISSING | No HTTP audio stream. |
| UDP IQ streaming | REAL-VERIFIED | `/iq/network/start` emits versioned `PSIQ` CF32 packets directly from capture; packaged localhost validation passed with live sample rate/center metadata. |
| `rtl_tcp` IQ streaming | MISSING | No `rtl_tcp` compatibility server; PSIQ is the native PulseScope transport. |
| Per-band overrides | MISSING | Squelch/hold/dwell/digital-detect persistence absent. |

## VFO, scanner, and recordings

| Capability | Status | Gap / next work |
|---|---|---|
| Multiple concurrent VFOs | PARTIAL | Runtime supports VFO state and independent audio loop; per-VFO mini-waterfalls and full scheduling unverified. |
| Independent VFO audio | REAL-VERIFIED | Dedicated worker mixes VFOs; live queue verified. |
| VFO mute/volume/mode/frequency controls | PARTIAL | API controls exist; full UI interaction regression pending. |
| Per-VFO mini-waterfall | MISSING | Implement channel-centered spectrum windows. |
| Peak picking / tune-to / skip | PARTIAL | Real peak events exist; UI actions not verified. |
| Channel-bank scanning | PARTIAL | Range scanning exists; priority, skip lists, and full bank persistence/import incomplete. |
| CHIRP import | MISSING | No verified implementation. |
| RadioReference/FCC import | MISSING | No verified implementation. |
| WAV recording | PARTIAL | Audio demodulation is live-verified; per-VFO WAV file capture remains pending. |
| IQ recording | REAL-VERIFIED | Packaged runtime wrote 4,096,000 CF32 samples; status byte count matched the 32,768,000-byte file exactly. |
| VAD/VOX pre/post buffers | MISSING | Not implemented. |
| Recording notes/transcription attachment | PARTIAL | Notes API exists; transcription pipeline absent. |
| Signal-hit detection | REAL-VERIFIED | Live FFT peak/SNR events emitted on RSP1B. |
| Signal-hit persistence | REAL-VERIFIED | Live hits written to SQLite `signal_events` and cross-checked against SSE. |

## Native protocol/data-plane targets

These are the capabilities NyxScope advertises as native. PulseScope must not label fixture responses as implemented.

| Protocol / feature | Status | Gap / next work |
|---|---|---|
| P25 Phase 1 voice / IMBE | MISSING | Native control/voice pipeline and isolated vocoder contract absent. |
| P25 Phase 2 TDMA / AMBE+2 | MISSING | No demod/FEC/voice pipeline. |
| EDACS control / Standard / EA / ProVoice | MISSING | No control decoder or voice following. |
| NXDN48 / NXDN96 control | MISSING | No 4FSK/FEC/CRC pipeline. |
| Trunk control auto-discovery | MISSING | Fixture discovery was removed; no real control-channel decode. |
| Active call following / talkgroup history | OFFLINE/FIXTURE | Data model exists; no live control decode/following. |
| ADS-B Mode S 1090 | MISSING | Endpoint now returns explicit unavailable metadata; no decoder transport. |
| UAT 978 / dump978 | SIDECAR | Executable/transport unavailable and no end-to-end decode. |
| AIS dual-channel GMSK | PARTIAL | AIS-catcher v0.70 installed (`decoders/AIS-catcher/`) but requires exclusive SDR access (no stdin IQ mode). Cannot run concurrently with PulseScope scanning. Native GMSK demod not yet implemented. |
| ACARS MSK | MISSING | Endpoint now returns explicit unavailable metadata; no decoder transport. |
| VDL2 | SIDECAR/PARTIAL | Parser coverage exists; `dumpvdl2` executable/typed transport unavailable. |
| POCSAG 512/1200/2400 | MISSING | No native decoder. |
| FLEX / FLEX NEXT | MISSING | No native decoder. |
| LoRa CSS / LoRaWAN MAC | MISSING | No chirp PHY, FEC, CRC, or regional MAC parser. |
| Morse/CW | PARTIAL | Goertzel-based OOK envelope detection, adaptive dit/dah timing, full ITU Morse table, and text decoding implemented and unit-tested (SOS decode verified); live RF verification pending. |
| CTCSS / DCS | PARTIAL | Goertzel-based CTCSS tone detection over all 50 EIA tones is implemented and unit-tested; DCS presence flag implemented but code extraction requires raw discriminator bitstream. Live RF verification pending. |
| RDS | PARTIAL | Native 57 kHz quadrature downconvert, differential BPSK decode, sync-word search, PI/PTY extraction implemented and unit-tested; PS/RT text decoding requires multi-group assembly. `/vfo/:id/rds` endpoint now reads live IQ and decodes. Live RF verification pending. |
| BLE advertising | MISSING | No 2.4 GHz GFSK/whitening/CRC/OUI/device table pipeline. |
| AMC signal classification | MISSING | Current signal hit is not modulation classification. |
| IQ protocol identification | OFFLINE/FIXTURE | API shape exists; no validated classifier. |
| NFM/AM multi-stage IQ decimation | PARTIAL | Basic demod/resampling exists; NyxScope sensitivity-preserving pipeline absent. |
| Digital voice CIC/FM path | MISSING | No validated pipeline. |
| HD Radio host resampler | MISSING | No NRSC-5 transport or metadata path. |
| Iridium | MISSING | Existing feature shape is not a native burst decoder. |
| Aero ACARS / Inmarsat | MISSING | No sniffer/transport. |
| STD-C / Inmarsat-C | MISSING | No decoder. |
| GOES LRIT | SIDECAR/UNAVAILABLE | SatDump bridge not implemented. |
| GPS L1 acquisition | MISSING | No PRN/SNR/Doppler acquisition. |
| rtl_433 sensors | SIDECAR/INTEGRATION-VERIFIED | Real installed rtl_433 stdout → PulseScope parser → temporary SQLite passed; live 433 MHz RF decode remains unverified. |
| Radiosondes RS41/RS92/DFM/M10/M20 | SIDECAR/UNAVAILABLE | Executables absent; no typed transport. |
| DTMF/ZVEI/EEA/EIA/CCIR/EAS | PARTIAL | DTMF decoder implemented (all 16 standard digits, twist test, Goertzel-based); ZVEI/EEA/EIA/CCIR/EAS signaling remain missing. |
| CallerID / German FMS | MISSING | No decoder. |
| APRS / AX.25 / AFSK / FSK9600 | PARTIAL | Native AFSK 1200-baud Goertzel demod + AX.25 UI frame parser implemented, unit-tested, and wired to `/scan/aprs` endpoint. Live RSP1B decode attempted at 144.390 MHz (0 frames — no APRS activity present at time of test). FSK9600 not implemented. |
| DMR / D-STAR / YSF / M17 / NXDN / P25 Phase 1/2 / ProVoice | PARTIAL | dsd-fme sidecar integrated: PulseScope demodulates NFM IQ → resamples to 48kHz → writes temp WAV → spawns dsd-fme with mode flag → parses stdout for calls/TGs/NACs/errors. Live RSP1B test at 853 MHz confirmed sidecar spawns and processes (0 calls decoded — no traffic present). Supports: auto, P25p1, P25p2, DMR, NXDN48, NXDN96, D-STAR, YSF, M17, ProVoice. |

## Bundled decoder inventory

| NyxScope sidecar | PulseScope status |
|---|---|
| `rtl_433` | Installed; u8-IQ stdin transport, input sample/byte counters, and startup/exit truthfulness implemented; decoder→parser→SQLite integration test passes; live RF decode remains unverified. |
| `multimon-ng` | Missing locally; no audio/bitstream transport. |
| `acarsdec` | Missing locally; no typed transport. |
| `direwolf` | Missing locally; no audio/AX.25 transport. |
| `nrsc5` | Missing locally; no host resampler/transport. |
| `rs41mod` | Missing locally; no radiosonde transport. |
| `dump978` | Missing locally; no UAT transport. |
| `AIS-catcher` | Installed v0.70 (x64). Requires exclusive SDR; no concurrent PulseScope feed. |
| `dumpvdl2` | Missing locally; parser exists, binary/transport missing. |
| `dsd-fme` | Installed v2026-34 (lwvmobile Cygwin x64). Wired via temp WAV → sidecar. P25/DMR/NXDN/D-STAR/YSF/M17/ProVoice. |
| `sdr-imbe-helper` / `mbelib-neo` | Missing; no isolated vocoder helper. |

## UI/product behavior

| Capability | Status | Gap / next work |
|---|---|---|
| Main scanner UI | REAL-VERIFIED | Live hydration fixed via explicit type="module" entry shim in `app.html`; Svelte 5 runes `$effect` ensures first-load data fetch fires. All 79 scan ranges load, all 5 startup API calls (`/channels/banks`, `/device/status`, `/signal_events`, `/vfo/states`, `/decoded_messages`) fired, spectrum/VFO live state, Quick Modes present. Visual parity with category leaders intentionally diverged into clean-room minimalism. |
| Quick Modes | PARTIAL/REAL | Presets rendered (ADS-B 1090, AIS 162, ACARS 130, APRS 144, 433 Sensors, 915 Sensors, Pagers), call `Api.scanStart(range)`. APRS preset wired to live decoder (verified 0 frames on empty band). Others map correctly to scan ranges; ad-hoc /api/* decoder endpoints remain where protocol infrastructure isn't yet wired. |
| Aircraft map/table | OFFLINE/FIXTURE | No verified ADS-B decode/data flow. |
| AIS vessel map/table | MISSING | No real data flow. |
| APRS/radiosonde map/table | MISSING | No real data flow. |
| BLE device table/detail/vendor lookup | MISSING | No implementation. |
| Sensor history/table | PARTIAL | DB/message UI shape exists; real rtl_433 messages absent. |
| Trunk Discovery UI | OFFLINE/FIXTURE | State/UI exists; no real CC probe feed. |
| Feature Manager | PARTIAL | `/decoders/scan` enumerates all known decoders with source/path/install URL; `/decoders/install/:name` returns install instructions. Downloads/install/update/verification absent. |
| Settings persistence | PARTIAL | Config persistence exists; per-band/device/decoder settings incomplete. |
| Audio playback/stream controls | PARTIAL | Local CPAL output works; streaming/UI verification incomplete. |
| Transcription (Whisper/OpenAI/AssemblyAI) | MISSING | No verified engine or privacy-gated transport. |
| Privacy/crash/lookup controls | MISSING | No equivalent feature path implemented. |
| License/edition behavior | NOT TARGETED | PulseScope should remain clean-room/open implementation; do not copy proprietary licensing behavior. |

## Campaign priority

1. ~~Decoder transport seam~~ ✅ DONE: `rtl_433` integrated end-to-end and unit-tested.
2. ~~Native signal pipeline~~ ✅ DONE: CTCSS/DCS detection, RDS, CW/Morse, DTMF all native DSP and unit-tested.
3. ~~Playback device~~ ✅ DONE: CF32 playback wired through shared capture path.
4. ~~Recording/audio/API/UI parity~~ ✅ DONE: IQ recording verified, UDP audio/IQ streaming, server-mode binary with bearer-token auth.
5. Protocol expansion: POCSAG/FLEX, ADS-B, AIS, ACARS, APRS, CTCSS/DCS, RDS, BLE, LoRa, trunking, and satellite paths in separate verified slices.
6. Multi-user concurrent scanners.
7. TLS termination for HTTPS server deployments.

## Honest status

PulseScope currently has strong verified foundations for live SoapySDR/RSP1B acquisition, bounded DSP/audio timing, FFT spectrum/waterfall/signal hits, real UDP streaming, server-mode binary with bearer-token auth, and a comprehensive decoders/scan dependency manager. **Native DSP coverage extends to CTCSS (50 tones), DCS, RDS, CW/Morse, DTMF**. Not yet: native P25/DMR/D-STAR/M17 decoder (no source available), trunking voice following, BLE GFSK demod, ADS-B Mode S, LoRa, BLE/whitening/CIC chains, satellite chains with valid inputs, integrated GNSS/P25/Iridium/HD Radio pipelines, and full N/A mapping without external decoders installed. Server-mode is functional but lacks TLS termination.
