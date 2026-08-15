// state.rs — application state shared between the Tauri command handlers and
// the HTTP/WS API server. Holds config, scanner core handle, DB pool, sidecars.

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use tokio::sync::{broadcast, watch};

use crate::audio::AudioSink;
use crate::config::{Config, ScanRange};
use crate::db::Db;
use crate::decoder_scheduler::DecoderScheduler;
use crate::device::DeviceLayer;
use crate::scanner::{ScannerDependencies, ScannerHandle};
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
    /// Bounded result ledger for idempotent v2 commands. A retried command ID
    /// returns its original response instead of applying a second retune or
    /// session takeover.
    pub command_results: Mutex<HashMap<String, serde_json::Value>>,
    /// Per-browser view state. Listener sessions never own the physical SDR;
    /// they describe each client's viewport and selected VFO while the shared
    /// receiver lease controls hardware-window retunes.
    pub listener_sessions: RwLock<HashMap<String, ListenerSession>>,
    pub trunking: RwLock<TrunkingRuntime>,
    pub sidecars: SidecarRegistry,
    pub decoder_scheduler: DecoderScheduler,
    /// Native ham decoder tasks are explicitly tied to their selected narrow
    /// operating window and are cancelled when the operator changes range.
    pub ham_decoder_tasks: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    /// Broadcasts every scanner event (spectrum, signal hit, decoded message,
    /// trunking update) to subscribed WS clients. High throughput — receivers
    /// may drop frames if they fall behind.
    pub events: broadcast::Sender<ScannerEvent>,
    /// Latest-only spectrum transport. A slow browser observes a sequence gap
    /// and resumes from the newest frame instead of building latency.
    pub spectrum: watch::Sender<SpectrumFrame>,
    pub data_dir: PathBuf,
    pub started_ms: i64,
    pub receiver_recoveries: AtomicU64,
    pub last_receiver_recovery_ms: AtomicI64,
    pub metrics: Arc<crate::operations::Metrics>,
}

impl AppState {
    pub fn new() -> Arc<Self> {
        let data_dir = crate::state::AppState::default_data_dir();
        let mut config = Config::load(&data_dir);
        if crate::depmanager::bootstrap_discovered_decoder_paths(&mut config, &data_dir) {
            tracing::info!("configured decoder paths from system discovery");
            let _ = config.save(&data_dir);
        }
        let decoder_scheduler =
            DecoderScheduler::with_trusted_keys(DecoderScheduler::load_trusted_keys(&data_dir));
        let db = Db::open(&data_dir.join("pulsescope.db")).expect("failed to open pulsescope.db");
        let (events_tx, _events_rx) = broadcast::channel(1024);
        let (spectrum_tx, _spectrum_rx) = watch::channel(SpectrumFrame::default());
        let device = Arc::new(DeviceLayer::new_mock());
        // Prefer a previously selected physical SDR; otherwise the first real
        // Soapy device. Mock is a fallback for machines with no hardware.
        let preferred = (!config.device.last_device_key.trim().is_empty())
            .then_some(config.device.last_device_key.as_str());
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
            command_results: Mutex::new(HashMap::new()),
            listener_sessions: RwLock::new(HashMap::new()),
            trunking: RwLock::new(TrunkingRuntime::default()),
            sidecars: SidecarRegistry::new(),
            decoder_scheduler,
            ham_decoder_tasks: Mutex::new(HashMap::new()),
            events: events_tx,
            spectrum: spectrum_tx,
            data_dir,
            started_ms: crate::scanner::now_ms(),
            receiver_recoveries: AtomicU64::new(0),
            last_receiver_recovery_ms: AtomicI64::new(0),
            metrics: Arc::new(crate::operations::Metrics::default()),
        })
    }

    /// Start a visible but silent first-run monitor so the desktop never opens
    /// onto an empty spectrum/VFO dashboard. Audio stays muted until the user
    /// explicitly unmutes a VFO.
    pub fn start_default_monitor(self: &Arc<Self>) {
        if let Err(error) = self.ensure_receiver_flow(false) {
            tracing::warn!(%error, "default monitor could not start");
        }
    }

    fn default_monitor_range(&self) -> Option<ScanRange> {
        let active_name = self
            .scanner
            .read()
            .as_ref()
            .and_then(|handle| handle.state.lock().active_range.clone());
        let config = self.config.read();
        active_name
            .as_deref()
            .and_then(|name| {
                config
                    .scan_ranges
                    .iter()
                    .find(|range| range.name == name)
                    .cloned()
            })
            .or_else(|| {
                config
                    .scan_ranges
                    .iter()
                    .find(|range| range.name == "FM Broadcast")
                    .cloned()
            })
            .or_else(|| config.scan_ranges.first().cloned())
    }

    fn ensure_receiver_flow(self: &Arc<Self>, recovering: bool) -> Result<(), String> {
        if !self.device.status().connected {
            return Err("device is disconnected".into());
        }
        self.receiver_session.lock().claim("scanner", false)?;
        let Some(range) = self.default_monitor_range() else {
            self.receiver_session.lock().release("scanner");
            return Err("no receiver ranges are configured".into());
        };
        let requested_rate = self
            .config
            .read()
            .device
            .sample_rate
            .max(range.sample_rate_hz);
        if self.device.set_sample_contract(requested_rate).is_err()
            || self
                .device
                .set_frequency(crate::scanner::initial_scan_center(&range, requested_rate))
                .is_err()
        {
            self.receiver_session.lock().release("scanner");
            return Err("device configuration failed".into());
        }

        if recovering {
            if let Some(handle) = self.scanner.write().take() {
                handle.abort();
            }
        } else {
            let existing = self
                .scanner
                .read()
                .as_ref()
                .map(|handle| handle.cmd_tx.clone());
            if let Some(command) = existing {
                if command
                    .send(crate::scanner::ScannerCommand::Start {
                        range: range.clone(),
                    })
                    .is_ok()
                {
                    return Ok(());
                }
            }
        }

        let cfg = self.config.read().scanner.clone();
        let wfm_deemphasis_us = self.config.read().demodulator.de_emphasis_us;
        let dependencies = ScannerDependencies {
            device: self.device.clone(),
            db: self.db.clone(),
            recording: self.recording.clone(),
            playback: self.playback.clone(),
            audio: self.audio.clone(),
            iq_network: self.iq_network.clone(),
            sidecars: self.sidecars.clone(),
            events_tx: self.events.clone(),
            spectrum_tx: self.spectrum.clone(),
            wfm_deemphasis_us,
        };
        let handle = ScannerHandle::spawn(cfg, dependencies);
        handle
            .cmd_tx
            .send(crate::scanner::ScannerCommand::Start { range })
            .map_err(|_| "receiver task did not accept its startup command".to_string())?;
        *self.scanner.write() = Some(handle);
        if recovering {
            self.observe_receiver_recovery();
        }
        Ok(())
    }

    fn observe_receiver_recovery(&self) {
        let now = crate::scanner::now_ms();
        self.receiver_recoveries.fetch_add(1, Ordering::Relaxed);
        self.last_receiver_recovery_ms.store(now, Ordering::Relaxed);
        tracing::warn!(timestamp_ms = now, "receiver flow restarted by supervisor");
    }

    /// Poll and execute due one-shot scan jobs. Jobs intentionally need an
    /// explicit duration so they cannot monopolize a receiver indefinitely.
    pub fn start_job_scheduler(self: &Arc<Self>) {
        let app = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tick.tick().await;
                app.run_due_jobs();
            }
        });
    }

    /// Re-probe after startup and on hotplug. Some vendor APIs publish the USB
    /// device a few seconds after the container process starts; previously that
    /// race permanently selected the mock source until a manual reconnect.
    pub fn start_hardware_supervisor(self: &Arc<Self>) {
        if std::env::var("PULSESCOPE_PREFER_PHYSICAL").as_deref() == Ok("0") {
            return;
        }
        let app = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            let mut next_hardware_probe = tokio::time::Instant::now();
            let mut hardware_probe_backoff = Duration::from_secs(2);
            loop {
                tick.tick().await;
                let status = app.device.status();
                if status.driver == "mock" {
                    let now = tokio::time::Instant::now();
                    if now < next_hardware_probe {
                        continue;
                    }
                    let devices = tokio::task::spawn_blocking(crate::device::DeviceLayer::discover)
                        .await
                        .unwrap_or_default();
                    let Some(candidate) =
                        devices.into_iter().find(|device| device.driver != "mock")
                    else {
                        next_hardware_probe = now + hardware_probe_backoff;
                        hardware_probe_backoff = hardware_probe_backoff
                            .saturating_mul(2)
                            .min(Duration::from_secs(30));
                        continue;
                    };
                    let device = app.device.clone();
                    let key = candidate.key.clone();
                    let connected = tokio::task::spawn_blocking(move || device.connect(&key)).await;
                    if !matches!(connected, Ok(Ok(()))) {
                        next_hardware_probe = now + hardware_probe_backoff;
                        hardware_probe_backoff = hardware_probe_backoff
                            .saturating_mul(2)
                            .min(Duration::from_secs(30));
                        continue;
                    }
                    hardware_probe_backoff = Duration::from_secs(2);
                    tracing::info!(driver = %candidate.driver, label = %candidate.label, "physical SDR selected after probe");
                }

                let now = crate::scanner::now_ms();
                let runtime = app
                    .scanner
                    .read()
                    .as_ref()
                    .map(|handle| handle.state.lock().clone());
                if receiver_needs_recovery(runtime.as_ref(), now) {
                    app.audio.clear_queue();
                    if let Err(error) = app.ensure_receiver_flow(true) {
                        tracing::warn!(%error, "receiver supervisor could not restore sample flow");
                    }
                }
            }
        });
    }

    fn run_due_jobs(self: &Arc<Self>) {
        let now = crate::scanner::now_ms();
        let Ok(jobs) = self.db.due_scheduled_jobs(now) else {
            return;
        };
        for job in jobs {
            let id = job.id.unwrap_or_default();
            if job.kind == "recording" {
                let payload: serde_json::Value = match serde_json::from_str(&job.payload_json) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = self.db.mark_scheduled_job(
                            id,
                            "failed",
                            &format!("invalid payload: {e}"),
                            false,
                            now,
                        );
                        continue;
                    }
                };
                let duration_ms = payload
                    .get("duration_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(60_000)
                    .clamp(1_000, 3_600_000);
                let path = self
                    .data_dir
                    .join("recordings")
                    .join(format!("job-{id}-{now}.cf32"));
                if std::fs::create_dir_all(path.parent().expect("recordings parent")).is_err()
                    || self.recording.lock().file.is_some()
                {
                    let _ = self.db.mark_scheduled_job(
                        id,
                        "blocked",
                        "recording already active or output directory unavailable",
                        false,
                        now,
                    );
                    continue;
                }
                match File::create(&path) {
                    Ok(file) => {
                        let mut rec = self.recording.lock();
                        rec.file = Some(file);
                        rec.path = Some(path);
                        rec.started_ms = Some(now);
                        rec.samples_written = 0;
                        rec.bytes_written = 0;
                        rec.write_error = None;
                    }
                    Err(e) => {
                        let _ =
                            self.db
                                .mark_scheduled_job(id, "failed", &e.to_string(), false, now);
                        continue;
                    }
                }
                let _ = self.db.mark_scheduled_job(id, "running", "", false, now);
                let app = self.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(duration_ms)).await;
                    let status = app.recording.lock().stop();
                    let error = status
                        .get("write_error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let _ = app.db.mark_scheduled_job(
                        id,
                        if error.is_empty() {
                            "completed"
                        } else {
                            "failed"
                        },
                        error,
                        false,
                        crate::scanner::now_ms(),
                    );
                });
                continue;
            }
            if job.kind != "scan" {
                let _ = self.db.mark_scheduled_job(
                    id,
                    "unsupported",
                    "executor currently supports scan jobs only",
                    false,
                    now,
                );
                continue;
            }
            let payload: serde_json::Value = match serde_json::from_str(&job.payload_json) {
                Ok(v) => v,
                Err(e) => {
                    let _ = self.db.mark_scheduled_job(
                        id,
                        "failed",
                        &format!("invalid payload: {e}"),
                        false,
                        now,
                    );
                    continue;
                }
            };
            let Some(range_name) = payload.get("range_name").and_then(|v| v.as_str()) else {
                let _ = self.db.mark_scheduled_job(
                    id,
                    "failed",
                    "scan job requires payload.range_name",
                    false,
                    now,
                );
                continue;
            };
            let duration_ms = payload
                .get("duration_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(60_000)
                .clamp(1_000, 3_600_000);
            let range = self
                .config
                .read()
                .scan_ranges
                .iter()
                .find(|r| r.name == range_name)
                .cloned();
            let Some(range) = range else {
                let _ = self
                    .db
                    .mark_scheduled_job(id, "failed", "unknown scan range", false, now);
                continue;
            };
            if let Some(handle) = self.scanner.read().as_ref() {
                let _ = handle.cmd_tx.send(crate::scanner::ScannerCommand::Stop);
            }
            self.audio.clear_queue();
            self.receiver_session.lock().release("scanner");
            let owner = format!("job:{id}");
            if let Err(error) = self.receiver_session.lock().claim(&owner, false) {
                let _ = self.db.mark_scheduled_job(id, "blocked", &error, true, now);
                continue;
            }
            let requested_rate = self
                .config
                .read()
                .device
                .sample_rate
                .max(range.sample_rate_hz);
            if self.device.set_sample_contract(requested_rate).is_err()
                || self
                    .device
                    .set_frequency(crate::scanner::initial_scan_center(&range, requested_rate))
                    .is_err()
            {
                self.receiver_session.lock().release(&owner);
                let _ = self.db.mark_scheduled_job(
                    id,
                    "failed",
                    "device configuration failed",
                    false,
                    now,
                );
                continue;
            }
            if let Some(handle) = self.scanner.read().as_ref() {
                let _ = handle
                    .cmd_tx
                    .send(crate::scanner::ScannerCommand::Start { range });
            } else {
                self.receiver_session.lock().release(&owner);
                let _ = self.db.mark_scheduled_job(
                    id,
                    "failed",
                    "scanner runtime unavailable",
                    false,
                    now,
                );
                continue;
            }
            let _ = self.db.mark_scheduled_job(id, "running", "", false, now);
            let app = self.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(duration_ms)).await;
                if let Some(handle) = app.scanner.read().as_ref() {
                    let _ = handle.cmd_tx.send(crate::scanner::ScannerCommand::Stop);
                }
                app.audio.clear_queue();
                app.receiver_session.lock().release(&owner);
                let _ =
                    app.db
                        .mark_scheduled_job(id, "completed", "", false, crate::scanner::now_ms());
            });
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
    pub revision: u64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ListenerSession {
    pub id: String,
    pub client_name: String,
    pub receiver_id: String,
    pub view_center_hz: u64,
    pub view_span_hz: u32,
    pub active_vfo_id: Option<usize>,
    pub revision: u64,
    pub updated_ms: i64,
}

impl ReceiverSession {
    pub fn claim(&mut self, owner: &str, force: bool) -> Result<(), String> {
        if let Some(current) = &self.owner {
            if current != owner && !force {
                return Err(format!("receiver is held by {current}"));
            }
            if current != owner {
                self.takeovers = self.takeovers.saturating_add(1);
            }
        }
        self.owner = Some(owner.to_owned());
        self.acquired_ms = Some(crate::scanner::now_ms());
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }
    pub fn release(&mut self, owner: &str) {
        if self.owner.as_deref() == Some(owner) {
            self.owner = None;
            self.acquired_ms = None;
            self.revision = self.revision.saturating_add(1);
        }
    }
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
        if self.file.is_none() || self.write_error.is_some() {
            return;
        }
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
#[derive(Clone, Debug, Default)]
pub struct SpectrumFrame {
    pub sequence: u64,
    pub captured_ms: i64,
    pub center_freq_hz: u64,
    pub sample_rate_hz: u32,
    pub usable_span_hz: u32,
    pub bins_dbfs: Vec<f32>,
}

fn receiver_needs_recovery(
    runtime: Option<&crate::scanner::ScannerRuntimeState>,
    now_ms: i64,
) -> bool {
    let Some(runtime) = runtime else {
        return true;
    };
    if !runtime.running || runtime.vfo_states.is_empty() {
        return true;
    }
    if runtime.latest_spectrum_ms <= 0 {
        return runtime.started_ms > 0 && now_ms.saturating_sub(runtime.started_ms) > 5_000;
    }
    now_ms.saturating_sub(runtime.latest_spectrum_ms) > 5_000
}

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
    use super::{receiver_needs_recovery, ListenerSession, ReceiverSession, RecordingState};
    use crate::scanner::{ScannerRuntimeState, VfoState};
    use rustfft::num_complex::Complex;
    use std::fs::{self, File};

    #[test]
    fn iq_recording_writes_cf32_bytes_and_counts_exactly() {
        let path = std::env::temp_dir().join(format!(
            "pulsescope-recording-test-{}.cf32",
            std::process::id()
        ));
        let mut state = RecordingState {
            file: Some(File::create(&path).unwrap()),
            path: Some(path.clone()),
            ..Default::default()
        };
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

    fn running_receiver(latest_spectrum_ms: i64) -> ScannerRuntimeState {
        ScannerRuntimeState {
            active_range: Some("FM Broadcast".into()),
            running: true,
            started_ms: 1_000,
            vfo_states: vec![VfoState {
                id: 0,
                frequency_hz: 100_000_000,
                mode: "wfm".into(),
                muted: true,
                volume: 0.7,
                audio_agc: true,
                squelch_open: false,
                strength_db: -120.0,
                audio_level_db: -120.0,
                locked: false,
                last_hit_ms: 0,
                snr_db: 0.0,
                noise_floor_db: -120.0,
            }],
            latest_spectrum_ms,
            ..Default::default()
        }
    }

    #[test]
    fn receiver_supervisor_recovers_missing_stopped_and_stale_flows() {
        assert!(receiver_needs_recovery(None, 10_000));
        assert!(receiver_needs_recovery(
            Some(&ScannerRuntimeState::default()),
            10_000
        ));
        assert!(receiver_needs_recovery(
            Some(&running_receiver(2_000)),
            10_000
        ));
        assert!(!receiver_needs_recovery(
            Some(&running_receiver(9_000)),
            10_000
        ));
    }

    #[test]
    fn receiver_session_revision_advances_only_on_applied_changes() {
        let mut session = ReceiverSession::default();
        session.claim("scanner", false).unwrap();
        assert_eq!(session.revision, 1);
        assert!(session.claim("operator", false).is_err());
        assert_eq!(session.revision, 1);
        session.claim("operator", true).unwrap();
        assert_eq!(session.revision, 2);
        assert_eq!(session.takeovers, 1);
        session.release("scanner");
        assert_eq!(session.revision, 2);
        session.release("operator");
        assert_eq!(session.revision, 3);
    }

    #[test]
    fn listener_session_serializes_independent_view_state() {
        let listener = ListenerSession {
            id: "browser-a".into(),
            client_name: "Phone".into(),
            receiver_id: "receiver-0".into(),
            view_center_hz: 100_100_000,
            view_span_hz: 2_000_000,
            active_vfo_id: Some(0),
            revision: 4,
            updated_ms: 123,
        };
        let json = serde_json::to_value(listener).unwrap();
        assert_eq!(json["view_span_hz"], 2_000_000);
        assert_eq!(json["revision"], 4);
    }
}
