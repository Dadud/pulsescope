//! Privacy-preserving normalized tracking, interchange, and evidence helpers.

use crate::db::Db;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const MAX_PAGE_SIZE: u32 = 1000;
pub const DEFAULT_RETENTION: usize = 100_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PositionEvent {
    pub id: Option<i64>,
    pub entity_kind: String,
    pub entity_id: String,
    pub timestamp_ms: i64,
    pub latitude: f64,
    pub longitude: f64,
    pub altitude_m: Option<f64>,
    pub speed_mps: Option<f64>,
    pub heading_deg: Option<f64>,
    pub accuracy_m: Option<f64>,
    pub vertical_accuracy_m: Option<f64>,
    pub source: String,
    pub source_message_id: Option<i64>,
    #[serde(default)]
    pub metadata: Value,
}

impl PositionEvent {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            matches!(
                self.entity_kind.as_str(),
                "aircraft" | "vessel" | "aprs" | "radiosonde"
            ),
            "unsupported entity_kind"
        );
        anyhow::ensure!(!self.entity_id.trim().is_empty(), "entity_id is required");
        anyhow::ensure!(
            self.latitude.is_finite() && (-90.0..=90.0).contains(&self.latitude),
            "invalid latitude"
        );
        anyhow::ensure!(
            self.longitude.is_finite() && (-180.0..=180.0).contains(&self.longitude),
            "invalid longitude"
        );
        anyhow::ensure!(
            !self.source.trim().is_empty(),
            "source attribution is required"
        );
        if let Some(v) = self.accuracy_m {
            anyhow::ensure!(v.is_finite() && v >= 0.0, "invalid accuracy");
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct TrackQuery {
    pub kind: Option<String>,
    pub entity_id: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

impl Db {
    pub fn insert_position(&self, p: &PositionEvent, retention: usize) -> anyhow::Result<i64> {
        p.validate()?;
        let c = self.conn();
        c.execute("INSERT INTO position_events(entity_kind,entity_id,timestamp_ms,latitude,longitude,altitude_m,speed_mps,heading_deg,accuracy_m,vertical_accuracy_m,source,source_message_id,metadata_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)", rusqlite::params![p.entity_kind,p.entity_id,p.timestamp_ms,p.latitude,p.longitude,p.altitude_m,p.speed_mps,p.heading_deg,p.accuracy_m,p.vertical_accuracy_m,p.source,p.source_message_id,p.metadata.to_string()])?;
        let id = c.last_insert_rowid();
        if retention > 0 {
            c.execute("DELETE FROM position_events WHERE id IN (SELECT id FROM position_events ORDER BY timestamp_ms DESC,id DESC LIMIT -1 OFFSET ?1)", [retention as i64])?;
        }
        Ok(id)
    }

    pub fn positions(&self, q: &TrackQuery, current: bool) -> anyhow::Result<Vec<PositionEvent>> {
        let limit = q.limit.unwrap_or(250).clamp(1, MAX_PAGE_SIZE);
        let offset = q.offset.unwrap_or(0);
        let c = self.conn();
        let sql = if current {
            "SELECT id,entity_kind,entity_id,timestamp_ms,latitude,longitude,altitude_m,speed_mps,heading_deg,accuracy_m,vertical_accuracy_m,source,source_message_id,metadata_json FROM (SELECT *,row_number() OVER(PARTITION BY entity_kind,entity_id ORDER BY timestamp_ms DESC,id DESC) rn FROM position_events WHERE (?1 IS NULL OR entity_kind=?1) AND (?2 IS NULL OR entity_id=?2) AND (?3 IS NULL OR timestamp_ms>=?3) AND (?4 IS NULL OR timestamp_ms<=?4)) WHERE rn=1 ORDER BY timestamp_ms DESC LIMIT ?5 OFFSET ?6"
        } else {
            "SELECT id,entity_kind,entity_id,timestamp_ms,latitude,longitude,altitude_m,speed_mps,heading_deg,accuracy_m,vertical_accuracy_m,source,source_message_id,metadata_json FROM position_events WHERE (?1 IS NULL OR entity_kind=?1) AND (?2 IS NULL OR entity_id=?2) AND (?3 IS NULL OR timestamp_ms>=?3) AND (?4 IS NULL OR timestamp_ms<=?4) ORDER BY timestamp_ms DESC,id DESC LIMIT ?5 OFFSET ?6"
        };
        let mut st = c.prepare(sql)?;
        let rows = st.query_map(
            rusqlite::params![q.kind, q.entity_id, q.from_ms, q.to_ms, limit, offset],
            |r| {
                Ok(PositionEvent {
                    id: Some(r.get(0)?),
                    entity_kind: r.get(1)?,
                    entity_id: r.get(2)?,
                    timestamp_ms: r.get(3)?,
                    latitude: r.get(4)?,
                    longitude: r.get(5)?,
                    altitude_m: r.get(6)?,
                    speed_mps: r.get(7)?,
                    heading_deg: r.get(8)?,
                    accuracy_m: r.get(9)?,
                    vertical_accuracy_m: r.get(10)?,
                    source: r.get(11)?,
                    source_message_id: r.get(12)?,
                    metadata: serde_json::from_str(&r.get::<_, String>(13)?).unwrap_or(json!({})),
                })
            },
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ChirpChannel {
    pub name: String,
    pub frequency_hz: u64,
    pub duplex: String,
    pub offset_hz: i64,
    pub tone: String,
    pub mode: String,
}

/// Strict, dependency-free parser for CHIRP's CSV interchange format.
pub fn parse_chirp_csv(input: &str) -> anyhow::Result<Vec<ChirpChannel>> {
    anyhow::ensure!(input.len() <= 10_000_000, "import exceeds 10 MB");
    let mut lines = input.lines();
    let header = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty import"))?;
    let cols: Vec<_> = header
        .split(',')
        .map(|s| s.trim_matches('\u{feff}').trim())
        .collect();
    let idx = |n: &str| {
        cols.iter()
            .position(|c| c.eq_ignore_ascii_case(n))
            .ok_or_else(|| anyhow::anyhow!("missing {n} column"))
    };
    let ni = idx("Name")?;
    let fi = idx("Frequency")?;
    let di = idx("Duplex").ok();
    let oi = idx("Offset").ok();
    let ti = idx("Tone").ok();
    let mi = idx("Mode").ok();
    let mut out = Vec::new();
    for (line_no, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v = parse_csv_line(line)?;
        let get = |i: usize| v.get(i).map(String::as_str).unwrap_or("").trim();
        let mhz: f64 = get(fi)
            .parse()
            .map_err(|_| anyhow::anyhow!("line {}: invalid frequency", line_no + 2))?;
        anyhow::ensure!(
            mhz.is_finite() && (0.001..=100_000.0).contains(&mhz),
            "line {}: frequency out of range",
            line_no + 2
        );
        let name = get(ni);
        anyhow::ensure!(
            !name.is_empty() && name.len() <= 128,
            "line {}: invalid name",
            line_no + 2
        );
        let off = oi.and_then(|i| get(i).parse::<f64>().ok()).unwrap_or(0.0);
        out.push(ChirpChannel {
            name: name.into(),
            frequency_hz: (mhz * 1e6).round() as u64,
            duplex: di.map(|i| get(i).to_string()).unwrap_or_default(),
            offset_hz: (off * 1e6).round() as i64,
            tone: ti.map(|i| get(i).to_string()).unwrap_or_default(),
            mode: mi
                .map(|i| get(i).to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "FM".into()),
        });
    }
    anyhow::ensure!(!out.is_empty(), "import contains no channels");
    Ok(out)
}
fn parse_csv_line(s: &str) -> anyhow::Result<Vec<String>> {
    let (mut out, mut cur, mut quote) = (vec![], String::new(), false);
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' if quote && it.peek() == Some(&'"') => {
                cur.push('"');
                it.next();
            }
            '"' => quote = !quote,
            ',' if !quote => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    anyhow::ensure!(!quote, "unterminated CSV quote");
    out.push(cur);
    Ok(out)
}

pub fn export_positions(items: &[PositionEvent], format: &str) -> anyhow::Result<(String, String)> {
    match format {
 "json"=>Ok(("application/json".into(),serde_json::to_string_pretty(items)?)),
 "geojson"=>Ok(("application/geo+json".into(),json!({"type":"FeatureCollection","features":items.iter().map(|p|json!({"type":"Feature","geometry":{"type":"Point","coordinates":[p.longitude,p.latitude,p.altitude_m]},"properties":{"id":p.entity_id,"kind":p.entity_kind,"timestamp_ms":p.timestamp_ms,"source":p.source,"accuracy_m":p.accuracy_m}})).collect::<Vec<_>>()}).to_string())),
 "csv"=>{let mut s="kind,id,timestamp_ms,latitude,longitude,altitude_m,source,accuracy_m\n".to_string();for p in items{s.push_str(&format!("{},{},{},{},{},{},{},{}\n",p.entity_kind,p.entity_id.replace(',',' '),p.timestamp_ms,p.latitude,p.longitude,p.altitude_m.map(|x|x.to_string()).unwrap_or_default(),p.source.replace(',',' '),p.accuracy_m.map(|x|x.to_string()).unwrap_or_default()));}Ok(("text/csv".into(),s))},
 "kml"=>{let mut s="<?xml version=\"1.0\"?><kml xmlns=\"http://www.opengis.net/kml/2.2\"><Document>".to_string();for p in items{s.push_str(&format!("<Placemark><name>{}</name><description>{} / {}</description><Point><coordinates>{},{},{}</coordinates></Point></Placemark>",xml(&p.entity_id),xml(&p.entity_kind),xml(&p.source),p.longitude,p.latitude,p.altitude_m.unwrap_or(0.0)));}s.push_str("</Document></kml>");Ok(("application/vnd.google-earth.kml+xml".into(),s))}, _=>anyhow::bail!("unsupported export format") }
}
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
pub fn sha256(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_chirp() {
        assert!(parse_chirp_csv("Name,Frequency\nX,nope").is_err());
        assert!(parse_chirp_csv("Name\nX").is_err());
    }
    #[test]
    fn quoted_chirp() {
        let x=parse_chirp_csv("Location,Name,Frequency,Duplex,Offset,Tone,Mode\n1,\"Local, Repeater\",146.940,-,0.600,Tone,FM").unwrap();
        assert_eq!(x[0].name, "Local, Repeater");
        assert_eq!(x[0].frequency_hz, 146_940_000);
    }
    #[test]
    fn hash_known() {
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
