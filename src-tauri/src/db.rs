// db.rs — SQLite schema, types, and connection pool for PulseScope.
//
// Schema mirrors the table shape used across the SDR scanner category
// (scan history, decoded messages, talkgroups, signal events, cases,
// spectrum occupancy). Implementation is original rusqlite code.

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Thin pool — rusqlite connections aren't Sync, so guard one Connection
/// with a Mutex. PulseScope has low concurrency (one writer, the UI reader).
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(include_str!("../migrations/001_init.sql"))?;
        conn.execute_batch(include_str!("../migrations/002_receiver_workspace.sql"))?;
        conn.execute_batch(include_str!("../migrations/003_trunk_watchlist.sql"))?;
        conn.execute_batch(include_str!("../migrations/004_network_sources.sql"))?;
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.pragma_update(None, "synchronous", "normal")?;
        conn.pragma_update(None, "foreign_keys", "on")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        // unreachable: we use parking_lot::Mutex below
        unreachable!()
    }
}

// Re-exports to keep call sites clean
impl Db {
    pub fn conn(&self) -> parking_lot::MutexGuard<'_, Connection> {
        self.conn.lock()
    }

    pub fn integrity_check(&self) -> anyhow::Result<bool> {
        Ok(self
            .conn()
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?
            == "ok")
    }
}

// ── types ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frequency {
    pub id: Option<i64>,
    pub frequency_hz: u64,
    pub strength_db: f32,
    pub snr_db: f32,
    pub mode: String,
    pub range_name: String,
    pub bandwidth_hz: u32,
    pub timestamp_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalEvent {
    pub id: Option<i64>,
    pub frequency_hz: u64,
    pub signal_class: String,
    pub top_family: String,
    pub top_confidence: f32,
    pub sub_protocol: String,
    pub symbol_rate: f32,
    pub bandwidth_hz: u32,
    pub snr_db: f32,
    pub decode_success: bool,
    pub decode_protocol: String,
    pub decode_summary: String,
    pub likely_proprietary: bool,
    pub waterfall_psd: String, // JSON blob
    pub range_name: String,
    pub timestamp_ms: i64,
    pub is_novel: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecodedMessage {
    pub id: Option<i64>,
    pub frequency_hz: u64,
    pub protocol: String,
    pub message_type: String,
    pub address: String,
    pub function_code: String,
    pub content: String,
    pub raw: String,
    pub encryption: String,
    pub timestamp_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Talkgroup {
    pub id: Option<i64>,
    pub system_name: String,
    pub talkgroup_id: String,
    pub alpha_tag: String,
    pub description: String,
    pub category: String,
    pub tag: String,
    pub mode: String,
    pub protocol: String,
    pub encrypted: bool,
    pub hit_count: i64,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SensorMessage {
    pub id: Option<i64>,
    pub timestamp: i64,
    pub frequency_hz: u64,
    pub model: String,
    pub sensor_id: String,
    pub raw_json: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Case {
    pub id: Option<i64>,
    pub name: String,
    pub description: String,
    pub status: String,
    pub tags: String,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaseAttachment {
    pub id: Option<i64>,
    pub case_id: i64,
    pub kind: String,
    pub r#ref: String,
    pub note: String,
    pub attached_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordingAnnotation {
    pub id: Option<i64>,
    pub recording_path: String,
    pub offset_ms: i64,
    pub text: String,
    pub created_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScheduledJob {
    pub id: Option<i64>,
    pub name: String,
    pub kind: String,
    pub payload_json: String,
    pub enabled: bool,
    pub next_run_ms: Option<i64>,
    pub last_run_ms: Option<i64>,
    pub last_status: String,
    pub last_error: String,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkIqSource {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub host: String,
    pub port: i64,
    pub enabled: bool,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivityTimelineEntry {
    pub kind: String,
    pub timestamp_ms: i64,
    pub frequency_hz: u64,
    pub protocol: String,
    pub summary: String,
    pub detail: String,
    #[serde(default)]
    pub correlation_group: String,
    #[serde(default)]
    pub correlation_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpectrumOccupancy {
    pub frequency_bucket_hz: u64,
    pub time_bucket_15min: i64,
    pub avg_power_db: f32,
    pub peak_power_db: f32,
    pub avg_above_floor_db: f32,
    pub sample_count: i64,
    pub noise_floor_db: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlacklistEntry {
    pub frequency_hz: u64,
    pub reason: String,
    pub temporary: bool,
    pub created_ms: i64,
}

impl Db {
    pub fn insert_signal_event(
        &self,
        frequency_hz: u64,
        _strength_db: f32,
        snr_db: f32,
        bandwidth_hz: u32,
        range_name: &str,
        timestamp_ms: i64,
    ) -> anyhow::Result<i64> {
        // Backward-compatible path: no classification provided.
        self.insert_classified_signal_event(
            frequency_hz,
            snr_db,
            bandwidth_hz,
            range_name,
            timestamp_ms,
            "unknown",
            "unknown",
            0.0,
            "",
            false,
            "",
            "",
            false,
            true,
        )
    }

    /// Insert a signal event with full auto-classification fields populated.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_classified_signal_event(
        &self,
        frequency_hz: u64,
        snr_db: f32,
        bandwidth_hz: u32,
        range_name: &str,
        timestamp_ms: i64,
        signal_class: &str,
        top_family: &str,
        top_confidence: f32,
        sub_protocol: &str,
        decode_success: bool,
        decode_protocol: &str,
        decode_summary: &str,
        likely_proprietary: bool,
        is_novel: bool,
    ) -> anyhow::Result<i64> {
        let c = self.conn();
        c.execute(
            "INSERT INTO signal_events (
                frequency_hz, signal_class, top_family, top_confidence, sub_protocol,
                bandwidth_hz, snr_db, decode_success, decode_protocol, decode_summary,
                likely_proprietary, waterfall_psd, range_name, timestamp_ms, is_novel
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'',?12,?13,?14)",
            rusqlite::params![
                frequency_hz as i64,
                signal_class,
                top_family,
                top_confidence,
                sub_protocol,
                bandwidth_hz as i64,
                snr_db,
                if decode_success { 1i64 } else { 0i64 },
                decode_protocol,
                decode_summary,
                if likely_proprietary { 1i64 } else { 0i64 },
                range_name,
                timestamp_ms,
                if is_novel { 1i64 } else { 0i64 },
            ],
        )?;
        Ok(c.last_insert_rowid())
    }

    pub fn recent_signal_events(&self, limit: u32) -> anyhow::Result<Vec<SignalEvent>> {
        let c = self.conn();
        let mut q = c.prepare("SELECT id,frequency_hz,signal_class,top_family,top_confidence,sub_protocol,symbol_rate,bandwidth_hz,snr_db,decode_success,decode_protocol,decode_summary,likely_proprietary,waterfall_psd,range_name,timestamp_ms,is_novel FROM signal_events ORDER BY timestamp_ms DESC LIMIT ?1")?;
        let rows = q.query_map([limit.clamp(1, 1000)], |r| {
            Ok(SignalEvent {
                id: Some(r.get(0)?),
                frequency_hz: r.get::<_, i64>(1)? as u64,
                signal_class: r.get(2)?,
                top_family: r.get(3)?,
                top_confidence: r.get(4)?,
                sub_protocol: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                symbol_rate: r.get::<_, Option<f32>>(6)?.unwrap_or_default(),
                bandwidth_hz: r.get::<_, Option<i64>>(7)?.unwrap_or_default() as u32,
                snr_db: r.get::<_, Option<f32>>(8)?.unwrap_or_default(),
                decode_success: r.get::<_, i64>(9)? != 0,
                decode_protocol: r.get::<_, Option<String>>(10)?.unwrap_or_default(),
                decode_summary: r.get::<_, Option<String>>(11)?.unwrap_or_default(),
                likely_proprietary: r.get::<_, i64>(12)? != 0,
                waterfall_psd: r.get::<_, Option<String>>(13)?.unwrap_or_default(),
                range_name: r.get::<_, Option<String>>(14)?.unwrap_or_default(),
                timestamp_ms: r.get(15)?,
                is_novel: r.get::<_, i64>(16)? != 0,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn insert_decoded_message(&self, m: &DecodedMessage) -> anyhow::Result<i64> {
        let c = self.conn();
        c.execute(
            "INSERT INTO decoded_messages (frequency_hz, protocol, message_type, address, \
             function_code, content, raw, encryption, timestamp_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![
                m.frequency_hz as i64, m.protocol, m.message_type, m.address,
                m.function_code, m.content, m.raw, m.encryption, m.timestamp_ms,
            ],
        )?;
        Ok(c.last_insert_rowid())
    }

    pub fn insert_talkgroup(&self, t: &Talkgroup) -> anyhow::Result<i64> {
        let c = self.conn();
        c.execute(
            "INSERT INTO talkgroups (system_name, talkgroup_id, alpha_tag, description, category, \
             tag, mode, protocol, encrypted, hit_count, first_seen_ms, last_seen_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            rusqlite::params![
                t.system_name,
                t.talkgroup_id,
                t.alpha_tag,
                t.description,
                t.category,
                t.tag,
                t.mode,
                t.protocol,
                t.encrypted,
                t.hit_count,
                t.first_seen_ms,
                t.last_seen_ms,
            ],
        )?;
        Ok(c.last_insert_rowid())
    }

    pub fn upsert_occupancy(&self, o: &SpectrumOccupancy) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO spectrum_occupancy (frequency_bucket_hz,time_bucket_15min,avg_power_db,peak_power_db,avg_above_floor_db,sample_count,noise_floor_db) VALUES (?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(frequency_bucket_hz,time_bucket_15min) DO UPDATE SET
               avg_power_db=(spectrum_occupancy.avg_power_db * spectrum_occupancy.sample_count + excluded.avg_power_db * excluded.sample_count) / (spectrum_occupancy.sample_count + excluded.sample_count),
               peak_power_db=MAX(spectrum_occupancy.peak_power_db, excluded.peak_power_db),
               avg_above_floor_db=(spectrum_occupancy.avg_above_floor_db * spectrum_occupancy.sample_count + excluded.avg_above_floor_db * excluded.sample_count) / (spectrum_occupancy.sample_count + excluded.sample_count),
               sample_count=spectrum_occupancy.sample_count + excluded.sample_count,
               noise_floor_db=(spectrum_occupancy.noise_floor_db * spectrum_occupancy.sample_count + excluded.noise_floor_db * excluded.sample_count) / (spectrum_occupancy.sample_count + excluded.sample_count)",
            rusqlite::params![o.frequency_bucket_hz as i64, o.time_bucket_15min, o.avg_power_db, o.peak_power_db, o.avg_above_floor_db, o.sample_count, o.noise_floor_db],
        )?;
        Ok(())
    }

    pub fn recent_occupancy(&self, limit: u32) -> anyhow::Result<Vec<SpectrumOccupancy>> {
        let c = self.conn();
        let mut q = c.prepare("SELECT frequency_bucket_hz,time_bucket_15min,avg_power_db,peak_power_db,avg_above_floor_db,sample_count,noise_floor_db FROM spectrum_occupancy ORDER BY time_bucket_15min DESC,frequency_bucket_hz LIMIT ?1")?;
        let rows = q.query_map([limit], |r| {
            Ok(SpectrumOccupancy {
                frequency_bucket_hz: r.get::<_, i64>(0)? as u64,
                time_bucket_15min: r.get(1)?,
                avg_power_db: r.get(2)?,
                peak_power_db: r.get(3)?,
                avg_above_floor_db: r.get(4)?,
                sample_count: r.get(5)?,
                noise_floor_db: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn occupancy_since_bucket(
        &self,
        since_bucket: i64,
    ) -> anyhow::Result<Vec<SpectrumOccupancy>> {
        let c = self.conn();
        let mut q = c.prepare(
            "SELECT frequency_bucket_hz,time_bucket_15min,avg_power_db,peak_power_db,avg_above_floor_db,sample_count,noise_floor_db \
             FROM spectrum_occupancy WHERE time_bucket_15min >= ?1 \
             ORDER BY time_bucket_15min ASC, frequency_bucket_hz ASC",
        )?;
        let rows = q.query_map([since_bucket], |r| {
            Ok(SpectrumOccupancy {
                frequency_bucket_hz: r.get::<_, i64>(0)? as u64,
                time_bucket_15min: r.get(1)?,
                avg_power_db: r.get(2)?,
                peak_power_db: r.get(3)?,
                avg_above_floor_db: r.get(4)?,
                sample_count: r.get(5)?,
                noise_floor_db: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn list_talkgroup_watchlist(&self) -> anyhow::Result<Vec<(String, String)>> {
        let c = self.conn();
        let mut q = c.prepare(
            "SELECT system_name, talkgroup_id FROM talkgroup_watchlist ORDER BY system_name, talkgroup_id",
        )?;
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn set_talkgroup_watched(
        &self,
        system_name: &str,
        talkgroup_id: &str,
        watched: bool,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        let c = self.conn();
        if watched {
            c.execute(
                "INSERT INTO talkgroup_watchlist (system_name, talkgroup_id, created_ms) VALUES (?1,?2,?3) \
                 ON CONFLICT(system_name, talkgroup_id) DO NOTHING",
                rusqlite::params![system_name, talkgroup_id, now_ms],
            )?;
        } else {
            c.execute(
                "DELETE FROM talkgroup_watchlist WHERE system_name=?1 AND talkgroup_id=?2",
                rusqlite::params![system_name, talkgroup_id],
            )?;
        }
        Ok(())
    }

    pub fn is_talkgroup_watched(&self, system_name: &str, talkgroup_id: &str) -> bool {
        self.conn()
            .query_row(
                "SELECT 1 FROM talkgroup_watchlist WHERE system_name=?1 AND talkgroup_id=?2 LIMIT 1",
                rusqlite::params![system_name, talkgroup_id],
                |_| Ok(()),
            )
            .optional()
            .ok()
            .flatten()
            .is_some()
    }

    pub fn list_network_iq_sources(&self) -> anyhow::Result<Vec<NetworkIqSource>> {
        let c = self.conn();
        let mut q = c.prepare(
            "SELECT id,label,kind,host,port,enabled,created_ms,updated_ms FROM network_iq_sources ORDER BY label",
        )?;
        let rows = q.query_map([], |r| {
            Ok(NetworkIqSource {
                id: r.get(0)?,
                label: r.get(1)?,
                kind: r.get(2)?,
                host: r.get(3)?,
                port: r.get(4)?,
                enabled: r.get::<_, i64>(5)? != 0,
                created_ms: r.get(6)?,
                updated_ms: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn upsert_network_iq_source(&self, source: &NetworkIqSource) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO network_iq_sources (id,label,kind,host,port,enabled,created_ms,updated_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(id) DO UPDATE SET label=excluded.label,kind=excluded.kind,host=excluded.host,port=excluded.port,enabled=excluded.enabled,updated_ms=excluded.updated_ms",
            rusqlite::params![
                source.id,
                source.label,
                source.kind,
                source.host,
                source.port,
                source.enabled,
                source.created_ms,
                source.updated_ms,
            ],
        )?;
        Ok(())
    }

    pub fn delete_network_iq_source(&self, id: &str) -> anyhow::Result<usize> {
        Ok(self
            .conn()
            .execute("DELETE FROM network_iq_sources WHERE id=?1", [id])?)
    }

    pub fn get_network_iq_source(&self, id: &str) -> anyhow::Result<Option<NetworkIqSource>> {
        Ok(self
            .list_network_iq_sources()?
            .into_iter()
            .find(|source| source.id == id))
    }

    pub fn set_active_network_source(&self, id: &str) -> anyhow::Result<Option<NetworkIqSource>> {
        let now = crate::scanner::now_ms();
        let mut selected = None;
        for mut source in self.list_network_iq_sources()? {
            let enabled = source.id == id;
            if enabled {
                source.enabled = true;
                source.updated_ms = now;
                selected = Some(source.clone());
            } else if source.enabled {
                source.enabled = false;
                source.updated_ms = now;
            } else {
                continue;
            }
            self.upsert_network_iq_source(&source)?;
        }
        Ok(selected)
    }

    pub fn clear_active_network_sources(&self) -> anyhow::Result<()> {
        let now = crate::scanner::now_ms();
        for mut source in self.list_network_iq_sources()? {
            if !source.enabled {
                continue;
            }
            source.enabled = false;
            source.updated_ms = now;
            self.upsert_network_iq_source(&source)?;
        }
        Ok(())
    }

    fn correlation_group_key(frequency_hz: u64, timestamp_ms: i64) -> String {
        let freq_bucket = (frequency_hz / 25_000) * 25_000;
        let time_bucket = timestamp_ms / (15 * 60 * 1000);
        format!("{freq_bucket}:{time_bucket}")
    }

    pub fn activity_timeline(&self, since_ms: i64, limit: u32) -> anyhow::Result<Vec<ActivityTimelineEntry>> {
        let mut entries = Vec::new();
        for message in self.recent_decoded_messages(limit)? {
            if message.timestamp_ms < since_ms {
                continue;
            }
            entries.push(ActivityTimelineEntry {
                kind: "decoded_message".into(),
                timestamp_ms: message.timestamp_ms,
                frequency_hz: message.frequency_hz,
                protocol: message.protocol.clone(),
                summary: message.content.clone(),
                detail: message.address.clone(),
                correlation_group: String::new(),
                correlation_count: 0,
            });
        }
        for event in self.recent_signal_events(limit)? {
            if event.timestamp_ms < since_ms {
                continue;
            }
            entries.push(ActivityTimelineEntry {
                kind: if event.is_novel {
                    "novel_signal".into()
                } else {
                    "signal_event".into()
                },
                timestamp_ms: event.timestamp_ms,
                frequency_hz: event.frequency_hz,
                protocol: event.decode_protocol.clone(),
                summary: event.decode_summary.clone(),
                detail: event.signal_class.clone(),
                correlation_group: String::new(),
                correlation_count: 0,
            });
        }
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.timestamp_ms));
        entries.truncate(limit as usize);
        let mut group_counts = std::collections::HashMap::new();
        for entry in &entries {
            *group_counts
                .entry(Self::correlation_group_key(
                    entry.frequency_hz,
                    entry.timestamp_ms,
                ))
                .or_insert(0u32) += 1;
        }
        for entry in &mut entries {
            let key = Self::correlation_group_key(entry.frequency_hz, entry.timestamp_ms);
            entry.correlation_group = key.clone();
            entry.correlation_count = *group_counts.get(&key).unwrap_or(&1);
        }
        Ok(entries)
    }

    pub fn decoded_message_count(&self) -> anyhow::Result<i64> {
        Ok(self
            .conn()
            .query_row("SELECT COUNT(*) FROM decoded_messages", [], |r| r.get(0))?)
    }

    pub fn messages_by_protocol(
        &self,
        protocol: Option<&str>,
        limit: u32,
    ) -> anyhow::Result<Vec<DecodedMessage>> {
        let c = self.conn();
        let mut out = Vec::new();
        if let Some(protocol) = protocol {
            let mut q = c.prepare("SELECT id,frequency_hz,protocol,message_type,address,function_code,content,raw,encryption,timestamp_ms FROM decoded_messages WHERE protocol=?1 ORDER BY id DESC LIMIT ?2")?;
            let rows = q.query_map(rusqlite::params![protocol, limit], |r| {
                Ok(DecodedMessage {
                    id: Some(r.get(0)?),
                    frequency_hz: r.get::<_, i64>(1)? as u64,
                    protocol: r.get(2)?,
                    message_type: r.get(3)?,
                    address: r.get(4)?,
                    function_code: r.get(5)?,
                    content: r.get(6)?,
                    raw: r.get(7)?,
                    encryption: r.get(8)?,
                    timestamp_ms: r.get(9)?,
                })
            })?;
            for row in rows {
                out.push(row?);
            }
        } else {
            let mut q = c.prepare("SELECT id,frequency_hz,protocol,message_type,address,function_code,content,raw,encryption,timestamp_ms FROM decoded_messages WHERE protocol NOT IN ('rtl_433') ORDER BY id DESC LIMIT ?1")?;
            let rows = q.query_map([limit], |r| {
                Ok(DecodedMessage {
                    id: Some(r.get(0)?),
                    frequency_hz: r.get::<_, i64>(1)? as u64,
                    protocol: r.get(2)?,
                    message_type: r.get(3)?,
                    address: r.get(4)?,
                    function_code: r.get(5)?,
                    content: r.get(6)?,
                    raw: r.get(7)?,
                    encryption: r.get(8)?,
                    timestamp_ms: r.get(9)?,
                })
            })?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    pub fn messages_by_protocols(
        &self,
        protocols: &[&str],
        limit: u32,
    ) -> anyhow::Result<Vec<DecodedMessage>> {
        let mut all = Vec::new();
        for protocol in protocols {
            all.extend(self.messages_by_protocol(Some(protocol), limit)?);
        }
        all.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms).then(b.id.cmp(&a.id)));
        all.truncate(limit as usize);
        Ok(all)
    }

    pub fn delete_messages_by_protocol(&self, protocol: &str) -> anyhow::Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM decoded_messages WHERE protocol=?1",
            rusqlite::params![protocol],
        )?)
    }

    pub fn recent_decoded_messages(&self, limit: u32) -> anyhow::Result<Vec<DecodedMessage>> {
        let c = self.conn();
        let mut stmt = c.prepare(
            "SELECT id, frequency_hz, protocol, message_type, address, function_code, content, \
             raw, encryption, timestamp_ms FROM decoded_messages ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |r| {
            Ok(DecodedMessage {
                id: Some(r.get::<_, i64>(0)?),
                frequency_hz: r.get::<_, i64>(1)? as u64,
                protocol: r.get(2)?,
                message_type: r.get(3)?,
                address: r.get(4)?,
                function_code: r.get(5)?,
                content: r.get(6)?,
                raw: r.get(7)?,
                encryption: r.get(8)?,
                timestamp_ms: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn list_talkgroups(&self) -> anyhow::Result<Vec<Talkgroup>> {
        let c = self.conn();
        let mut q = c.prepare("SELECT id,system_name,talkgroup_id,alpha_tag,description,category,tag,mode,protocol,encrypted,hit_count,first_seen_ms,last_seen_ms FROM talkgroups ORDER BY system_name,talkgroup_id")?;
        let rows = q.query_map([], |r| {
            Ok(Talkgroup {
                id: Some(r.get(0)?),
                system_name: r.get(1)?,
                talkgroup_id: r.get(2)?,
                alpha_tag: r.get(3)?,
                description: r.get(4)?,
                category: r.get(5)?,
                tag: r.get(6)?,
                mode: r.get(7)?,
                protocol: r.get(8)?,
                encrypted: r.get::<_, i64>(9)? != 0,
                hit_count: r.get(10)?,
                first_seen_ms: r.get(11)?,
                last_seen_ms: r.get(12)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn upsert_talkgroup(&self, t: &Talkgroup) -> anyhow::Result<()> {
        self.conn().execute("INSERT INTO talkgroups (system_name,talkgroup_id,alpha_tag,description,category,tag,mode,protocol,encrypted,hit_count,first_seen_ms,last_seen_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(system_name,talkgroup_id) DO UPDATE SET alpha_tag=excluded.alpha_tag,description=excluded.description,category=excluded.category,tag=excluded.tag,mode=excluded.mode,protocol=excluded.protocol,encrypted=excluded.encrypted,hit_count=excluded.hit_count,last_seen_ms=excluded.last_seen_ms", rusqlite::params![t.system_name,t.talkgroup_id,t.alpha_tag,t.description,t.category,t.tag,t.mode,t.protocol,t.encrypted,t.hit_count,t.first_seen_ms,t.last_seen_ms])?;
        Ok(())
    }

    pub fn delete_talkgroup_system(&self, system: &str) -> anyhow::Result<usize> {
        Ok(self
            .conn()
            .execute("DELETE FROM talkgroups WHERE system_name=?1", [system])?)
    }

    pub fn talkgroup_systems(&self) -> anyhow::Result<Vec<String>> {
        let c = self.conn();
        let mut q =
            c.prepare("SELECT DISTINCT system_name FROM talkgroups ORDER BY system_name")?;
        let rows = q.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn export_talkgroups(&self) -> anyhow::Result<Vec<Talkgroup>> {
        self.list_talkgroups()
    }

    pub fn list_cases(&self) -> anyhow::Result<Vec<Case>> {
        let c = self.conn();
        let mut q = c.prepare("SELECT id,name,description,status,tags,created_ms,updated_ms FROM cases ORDER BY updated_ms DESC")?;
        let rows = q.query_map([], |r| {
            Ok(Case {
                id: Some(r.get(0)?),
                name: r.get(1)?,
                description: r.get(2)?,
                status: r.get(3)?,
                tags: r.get(4)?,
                created_ms: r.get(5)?,
                updated_ms: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn create_case(&self, case: &Case) -> anyhow::Result<i64> {
        let c = self.conn();
        c.execute("INSERT INTO cases (name,description,status,tags,created_ms,updated_ms) VALUES (?1,?2,?3,?4,?5,?6)", rusqlite::params![case.name,case.description,case.status,case.tags,case.created_ms,case.updated_ms])?;
        Ok(c.last_insert_rowid())
    }

    pub fn get_case(&self, id: i64) -> anyhow::Result<Option<Case>> {
        let c = self.conn();
        let mut q = c.prepare(
            "SELECT id,name,description,status,tags,created_ms,updated_ms FROM cases WHERE id=?1",
        )?;
        Ok(q.query_row([id], |r| {
            Ok(Case {
                id: Some(r.get(0)?),
                name: r.get(1)?,
                description: r.get(2)?,
                status: r.get(3)?,
                tags: r.get(4)?,
                created_ms: r.get(5)?,
                updated_ms: r.get(6)?,
            })
        })
        .optional()?)
    }

    pub fn delete_case(&self, id: i64) -> anyhow::Result<usize> {
        Ok(self.conn().execute("DELETE FROM cases WHERE id=?1", [id])?)
    }

    pub fn add_case_attachment(&self, a: &CaseAttachment) -> anyhow::Result<i64> {
        let c = self.conn();
        c.execute("INSERT INTO case_attachments (case_id,kind,ref,note,attached_ms) VALUES (?1,?2,?3,?4,?5)",rusqlite::params![a.case_id,a.kind,a.r#ref,a.note,a.attached_ms])?;
        Ok(c.last_insert_rowid())
    }
    pub fn case_attachments(&self, case_id: i64) -> anyhow::Result<Vec<CaseAttachment>> {
        let c = self.conn();
        let mut q=c.prepare("SELECT id,case_id,kind,ref,note,attached_ms FROM case_attachments WHERE case_id=?1 ORDER BY attached_ms DESC")?;
        let rows = q.query_map([case_id], |r| {
            Ok(CaseAttachment {
                id: Some(r.get(0)?),
                case_id: r.get(1)?,
                kind: r.get(2)?,
                r#ref: r.get(3)?,
                note: r.get(4)?,
                attached_ms: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn case_attachment(&self, id: i64) -> anyhow::Result<Option<CaseAttachment>> {
        let c = self.conn();
        let mut q = c.prepare(
            "SELECT id,case_id,kind,ref,note,attached_ms FROM case_attachments WHERE id=?1",
        )?;
        Ok(q.query_row([id], |r| {
            Ok(CaseAttachment {
                id: Some(r.get(0)?),
                case_id: r.get(1)?,
                kind: r.get(2)?,
                r#ref: r.get(3)?,
                note: r.get(4)?,
                attached_ms: r.get(5)?,
            })
        })
        .optional()?)
    }
    pub fn delete_case_attachment(&self, id: i64) -> anyhow::Result<usize> {
        Ok(self
            .conn()
            .execute("DELETE FROM case_attachments WHERE id=?1", [id])?)
    }

    pub fn add_annotation(&self, a: &RecordingAnnotation) -> anyhow::Result<i64> {
        let c = self.conn();
        c.execute("INSERT INTO recording_annotations (recording_path,offset_ms,text,created_ms) VALUES (?1,?2,?3,?4)", rusqlite::params![a.recording_path,a.offset_ms,a.text,a.created_ms])?;
        Ok(c.last_insert_rowid())
    }

    pub fn list_annotations(&self) -> anyhow::Result<Vec<RecordingAnnotation>> {
        let c = self.conn();
        let mut q = c.prepare("SELECT id,recording_path,offset_ms,text,created_ms FROM recording_annotations ORDER BY created_ms DESC")?;
        let rows = q.query_map([], |r| {
            Ok(RecordingAnnotation {
                id: Some(r.get(0)?),
                recording_path: r.get(1)?,
                offset_ms: r.get(2)?,
                text: r.get(3)?,
                created_ms: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn update_annotation(&self, id: i64, a: &RecordingAnnotation) -> anyhow::Result<usize> {
        Ok(self.conn().execute(
            "UPDATE recording_annotations SET recording_path=?1,offset_ms=?2,text=?3 WHERE id=?4",
            rusqlite::params![a.recording_path, a.offset_ms, a.text, id],
        )?)
    }

    pub fn delete_annotation(&self, id: i64) -> anyhow::Result<usize> {
        Ok(self
            .conn()
            .execute("DELETE FROM recording_annotations WHERE id=?1", [id])?)
    }

    pub fn due_scheduled_jobs(&self, now_ms: i64) -> anyhow::Result<Vec<ScheduledJob>> {
        let c = self.conn();
        let mut q=c.prepare("SELECT id,name,kind,payload_json,enabled,next_run_ms,last_run_ms,last_status,last_error,created_ms,updated_ms FROM scheduled_jobs WHERE enabled=1 AND next_run_ms IS NOT NULL AND next_run_ms<=?1 ORDER BY next_run_ms,id")?;
        let rows = q.query_map([now_ms], |r| {
            Ok(ScheduledJob {
                id: Some(r.get(0)?),
                name: r.get(1)?,
                kind: r.get(2)?,
                payload_json: r.get(3)?,
                enabled: r.get::<_, i64>(4)? != 0,
                next_run_ms: r.get(5)?,
                last_run_ms: r.get(6)?,
                last_status: r.get(7)?,
                last_error: r.get(8)?,
                created_ms: r.get(9)?,
                updated_ms: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn mark_scheduled_job(
        &self,
        id: i64,
        status: &str,
        error: &str,
        enabled: bool,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        self.conn().execute("UPDATE scheduled_jobs SET enabled=?1,next_run_ms=NULL,last_run_ms=?2,last_status=?3,last_error=?4,updated_ms=?2 WHERE id=?5",rusqlite::params![enabled,now_ms,status,error,id])?;
        Ok(())
    }

    pub fn list_scheduled_jobs(&self) -> anyhow::Result<Vec<ScheduledJob>> {
        let c = self.conn();
        let mut q=c.prepare("SELECT id,name,kind,payload_json,enabled,next_run_ms,last_run_ms,last_status,last_error,created_ms,updated_ms FROM scheduled_jobs ORDER BY next_run_ms IS NULL,next_run_ms,id")?;
        let rows = q.query_map([], |r| {
            Ok(ScheduledJob {
                id: Some(r.get(0)?),
                name: r.get(1)?,
                kind: r.get(2)?,
                payload_json: r.get(3)?,
                enabled: r.get::<_, i64>(4)? != 0,
                next_run_ms: r.get(5)?,
                last_run_ms: r.get(6)?,
                last_status: r.get(7)?,
                last_error: r.get(8)?,
                created_ms: r.get(9)?,
                updated_ms: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn create_scheduled_job(&self, job: &ScheduledJob) -> anyhow::Result<i64> {
        let c = self.conn();
        c.execute("INSERT INTO scheduled_jobs(name,kind,payload_json,enabled,next_run_ms,last_run_ms,last_status,last_error,created_ms,updated_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",rusqlite::params![job.name,job.kind,job.payload_json,job.enabled,job.next_run_ms,job.last_run_ms,job.last_status,job.last_error,job.created_ms,job.updated_ms])?;
        Ok(c.last_insert_rowid())
    }
    pub fn delete_scheduled_job(&self, id: i64) -> anyhow::Result<usize> {
        Ok(self
            .conn()
            .execute("DELETE FROM scheduled_jobs WHERE id=?1", [id])?)
    }

    pub fn list_blacklist(&self) -> anyhow::Result<Vec<BlacklistEntry>> {
        let c = self.conn();
        let mut q = c.prepare(
            "SELECT frequency_hz,reason,temporary,created_ms FROM blacklist ORDER BY frequency_hz",
        )?;
        let rows = q.query_map([], |r| {
            Ok(BlacklistEntry {
                frequency_hz: r.get::<_, i64>(0)? as u64,
                reason: r.get(1)?,
                temporary: r.get::<_, i64>(2)? != 0,
                created_ms: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn add_blacklist(&self, e: &BlacklistEntry) -> anyhow::Result<()> {
        self.conn().execute("INSERT INTO blacklist (frequency_hz,reason,temporary,created_ms) VALUES (?1,?2,?3,?4) ON CONFLICT(frequency_hz) DO UPDATE SET reason=excluded.reason,temporary=excluded.temporary", rusqlite::params![e.frequency_hz as i64,e.reason,e.temporary,e.created_ms])?;
        Ok(())
    }

    pub fn remove_blacklist(&self, frequency_hz: u64) -> anyhow::Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM blacklist WHERE frequency_hz=?1",
            [frequency_hz as i64],
        )?)
    }

    pub fn clear_blacklist(&self, temporary_only: bool) -> anyhow::Result<usize> {
        if temporary_only {
            Ok(self
                .conn()
                .execute("DELETE FROM blacklist WHERE temporary=1", [])?)
        } else {
            Ok(self.conn().execute("DELETE FROM blacklist", [])?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Db;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn recent_signal_events_are_newest_first_and_limited() {
        let path = std::env::temp_dir().join(format!(
            "pulsescope-db-{}.sqlite",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Db::open(&path).unwrap();
        db.insert_signal_event(433_920_000, 0.0, 3.0, 200_000, "ISM 433", 100)
            .unwrap();
        db.insert_signal_event(433_920_500, 0.0, 8.0, 200_000, "ISM 433", 200)
            .unwrap();
        db.insert_signal_event(433_921_000, 0.0, 5.0, 200_000, "ISM 433", 300)
            .unwrap();
        let rows = db.recent_signal_events(2).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].timestamp_ms, 300);
        assert_eq!(rows[1].timestamp_ms, 200);
        assert_eq!(db.recent_signal_events(0).unwrap().len(), 1);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn receiver_workspace_migration_is_idempotent_and_persistent() {
        let path = std::env::temp_dir().join(format!(
            "pulsescope-workspace-db-{}.sqlite",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let db = Db::open(&path).unwrap();
            db.conn().execute(
                "INSERT INTO receiver_profiles(id,name,center_frequency_hz,sample_rate_hz,bandwidth_hz,mode,created_ms,updated_ms) VALUES('fm','Local FM',99500000,10000000,8000000,'wfm',1,1)",
                [],
            ).unwrap();
            db.conn().execute(
                "INSERT INTO receiver_bookmarks(label,frequency_hz,mode,bandwidth_hz,profile_id,created_ms,updated_ms) VALUES('Station',99700000,'wfm',200000,'fm',1,1)",
                [],
            ).unwrap();
        }
        let reopened = Db::open(&path).unwrap();
        let count: i64 = reopened
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM receiver_bookmarks WHERE profile_id='fm'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn trunk_watchlist_migration_is_idempotent_and_tracks_entries() {
        let path = std::env::temp_dir().join(format!(
            "pulsescope-watchlist-db-{}.sqlite",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Db::open(&path).unwrap();
        db.set_talkgroup_watched("County", "1234", true, 1_000).unwrap();
        db.set_talkgroup_watched("County", "5678", true, 1_000).unwrap();
        db.set_talkgroup_watched("County", "5678", false, 2_000).unwrap();
        let rows = db.list_talkgroup_watchlist().unwrap();
        assert_eq!(rows, vec![("County".into(), "1234".into())]);
        assert!(db.is_talkgroup_watched("County", "1234"));
        assert!(!db.is_talkgroup_watched("County", "5678"));
        drop(db);
        let reopened = Db::open(&path).unwrap();
        assert_eq!(reopened.list_talkgroup_watchlist().unwrap().len(), 1);
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn activity_timeline_assigns_correlation_groups() {
        let path = std::env::temp_dir().join(format!(
            "pulsescope-activity-timeline-db-{}.sqlite",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Db::open(&path).unwrap();
        let bucket = 1_700_000_000_000_i64;
        db.insert_classified_signal_event(
            146_000_000,
            12.0,
            12_500,
            "2m",
            bucket,
            "voice",
            "amateur",
            0.8,
            "fm",
            false,
            "aprs",
            "first",
            false,
            false,
        )
        .unwrap();
        db.insert_classified_signal_event(
            146_010_000,
            10.0,
            12_500,
            "2m",
            bucket + 1_000,
            "data",
            "amateur",
            0.7,
            "aprs",
            false,
            "aprs",
            "second",
            false,
            false,
        )
        .unwrap();
        let entries = db.activity_timeline(bucket - 1_000, 10).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].correlation_group, entries[1].correlation_group);
        assert_eq!(entries[0].correlation_count, 2);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn network_source_migration_persists_registry_entries() {
        let path = std::env::temp_dir().join(format!(
            "pulsescope-network-source-db-{}.sqlite",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Db::open(&path).unwrap();
        let source = super::NetworkIqSource {
            id: "rtl-roof".into(),
            label: "Roof RTL-TCP".into(),
            kind: "rtl_tcp".into(),
            host: "192.168.1.50".into(),
            port: 1234,
            enabled: false,
            created_ms: 1_000,
            updated_ms: 1_000,
        };
        db.upsert_network_iq_source(&source).unwrap();
        let listed = db.list_network_iq_sources().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].label, "Roof RTL-TCP");
        db.delete_network_iq_source("rtl-roof").unwrap();
        assert!(db.list_network_iq_sources().unwrap().is_empty());
        drop(db);
        let reopened = Db::open(&path).unwrap();
        assert!(reopened.list_network_iq_sources().unwrap().is_empty());
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn occupancy_since_bucket_returns_ordered_rows() {
        let path = std::env::temp_dir().join(format!(
            "pulsescope-occupancy-heatmap-db-{}.sqlite",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Db::open(&path).unwrap();
        for (bucket, freq) in [(1_i64, 100_000_000_u64), (2, 101_000_000), (2, 100_000_000)] {
            db.upsert_occupancy(&super::SpectrumOccupancy {
                frequency_bucket_hz: freq,
                time_bucket_15min: bucket,
                avg_power_db: -70.0,
                peak_power_db: -60.0,
                avg_above_floor_db: 20.0,
                sample_count: 1,
                noise_floor_db: -90.0,
            })
            .unwrap();
        }
        let rows = db.occupancy_since_bucket(2).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].time_bucket_15min, 2);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn occupancy_upsert_averages_within_the_same_15min_bucket() {
        let path = std::env::temp_dir().join(format!(
            "pulsescope-occupancy-db-{}.sqlite",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Db::open(&path).unwrap();
        let first = super::SpectrumOccupancy {
            frequency_bucket_hz: 100_000_000,
            time_bucket_15min: 1,
            avg_power_db: -80.0,
            peak_power_db: -70.0,
            avg_above_floor_db: 10.0,
            sample_count: 1,
            noise_floor_db: -90.0,
        };
        let second = super::SpectrumOccupancy {
            avg_power_db: -60.0,
            peak_power_db: -50.0,
            avg_above_floor_db: 30.0,
            sample_count: 1,
            noise_floor_db: -90.0,
            ..first
        };
        db.upsert_occupancy(&first).unwrap();
        db.upsert_occupancy(&second).unwrap();
        let rows = db.recent_occupancy(8).unwrap();
        assert_eq!(rows.len(), 1);
        assert!((rows[0].avg_power_db + 70.0).abs() < 0.01);
        assert!((rows[0].peak_power_db + 50.0).abs() < 0.01);
        assert_eq!(rows[0].sample_count, 2);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
