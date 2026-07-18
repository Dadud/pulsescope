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

<!-- BEGIN GENERATED ROUTE INVENTORY -->

## Complete route inventory (generated)

> Generated by `scripts/check-api-contracts.py`; both bare and `/api`-prefixed forms are served.

| Domain | Method | Path |
|---|---|---|
| decoders | POST | `/aero/check` |
| decoders | POST | `/aero/clear` |
| decoders | POST | `/aero/enable` |
| decoders | GET | `/aero/messages` |
| decoders | GET | `/aero/status` |
| decoders | GET | `/aero/stderr` |
| core | GET | `/aircraft/lookup` |
| recording | POST | `/audio/network/start` |
| recording | GET | `/audio/network/status` |
| recording | POST | `/audio/network/stop` |
| core | GET | `/blacklist` |
| core | POST | `/blacklist` |
| core | POST | `/blacklist/add` |
| core | POST | `/blacklist/clear` |
| core | POST | `/blacklist/clear-temporary` |
| core | POST | `/blacklist/remove` |
| decoders | POST | `/ble/clear` |
| decoders | GET | `/ble/devices` |
| decoders | GET | `/ble/file` |
| decoders | GET | `/ble/status` |
| recording | GET | `/cases` |
| recording | POST | `/cases` |
| recording | DELETE | `/cases/:id` |
| recording | GET | `/cases/:id` |
| recording | POST | `/cases/:id/attach` |
| recording | DELETE | `/cases/attachments/:att_id` |
| recording | GET | `/cases/attachments/:att_id` |
| scanner | GET | `/channels/bank-scan-config` |
| scanner | PUT | `/channels/bank-scan-config` |
| scanner | GET | `/channels/banks` |
| scanner | POST | `/channels/banks` |
| scanner | POST | `/channels/banks/create` |
| scanner | POST | `/channels/banks/delete` |
| scanner | POST | `/channels/import` |
| scanner | GET | `/channels/scan-config` |
| scanner | POST | `/channels/scan/start` |
| scanner | POST | `/channels/scan/stop` |
| core | POST | `/close` |
| core | GET | `/debug/classifications` |
| core | GET | `/debug/dsd_stderr` |
| core | GET | `/debug/log` |
| core | GET | `/debug/log/tail` |
| core | GET | `/debug/multimon_raw` |
| core | GET | `/debug/noise_floor` |
| core | GET | `/debug/p25_acq` |
| core | GET | `/debug/p25_squelch` |
| core | GET | `/debug/provoice_stderr` |
| core | GET | `/debug/rtl433_stderr` |
| core | GET | `/debug/stats` |
| core | GET | `/debug/trunking/p25_use_vfo_fir` |
| core | GET | `/debug/trunking/per_cc_stats` |
| core | GET | `/debug/vdl2_stderr` |
| decoders | GET | `/decoded_messages` |
| decoders | POST | `/decoders/install/:name` |
| decoders | GET | `/decoders/scan` |
| device | GET | `/device/capabilities` |
| device | POST | `/device/connect` |
| device | POST | `/device/control` |
| device | POST | `/device/disconnect` |
| device | POST | `/device/frequency` |
| device | POST | `/device/gain` |
| device | POST | `/device/hackrf_amp` |
| device | GET | `/device/mdns_scan` |
| device | POST | `/device/sample_rate` |
| device | GET | `/device/status` |
| device | POST | `/device/test` |
| device | GET | `/devices` |
| core | GET | `/digital_voice/check` |
| core | GET | `/event-stream` |
| core | GET | `/events` |
| decoders | GET | `/feature-packs` |
| decoders | POST | `/feature-packs/:id/enable` |
| decoders | POST | `/glonass/clear` |
| decoders | POST | `/glonass/enable` |
| decoders | GET | `/glonass/status` |
| decoders | POST | `/goes_lrit/check` |
| decoders | POST | `/goes_lrit/enable` |
| decoders | GET | `/goes_lrit/satellite` |
| decoders | PUT | `/goes_lrit/satellite` |
| decoders | GET | `/goes_lrit/status` |
| decoders | POST | `/gps/clear` |
| decoders | POST | `/gps/enable` |
| decoders | GET | `/gps/status` |
| decoders | GET | `/hd_radio/aas/:filename` |
| decoders | POST | `/hd_radio/check` |
| decoders | POST | `/hd_radio/enable` |
| decoders | GET | `/hd_radio/messages` |
| decoders | GET | `/hd_radio/status` |
| core | GET | `/health` |
| core | GET | `/instances` |
| core | GET | `/intercept_results` |
| recording | GET | `/iq/consumers` |
| recording | POST | `/iq/network/start` |
| recording | GET | `/iq/network/status` |
| recording | POST | `/iq/network/stop` |
| recording | POST | `/iq_recording/start` |
| recording | GET | `/iq_recording/status` |
| recording | POST | `/iq_recording/stop` |
| decoders | POST | `/iridium/check` |
| decoders | POST | `/iridium/clear` |
| decoders | POST | `/iridium/enable` |
| decoders | GET | `/iridium/messages` |
| decoders | POST | `/iridium/quick-start` |
| decoders | GET | `/iridium/status` |
| decoders | GET | `/iridium/stderr` |
| scanner | GET | `/jobs` |
| scanner | POST | `/jobs` |
| scanner | DELETE | `/jobs/:id` |
| decoders | GET | `/lora/messages` |
| decoders | GET | `/lora/regions` |
| decoders | GET | `/protocol_messages` |
| device | GET | `/receiver/session` |
| device | POST | `/receiver/session/claim` |
| device | POST | `/receiver/session/release` |
| device | GET | `/receiver_location` |
| device | PUT | `/receiver_location` |
| core | POST | `/reconnect` |
| recording | POST | `/recording/iq/capture` |
| recording | POST | `/recording/iq/playback/start` |
| recording | GET | `/recording/iq/playback/status` |
| recording | POST | `/recording/iq/playback/stop` |
| recording | POST | `/recording/iq/stop` |
| recording | GET | `/recordings/annotations` |
| recording | POST | `/recordings/annotations` |
| recording | DELETE | `/recordings/annotations/:id` |
| recording | GET | `/recordings/annotations/:id` |
| recording | PUT | `/recordings/annotations/:id` |
| decoders | GET | `/rtl433_messages` |
| scanner | GET | `/scan/acars` |
| scanner | POST | `/scan/acars` |
| scanner | GET | `/scan/adsb` |
| scanner | GET | `/scan/aero` |
| scanner | GET | `/scan/ais` |
| scanner | POST | `/scan/ais` |
| scanner | GET | `/scan/aprs` |
| scanner | GET | `/scan/ble` |
| scanner | GET | `/scan/ctcss` |
| scanner | POST | `/scan/digital_voice` |
| scanner | POST | `/scan/identify_protocol` |
| scanner | POST | `/scan/lock` |
| scanner | GET | `/scan/lora` |
| scanner | POST | `/scan/pocsag` |
| scanner | POST | `/scan/start` |
| scanner | GET | `/scan/status` |
| scanner | POST | `/scan/stop` |
| scanner | POST | `/scan/uat` |
| scanner | POST | `/scan/unlock` |
| scanner | POST | `/scan/vdl2` |
| scanner | GET | `/scanner/max-vfos` |
| core | GET | `/settings` |
| core | PUT | `/settings` |
| decoders | GET | `/sidecars/:name/stderr` |
| decoders | POST | `/sidecars/start_all` |
| decoders | GET | `/sidecars/status` |
| scanner | GET | `/signal_events` |
| scanner | POST | `/signal_id/auto_decode` |
| scanner | POST | `/signal_id/classify` |
| scanner | POST | `/signal_id/file` |
| scanner | GET | `/signal_id/fingerprints` |
| scanner | DELETE | `/signal_id/fingerprints/:id` |
| scanner | GET | `/signal_id/fingerprints/:id` |
| scanner | POST | `/signal_id/fingerprints/match` |
| scanner | POST | `/signal_id/polyphase_extract` |
| scanner | POST | `/signal_id/segment_bursts` |
| core | GET | `/slots` |
| scanner | GET | `/spectrum` |
| scanner | GET | `/spectrum_occupancy` |
| decoders | POST | `/stdc/check` |
| decoders | POST | `/stdc/clear` |
| decoders | POST | `/stdc/enable` |
| decoders | GET | `/stdc/messages` |
| decoders | GET | `/stdc/status` |
| trunking | GET | `/talkgroups` |
| trunking | POST | `/talkgroups` |
| trunking | POST | `/talkgroups/delete-system` |
| trunking | GET | `/talkgroups/export` |
| trunking | POST | `/talkgroups/import` |
| trunking | GET | `/talkgroups/systems` |
| trunking | POST | `/talkgroups/update` |
| recording | POST | `/transcription/start` |
| recording | GET | `/transcription/status` |
| recording | POST | `/transcription/stop` |
| recording | GET | `/transcription/transcripts` |
| trunking | GET | `/trunking/calls` |
| trunking | POST | `/trunking/discovery/clear` |
| trunking | POST | `/trunking/discovery/delete` |
| trunking | POST | `/trunking/discovery/identify` |
| trunking | GET | `/trunking/discovery/log` |
| trunking | POST | `/trunking/discovery/log/clear` |
| trunking | GET | `/trunking/discovery/notes` |
| trunking | POST | `/trunking/discovery/notes` |
| trunking | POST | `/trunking/discovery/promote` |
| trunking | GET | `/trunking/discovery/results` |
| trunking | GET | `/trunking/discovery/snapshot` |
| trunking | POST | `/trunking/discovery/start` |
| trunking | POST | `/trunking/discovery/stop` |
| trunking | POST | `/trunking/import` |
| trunking | POST | `/trunking/lock` |
| trunking | POST | `/trunking/start` |
| trunking | GET | `/trunking/status` |
| trunking | POST | `/trunking/stop` |
| trunking | GET | `/trunking/zone/active` |
| trunking | POST | `/trunking/zone/delete` |
| trunking | POST | `/trunking/zone/upsert` |
| scanner | POST | `/vfo/:id/audio_agc` |
| scanner | POST | `/vfo/:id/frequency` |
| scanner | POST | `/vfo/:id/identify` |
| scanner | POST | `/vfo/:id/mode` |
| scanner | POST | `/vfo/:id/mute` |
| scanner | GET | `/vfo/:id/rds` |
| scanner | POST | `/vfo/:id/volume` |
| scanner | GET | `/vfo/diagnostics` |
| scanner | GET | `/vfo/states` |

<!-- END GENERATED ROUTE INVENTORY -->
