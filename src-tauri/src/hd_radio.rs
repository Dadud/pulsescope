//! HD Radio (NRSC-5) sidecar discovery and SIS/ID3 line parser.
//!
//! OFDM demodulation is delegated to nrsc5 when installed. Without that
//! binary, status stays `available: false`. Encrypted or unparsed AAS is
//! never claimed as decoded audio.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HdRadioEvent {
    pub kind: String,
    pub station: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub program: Option<u32>,
    pub raw: String,
}

pub fn find_nrsc5() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PULSESCOPE_NRSC5") {
        let path = PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
    }
    which::which("nrsc5").ok()
}

/// nrsc5 reads CF32 from stdin when `-r -` is set. Frequency is MHz.
pub fn nrsc5_stdin_args(frequency_hz: u64, program: u32) -> Vec<String> {
    let mhz = frequency_hz as f64 / 1_000_000.0;
    vec![
        "-r".into(),
        "-".into(),
        format!("{mhz:.3}"),
        program.to_string(),
    ]
}

pub fn event_to_decoded(event: &HdRadioEvent, frequency_hz: u64) -> crate::db::DecodedMessage {
    crate::db::DecodedMessage {
        id: None,
        frequency_hz,
        protocol: "hd_radio".into(),
        message_type: event.kind.clone(),
        address: event.station.clone().unwrap_or_default(),
        function_code: event.program.map(|p| p.to_string()).unwrap_or_default(),
        content: event
            .title
            .clone()
            .or_else(|| event.artist.clone())
            .or_else(|| event.station.clone())
            .unwrap_or_else(|| event.kind.clone()),
        raw: event.raw.clone(),
        encryption: "none".into(),
        timestamp_ms: crate::scanner::now_ms(),
    }
}

pub fn parse_nrsc5_line(line: &str) -> Option<HdRadioEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let kind = value
            .get("type")
            .or_else(|| value.get("kind"))
            .and_then(|v| v.as_str())
            .unwrap_or("event")
            .to_string();
        return Some(HdRadioEvent {
            kind,
            station: value
                .get("name")
                .or_else(|| value.get("station"))
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            title: value
                .get("title")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            artist: value
                .get("artist")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            program: value
                .get("program")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
            raw: trimmed.to_string(),
        });
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("station name:") || lower.starts_with("sis:") {
        let station = trimmed
            .split_once(':')
            .map(|(_, rest)| rest.trim().to_string());
        return Some(HdRadioEvent {
            kind: "sis".into(),
            station,
            title: None,
            artist: None,
            program: None,
            raw: trimmed.to_string(),
        });
    }
    if lower.starts_with("title:") {
        return Some(HdRadioEvent {
            kind: "id3".into(),
            station: None,
            title: trimmed
                .split_once(':')
                .map(|(_, rest)| rest.trim().to_string()),
            artist: None,
            program: None,
            raw: trimmed.to_string(),
        });
    }
    if lower.starts_with("artist:") {
        return Some(HdRadioEvent {
            kind: "id3".into(),
            station: None,
            title: None,
            artist: trimmed
                .split_once(':')
                .map(|(_, rest)| rest.trim().to_string()),
            program: None,
            raw: trimmed.to_string(),
        });
    }
    None
}

pub fn parse_nrsc5_output(text: &str) -> Vec<HdRadioEvent> {
    text.lines().filter_map(parse_nrsc5_line).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_sis_and_id3() {
        let events = parse_nrsc5_output(
            "{\"type\":\"sis\",\"name\":\"KXYZ-FM\"}\n{\"type\":\"id3\",\"title\":\"HELLO\",\"artist\":\"PULSE\"}\n",
        );
        assert!(events
            .iter()
            .any(|e| e.station.as_deref() == Some("KXYZ-FM")));
        assert!(events.iter().any(|e| e.title.as_deref() == Some("HELLO")));
        assert!(events.iter().any(|e| e.artist.as_deref() == Some("PULSE")));
    }

    #[test]
    fn parses_text_sis() {
        let events = parse_nrsc5_output("Station name: KXYZ-FM\nTitle: HELLO\n");
        assert_eq!(events[0].kind, "sis");
        assert_eq!(events[1].title.as_deref(), Some("HELLO"));
    }

    #[test]
    fn stdin_args_use_mhz_and_program() {
        let args = nrsc5_stdin_args(88_500_000, 1);
        assert_eq!(args, ["-r", "-", "88.500", "1"]);
    }
}
