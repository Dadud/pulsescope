//! Operational reliability primitives shared by the API and capture runtime.
use axum::response::IntoResponse;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message: &'static str,
    pub request_id: String,
}

/// Stable public error plus private context suitable for structured logs.
#[derive(Debug)]
pub struct OperationalError {
    pub status: axum::http::StatusCode,
    pub code: &'static str,
    pub safe_message: &'static str,
    pub context: anyhow::Error,
    pub request_id: String,
}

impl axum::response::IntoResponse for OperationalError {
    fn into_response(self) -> axum::response::Response {
        tracing::error!(code=self.code, request_id=%self.request_id, error=?self.context, "request failed");
        (
            self.status,
            axum::Json(ErrorBody {
                code: self.code,
                message: self.safe_message,
                request_id: self.request_id,
            }),
        )
            .into_response()
    }
}

#[derive(Default)]
pub struct Metrics {
    pub capture_samples: AtomicU64,
    pub dropped_samples: AtomicU64,
    pub ring_used: AtomicU64,
    pub ring_capacity: AtomicU64,
    pub fft_latency_us: AtomicU64,
    pub decoder_frames: AtomicU64,
    pub invalid_frames: AtomicU64,
    pub audio_underruns: AtomicU64,
    pub websocket_lagged: AtomicU64,
    pub database_latency_us: AtomicU64,
    pub sidecar_restarts: AtomicU64,
}

impl Metrics {
    pub fn snapshot(&self) -> BTreeMap<&'static str, u64> {
        let mut out = BTreeMap::new();
        for (key, value) in [
            ("capture_samples_total", &self.capture_samples),
            ("capture_dropped_samples_total", &self.dropped_samples),
            ("capture_ring_used_samples", &self.ring_used),
            ("capture_ring_capacity_samples", &self.ring_capacity),
            ("fft_latency_microseconds", &self.fft_latency_us),
            ("decoder_frames_total", &self.decoder_frames),
            ("decoder_invalid_frames_total", &self.invalid_frames),
            ("audio_underrun_samples_total", &self.audio_underruns),
            ("websocket_lagged_events_total", &self.websocket_lagged),
            ("database_latency_microseconds", &self.database_latency_us),
            ("sidecar_restarts_total", &self.sidecar_restarts),
        ] {
            out.insert(key, value.load(Ordering::Relaxed));
        }
        out
    }
    pub fn time_database<T>(&self, f: impl FnOnce() -> T) -> T {
        let now = Instant::now();
        let value = f();
        self.database_latency_us
            .store(now.elapsed().as_micros() as u64, Ordering::Relaxed);
        value
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ComponentHealth {
    pub name: String,
    pub status: &'static str,
    pub detail: String,
}

pub fn redact(input: &str) -> String {
    let sensitive =
        regex::Regex::new(r#"(?i)(token|password|secret|authorization)(\s*[=:]\s*)([^\s,"}]+)"#)
            .expect("redaction regex");
    let home = std::env::var("HOME").unwrap_or_default();
    let value = sensitive.replace_all(input, "$1$2[REDACTED]").into_owned();
    if home.is_empty() {
        value
    } else {
        value.replace(&home, "~")
    }
}

/// Produce a bounded ZIP containing redacted state. No recordings or raw IQ
/// are ever included. Individual inputs are capped to prevent bundle abuse.
pub fn diagnostic_bundle(data_dir: &Path, payloads: &[(&str, String)]) -> anyhow::Result<Vec<u8>> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(cursor);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, value) in payloads {
        zip.start_file(name, options)?;
        let redacted = redact(value);
        zip.write_all(&redacted.as_bytes()[..redacted.len().min(2 * 1024 * 1024)])?;
    }
    let marker = data_dir.join("recovery.log");
    if let Ok(value) = fs::read_to_string(marker) {
        zip.start_file("recovery.log", options)?;
        zip.write_all(redact(&value).as_bytes())?;
    }
    Ok(zip.finish()?.into_inner())
}

/// Keep at most `files` generations and bound each active log by `max_bytes`.
pub fn rotate_log(path: &Path, max_bytes: u64, files: usize) -> std::io::Result<()> {
    if fs::metadata(path).map(|m| m.len()).unwrap_or(0) < max_bytes {
        return Ok(());
    }
    for n in (1..files).rev() {
        let from = PathBuf::from(format!("{}.{}", path.display(), n));
        let to = PathBuf::from(format!("{}.{}", path.display(), n + 1));
        if from.exists() {
            let _ = fs::rename(from, to);
        }
    }
    if files > 0 {
        fs::rename(path, PathBuf::from(format!("{}.1", path.display())))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redacts_credentials_and_home() {
        std::env::set_var("HOME", "/private/alice");
        let s = redact("token=abc password: xyz /private/alice/file");
        assert!(!s.contains("abc"));
        assert!(!s.contains("xyz"));
        assert!(s.contains("~/file"));
    }
    #[test]
    fn rotation_is_bounded() {
        let d = std::env::temp_dir().join(format!("ps-log-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        let p = d.join("app.log");
        fs::write(&p, b"12345").unwrap();
        rotate_log(&p, 2, 2).unwrap();
        assert!(d.join("app.log.1").exists());
        fs::remove_dir_all(d).unwrap();
    }
}
