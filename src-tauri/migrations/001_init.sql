-- PulseScope SQLite schema — mirrors the table shape used across the
-- desktop SDR scanner category (scan history, decoded messages, talkgroups,
-- signal events, cases, spectrum occupancy). Implementation is original.

PRAGMA foreign_keys = ON;

-- decoded sidecar output (ACARS, VDL2, POCSAG, ADS-B, APRS, …)
CREATE TABLE IF NOT EXISTS decoded_messages (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    frequency_hz  INTEGER NOT NULL,
    protocol      TEXT    NOT NULL,
    message_type  TEXT,
    address       TEXT,
    function_code TEXT,
    content       TEXT,
    raw           TEXT,
    encryption    TEXT,
    timestamp_ms  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_decoded_ts ON decoded_messages(timestamp_ms DESC);
CREATE INDEX IF NOT EXISTS idx_decoded_freq ON decoded_messages(frequency_hz);
CREATE INDEX IF NOT EXISTS idx_decoded_proto ON decoded_messages(protocol);

-- per-scan-hit frequencies (latest observation wins; history is in signal_events)
CREATE TABLE IF NOT EXISTS frequencies (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    frequency_hz  INTEGER NOT NULL,
    strength_db   REAL,
    snr_db        REAL,
    mode          TEXT,
    range_name    TEXT,
    bandwidth_hz  INTEGER,
    timestamp_ms  INTEGER NOT NULL,
    UNIQUE(frequency_hz)
);
CREATE INDEX IF NOT EXISTS idx_freq_ts ON frequencies(timestamp_ms DESC);

-- raw signal detection / classification events
CREATE TABLE IF NOT EXISTS signal_events (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    frequency_hz      INTEGER NOT NULL,
    signal_class      TEXT,
    top_family        TEXT,
    top_confidence    REAL,
    sub_protocol      TEXT,
    symbol_rate       REAL,
    bandwidth_hz      INTEGER,
    snr_db            REAL,
    decode_success    INTEGER NOT NULL DEFAULT 0,
    decode_protocol   TEXT,
    decode_summary    TEXT,
    likely_proprietary INTEGER NOT NULL DEFAULT 0,
    waterfall_psd     TEXT,
    range_name        TEXT,
    timestamp_ms      INTEGER NOT NULL,
    is_novel          INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_signal_ts ON signal_events(timestamp_ms DESC);
CREATE INDEX IF NOT EXISTS idx_signal_freq ON signal_events(frequency_hz);

-- P25 / NXDN / EDACS / DMR talkgroup directory
CREATE TABLE IF NOT EXISTS talkgroups (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    system_name   TEXT NOT NULL,
    talkgroup_id  TEXT NOT NULL,
    alpha_tag     TEXT,
    description   TEXT,
    category      TEXT,
    tag           TEXT,
    mode          TEXT,
    protocol      TEXT,
    encrypted     INTEGER NOT NULL DEFAULT 0,
    hit_count     INTEGER NOT NULL DEFAULT 0,
    first_seen_ms INTEGER NOT NULL,
    last_seen_ms  INTEGER NOT NULL,
    UNIQUE(system_name, talkgroup_id)
);

-- rtl_433 / sensor packets
CREATE TABLE IF NOT EXISTS sensor_messages (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp    INTEGER NOT NULL,
    frequency_hz INTEGER NOT NULL,
    model        TEXT,
    sensor_id    TEXT,
    raw_json     TEXT
);
CREATE INDEX IF NOT EXISTS idx_sensor_ts ON sensor_messages(timestamp DESC);

-- analyst-maintained frequency exclusions
CREATE TABLE IF NOT EXISTS blacklist (
    frequency_hz INTEGER PRIMARY KEY,
    reason       TEXT NOT NULL DEFAULT '',
    temporary    INTEGER NOT NULL DEFAULT 0,
    created_ms   INTEGER NOT NULL
);

-- spectrum occupancy: 15-min time buckets per ~10 kHz frequency bucket
CREATE TABLE IF NOT EXISTS spectrum_occupancy (
    frequency_bucket_hz  INTEGER NOT NULL,
    time_bucket_15min    INTEGER NOT NULL,
    avg_power_db         REAL,
    peak_power_db        REAL,
    avg_above_floor_db   REAL,
    sample_count         INTEGER NOT NULL DEFAULT 0,
    noise_floor_db       REAL,
    PRIMARY KEY (frequency_bucket_hz, time_bucket_15min)
);

-- cases (analyst groupings) + attachments
CREATE TABLE IF NOT EXISTS cases (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    description TEXT,
    status      TEXT NOT NULL DEFAULT 'open',
    tags        TEXT,
    created_ms  INTEGER NOT NULL,
    updated_ms  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS case_attachments (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id     INTEGER NOT NULL,
    kind        TEXT NOT NULL,        -- 'decoded_message' | 'recording' | 'signal_event' | 'note'
    ref         TEXT NOT NULL,        -- ID or path
    note        TEXT,
    attached_ms INTEGER NOT NULL,
    FOREIGN KEY (case_id) REFERENCES cases(id) ON DELETE CASCADE
);
DELETE FROM case_attachments WHERE id NOT IN (SELECT MIN(id) FROM case_attachments GROUP BY case_id,kind,ref);
CREATE UNIQUE INDEX IF NOT EXISTS idx_case_attachment_unique ON case_attachments(case_id, kind, ref);

-- Normalized geospatial observations.  A row is an immutable observation;
-- current tracks are derived by selecting the newest row for each entity.
CREATE TABLE IF NOT EXISTS position_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_kind TEXT NOT NULL CHECK(entity_kind IN ('aircraft','vessel','aprs','radiosonde')),
    entity_id TEXT NOT NULL,
    timestamp_ms INTEGER NOT NULL,
    latitude REAL NOT NULL CHECK(latitude >= -90 AND latitude <= 90),
    longitude REAL NOT NULL CHECK(longitude >= -180 AND longitude <= 180),
    altitude_m REAL,
    speed_mps REAL,
    heading_deg REAL,
    accuracy_m REAL,
    vertical_accuracy_m REAL,
    source TEXT NOT NULL,
    source_message_id INTEGER,
    metadata_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_position_entity_time ON position_events(entity_kind, entity_id, timestamp_ms DESC);
CREATE INDEX IF NOT EXISTS idx_position_time ON position_events(timestamp_ms DESC);

CREATE TABLE IF NOT EXISTS radio_channels (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    frequency_hz INTEGER NOT NULL CHECK(frequency_hz > 0),
    duplex TEXT NOT NULL DEFAULT '', offset_hz INTEGER NOT NULL DEFAULT 0,
    tone TEXT NOT NULL DEFAULT '', mode TEXT NOT NULL DEFAULT 'FM',
    source TEXT NOT NULL DEFAULT 'chirp', imported_ms INTEGER NOT NULL,
    UNIQUE(name, frequency_hz)
);

CREATE TABLE IF NOT EXISTS recording_annotations (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    recording_path TEXT NOT NULL,
    offset_ms      INTEGER NOT NULL,
    text           TEXT,
    created_ms     INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS scheduled_jobs (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    name            TEXT NOT NULL,
    kind            TEXT NOT NULL, -- scan | recording | decode
    payload_json    TEXT NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1,
    next_run_ms     INTEGER,
    last_run_ms     INTEGER,
    last_status     TEXT NOT NULL DEFAULT 'pending',
    last_error      TEXT NOT NULL DEFAULT '',
    created_ms      INTEGER NOT NULL,
    updated_ms      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_due ON scheduled_jobs(enabled, next_run_ms);

-- query planner stats
ANALYZE;
