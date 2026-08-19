-- Durable receiver workspace objects. These are server-owned so every LAN
-- client sees the same profiles and bookmarks.
CREATE TABLE IF NOT EXISTS receiver_profiles (
    id                   TEXT PRIMARY KEY,
    name                 TEXT NOT NULL,
    center_frequency_hz  INTEGER NOT NULL,
    sample_rate_hz       INTEGER NOT NULL,
    bandwidth_hz         INTEGER NOT NULL,
    mode                 TEXT NOT NULL DEFAULT 'nfm',
    region               TEXT NOT NULL DEFAULT '',
    deemphasis_us        INTEGER,
    gain_policy_json     TEXT NOT NULL DEFAULT '{}',
    decoder_policy_json  TEXT NOT NULL DEFAULT '{}',
    created_ms           INTEGER NOT NULL,
    updated_ms           INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS receiver_bookmarks (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    label         TEXT NOT NULL,
    frequency_hz  INTEGER NOT NULL,
    mode          TEXT NOT NULL DEFAULT 'nfm',
    bandwidth_hz  INTEGER NOT NULL DEFAULT 12500,
    profile_id    TEXT,
    color         TEXT NOT NULL DEFAULT '',
    decoder       TEXT NOT NULL DEFAULT '',
    notes         TEXT NOT NULL DEFAULT '',
    enabled       INTEGER NOT NULL DEFAULT 1,
    created_ms    INTEGER NOT NULL,
    updated_ms    INTEGER NOT NULL,
    FOREIGN KEY (profile_id) REFERENCES receiver_profiles(id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS idx_receiver_bookmarks_frequency ON receiver_bookmarks(frequency_hz);
