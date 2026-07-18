# Protocol implementation status

PulseScope uses **recorded-IQ verified** as its availability boundary. Finding a
decoder executable, enabling configuration, parsing a hand-built JSON line, or
rendering an empty UI is not availability. Every unavailable family has a
tracked slice below. A slice may move to **AVAILABLE** only after a legally
redistributable IQ recording passes PHY → transport → parser → persistence →
event delivery → metrics → UI, its corrupt/truncated negative fixtures fail
closed, and an opt-in live test passes when suitable equipment is available.

The machine-readable contracts for the eight priority slices (including
frequency plan, bandwidth, sample rate, synchronization, modulation, FEC,
checksum, versioned message schema, fixture, transport, and UI outcome) live in
`src-tauri/src/protocols.rs` and are exposed at `GET /protocols/slices`.

| Tracked slice | State | Completion gate / next work |
|---|---|---|
| BLE advertising | UNAVAILABLE | Legal three-channel IQ fixture, native GFSK/whitening/CRC, persistence/events/metrics/UI, negative CRC fixture |
| LoRa / LoRaWAN | UNAVAILABLE | Legal regional IQ fixture, CSS/FEC/MIC parser, persistence/events/metrics/UI, invalid MIC/CRC fixtures |
| FLEX paging | UNAVAILABLE | Legal IQ fixture, 2/4FSK sync/FEC/parser, persistence/events/metrics/UI, corrupt BCH fixture |
| HD Radio (nrsc5) | UNAVAILABLE | Pin nrsc5, IQ resampler/transport, legal fixture, typed events/persistence/metrics/UI, malformed event test |
| GNSS GPS L1 acquisition | UNAVAILABLE | Legal/synthetic-signal-authorized IQ fixture, PRN acquisition, persistence/events/metrics/UI, noise-only fixture |
| GOES (SatDump) | UNAVAILABLE | Pin SatDump, legal IQ fixture, typed product transport/persistence/events/metrics/UI, corrupt CADU test |
| Radiosondes (RS41/RS92/DFM/M10/M20) | UNAVAILABLE | Licensed/legal IQ fixtures by model, pinned adapters, typed telemetry/persistence/events/metrics/map, bad CRC tests |
| Iridium | UNAVAILABLE | Legal burst fixture and reviewed decoder, privacy-safe parser/persistence/events/metrics/UI, invalid checksum test |
| P25 Phase 1 / IMBE | UNAVAILABLE | Legal IQ fixture and isolated legal vocoder contract |
| P25 Phase 2 / AMBE+2 | UNAVAILABLE | Legal IQ fixture and isolated legal vocoder contract |
| EDACS / ProVoice | UNAVAILABLE | Legal control/voice IQ fixtures and decoder transport |
| NXDN48 / NXDN96 | UNAVAILABLE | Legal IQ fixture and full FEC/CRC path |
| Trunk discovery / call following | UNAVAILABLE | Real control-channel IQ fixtures and deterministic scheduler tests |
| ADS-B Mode S 1090 | UNAVAILABLE | Legal IQ fixture through native PHY/parser/event/UI |
| UAT 978 | UNAVAILABLE | Pin dump978 transport and legal IQ fixture |
| ACARS | UNAVAILABLE | Legal IQ fixture and MSK transport/parser |
| VDL2 | UNAVAILABLE | Pin dumpvdl2 and legal IQ fixture |
| POCSAG | UNAVAILABLE | Legal IQ fixture through native decoder and pager UI |
| AIS live GMSK | UNAVAILABLE | Legal dual-channel IQ fixture and shared-IQ decoder transport |
| Aero / Inmarsat / STD-C | UNAVAILABLE | Legal fixtures and audited transports per family |
| Caller ID / German FMS | UNAVAILABLE | Legal audio/IQ fixtures and native parsers |
| ZVEI / EEA / EIA / CCIR / EAS | UNAVAILABLE | Legal fixtures and tone/message parsers |

Hardware suitability and decoder completion are deliberately separate. The API
reports both: a capable radio does not make an unfinished decoder available.
Unsupported sample rates, disconnected hardware, and the known RTL-SDR BLE
frequency limitation return actionable guidance instead of a success response.
