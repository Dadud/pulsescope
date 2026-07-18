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

## Audio streaming and recording library

`GET /audio/vfo/:id/stream.wav` streams authenticated mono PCM WAV with bounded per-client backpressure. `POST`/`DELETE /audio/vfo/:id/record` starts/stops per-VFO WAV recording; the start body includes frequency/mode, optional signal/case metadata, and `vox` (`enabled`, `threshold_db`, `pre_roll_ms`, `post_roll_ms`). `GET /audio/recordings/status` reports active recordings and elapsed time.

`GET /recordings?page=1&page_size=25` lists safely rooted files and disk-space warnings. `GET`, `PUT`, and `DELETE /recordings/:name` inspect, rename, and delete; `GET /recordings/:name/download` downloads. Names are single path components. See [FORMATS.md](FORMATS.md) for CF32, WAV, PSAU, and PSIQ wire definitions.
