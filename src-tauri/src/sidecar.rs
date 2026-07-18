// sidecar.rs — process launcher for the open-source GPL decoder binaries.
//
// PulseScope does NOT link any GPL code. Each decoder is spawned as a child
// process; we feed it I/Q or audio on stdin (or via a localhost socket) and
// parse its textual stdout for decoded messages, which we broadcast over the
// event stream and persist to SQLite.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::broadcast;

use crate::db::Db;
use crate::state::ScannerEvent;

const MAX_LINE_BYTES: usize = 64 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(3);
const IO_TIMEOUT: Duration = Duration::from_millis(750);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RESTARTS: u8 = 3;

#[derive(Clone)]
pub struct SidecarRegistry {
    children: Arc<Mutex<HashMap<String, Arc<Mutex<Child>>>>>,
    inputs: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<ChildStdin>>>>>,
    stderr: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    input_stats: Arc<Mutex<HashMap<String, (u64, u64)>>>,
    failures: Arc<Mutex<HashMap<String, String>>>,
    restarts: Arc<Mutex<HashMap<String, u8>>>,
}

pub fn encode_u8_iq(samples: &[rustfft::num_complex::Complex<f32>]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let i = ((sample.re.clamp(-1.0, 1.0) * 127.5) + 127.5).round() as u8;
        let q = ((sample.im.clamp(-1.0, 1.0) * 127.5) + 127.5).round() as u8;
        bytes.extend_from_slice(&[i, q]);
    }
    bytes
}

impl SidecarRegistry {
    pub fn new() -> Self {
        Self {
            children: Arc::new(Mutex::new(HashMap::new())),
            inputs: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            stderr: Arc::new(Mutex::new(HashMap::new())),
            input_stats: Arc::new(Mutex::new(HashMap::new())),
            failures: Arc::new(Mutex::new(HashMap::new())),
            restarts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn is_running(&self, name: &str) -> bool {
        self.children
            .lock()
            .get(name)
            .map(|child| {
                let mut child = child.lock();
                match child.try_wait() {
                    Ok(Some(_)) => false,
                    Ok(None) => true,
                    Err(_) => false,
                }
            })
            .unwrap_or(false)
    }

    pub async fn spawn_decoder(
        &self,
        name: &str,
        exe: PathBuf,
        args: Vec<String>,
        db: Db,
        events_tx: broadcast::Sender<ScannerEvent>,
    ) -> anyhow::Result<()> {
        let mut cmd = Command::new(&exe);
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");
        let protocol = name.to_string();

        let mut stdout = tokio::io::BufReader::new(stdout);
        let events_tx_clone = events_tx.clone();
        let protocol_for_task = protocol.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = stdout.lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) if line.len() <= MAX_LINE_BYTES => {
                        tracing::debug!(proto = %protocol_for_task, line = %line, "sidecar");
                        if let Some(m) = parse_line(&protocol_for_task, &line) {
                            let _ = db.insert_decoded_message(&m);
                            let _ = events_tx_clone.send(ScannerEvent::DecodedMessage(m));
                        }
                    }
                    Ok(Some(_)) => {
                        tracing::warn!(proto=%protocol_for_task, "oversized sidecar line discarded");
                    }
                    Ok(None) => break,
                    Err(e) => {
                        tracing::warn!(proto = %protocol_for_task, err = %e, "sidecar stdout error");
                        break;
                    }
                }
            }
        });

        let stderr_log = self.stderr.clone();
        let stderr_name = protocol.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(sidecar = %stderr_name, line = %line, "sidecar stderr");
                let mut logs = stderr_log.lock();
                let buffer = logs.entry(stderr_name.clone()).or_default();
                if buffer.len() >= 200 {
                    buffer.pop_front();
                }
                buffer.push_back(line.chars().take(MAX_LINE_BYTES).collect());
            }
        });

        self.children
            .lock()
            .insert(name.to_string(), Arc::new(Mutex::new(child)));
        self.input_stats.lock().insert(name.to_string(), (0, 0));
        self.inputs
            .lock()
            .await
            .insert(name.to_string(), Arc::new(tokio::sync::Mutex::new(stdin)));
        tokio::time::timeout(
            STARTUP_TIMEOUT,
            tokio::time::sleep(Duration::from_millis(250)),
        )
        .await
        .map_err(|_| anyhow::anyhow!("decoder readiness timed out"))?;
        let exited = self
            .children
            .lock()
            .get(name)
            .and_then(|child| child.lock().try_wait().ok().flatten());
        if let Some(status) = exited {
            self.inputs.lock().await.remove(name);
            self.children.lock().remove(name);
            let detail = format!("decoder exited during startup with {status}");
            self.failures.lock().insert(name.into(), detail.clone());
            return Err(anyhow::anyhow!(detail));
        }
        Ok(())
    }

    pub async fn feed_iq(&self, samples: &[rustfft::num_complex::Complex<f32>]) {
        use tokio::io::AsyncWriteExt;
        let handles: Vec<_> = self
            .inputs
            .lock()
            .await
            .iter()
            .filter(|(name, _)| name.as_str() == "rtl_433")
            .map(|(_, handle)| handle.clone())
            .collect();
        if handles.is_empty() {
            return;
        }
        let bytes = encode_u8_iq(samples);
        if let Some((sample_count, byte_count)) = self.input_stats.lock().get_mut("rtl_433") {
            *sample_count += samples.len() as u64;
            *byte_count += bytes.len() as u64;
        }
        for handle in handles {
            let mut stdin = handle.lock().await;
            match tokio::time::timeout(IO_TIMEOUT, stdin.write_all(&bytes)).await {
                Err(e) => { self.failures.lock().insert("rtl_433".into(), format!("stdin backpressure timeout: {e}")); }
                Ok(Err(e)) => { self.failures.lock().insert("rtl_433".into(), format!("stdin closed: {e}")); }
                Ok(Ok(())) => if let Err(e) = stdin.flush().await { tracing::debug!(error = %e, "sidecar IQ input closed"); },
            }
        }
    }

    pub async fn kill(&self, name: &str) -> anyhow::Result<()> {
        self.inputs.lock().await.remove(name);
        let child_arc = self.children.lock().remove(name);
        if let Some(child_arc) = child_arc {
            let mut child = child_arc.lock();
            // Closing stdin first gives well-behaved tools an EOF. Poll for a
            // bounded grace period, then force termination.
            let deadline = std::time::Instant::now() + SHUTDOWN_TIMEOUT;
            while std::time::Instant::now() < deadline {
                if child.try_wait()?.is_some() {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            child.start_kill()?;
        }
        Ok(())
    }

    pub async fn kill_all(&self) -> anyhow::Result<()> {
        self.inputs.lock().await.clear();
        let kids: Vec<_> = self.children.lock().drain().collect();
        for (_, child_arc) in kids {
            let mut child = child_arc.lock();
            let _ = child.start_kill();
        }
        Ok(())
    }

    pub fn stderr(&self, name: &str) -> Vec<String> {
        self.stderr
            .lock()
            .get(name)
            .map(|v| v.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn statuses(&self) -> Vec<SidecarStatus> {
        self.children
            .lock()
            .iter()
            .map(|(name, child)| {
                let mut child = child.lock();
                let running = matches!(child.try_wait(), Ok(None));
                let exit_code = if running {
                    None
                } else {
                    child
                        .try_wait()
                        .ok()
                        .flatten()
                        .and_then(|status| status.code())
                };
                let stats = self.input_stats.lock().get(name).copied().unwrap_or((0, 0));
                SidecarStatus {
                    name: name.clone(),
                    running,
                    healthy: running && !self.failures.lock().contains_key(name),
                    pid: if running { child.id() } else { None },
                    exit_code,
                    input_samples: stats.0,
                    input_bytes: stats.1,
                    output_messages: 0,
                    restarts: *self.restarts.lock().get(name).unwrap_or(&0),
                    restart_limit: MAX_RESTARTS,
                    failure: self.failures.lock().get(name).cloned(),
                }
            })
            .collect()
    }
}

/// Parse one upstream decoder line into the common message model. These
/// parsers consume documented stdout formats, not proprietary application code.
pub fn parse_line(protocol: &str, line: &str) -> Option<crate::db::DecodedMessage> {
    use crate::db::DecodedMessage;
    let now = crate::scanner::now_ms();
    let p = protocol.to_ascii_lowercase();
    let lower = line.to_ascii_lowercase();
    let mut msg = DecodedMessage {
        id: None,
        frequency_hz: default_frequency(&p),
        protocol: p.clone(),
        message_type: "text".into(),
        address: String::new(),
        function_code: String::new(),
        content: line.to_string(),
        raw: line.to_string(),
        encryption: "unknown".into(),
        timestamp_ms: now,
    };

    if p == "rtl_433" {
        msg.message_type = "sensor".into();
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            msg.content = value.to_string();
            msg.address = value.get("id").map(ToString::to_string).unwrap_or_default();
            if let Some(model) = value.get("model").and_then(|v| v.as_str()) {
                msg.content = format!("model={model} {value}");
            }
            if let Some(freq) = value.get("freq").and_then(|v| v.as_f64()) {
                msg.frequency_hz = if freq < 1_000_000.0 {
                    (freq * 1_000_000.0) as u64
                } else {
                    freq as u64
                };
            }
        } else {
            // rtl_433 is launched with -F json. Do not turn banners, warnings,
            // or malformed lines into persisted sensor messages.
            return None;
        }
        return Some(msg);
    }

    if p.contains("multimon") || p == "pocsag" {
        msg.protocol = "pocsag".into();
        msg.message_type = "pager".into();
        msg.address = field_after(line, "Address:").unwrap_or_default();
        msg.function_code = field_after(line, "Function:").unwrap_or_default();
        msg.content = field_after(line, "Alpha:")
            .or_else(|| field_after(line, "Numeric:"))
            .unwrap_or_else(|| line.to_string());
        return Some(msg);
    }

    if p.contains("dumpvdl2") || p == "vdl2" {
        msg.protocol = "vdl2".into();
        msg.message_type = "aircraft_datalink".into();
        msg.address = field_after(line, "AC:")
            .map(|v| v.split(',').next().unwrap_or(&v).trim().to_string())
            .unwrap_or_default();
        msg.content = field_after(line, "Text:").unwrap_or_else(|| line.to_string());
        return Some(msg);
    }

    if p.contains("acars") {
        msg.protocol = "acars".into();
        msg.message_type = "aircraft_datalink".into();
        msg.content = field_after(line, "Message:").unwrap_or_else(|| line.to_string());
        msg.address = line
            .split_whitespace()
            .find(|v| v.len() == 6 && v.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or_default()
            .into();
        return Some(msg);
    }

    if p.contains("direwolf") || p == "aprs" {
        msg.protocol = "aprs".into();
        msg.message_type = "packet".into();
        if let Some((head, body)) = line.split_once(':') {
            msg.address = head.split('>').next().unwrap_or_default().into();
            msg.content = body.to_string();
        }
        return Some(msg);
    }

    if p.contains("dsd") || p == "p25" || p == "dmr" {
        msg.protocol = if lower.contains("p25") {
            "p25".into()
        } else {
            "digital_voice".into()
        };
        msg.message_type = "voice_metadata".into();
        msg.address = field_after(line, "TG:")
            .or_else(|| field_after(line, "Talkgroup:"))
            .unwrap_or_default();
        let encrypted = lower
            .split("encrypted:")
            .nth(1)
            .map(str::trim_start)
            .map(|v| v.starts_with("yes") || v.starts_with("true"))
            .unwrap_or_else(|| lower.contains("encrypted") && !lower.contains("not encrypted"));
        msg.encryption = if encrypted { "encrypted" } else { "none" }.into();
        return Some(msg);
    }

    if p.contains("rs41") || p.contains("radiosonde") {
        msg.protocol = "rs41".into();
        msg.message_type = "telemetry".into();
        msg.address = field_after(line, "serial=").unwrap_or_default();
        return Some(msg);
    }

    None
}

fn default_frequency(protocol: &str) -> u64 {
    match protocol {
        "rtl_433" => 433_920_000,
        "acarsdec" | "acars" => 131_550_000,
        "dumpvdl2" | "vdl2" => 136_975_000,
        "direwolf" | "aprs" => 144_390_000,
        "rs41" | "radiosonde" => 402_500_000,
        _ => 0,
    }
}

fn field_after(line: &str, marker: &str) -> Option<String> {
    let (_, value) = line.split_once(marker)?;
    let mut value = value.trim().trim_start_matches(':').trim();
    for next in [
        " Address:",
        " Function:",
        " Alpha:",
        " Numeric:",
        " Text:",
        " Message:",
        " TG:",
        " Talkgroup:",
        " encrypted:",
        " lat=",
    ] {
        if let Some((head, _)) = value.split_once(next) {
            value = head.trim();
        }
    }
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SidecarStatus {
    pub name: String,
    pub running: bool,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub input_samples: u64,
    pub input_bytes: u64,
    pub output_messages: u64,
    pub healthy: bool,
    pub restarts: u8,
    pub restart_limit: u8,
    pub failure: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::parse_line;

    #[test]
    fn rtl433_process_stdout_persists_through_normal_pipeline() {
        use std::process::Command;
        use std::time::{SystemTime, UNIX_EPOCH};
        let output = match Command::new("rtl_433.exe")
            .args(["-y", "{25}fb2dd58", "-F", "json"])
            .output()
        {
            Ok(output) => output,
            Err(error) => {
                eprintln!("rtl_433 unavailable: {error}");
                return;
            }
        };
        assert!(
            output.status.success(),
            "rtl_433 failed: {:?}",
            output.status
        );
        let path = std::env::temp_dir().join(format!(
            "pulsescope-sidecar-{}.db",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = crate::db::Db::open(&path).unwrap();
        let mut parsed = 0;
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some(message) = parse_line("rtl_433", line) {
                db.insert_decoded_message(&message).unwrap();
                parsed += 1;
            }
        }
        assert!(
            parsed >= 1,
            "decoder emitted no parseable JSON: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let stored = db.messages_by_protocol(Some("rtl_433"), 10).unwrap();
        assert_eq!(stored.len(), parsed);
        assert!(stored
            .iter()
            .any(|m| m.protocol == "rtl_433" && m.message_type == "sensor"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn encodes_interleaved_u8_iq_transport() {
        use rustfft::num_complex::Complex;
        assert_eq!(
            super::encode_u8_iq(&[Complex::new(-1.0, 1.0), Complex::new(0.0, 0.0)]),
            vec![0, 255, 128, 128]
        );
    }

    #[test]
    fn parses_rtl433_json() {
        let msg = parse_line("rtl_433", r#"{"model":"Test","id":42,"freq":433.92}"#).unwrap();
        assert_eq!(msg.protocol, "rtl_433");
        assert_eq!(msg.address, "42");
        assert_eq!(msg.frequency_hz, 433_920_000);
    }

    #[test]
    fn parses_pocsag_fields() {
        let msg = parse_line(
            "multimon-ng",
            "POCSAG512: Address: 123 Function: 0 Alpha: HELLO",
        )
        .unwrap();
        assert_eq!(msg.protocol, "pocsag");
        assert_eq!(msg.address, "123");
        assert_eq!(msg.content, "HELLO");
    }

    #[test]
    fn parses_additional_decoder_protocols() {
        let acars = parse_line("acarsdec", "ABC123 Message: WEATHER OK").unwrap();
        assert_eq!(acars.protocol, "acars");
        assert_eq!(acars.content, "WEATHER OK");
        let vdl = parse_line("dumpvdl2", "AC: ABC123, Label: H1, M: S, Text: CPDLC TEST").unwrap();
        assert_eq!(vdl.protocol, "vdl2");
        assert_eq!(vdl.address, "ABC123");
        let dsd = parse_line("dsd-neo", "P25 TG: 42 encrypted: yes").unwrap();
        assert_eq!(dsd.message_type, "voice_metadata");
        assert_eq!(dsd.address, "42");
        assert_eq!(dsd.encryption, "encrypted");
        let clear = parse_line("dsd-neo", "P25 TG: 42 Encrypted: NO").unwrap();
        assert_eq!(clear.encryption, "none");
        let sonde = parse_line("rs41mod", "serial=RS41-001 lat=39.7").unwrap();
        assert_eq!(sonde.protocol, "rs41");
        assert_eq!(sonde.address, "RS41-001");
    }
}
