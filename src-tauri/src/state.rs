// state.rs — application state shared between the Tauri command handlers and
// the HTTP/WS API server. Holds config, scanner core handle, DB pool, sidecars.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use tokio::sync::broadcast;

use crate::audio::AudioSink;
use crate::config::Config;
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

        Arc::new(Self {
            config: RwLock::new(config),
            db,
            device: Arc::new(DeviceLayer::new_mock()),
            audio: Arc::new(AudioSink::new()),
            recording: Arc::new(Mutex::new(RecordingState::default())),
            playback: Arc::new(Mutex::new(None)),
            iq_network: crate::capture::IqNetworkSink::new(),
            scanner: RwLock::new(None),
            trunking: RwLock::new(TrunkingRuntime::default()),
            sidecars: SidecarRegistry::new(),
            events: events_tx,
            data_dir,
            started_ms: crate::scanner::now_ms(),
        })
    }

    pub fn default_data_dir() -> PathBuf {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            PathBuf::from(home).join("pulsescope")
        } else {
            PathBuf::from("./pulsescope-data")
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
