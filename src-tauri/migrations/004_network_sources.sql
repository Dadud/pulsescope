-- Append-only: registered external IQ/network sources for the multi-device registry.
CREATE TABLE IF NOT EXISTS network_iq_sources (
    id           TEXT PRIMARY KEY,
    label        TEXT NOT NULL,
    kind         TEXT NOT NULL,
    host         TEXT NOT NULL,
    port         INTEGER NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 0,
    created_ms   INTEGER NOT NULL,
    updated_ms   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_network_iq_sources_enabled ON network_iq_sources(enabled);
