// state.rs — application state shared between the Tauri command handlers and
// the HTTP/WS API server. Holds config, scanner core handle, DB pool, sidecars.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use tokio::sync::broadcast;

use crate::audio::AudioSink;
use crate::config::{Config, ScanRange};
use crate::db::Db;
use crate::device::DeviceLayer;
use crate::scanner::ScannerHandle;
use crate::sidecar::SidecarRegistry;

/// Global application state. Cheap to `Arc::clone`, all interior-shared.
pub struct AppState {
    pub config: RwLock<Config>,
    pub db: Db,
    pub device: Arc<DeviceLayer>,
    pub audio: Arc<AudioSink>,
    pub recording: Arc<Mutex<RecordingState>>,
    pub playback: Arc<Mutex<Option<crate::capture::PlaybackReader>>>,
    pub iq_network: crate::capture::IqNetworkSink,
    pub scanner: RwLock<Option<ScannerHandle>>,
    /// Exclusive physical SDR lease. Consumers must claim this before retuning
    /// or creating a capture stream; force takeover is explicit and visible.
    pub receiver_session: Mutex<ReceiverSession>,
    pub trunking: RwLock<TrunkingRuntime>,
    pub sidecars: SidecarRegistry,
    /// Broadcasts every scanner event (spectrum, signal hit, decoded message,
    /// trunking update) to subscribed WS clients. High throughput — receivers
    /// may drop frames if they fall behind.
    pub events: broadcast::Sender<ScannerEvent>,
    pub data_dir: PathBuf,
    pub started_ms: i64,
}

impl AppState {
    pub fn new() -> Arc<Self> {
        let data_dir = crate::state::AppState::default_data_dir();
        let config = Config::load(&data_dir);
        let db = Db::open(&data_dir.join("pulsescope.db"))
            .expect("failed to open pulsescope.db");
        let (events_tx, _events_rx) = broadcast::channel(1024);
        let device = Arc::new(DeviceLayer::new_mock());
        // Prefer a previously selected physical SDR; otherwise the first real
        // Soapy device. Mock is a fallback for machines with no hardware.
        let preferred = (!config.device.last_device_key.trim().is_empty()).then_some(config.device.last_device_key.as_str());
        if let Err(error) = device.auto_connect(preferred) {
            tracing::warn!(%error, "physical SDR auto-connect failed; using mock fallback");
        }

        Arc::new(Self {
            config: RwLock::new(config),
            db,
            device,
            audio: Arc::new(AudioSink::new()),
            recording: Arc::new(Mutex::new(RecordingState::default())),
            playback: Arc::new(Mutex::new(None)),
            iq_network: crate::capture::IqNetworkSink::new(),
            scanner: RwLock::new(None),
            receiver_session: Mutex::new(ReceiverSession::default()),
            trunking: RwLock::new(TrunkingRuntime::default()),
            sidecars: SidecarRegistry::new(),
            events: events_tx,
            data_dir,
            started_ms: crate::scanner::now_ms(),
        })
    }

    /// Start a visible but silent first-run monitor so the desktop never opens
    /// onto an empty spectrum/VFO dashboard. Audio stays muted until the user
    /// explicitly unmutes a VFO.
    pub fn start_default_monitor(self: &Arc<Self>) {
        if !self.device.status().connected || self.scanner.read().is_some() { return; }
        if let Err(error) = self.receiver_session.lock().claim("scanner", false) {
            tracing::warn!(%error, "default monitor could not claim receiver");
            return;
        }
        let range: Option<ScanRange> = self.config.read().scan_ranges.iter()
            .find(|r| r.name == "FM Broadcast")
            .cloned()
            .or_else(|| self.config.read().scan_ranges.first().cloned());
        let Some(range) = range else { return; };
        if self.device.set_sample_rate(range.sample_rate_hz).is_err()
            || self.device.set_bandwidth(range.channel_bw_hz).is_err()
            || self.device.set_frequency(range.start_hz).is_err() {
            self.receiver_session.lock().release("scanner");
            return;
        }
        let cfg = self.config.read().scanner.clone();
        let handle = ScannerHandle::spawn(cfg, self.device.clone(), self.db.clone(), self.recording.clone(), self.playback.clone(), self.audio.clone(), self.iq_network.clone(), self.sidecars.clone(), self.events.clone());
        let _ = handle.cmd_tx.send(crate::scanner::ScannerCommand::Start { range });
        *self.scanner.write() = Some(handle);
    }

    /// Poll and execute due one-shot scan jobs. Jobs intentionally need an
    /// explicit duration so they cannot monopolize a receiver indefinitely.
    pub fn start_job_scheduler(self: &Arc<Self>) {
        let app = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop { tick.tick().await; app.run_due_jobs(); }
        });
    }

    fn run_due_jobs(self: &Arc<Self>) {
        let now = crate::scanner::now_ms();
        let Ok(jobs) = self.db.due_scheduled_jobs(now) else { return; };
        for job in jobs {
            let id = job.id.unwrap_or_default();
            if job.kind == "recording" {
                let payload: serde_json::Value = match serde_json::from_str(&job.payload_json) { Ok(v) => v, Err(e) => { let _=self.db.mark_scheduled_job(id,"failed",&format!("invalid payload: {e}"),false,now); continue; } };
                let duration_ms = payload.get("duration_ms").and_then(|v|v.as_u64()).unwrap_or(60_000).clamp(1_000, 3_600_000);
                let path = self.data_dir.join("recordings").join(format!("job-{id}-{now}.cf32"));
                if std::fs::create_dir_all(path.parent().expect("recordings parent")).is_err() || self.recording.lock().file.is_some() { let _=self.db.mark_scheduled_job(id,"blocked","recording already active or output directory unavailable",false,now); continue; }
                match File::create(&path) { Ok(file) => { let mut rec=self.recording.lock(); rec.file=Some(file); rec.path=Some(path); rec.started_ms=Some(now); rec.samples_written=0; rec.bytes_written=0; rec.write_error=None; }, Err(e) => { let _=self.db.mark_scheduled_job(id,"failed",&e.to_string(),false,now); continue; } }
                let _=self.db.mark_scheduled_job(id,"running","",false,now);
                let app=self.clone(); tokio::spawn(async move { tokio::time::sleep(Duration::from_millis(duration_ms)).await; let status=app.recording.lock().stop(); let error=status.get("write_error").and_then(|v|v.as_str()).unwrap_or(""); let _=app.db.mark_scheduled_job(id,if error.is_empty(){"completed"}else{"failed"},error,false,crate::scanner::now_ms()); });
                continue;
            }
            if job.kind != "scan" {
                let _ = self.db.mark_scheduled_job(id, "unsupported", "executor currently supports scan jobs only", false, now);
                continue;
            }
            let payload: serde_json::Value = match serde_json::from_str(&job.payload_json) { Ok(v) => v, Err(e) => { let _=self.db.mark_scheduled_job(id,"failed",&format!("invalid payload: {e}"),false,now); continue; } };
            let Some(range_name) = payload.get("range_name").and_then(|v|v.as_str()) else { let _=self.db.mark_scheduled_job(id,"failed","scan job requires payload.range_name",false,now); continue; };
            let duration_ms = payload.get("duration_ms").and_then(|v|v.as_u64()).unwrap_or(60_000).clamp(1_000, 3_600_000);
            let range = self.config.read().scan_ranges.iter().find(|r|r.name==range_name).cloned();
            let Some(range) = range else { let _=self.db.mark_scheduled_job(id,"failed","unknown scan range",false,now); continue; };
            if let Some(handle)=self.scanner.read().as_ref() { let _=handle.cmd_tx.send(crate::scanner::ScannerCommand::Stop); }
            self.audio.clear_queue(); self.receiver_session.lock().release("scanner");
            let owner=format!("job:{id}");
            if let Err(error)=self.receiver_session.lock().claim(&owner,false) { let _=self.db.mark_scheduled_job(id,"blocked",&error,true,now); continue; }
            if self.device.set_sample_rate(range.sample_rate_hz).is_err() || self.device.set_bandwidth(range.channel_bw_hz).is_err() || self.device.set_frequency(range.start_hz).is_err() { self.receiver_session.lock().release(&owner); let _=self.db.mark_scheduled_job(id,"failed","device configuration failed",false,now); continue; }
            if let Some(handle)=self.scanner.read().as_ref() { let _=handle.cmd_tx.send(crate::scanner::ScannerCommand::Start { range }); } else { self.receiver_session.lock().release(&owner); let _=self.db.mark_scheduled_job(id,"failed","scanner runtime unavailable",false,now); continue; }
            let _=self.db.mark_scheduled_job(id,"running","",false,now);
            let app=self.clone(); tokio::spawn(async move { tokio::time::sleep(Duration::from_millis(duration_ms)).await; if let Some(handle)=app.scanner.read().as_ref(){let _=handle.cmd_tx.send(crate::scanner::ScannerCommand::Stop);} app.audio.clear_queue(); app.receiver_session.lock().release(&owner); let _=app.db.mark_scheduled_job(id,"completed","",false,crate::scanner::now_ms()); });
        }
    }

    pub fn default_data_dir() -> PathBuf {
        if let Some(path) = std::env::var_os("PULSESCOPE_DATA_DIR") {
            return PathBuf::from(path);
        }
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            PathBuf::from(home).join("pulsescope")
        } else {
            PathBuf::from("./pulsescope-data")
        }
    }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct ReceiverSession {
    pub owner: Option<String>,
    pub acquired_ms: Option<i64>,
    pub takeovers: u64,
}

impl ReceiverSession {
    pub fn claim(&mut self, owner: &str, force: bool) -> Result<(), String> {
        if let Some(current) = &self.owner {
            if current != owner && !force { return Err(format!("receiver is held by {current}")); }
            if current != owner { self.takeovers = self.takeovers.saturating_add(1); }
        }
        self.owner = Some(owner.to_owned());
        self.acquired_ms = Some(crate::scanner::now_ms());
        Ok(())
    }
    pub fn release(&mut self, owner: &str) { if self.owner.as_deref() == Some(owner) { self.owner = None; self.acquired_ms = None; } }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct TrunkingRuntime {
    pub running: bool,
    pub locked: bool,
    pub system: Option<String>,
    pub control_channel_hz: Option<u64>,
    pub active_talkgroup: Option<String>,
    pub voice_channels: Vec<u64>,
    pub calls: Vec<TrunkingCall>,
    pub discovery_running: bool,
    pub discovery_results: Vec<serde_json::Value>,
    pub zones: Vec<serde_json::Value>,
    pub log: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct TrunkingCall {
    pub timestamp_ms: i64,
    pub talkgroup: String,
    pub frequency_hz: u64,
    pub duration_ms: u64,
    pub encrypted: bool,
}

#[derive(Default)]
pub struct RecordingState {
    pub started_ms: Option<i64>,
    pub path: Option<PathBuf>,
    pub file: Option<File>,
    pub samples_written: u64,
    pub bytes_written: u64,
    pub write_error: Option<String>,
}

impl RecordingState {
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "recording": self.file.is_some(),
            "path": self.path,
            "started_ms": self.started_ms,
            "samples_written": self.samples_written,
            "bytes_written": self.bytes_written,
            "write_error": self.write_error,
            "format": "cf32-le",
        })
    }

    pub fn write_iq(&mut self, samples: &[rustfft::num_complex::Complex<f32>]) {
        if self.file.is_none() || self.write_error.is_some() { return; }
        let mut bytes = Vec::with_capacity(samples.len() * 8);
        for sample in samples {
            bytes.extend_from_slice(&sample.re.to_le_bytes());
            bytes.extend_from_slice(&sample.im.to_le_bytes());
        }
        let result = self.file.as_mut().expect("file checked").write_all(&bytes);
        match result {
            Ok(()) => {
                self.samples_written += samples.len() as u64;
                self.bytes_written += bytes.len() as u64;
            }
            Err(error) => {
                self.write_error = Some(error.to_string());
                self.file = None;
            }
        }
    }

    pub fn stop(&mut self) -> serde_json::Value {
        if let Some(mut file) = self.file.take() {
            let _ = file.flush();
        }
        let status = self.status();
        self.started_ms = None;
        status
    }
}

/// Tagged event sent over `/event-stream` and `ws://127.0.0.1:8765/events`.
/// UI consumers pick the channels they care about.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "kind", content = "data")]
pub enum ScannerEvent {
    /// FFT power-spectrum frame at a fixed update rate (default 20 Hz).
    Spectrum { range: String, bins: Vec<f32> },
    /// Signal that crossed squelch.
    SignalHit {
        frequency_hz: u64,
        strength_db: f32,
        snr_db: f32,
        bandwidth_hz: u32,
        protocol: String,
        family: String,
        confidence: f32,
        decoder: String,
    },
    /// Latest VFO state snapshot (per VFO: frequency, mode, mute, gain).
    VfoStates(Vec<crate::scanner::VfoState>),
    /// Decoded text message from a sidecar decoder.
    DecodedMessage(crate::db::DecodedMessage),
    /// Trunking controller state change.
    TrunkingUpdate(crate::scanner::TrunkingState),
    /// Spectrum occupancy bucket (for the long-term band-use map).
    SpectrumOccupancy(crate::db::SpectrumOccupancy),
}

#[cfg(test)]
mod recording_tests {
    use super::RecordingState;
    use rustfft::num_complex::Complex;
    use std::fs::{self, File};
    use std::path::PathBuf;

    #[test]
    fn iq_recording_writes_cf32_bytes_and_counts_exactly() {
        let path = PathBuf::from(std::env::temp_dir()).join(format!("pulsescope-recording-test-{}.cf32", std::process::id()));
        let mut state = RecordingState::default();
        state.file = Some(File::create(&path).unwrap());
        state.path = Some(path.clone());
        let samples = [Complex::new(0.25, -0.5), Complex::new(-1.0, 1.0)];
        state.write_iq(&samples);
        assert_eq!(state.samples_written, 2);
        assert_eq!(state.bytes_written, 16);
        assert_eq!(state.write_error, None);
        state.stop();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 16);
        assert_eq!(&bytes[0..4], &0.25f32.to_le_bytes());
        assert_eq!(&bytes[4..8], &(-0.5f32).to_le_bytes());
        fs::remove_file(path).unwrap();
    }
}
