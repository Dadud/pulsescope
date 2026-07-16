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
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.pragma_update(None, "synchronous", "normal")?;
        conn.pragma_update(None, "foreign_keys", "on")?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
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
    pub fn insert_signal_event(&self, frequency_hz: u64, _strength_db: f32, snr_db: f32, bandwidth_hz: u32, range_name: &str, timestamp_ms: i64) -> anyhow::Result<i64> {
        let c = self.conn();
        c.execute("INSERT INTO signal_events (frequency_hz, signal_class, top_family, top_confidence, bandwidth_hz, snr_db, decode_success, decode_protocol, decode_summary, likely_proprietary, waterfall_psd, range_name, timestamp_ms, is_novel) VALUES (?1,'unknown','unknown',0.0,?2,?3,0,'','',0,'',?4,?5,1)", rusqlite::params![frequency_hz as i64, bandwidth_hz as i64, snr_db, range_name, timestamp_ms])?;
        Ok(c.last_insert_rowid())
    }

    pub fn recent_signal_events(&self, limit: u32) -> anyhow::Result<Vec<SignalEvent>> {
        let c = self.conn();
        let mut q = c.prepare("SELECT id,frequency_hz,signal_class,top_family,top_confidence,sub_protocol,symbol_rate,bandwidth_hz,snr_db,decode_success,decode_protocol,decode_summary,likely_proprietary,waterfall_psd,range_name,timestamp_ms,is_novel FROM signal_events ORDER BY timestamp_ms DESC LIMIT ?1")?;
        let rows = q.query_map([limit.clamp(1, 1000)], |r| Ok(SignalEvent {
            id: Some(r.get(0)?), frequency_hz: r.get::<_, i64>(1)? as u64, signal_class: r.get(2)?, top_family: r.get(3)?, top_confidence: r.get(4)?, sub_protocol: r.get::<_, Option<String>>(5)?.unwrap_or_default(), symbol_rate: r.get::<_, Option<f32>>(6)?.unwrap_or_default(), bandwidth_hz: r.get::<_, Option<i64>>(7)?.unwrap_or_default() as u32, snr_db: r.get::<_, Option<f32>>(8)?.unwrap_or_default(), decode_success: r.get::<_, i64>(9)? != 0, decode_protocol: r.get::<_, Option<String>>(10)?.unwrap_or_default(), decode_summary: r.get::<_, Option<String>>(11)?.unwrap_or_default(), likely_proprietary: r.get::<_, i64>(12)? != 0, waterfall_psd: r.get::<_, Option<String>>(13)?.unwrap_or_default(), range_name: r.get::<_, Option<String>>(14)?.unwrap_or_default(), timestamp_ms: r.get(15)?, is_novel: r.get::<_, i64>(16)? != 0,
        }))?;
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
                t.system_name, t.talkgroup_id, t.alpha_tag, t.description,
                t.category, t.tag, t.mode, t.protocol, t.encrypted,
                t.hit_count, t.first_seen_ms, t.last_seen_ms,
            ],
        )?;
        Ok(c.last_insert_rowid())
    }

    pub fn upsert_occupancy(&self, o: &SpectrumOccupancy) -> anyhow::Result<()> {
        self.conn().execute("INSERT INTO spectrum_occupancy (frequency_bucket_hz,time_bucket_15min,avg_power_db,peak_power_db,avg_above_floor_db,sample_count,noise_floor_db) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(frequency_bucket_hz,time_bucket_15min) DO UPDATE SET avg_power_db=excluded.avg_power_db,peak_power_db=excluded.peak_power_db,avg_above_floor_db=excluded.avg_above_floor_db,sample_count=excluded.sample_count,noise_floor_db=excluded.noise_floor_db", rusqlite::params![o.frequency_bucket_hz as i64,o.time_bucket_15min,o.avg_power_db,o.peak_power_db,o.avg_above_floor_db,o.sample_count,o.noise_floor_db])?;
        Ok(())
    }

    pub fn recent_occupancy(&self, limit: u32) -> anyhow::Result<Vec<SpectrumOccupancy>> {
        let c = self.conn();
        let mut q = c.prepare("SELECT frequency_bucket_hz,time_bucket_15min,avg_power_db,peak_power_db,avg_above_floor_db,sample_count,noise_floor_db FROM spectrum_occupancy ORDER BY time_bucket_15min DESC,frequency_bucket_hz LIMIT ?1")?;
        let rows = q.query_map([limit], |r| Ok(SpectrumOccupancy { frequency_bucket_hz: r.get::<_, i64>(0)? as u64, time_bucket_15min: r.get(1)?, avg_power_db: r.get(2)?, peak_power_db: r.get(3)?, avg_above_floor_db: r.get(4)?, sample_count: r.get(5)?, noise_floor_db: r.get(6)? }))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn decoded_message_count(&self) -> anyhow::Result<i64> {
        Ok(self.conn().query_row("SELECT COUNT(*) FROM decoded_messages", [], |r| r.get(0))?)
    }

    pub fn messages_by_protocol(&self, protocol: Option<&str>, limit: u32) -> anyhow::Result<Vec<DecodedMessage>> {
        let c = self.conn();
        let mut out = Vec::new();
        if let Some(protocol) = protocol {
            let mut q = c.prepare("SELECT id,frequency_hz,protocol,message_type,address,function_code,content,raw,encryption,timestamp_ms FROM decoded_messages WHERE protocol=?1 ORDER BY id DESC LIMIT ?2")?;
            let rows = q.query_map(rusqlite::params![protocol, limit], |r| Ok(DecodedMessage { id: Some(r.get(0)?), frequency_hz: r.get::<_, i64>(1)? as u64, protocol: r.get(2)?, message_type: r.get(3)?, address: r.get(4)?, function_code: r.get(5)?, content: r.get(6)?, raw: r.get(7)?, encryption: r.get(8)?, timestamp_ms: r.get(9)? }))?;
            for row in rows { out.push(row?); }
        } else {
            let mut q = c.prepare("SELECT id,frequency_hz,protocol,message_type,address,function_code,content,raw,encryption,timestamp_ms FROM decoded_messages WHERE protocol NOT IN ('rtl_433') ORDER BY id DESC LIMIT ?1")?;
            let rows = q.query_map([limit], |r| Ok(DecodedMessage { id: Some(r.get(0)?), frequency_hz: r.get::<_, i64>(1)? as u64, protocol: r.get(2)?, message_type: r.get(3)?, address: r.get(4)?, function_code: r.get(5)?, content: r.get(6)?, raw: r.get(7)?, encryption: r.get(8)?, timestamp_ms: r.get(9)? }))?;
            for row in rows { out.push(row?); }
        }
        Ok(out)
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
                protocol: r.get(2)?, message_type: r.get(3)?, address: r.get(4)?,
                function_code: r.get(5)?, content: r.get(6)?, raw: r.get(7)?,
                encryption: r.get(8)?, timestamp_ms: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn list_talkgroups(&self) -> anyhow::Result<Vec<Talkgroup>> {
        let c = self.conn();
        let mut q = c.prepare("SELECT id,system_name,talkgroup_id,alpha_tag,description,category,tag,mode,protocol,encrypted,hit_count,first_seen_ms,last_seen_ms FROM talkgroups ORDER BY system_name,talkgroup_id")?;
        let rows = q.query_map([], |r| Ok(Talkgroup { id: Some(r.get(0)?), system_name: r.get(1)?, talkgroup_id: r.get(2)?, alpha_tag: r.get(3)?, description: r.get(4)?, category: r.get(5)?, tag: r.get(6)?, mode: r.get(7)?, protocol: r.get(8)?, encrypted: r.get::<_, i64>(9)? != 0, hit_count: r.get(10)?, first_seen_ms: r.get(11)?, last_seen_ms: r.get(12)? }))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn upsert_talkgroup(&self, t: &Talkgroup) -> anyhow::Result<()> {
        self.conn().execute("INSERT INTO talkgroups (system_name,talkgroup_id,alpha_tag,description,category,tag,mode,protocol,encrypted,hit_count,first_seen_ms,last_seen_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(system_name,talkgroup_id) DO UPDATE SET alpha_tag=excluded.alpha_tag,description=excluded.description,category=excluded.category,tag=excluded.tag,mode=excluded.mode,protocol=excluded.protocol,encrypted=excluded.encrypted,hit_count=excluded.hit_count,last_seen_ms=excluded.last_seen_ms", rusqlite::params![t.system_name,t.talkgroup_id,t.alpha_tag,t.description,t.category,t.tag,t.mode,t.protocol,t.encrypted,t.hit_count,t.first_seen_ms,t.last_seen_ms])?;
        Ok(())
    }

    pub fn delete_talkgroup_system(&self, system: &str) -> anyhow::Result<usize> {
        Ok(self.conn().execute("DELETE FROM talkgroups WHERE system_name=?1", [system])?)
    }

    pub fn talkgroup_systems(&self) -> anyhow::Result<Vec<String>> {
        let c = self.conn();
        let mut q = c.prepare("SELECT DISTINCT system_name FROM talkgroups ORDER BY system_name")?;
        let rows = q.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn export_talkgroups(&self) -> anyhow::Result<Vec<Talkgroup>> { self.list_talkgroups() }

    pub fn list_cases(&self) -> anyhow::Result<Vec<Case>> {
        let c = self.conn();
        let mut q = c.prepare("SELECT id,name,description,status,tags,created_ms,updated_ms FROM cases ORDER BY updated_ms DESC")?;
        let rows = q.query_map([], |r| Ok(Case { id: Some(r.get(0)?), name: r.get(1)?, description: r.get(2)?, status: r.get(3)?, tags: r.get(4)?, created_ms: r.get(5)?, updated_ms: r.get(6)? }))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn create_case(&self, case: &Case) -> anyhow::Result<i64> {
        let c = self.conn();
        c.execute("INSERT INTO cases (name,description,status,tags,created_ms,updated_ms) VALUES (?1,?2,?3,?4,?5,?6)", rusqlite::params![case.name,case.description,case.status,case.tags,case.created_ms,case.updated_ms])?;
        Ok(c.last_insert_rowid())
    }

    pub fn get_case(&self, id: i64) -> anyhow::Result<Option<Case>> {
        let c = self.conn();
        let mut q = c.prepare("SELECT id,name,description,status,tags,created_ms,updated_ms FROM cases WHERE id=?1")?;
        Ok(q.query_row([id], |r| Ok(Case { id: Some(r.get(0)?), name: r.get(1)?, description: r.get(2)?, status: r.get(3)?, tags: r.get(4)?, created_ms: r.get(5)?, updated_ms: r.get(6)? })).optional()?)
    }

    pub fn delete_case(&self, id: i64) -> anyhow::Result<usize> {
        Ok(self.conn().execute("DELETE FROM cases WHERE id=?1", [id])?)
    }

    pub fn add_case_attachment(&self, a: &CaseAttachment) -> anyhow::Result<i64> { let c=self.conn(); c.execute("INSERT INTO case_attachments (case_id,kind,ref,note,attached_ms) VALUES (?1,?2,?3,?4,?5)",rusqlite::params![a.case_id,a.kind,a.r#ref,a.note,a.attached_ms])?; Ok(c.last_insert_rowid()) }
    pub fn case_attachments(&self, case_id: i64) -> anyhow::Result<Vec<CaseAttachment>> { let c=self.conn(); let mut q=c.prepare("SELECT id,case_id,kind,ref,note,attached_ms FROM case_attachments WHERE case_id=?1 ORDER BY attached_ms DESC")?; let rows=q.query_map([case_id],|r|Ok(CaseAttachment{id:Some(r.get(0)?),case_id:r.get(1)?,kind:r.get(2)?,r#ref:r.get(3)?,note:r.get(4)?,attached_ms:r.get(5)?}))?; Ok(rows.collect::<Result<Vec<_>,_>>()?) }
    pub fn case_attachment(&self, id: i64) -> anyhow::Result<Option<CaseAttachment>> { let c=self.conn(); let mut q=c.prepare("SELECT id,case_id,kind,ref,note,attached_ms FROM case_attachments WHERE id=?1")?; Ok(q.query_row([id],|r|Ok(CaseAttachment{id:Some(r.get(0)?),case_id:r.get(1)?,kind:r.get(2)?,r#ref:r.get(3)?,note:r.get(4)?,attached_ms:r.get(5)?})).optional()?) }
    pub fn delete_case_attachment(&self, id: i64) -> anyhow::Result<usize> { Ok(self.conn().execute("DELETE FROM case_attachments WHERE id=?1",[id])?) }

    pub fn add_annotation(&self, a: &RecordingAnnotation) -> anyhow::Result<i64> {
        let c = self.conn();
        c.execute("INSERT INTO recording_annotations (recording_path,offset_ms,text,created_ms) VALUES (?1,?2,?3,?4)", rusqlite::params![a.recording_path,a.offset_ms,a.text,a.created_ms])?;
        Ok(c.last_insert_rowid())
    }

    pub fn list_annotations(&self) -> anyhow::Result<Vec<RecordingAnnotation>> {
        let c = self.conn();
        let mut q = c.prepare("SELECT id,recording_path,offset_ms,text,created_ms FROM recording_annotations ORDER BY created_ms DESC")?;
        let rows = q.query_map([], |r| Ok(RecordingAnnotation { id: Some(r.get(0)?), recording_path: r.get(1)?, offset_ms: r.get(2)?, text: r.get(3)?, created_ms: r.get(4)? }))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn update_annotation(&self, id: i64, a: &RecordingAnnotation) -> anyhow::Result<usize> {
        Ok(self.conn().execute("UPDATE recording_annotations SET recording_path=?1,offset_ms=?2,text=?3 WHERE id=?4", rusqlite::params![a.recording_path,a.offset_ms,a.text,id])?)
    }

    pub fn delete_annotation(&self, id: i64) -> anyhow::Result<usize> {
        Ok(self.conn().execute("DELETE FROM recording_annotations WHERE id=?1", [id])?)
    }

    pub fn list_blacklist(&self) -> anyhow::Result<Vec<BlacklistEntry>> {
        let c = self.conn();
        let mut q = c.prepare("SELECT frequency_hz,reason,temporary,created_ms FROM blacklist ORDER BY frequency_hz")?;
        let rows = q.query_map([], |r| Ok(BlacklistEntry { frequency_hz: r.get::<_, i64>(0)? as u64, reason: r.get(1)?, temporary: r.get::<_, i64>(2)? != 0, created_ms: r.get(3)? }))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn add_blacklist(&self, e: &BlacklistEntry) -> anyhow::Result<()> {
        self.conn().execute("INSERT INTO blacklist (frequency_hz,reason,temporary,created_ms) VALUES (?1,?2,?3,?4) ON CONFLICT(frequency_hz) DO UPDATE SET reason=excluded.reason,temporary=excluded.temporary", rusqlite::params![e.frequency_hz as i64,e.reason,e.temporary,e.created_ms])?;
        Ok(())
    }

    pub fn remove_blacklist(&self, frequency_hz: u64) -> anyhow::Result<usize> {
        Ok(self.conn().execute("DELETE FROM blacklist WHERE frequency_hz=?1", [frequency_hz as i64])?)
    }

    pub fn clear_blacklist(&self, temporary_only: bool) -> anyhow::Result<usize> {
        if temporary_only { Ok(self.conn().execute("DELETE FROM blacklist WHERE temporary=1", [])?) }
        else { Ok(self.conn().execute("DELETE FROM blacklist", [])?) }
    }
}

#[cfg(test)]
mod tests {
    use super::Db;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn recent_signal_events_are_newest_first_and_limited() {
        let path = std::env::temp_dir().join(format!("pulsescope-db-{}.sqlite", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        let db = Db::open(&path).unwrap();
        db.insert_signal_event(433_920_000, 0.0, 3.0, 200_000, "ISM 433", 100).unwrap();
        db.insert_signal_event(433_920_500, 0.0, 8.0, 200_000, "ISM 433", 200).unwrap();
        db.insert_signal_event(433_921_000, 0.0, 5.0, 200_000, "ISM 433", 300).unwrap();
        let rows = db.recent_signal_events(2).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].timestamp_ms, 300);
        assert_eq!(rows[1].timestamp_ms, 200);
        assert_eq!(db.recent_signal_events(0).unwrap().len(), 1);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}

