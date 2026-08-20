# PulseScope HTTP/WS API

The Rust backend binds a local HTTP + WebSocket server to `127.0.0.1:8765`.
The Svelte frontend talks to it directly; third-party tools can use the same
wire format. This contract mirrors the endpoint shape used across the desktop
SDR scanner category so integrations port cleanly.

## Base URL

```
http://127.0.0.1:8765
ws://127.0.0.1:8765
```

## Health / versioning

| Method | Path        | Description |
|--------|-------------|-------------|
| GET    | `/health`   | `{ status, name, version }` |

## Version 2 control plane

The v2 API is mounted both at `/api/v2/...` and `/v2/...`; web clients should use the `/api` namespace. It reports desired and actual receiver state separately. Mutating commands require a unique `command_id` and the last observed `expected_revision`; retries with the same command ID return the original response and stale revisions return HTTP 409.

| Method | Path | Description |
|---|---|---|
| GET | `/api/v2/features` | Machine-readable release maturity, visibility, evidence, and open gates |
| GET | `/api/v2/devices` | Discovered devices and active lifecycle. Registered network IQ sources include `adapter_ready` and ingest counters when selected. |
| POST | `/api/v2/devices/:id/select` | Select a discovered USB device or a registered network IQ source (`raw_udp`, `rtl_tcp`, `spyserver`, `ka9q`) |
| GET | `/api/v2/devices/:id/capabilities` | Runtime RF ranges, rates, bandwidth, MTU, antennas, gains, and settings |
| GET/POST | `/api/v2/network-sources` | Persist RTL-TCP, SpyServer, PSIQ `raw_udp`, and KA9Q registry entries |
| DELETE | `/api/v2/network-sources/:id` | Remove a network IQ registry entry |
| GET | `/api/v2/receivers` | Receiver desired/actual state and revision |
| POST | `/api/v2/receivers/:id/tune` | `{ command_id, expected_revision, frequency_hz }` |
| GET | `/api/v2/receivers/:id/controls` | Generated control contract plus actual values |
| GET/POST | `/api/v2/sessions` | List or claim/release the physical receiver lease |
| GET | `/api/v2/hardware-windows` | Shared capture window, usable edges, owner, and revision |
| GET/POST | `/api/v2/listener-sessions` | Independent per-browser viewport and selected-VFO state |
| GET/POST | `/api/v2/profiles` | Persistent server-owned receiver profiles |
| GET/DELETE | `/api/v2/profiles/:id` | Read or remove a receiver profile |
| POST | `/api/v2/profiles/:id/apply` | Atomically apply the stored sample-rate, bandwidth, and center-frequency contract. Updates VFO 0 mode/frequency without parking scanner Hold. |
| GET/POST | `/api/v2/bookmarks` | Persistent shared frequency bookmarks |
| DELETE | `/api/v2/bookmarks/:id` | Remove a shared bookmark |
| GET | `/api/v2/bandplans` | Configured RF bands and scan defaults |
| GET | `/api/v2/system/health` | Capture, FFT, VFO, audio, decoder, client, and recovery freshness |
| GET | `/api/v2/decoders/catalog` | Truthful beta/installed decoder catalog and missing verification gate |
| GET | `/api/v2/decoder-jobs` | Isolated decoder-process state |
| GET | `/api/v2/recordings` | Active recording and persisted files |
| GET | `/api/v2/media/capabilities` | Truthful media transports and acceptance status |
| POST | `/api/v2/media/sessions` | WebRTC negotiation (HTTP 501 with PCM fallback). Response includes the ICE-lite Opus SDP/RTP contract (PT 111, 20 ms, timestamp += 960, ice-ufrag/pwd, fingerprint placeholder) while ICE/DTLS and libopus remain unimplemented. |

### Network IQ sources

Implemented ingest kinds are `raw_udp` (PulseScope PSIQ), `rtl_tcp`, `spyserver`, and `ka9q`. `POST /api/v2/devices/:id/select` with a registry id occupies the shared hardware window. RTL-TCP and SpyServer forward live frequency and sample-rate commands; SpyServer rates must equal the reported `MaximumSampleRate / 2^n` and are rejected otherwise. PSIQ takes center and rate from packet metadata. KA9Q is receive-only RTP v2 or raw s16le IQ; a multicast host joins that group on `0.0.0.0:port`. KiwiSDR is not implemented.

### Binary spectrum stream v3

`ws(s)://host/api/v2/spectrum/stream` is a latest-frame-only stream. Every packet starts with a fixed 64-byte little-endian header followed by one unsigned byte per FFT bin. The floor and scale fields convert a bin with `floor_dbfs + byte * scale_db`.

| Offset | Type | Meaning |
|---:|---|---|
| 0 | `[u8;4]` | `PSF3` |
| 4 | `u16` | protocol version 3 |
| 6 | `u16` | flags |
| 8 | `u64` | frame sequence |
| 16 | `i64` | capture Unix time in milliseconds |
| 24 | `u64` | capture center frequency Hz |
| 32 | `u32` | sample rate Hz |
| 36 | `u32` | usable span Hz |
| 40 | `u32` | bin count |
| 44 | `f32` | floor dBFS |
| 48 | `f32` | dB per integer step |
| 52 | `u32` | receiver ID (`0` = `receiver-0`) |
| 56 | `u64` | receiver-session revision |

### Browser PCM compatibility stream

`/audio/stream` remains the fallback until WebRTC parity is complete. Frames are 20 ms, 48 kHz float PCM. General receiver audio is mono; WFM with one audible VFO is stereo. The browser batches three wire frames for a 60 ms scheduling cadence and owns the LAN jitter buffer.

## Settings

| Method | Path         | Description |
|--------|--------------|-------------|
| GET    | `/settings`  | Full config (mirrors `$HOME/pulsescope/config.toml`) |
| PUT    | `/settings`  | Replace full config |

## Device

| Method | Path                    | Body / query                     |
|--------|-------------------------|----------------------------------|
| GET    | `/devices`              | — list discovered SDRs           |
| POST   | `/device/connect`       | `{ key, label? }`                |
| POST   | `/device/disconnect`    | —                                |
| GET    | `/device/status`        | — connection + sample-rate state |
| POST   | `/device/gain`          | `{ gain }` ("auto" or numeric)   |
| POST   | `/device/sample_rate`   | `{ sample_rate }`                |
| GET    | `/device/mdns_scan`     | — PlutoSDR / network SDR search  |

## Scanner / channels

| Method | Path                          | Body                         |
|--------|-------------------------------|------------------------------|
| GET    | `/channels/banks`             | — list scan-range presets    |
| GET    | `/channels/scan-config`       | — FFT/squelch/max-VFO knobs  |
| POST   | `/channels/scan/start`        | `{ range_name }` — named bank, `enabled`, `*`, or `Bookmarks` |
| POST   | `/channels/scan/stop`         | —                            |
| POST   | `/channels/bank-scan-config`  | `{ bank_name, … }`           |
| GET    | `/scanner/max-vfos`           | — license-free max-VFO count |
| GET    | `/scan/status`                | — running / locked / holding |
| POST   | `/scan/lock`                  | — hold on the current hit    |
| POST   | `/scan/unlock`                | — resume after hold          |
| POST   | `/scan/skip`                  | — temporary lockout + resume |
| POST   | `/scan/lockout`               | — persistent lockout + resume |

## VFOs (virtual channels inside one captured span)

| Method | Path                       | Body              |
|--------|----------------------------|-------------------|
| GET    | `/vfo/states`              | —                 |
| POST   | `/vfo/add`                 | `{ frequency_hz?, mode? }` — allocate an operator slot; `409` past `/scanner/max-vfos`, `400` outside the passband |
| POST   | `/vfo/:id/remove`          | — release an operator slot; the last remaining VFO is kept |
| POST   | `/vfo/:id/mute`            | `{ id, on }` `on: true` **mutes** the VFO |
| POST   | `/vfo/:id/volume`          | `{ id, value }`   |
| POST   | `/vfo/:id/audio_agc`       | `{ id, on }`      |
| POST   | `/vfo/:id/identify`        | — signal-ID on a snapshot of live IQ mixed to this VFO |
| GET    | `/vfo/:id/rds`             | — PI/PS from a 190 kHz WFM multiplex mixed to this VFO |
| GET    | `/vfo/diagnostics`         | — per-VFO stats   |
| GET    | `/scan/ctcss`              | — CTCSS/DCS on the unmuted or locked VFO |
| GET    | `/scan/aprs`               | — AX.25 AFSK on the unmuted or locked VFO |

The sweep only allocates a VFO when it confirms a hit, so `/vfo/add` is the
only way to obtain a second slot while holding on one channel.

A manual retune — `/device/frequency`, `/api/v2/receivers/:id/tune`, or
`/vfo/:id/frequency` past the edge of the capture window — reselects the
narrowest configured bank containing the new center. Native decoders are
chosen by bank name, so without this a receiver tuned to a LoRa channel while
`FM Broadcast` was selected would discard every sample. Any VFO left outside
the new passband is re-homed to the new center and adopts that bank's default
mode; a VFO still inside the window is untouched.

## Spectrum / signal ID

| Method | Path                                |
|--------|-------------------------------------|
| GET    | `/spectrum`                         |
| GET    | `/spectrum_occupancy`               |
| GET    | `/signal_id/fingerprints`           |
| POST   | `/signal_id/fingerprints/match`     |
| POST   | `/signal_id/segment_bursts`         |
| POST   | `/signal_id/polyphase_extract`      |
| POST   | `/scan/identify_protocol`           |

## Decoded messages / sensor packets

| Method | Path                                  | Query |
|--------|---------------------------------------|-------|
| GET    | `/decoded_messages`                   | `?limit=` |
| GET    | `/rtl433_messages`                    |         |
| GET    | `/protocol_messages`                  |         |

## Trunking

| Method | Path                                     |
|--------|------------------------------------------|
| GET    | `/trunking/status`                       |
| POST   | `/trunking/start`                        | Native P25 TSBK observer with VFO FIR. Recovers group-voice-grant and IDEN_UP from C4FM IQ fixtures. Follows a VFO only when a recovered IDEN or imported channel table maps the grant; never invents talkgroups. `available: false` until live control-channel hardware verification. |
| POST   | `/trunking/stop`                         |
| POST   | `/trunking/lock`                         |
| GET    | `/trunking/calls`                        |
| POST   | `/trunking/import`                       |
| POST   | `/trunking/discovery/start`              |
| POST   | `/trunking/discovery/stop`               |
| GET    | `/trunking/discovery/results`            |
| POST   | `/trunking/discovery/promote`            |
| POST   | `/trunking/zone/upsert`                  |
| POST   | `/trunking/zone/delete`                  |
| GET    | `/trunking/zone/active`                  |

## Talkgroups

| Method | Path                            |
|--------|---------------------------------|
| GET    | `/talkgroups`                   |
| POST   | `/talkgroups/update`            |
| POST   | `/talkgroups/import`            |
| GET    | `/talkgroups/export`            |
| POST   | `/talkgroups/delete-system`     |
| GET    | `/talkgroups/systems`           |

## Aviation / satellite decoders

| Method | Path                       |
|--------|----------------------------|
| POST   | `/aero/enable`             |
| GET    | `/aero/status`             |
| GET    | `/aero/messages`           |
| POST   | `/iridium/enable`          |
| GET    | `/iridium/status`          |
| GET    | `/iridium/messages`        |
| POST   | `/iridium/quick-start`     |
| POST   | `/stdc/enable`             |
| GET    | `/stdc/status`             |
| GET    | `/stdc/messages`           |
| POST   | `/gps/enable`              |
| GET    | `/gps/status`              |
| POST   | `/glonass/enable`          |
| GET    | `/glonass/status`          |
| POST   | `/goes_lrit/enable`        | persist enable flag; native CADU identification is fixture-verified |
| GET    | `/goes_lrit/status`        | native CADU availability; `image_pipeline` stays false until SatDump hardware |
| GET    | `/goes_lrit/products`      | checksum-valid `goes.product.v1` events only; never lists `output_image_dir` |
| POST   | `/goes_lrit/clear`         | delete persisted GOES products |
| GET    | `/goes_lrit/check`         | POST also accepted |
| GET    | `/radiosonde/status`       | native GFSK availability plus optional rs41mod path |
| POST   | `/radiosonde/enable`       | persist enable flag and spawn rs41mod stdin audio sidecar when installed |
| GET    | `/radiosonde/telemetry`    | checksum-valid radiosonde frames |
| POST   | `/radiosonde/clear`        | delete persisted radiosonde telemetry |
| GET    | `/scan/goes`               | snapshot BPSK CADU decode on the live IQ ring |
| GET    | `/scan/radiosonde`         | snapshot GFSK radiosonde decode on the live IQ ring |
| GET    | `/protocols/slices`        | `{slices:[...]}` with `available` only after recorded-IQ e2e |
| GET    | `/hd_radio/check`          | POST also accepted; reports nrsc5 path. `available: false` until OFDM IQ e2e |
| GET    | `/hd_radio/messages`       | persisted `hd_radio` events |
| GET    | `/hd_radio/status`         | enabled/config plus install state |
| POST   | `/hd_radio/enable`         | persist enable flag and spawn nrsc5 stdin IQ sidecar when the binary exists; does not claim OFDM decode |

## BLE / LoRa / sensors

| Method | Path                |
|--------|---------------------|
| GET    | `/ble/devices`      | unique advertising addresses from native GFSK decode |
| GET    | `/ble/status`       | native decoder status; RTL-SDR cannot cover 2.4 GHz |
| POST   | `/ble/clear`        | delete persisted BLE advertisements |
| GET    | `/lora/messages`    | MeshCore / Meshtastic / Reticulum / Modbus / LoRaWAN identifications |
| GET    | `/lora/regions`     | documented regional plans, not discovered radios |
| GET    | `/scan/ble`         | snapshot GFSK decode on the live IQ ring |
| GET    | `/scan/lora`        | snapshot CSS decode + payload classify; Meshtastic/MeshCore public-default PSK plaintext; private keys and LoRaWAN stay opaque |

## Recording / streaming / transcription

| Method | Path                              |
|--------|-----------------------------------|
| POST   | `/recording/iq/capture`           |
| POST   | `/recording/iq/stop`              |
| POST   | `/iq_recording/start`             |
| POST   | `/iq_recording/stop`              |
| GET    | `/iq_recording/status`            |
| GET    | `/recordings/annotations`         |
| POST   | `/recordings/annotations`         |
| POST   | `/transcription/start`            | local whisper.cpp transport when installed, using the bounded 16 kHz demod PCM ring first; otherwise `available: false` |
| POST   | `/transcription/stop`             | —                            |
| GET    | `/transcription/status`           | `available` follows whisper-cli presence; catalog stays development; `pcm_ring_samples` reports recent demod history |
| GET    | `/transcription/transcripts`      | in-memory segments from the last start |

## Cases

| Method | Path                     |
|--------|--------------------------|
| GET    | `/cases`                 |
| POST   | `/cases`                 |
| GET    | `/cases/:id`             |
| DELETE | `/cases/:id`             |
| POST   | `/cases/:id/attach`      |

## Blacklist / lookups

| Method | Path                  |
|--------|-----------------------|
| GET    | `/blacklist`          |
| POST   | `/blacklist/add`      |
| POST   | `/blacklist/remove`   |
| POST   | `/blacklist/clear`    |
| GET    | `/aircraft/lookup`    |

## Events — live stream

Two transports, same JSON payload shape:

- **WebSocket** — `ws://127.0.0.1:8765/events`
- **SSE** — `GET /event-stream`

Each event is a tagged union:

```json
{ "kind": "Spectrum",         "data": { "range": "2m Amateur", "bins": [-95.3, -94.8, …] } }
{ "kind": "SignalHit",        "data": { "frequency_hz": 144150000, "strength_db": -42.1, "snr_db": 18.0, "bandwidth_hz": 12500 } }
{ "kind": "VfoStates",        "data": [ { "id": 0, "frequency_hz": 144150000, "mode": "nfm", "muted": false, … } ] }
{ "kind": "DecodedMessage",   "data": { "frequency_hz": 136975000, "protocol": "vdl2", "content": "...", … } }
{ "kind": "TrunkingUpdate",   "data": { "system": "…", "active_talkgroup": "1234", … } }
{ "kind": "SpectrumOccupancy","data": { "frequency_bucket_hz": 144150000, … } }
```

## Debug

| Method | Path                  |
|--------|-----------------------|
| GET    | `/debug/stats`        |
| GET    | `/debug/log/tail`     |
| GET    | `/debug/noise_floor`  |
