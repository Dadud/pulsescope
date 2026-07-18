-- Normalized trunking inventory and observed call lifecycle.
CREATE TABLE IF NOT EXISTS trunk_systems (
 id INTEGER PRIMARY KEY AUTOINCREMENT, protocol TEXT NOT NULL, system_key TEXT NOT NULL UNIQUE,
 name TEXT NOT NULL, wacn INTEGER, system_id INTEGER, created_ms INTEGER NOT NULL, last_seen_ms INTEGER
);
CREATE TABLE IF NOT EXISTS trunk_sites (
 id INTEGER PRIMARY KEY AUTOINCREMENT, system_id INTEGER NOT NULL, rfss_id INTEGER NOT NULL, site_id INTEGER NOT NULL,
 name TEXT NOT NULL DEFAULT '', control_channel_hz INTEGER, last_seen_ms INTEGER,
 UNIQUE(system_id,rfss_id,site_id), FOREIGN KEY(system_id) REFERENCES trunk_systems(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS trunk_control_channels (
 id INTEGER PRIMARY KEY AUTOINCREMENT, site_id INTEGER NOT NULL, frequency_hz INTEGER NOT NULL,
 is_primary INTEGER NOT NULL DEFAULT 0, first_seen_ms INTEGER NOT NULL, last_seen_ms INTEGER NOT NULL,
 confidence REAL NOT NULL DEFAULT 0, evidence_json TEXT NOT NULL DEFAULT '{}',
 UNIQUE(site_id,frequency_hz), FOREIGN KEY(site_id) REFERENCES trunk_sites(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS trunk_talkgroups (
 id INTEGER PRIMARY KEY AUTOINCREMENT, system_id INTEGER NOT NULL, talkgroup_id INTEGER NOT NULL,
 alpha_tag TEXT NOT NULL DEFAULT '', description TEXT NOT NULL DEFAULT '', policy TEXT NOT NULL DEFAULT 'allow',
 priority INTEGER NOT NULL DEFAULT 0, locked_out INTEGER NOT NULL DEFAULT 0, encrypted_seen INTEGER NOT NULL DEFAULT 0,
 first_seen_ms INTEGER NOT NULL, last_seen_ms INTEGER NOT NULL, UNIQUE(system_id,talkgroup_id),
 FOREIGN KEY(system_id) REFERENCES trunk_systems(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS trunk_units (
 id INTEGER PRIMARY KEY AUTOINCREMENT, system_id INTEGER NOT NULL, unit_id INTEGER NOT NULL, alpha_tag TEXT NOT NULL DEFAULT '',
 first_seen_ms INTEGER NOT NULL, last_seen_ms INTEGER NOT NULL, UNIQUE(system_id,unit_id),
 FOREIGN KEY(system_id) REFERENCES trunk_systems(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS trunk_grants (
 id INTEGER PRIMARY KEY AUTOINCREMENT, site_id INTEGER NOT NULL, talkgroup_id INTEGER NOT NULL, source_unit_id INTEGER,
 service_options INTEGER NOT NULL DEFAULT 0, channel_identifier INTEGER NOT NULL, channel_number INTEGER NOT NULL,
 frequency_hz INTEGER, observed_ms INTEGER NOT NULL, confidence REAL NOT NULL, evidence_json TEXT NOT NULL,
 FOREIGN KEY(site_id) REFERENCES trunk_sites(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS trunk_calls (
 id TEXT PRIMARY KEY, system_id INTEGER NOT NULL, site_id INTEGER NOT NULL, grant_id INTEGER,
 talkgroup_id INTEGER NOT NULL, source_unit_id INTEGER, frequency_hz INTEGER NOT NULL, protocol TEXT NOT NULL,
 decoder TEXT NOT NULL, encrypted INTEGER NOT NULL DEFAULT 0, started_ms INTEGER NOT NULL, ended_ms INTEGER,
 termination_reason TEXT, recording_path TEXT, audio_sidecar_id TEXT,
 FOREIGN KEY(system_id) REFERENCES trunk_systems(id), FOREIGN KEY(site_id) REFERENCES trunk_sites(id),
 FOREIGN KEY(grant_id) REFERENCES trunk_grants(id)
);
CREATE INDEX IF NOT EXISTS idx_trunk_calls_started ON trunk_calls(started_ms DESC);
CREATE INDEX IF NOT EXISTS idx_trunk_grants_observed ON trunk_grants(observed_ms DESC);
