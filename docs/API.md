# PulseScope HTTP/WS API

This is the public route contract. Pre-1.0 routes can be experimental or
dependency-gated; consult `FEATURE_STATUS.md`. Adding a backend route requires an
update here in the same change. Undocumented routes are release blockers.

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
| POST   | `/channels/scan/start`        | `{ range_name }`             |
| POST   | `/channels/scan/stop`         | —                            |
| POST   | `/channels/bank-scan-config`  | `{ bank_name, … }`           |
| GET    | `/scanner/max-vfos`           | — license-free max-VFO count |

## VFOs (virtual channels inside one captured span)

| Method | Path                       | Body              |
|--------|----------------------------|-------------------|
| GET    | `/vfo/states`              | —                 |
| POST   | `/vfo/:id/mute`            | `{ id, on }`      |
| POST   | `/vfo/:id/volume`          | `{ id, value }`   |
| POST   | `/vfo/:id/audio_agc`       | `{ id, on }`      |
| POST   | `/vfo/:id/identify`        | — signal-ID       |
| GET    | `/vfo/diagnostics`         | — per-VFO stats   |

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
| POST   | `/trunking/start`                        |
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
| POST   | `/goes_lrit/enable`        |
| GET    | `/goes_lrit/status`        |
| GET    | `/hd_radio/check`          |
| GET    | `/hd_radio/messages`       |

## BLE / LoRa / sensors

| Method | Path                |
|--------|---------------------|
| GET    | `/ble/devices`      |
| GET    | `/ble/status`       |
| GET    | `/lora/messages`    |
| GET    | `/lora/regions`     |
| GET    | `/scan/ble`         |
| GET    | `/scan/lora`        |

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
| POST   | `/transcription/start`            |
| POST   | `/transcription/stop`             |
| GET    | `/transcription/status`           |
| GET    | `/transcription/transcripts`      |

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

## Registered route inventory

This inventory is checked against `src-tauri/src/api.rs`; sections above define payloads for stable routes. Routes without a detailed payload contract are experimental.

- `/aero/check`
- `/aero/clear`
- `/aero/enable`
- `/aero/messages`
- `/aero/status`
- `/aero/stderr`
- `/aircraft/lookup`
- `/audio/network/start`
- `/audio/network/status`
- `/audio/network/stop`
- `/blacklist`
- `/blacklist/add`
- `/blacklist/clear`
- `/blacklist/clear-temporary`
- `/blacklist/remove`
- `/ble/clear`
- `/ble/devices`
- `/ble/file`
- `/ble/status`
- `/cases`
- `/cases/:id`
- `/cases/:id/attach`
- `/cases/attachments/:att_id`
- `/channels/bank-scan-config`
- `/channels/banks`
- `/channels/banks/create`
- `/channels/banks/delete`
- `/channels/import`
- `/channels/scan-config`
- `/channels/scan/start`
- `/channels/scan/stop`
- `/close`
- `/debug/classifications`
- `/debug/dsd_stderr`
- `/debug/log`
- `/debug/log/tail`
- `/debug/multimon_raw`
- `/debug/noise_floor`
- `/debug/p25_acq`
- `/debug/p25_squelch`
- `/debug/provoice_stderr`
- `/debug/rtl433_stderr`
- `/debug/stats`
- `/debug/trunking/p25_use_vfo_fir`
- `/debug/trunking/per_cc_stats`
- `/debug/vdl2_stderr`
- `/decoded_messages`
- `/decoders/install/:name`
- `/decoders/scan`
- `/device/capabilities`
- `/device/connect`
- `/device/control`
- `/device/disconnect`
- `/device/frequency`
- `/device/gain`
- `/device/hackrf_amp`
- `/device/mdns_scan`
- `/device/sample_rate`
- `/device/status`
- `/device/test`
- `/devices`
- `/digital_voice/check`
- `/event-stream`
- `/events`
- `/feature-packs`
- `/feature-packs/:id/enable`
- `/glonass/clear`
- `/glonass/enable`
- `/glonass/status`
- `/goes_lrit/check`
- `/goes_lrit/enable`
- `/goes_lrit/satellite`
- `/goes_lrit/status`
- `/gps/clear`
- `/gps/enable`
- `/gps/status`
- `/hd_radio/aas/:filename`
- `/hd_radio/check`
- `/hd_radio/enable`
- `/hd_radio/messages`
- `/hd_radio/status`
- `/health`
- `/instances`
- `/intercept_results`
- `/iq/consumers`
- `/iq/network/start`
- `/iq/network/status`
- `/iq/network/stop`
- `/iq_recording/start`
- `/iq_recording/status`
- `/iq_recording/stop`
- `/iridium/check`
- `/iridium/clear`
- `/iridium/enable`
- `/iridium/messages`
- `/iridium/quick-start`
- `/iridium/status`
- `/iridium/stderr`
- `/jobs`
- `/jobs/:id`
- `/lora/messages`
- `/lora/regions`
- `/protocol_messages`
- `/receiver/session`
- `/receiver/session/claim`
- `/receiver/session/release`
- `/receiver_location`
- `/reconnect`
- `/recording/iq/capture`
- `/recording/iq/playback/start`
- `/recording/iq/playback/status`
- `/recording/iq/playback/stop`
- `/recording/iq/stop`
- `/recordings/annotations`
- `/recordings/annotations/:id`
- `/rtl433_messages`
- `/scan/acars`
- `/scan/adsb`
- `/scan/aero`
- `/scan/ais`
- `/scan/aprs`
- `/scan/ble`
- `/scan/ctcss`
- `/scan/digital_voice`
- `/scan/identify_protocol`
- `/scan/lock`
- `/scan/lora`
- `/scan/pocsag`
- `/scan/start`
- `/scan/status`
- `/scan/stop`
- `/scan/uat`
- `/scan/unlock`
- `/scan/vdl2`
- `/scanner/max-vfos`
- `/settings`
- `/sidecars/:name/stderr`
- `/sidecars/start_all`
- `/sidecars/status`
- `/signal_events`
- `/signal_id/auto_decode`
- `/signal_id/classify`
- `/signal_id/file`
- `/signal_id/fingerprints`
- `/signal_id/fingerprints/:id`
- `/signal_id/fingerprints/match`
- `/signal_id/polyphase_extract`
- `/signal_id/segment_bursts`
- `/slots`
- `/spectrum`
- `/spectrum_occupancy`
- `/stdc/check`
- `/stdc/clear`
- `/stdc/enable`
- `/stdc/messages`
- `/stdc/status`
- `/talkgroups`
- `/talkgroups/delete-system`
- `/talkgroups/export`
- `/talkgroups/import`
- `/talkgroups/systems`
- `/talkgroups/update`
- `/transcription/start`
- `/transcription/status`
- `/transcription/stop`
- `/transcription/transcripts`
- `/trunking/calls`
- `/trunking/discovery/clear`
- `/trunking/discovery/delete`
- `/trunking/discovery/identify`
- `/trunking/discovery/log`
- `/trunking/discovery/log/clear`
- `/trunking/discovery/notes`
- `/trunking/discovery/promote`
- `/trunking/discovery/results`
- `/trunking/discovery/snapshot`
- `/trunking/discovery/start`
- `/trunking/discovery/stop`
- `/trunking/import`
- `/trunking/lock`
- `/trunking/start`
- `/trunking/status`
- `/trunking/stop`
- `/trunking/zone/active`
- `/trunking/zone/delete`
- `/trunking/zone/upsert`
- `/vfo/:id/audio_agc`
- `/vfo/:id/frequency`
- `/vfo/:id/identify`
- `/vfo/:id/mode`
- `/vfo/:id/mute`
- `/vfo/:id/rds`
- `/vfo/:id/volume`
- `/vfo/diagnostics`
- `/vfo/states`
