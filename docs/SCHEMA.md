# PulseScope SQLite schema

Database file: `$HOME/pulsescope/pulsescope.db` (WAL mode, foreign keys on).

## Tables

### `decoded_messages`
Sidecar decoder output (ACARS, VDL2, POCSAG, APRS, ADS-B, etc.).

| column          | type    | notes |
|-----------------|---------|-------|
| id              | INTEGER | PK autoincrement |
| frequency_hz    | INTEGER | NOT NULL |
| protocol        | TEXT    | e.g. `rtl_433`, `vdl2`, `pocsag` |
| message_type    | TEXT    | |
| address         | TEXT    | station / aircraft / sensor ID |
| function_code   | TEXT    | |
| content         | TEXT    | decoded text |
| raw             | TEXT    | full sidecar line |
| encryption      | TEXT    | `none` / `aes` / unknown |
| timestamp_ms    | INTEGER | NOT NULL |

### `frequencies`
Latest observation per frequency (history → `signal_events`).

| column         | type    |
|----------------|---------|
| id             | INTEGER PK |
| frequency_hz   | INTEGER UNIQUE |
| strength_db    | REAL |
| snr_db         | REAL |
| mode           | TEXT |
| range_name     | TEXT |
| bandwidth_hz   | INTEGER |
| timestamp_ms   | INTEGER |

### `signal_events`
Raw signal detection + classification events.

| column                | type    |
|-----------------------|---------|
| id                    | INTEGER PK |
| frequency_hz          | INTEGER |
| signal_class          | TEXT |
| top_family            | TEXT |
| top_confidence        | REAL |
| sub_protocol          | TEXT |
| symbol_rate           | REAL |
| bandwidth_hz          | INTEGER |
| snr_db                | REAL |
| decode_success        | INTEGER (0/1) |
| decode_protocol       | TEXT |
| decode_summary        | TEXT |
| likely_proprietary    | INTEGER (0/1) |
| waterfall_psd         | TEXT (JSON) |
| range_name            | TEXT |
| timestamp_ms          | INTEGER |
| is_novel              | INTEGER (0/1) |

### `talkgroups`
Trunking talkgroup directory.

| column         | type    |
|----------------|---------|
| id             | INTEGER PK |
| system_name    | TEXT |
| talkgroup_id   | TEXT |
| alpha_tag      | TEXT |
| description    | TEXT |
| category       | TEXT |
| tag            | TEXT |
| mode           | TEXT |
| protocol       | TEXT (p25 / nxdn / edacs / dmr) |
| encrypted      | INTEGER (0/1) |
| hit_count      | INTEGER |
| first_seen_ms  | INTEGER |
| last_seen_ms   | INTEGER |

Unique: `(system_name, talkgroup_id)`.

### `sensor_messages`
rtl_433 sensor packets.

| column         | type    |
|----------------|---------|
| id             | INTEGER PK |
| timestamp      | INTEGER |
| frequency_hz   | INTEGER |
| model          | TEXT |
| sensor_id      | TEXT |
| raw_json       | TEXT |

### `spectrum_occupancy`
Long-term band usage map. 15-min time buckets per ~10 kHz frequency bucket.

| column                | type |
|-----------------------|------|
| frequency_bucket_hz   | INTEGER |
| time_bucket_15min     | INTEGER |
| avg_power_db          | REAL |
| peak_power_db         | REAL |
| avg_above_floor_db    | REAL |
| sample_count          | INTEGER |
| noise_floor_db        | REAL |

PK: `(frequency_bucket_hz, time_bucket_15min)`.

### `cases` + `case_attachments`
Analyst case grouping.

`cases` columns: `id, name, description, status, tags, created_ms, updated_ms`.
`case_attachments` columns: `id, case_id, kind, ref, note, attached_ms`

Duplicate `(case_id, kind, ref)` attachments are rejected. Supported kinds are
`decoded_message`, `signal_event`, `recording`, `track`, `note`, and
`lookup_result`.

### `position_events`

Immutable, normalized observations for `aircraft`, `vessel`, `aprs`, and
`radiosonde` entities. Coordinates use WGS84 decimal degrees; altitude, speed,
and horizontal/vertical accuracy use SI units. Every row includes a local
timestamp, source attribution, optional decoded-message reference, and JSON
protocol metadata. Current tracks are derived rather than stored separately.

### `radio_channels`

Validated channel rows imported from CHIRP CSV. The `(name, frequency_hz)` pair
is unique so repeated imports are idempotent and report duplicates.
(foreign key on `case_id` → `cases(id)` with cascade delete).

### `recording_annotations`
In-recording text markers.

| column           | type |
|------------------|------|
| id               | INTEGER PK |
| recording_path   | TEXT |
| offset_ms        | INTEGER |
| text             | TEXT |
| created_ms       | INTEGER |

## Indices
- `decoded_messages`: `(timestamp_ms DESC)`, `frequency_hz`, `protocol`
- `frequencies`: `timestamp_ms DESC`
- `signal_events`: `timestamp_ms DESC`, `frequency_hz`
- `sensor_messages`: `timestamp DESC`
