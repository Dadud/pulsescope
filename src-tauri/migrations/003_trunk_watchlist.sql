-- Append-only: persisted talkgroup watchlist for trunk follower filtering.
CREATE TABLE IF NOT EXISTS talkgroup_watchlist (
    system_name   TEXT NOT NULL,
    talkgroup_id  TEXT NOT NULL,
    created_ms    INTEGER NOT NULL,
    PRIMARY KEY (system_name, talkgroup_id)
);
CREATE INDEX IF NOT EXISTS idx_talkgroup_watchlist_system ON talkgroup_watchlist(system_name);
