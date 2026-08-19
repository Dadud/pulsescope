// scanner.rs — clean-room FFT-based scanner core.
//
// No code from any proprietary scanner is used here. The DSP is built from
// public primitives: windowed complex-IQ FFT (rustfft), Hann apodization,
// power-spectrum averaging, hysteresis squelch. The VFO bank
// runs multiple virtual channels inside one captured I/Q span — the same
// ergonomics used across the SDR scanner category, implemented originally.

use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rustfft::{FftPlanner};
use rustfft::num_complex::Complex;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

use crate::adsb::AdsbDecoder;
use crate::audio::AudioSink;
use crate::auto_decode;
use crate::capture::{CaptureWorker, IqNetworkSink, IqRing};
use crate::config::{ScanRange, ScannerConfig};
use crate::demod::{decimate_average, decimate_complex_average, demodulate, low_pass_complex, mix_down, resample_linear, Mode};
use crate::device::DeviceLayer;
use crate::db::Db;
use crate::signal_id;
use crate::state::{RecordingState, ScannerEvent};
use crate::sidecar::SidecarRegistry;

/// Handle shared between the API and the UI. Cloning is cheap.
#[derive(Clone)]
pub struct ScannerHandle {
    pub cmd_tx: mpsc::UnboundedSender<ScannerCommand>,
    pub state: Arc<Mutex<ScannerRuntimeState>>,
    pub iq_consumers: Vec<IqRing>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScannerRuntimeState {
    pub active_range: Option<String>,
    pub running: bool,
    pub vfo_states: Vec<VfoState>,
    pub latest_spectrum: Vec<f32>,
    pub frames_processed: u64,
    /// Backend capture timestamp for the currently retained FFT frame.
    pub latest_spectrum_ms: i64,
    pub scan_locked: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VfoState {
    pub id: u32,
    pub frequency_hz: u64,
    pub mode: String,
    pub muted: bool,
    pub volume: f32,
    pub audio_agc: bool,
    pub squelch_open: bool,
    pub strength_db: f32,
    pub audio_level_db: f32,
    /// scan = hopping search head, signal = parked on a detected emitter, idle = free slot
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub decoder: String,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub candidates: Vec<crate::signal_id::ProtocolCandidate>,
    /// When true the scanner will not auto-reassign this VFO.
    #[serde(default)]
    pub locked: bool,
    /// ARRL band-plan label when assigned from a detected signal.
    #[serde(default)]
    pub segment_label: String,
}

impl Default for VfoState {
    fn default() -> Self {
        Self {
            id: 0,
            frequency_hz: 0,
            mode: "nfm".into(),
            muted: true,
            volume: 0.7,
            audio_agc: true,
            squelch_open: false,
            strength_db: -120.0,
            audio_level_db: -120.0,
            role: "idle".into(),
            protocol: String::new(),
            family: String::new(),
            decoder: String::new(),
            confidence: 0.0,
            candidates: Vec::new(),
            locked: false,
            segment_label: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrunkingState {
    pub system: String,
    pub active_talkgroup: Option<String>,
    pub control_channel_hz: u64,
    pub voice_channels: Vec<u64>,
}

pub enum ScannerCommand {
    Start { range: ScanRange },
    Stop,
    SetVfoFrequency { id: u32, frequency_hz: u64 },
    SetVfoMode { id: u32, mode: String },
    SetVfoMute { id: u32, muted: bool },
    SetVfoVolume { id: u32, volume: f32 },
    ToggleVfoAgc { id: u32, on: bool },
    SetVfoLocked { id: u32, locked: bool },
    Shutdown,
}

/// Dedicated audio consumer; keeps demodulation and CPAL feeding off the FFT loop.
const AUDIO_IQ_CHUNK: usize = 131_072;
struct AudioWorker { stop: Arc<AtomicBool>, thread: Option<std::thread::JoinHandle<()>> }
impl AudioWorker {
    fn start(ring: IqRing, audio: Arc<AudioSink>, device: Arc<DeviceLayer>, state: Arc<Mutex<ScannerRuntimeState>>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut previous = Vec::<Option<Complex<f32>>>::new();
            let mut phases = Vec::<f64>::new();
            let mut filter_states = Vec::<Complex<f32>>::new();
            while !stop_thread.load(Ordering::Acquire) {
                let Some(iq) = ring.take_exact(AUDIO_IQ_CHUNK) else { std::thread::sleep(Duration::from_millis(1)); continue; };
                let vfos = state.lock().vfo_states.clone();
                if previous.len() != vfos.len() { previous = vec![None; vfos.len()]; phases = vec![0.0; vfos.len()]; filter_states = vec![Complex::new(0.0, 0.0); vfos.len()]; }
                let sample_rate = device.status().sample_rate.max(1);
                let predecimation = (sample_rate / 500_000).max(1) as usize;
                let iq = decimate_complex_average(&iq, predecimation);
                let effective_rate = (sample_rate / predecimation as u32).max(1);
                let decimation = (effective_rate / audio.sample_rate().max(1)).max(1) as usize;
                let mut mixed = Vec::<f32>::new();
                let mut active = 0usize;
                for (idx, vfo) in vfos.iter().enumerate() {
                    if vfo.muted { continue; }
                    let offset = vfo.frequency_hz as f64 - device.status().center_freq_hz as f64;
                    let baseband = mix_down(&iq, offset, effective_rate, &mut phases[idx]);
                    let mode = Mode::parse(&vfo.mode);
                    let cutoff_hz = match mode { Mode::Wfm => 100_000.0, Mode::Nfm => 12_500.0, _ => 5_000.0 };
                    let baseband = low_pass_complex(&baseband, cutoff_hz, effective_rate, &mut filter_states[idx]);
                    let mut pcm = demodulate(mode, &baseband, &mut previous[idx]);
                    if vfo.audio_agc {
                        apply_audio_agc(&mut pcm);
                    }
                    let pcm = decimate_average(&pcm, decimation);
                    if mixed.len() < pcm.len() { mixed.resize(pcm.len(), 0.0); }
                    let rms = (pcm.iter().map(|v| v * v).sum::<f32>() / pcm.len().max(1) as f32).sqrt();
                    if let Some(current) = state.lock().vfo_states.iter_mut().find(|x| x.id == vfo.id) { current.audio_level_db = 20.0 * rms.max(1e-6).log10() + 60.0; }
                    for (dst, sample) in mixed.iter_mut().zip(pcm.iter()) { *dst += *sample * vfo.volume; }
                    active += 1;
                }
                if active == 0 { continue; }
                if active > 1 { for sample in &mut mixed { *sample /= active as f32; } }
                let decimated_rate = (effective_rate / decimation as u32).max(1);
                let output = resample_linear(&mixed, decimated_rate, audio.sample_rate());
                audio.push(&output, 1.0);
            }
        });
        Self { stop, thread: Some(thread) }
    }
}
impl Drop for AudioWorker {
    fn drop(&mut self) { self.stop.store(true, Ordering::Release); if let Some(thread) = self.thread.take() { let _ = thread.join(); } }
}

fn apply_audio_agc(samples: &mut [f32]) {
    if samples.is_empty() {
        return;
    }
    let rms = (samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32).sqrt();
    if rms > 1e-5 {
        let gain = (0.18 / rms).min(10.0);
        for sample in samples.iter_mut() {
            *sample *= gain;
        }
    }
}

fn build_scan_grid(range: &ScanRange, step_hz: u32) -> Vec<u64> {
    let step = step_hz.max(1) as u64;
    let mut freqs = Vec::new();
    let mut f = range.start_hz;
    while f <= range.end_hz {
        freqs.push(f);
        f = f.saturating_add(step);
    }
    if freqs.is_empty() {
        freqs.push(range.start_hz);
    }
    freqs
}

fn spread_vfo_frequencies(vfos: &mut [VfoState], center_hz: u64, sample_rate: u32) {
    if vfos.is_empty() || sample_rate == 0 {
        return;
    }
    let half = sample_rate as u64 / 2;
    let low = center_hz.saturating_sub(half);
    let span = sample_rate as u64;
    let n = vfos.len();
    for (i, vfo) in vfos.iter_mut().enumerate() {
        let frac = (i + 1) as f64 / (n + 1) as f64;
        vfo.frequency_hz = low + (span as f64 * frac) as u64;
    }
}

fn vfo_strength_db(bins: &[f32], vfo_freq_hz: u64, center_hz: u64, sample_rate: u32) -> f32 {
    if bins.is_empty() || sample_rate == 0 {
        return -120.0;
    }
    let offset = vfo_freq_hz as f64 - center_hz as f64;
    let normalized = offset / sample_rate as f64 + 0.5;
    let bin = ((normalized * bins.len() as f64).round() as isize)
        .clamp(0, bins.len() as isize - 1) as usize;
    let start = bin.saturating_sub(2);
    let end = (bin + 3).min(bins.len());
    bins[start..end]
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max)
}

fn recenter_device_for_frequency(device: &DeviceLayer, frequency_hz: u64) {
    if let Err(error) = device.set_frequency(frequency_hz) {
        tracing::warn!(%error, frequency_hz, "failed to recenter device for VFO hop");
    }
}

fn band_fits_capture_window(range: &ScanRange, sample_rate: u32) -> bool {
    range.end_hz.saturating_sub(range.start_hz) <= sample_rate as u64
}

fn frequency_from_bin(bin: usize, bin_count: usize, center_hz: u64, sample_rate: u32) -> u64 {
    if bin_count == 0 || sample_rate == 0 {
        return center_hz;
    }
    let offset = bin as f64 / bin_count as f64 - 0.5;
    (center_hz as f64 + offset * sample_rate as f64).max(0.0) as u64
}

struct DetectedPeak {
    bin: usize,
    strength_db: f32,
    snr_db: f32,
    frequency_hz: u64,
}

fn find_signal_peaks(
    bins: &[f32],
    noise_floor: f32,
    squelch_db: f32,
    min_bins: usize,
    center_hz: u64,
    sample_rate: u32,
    range: &ScanRange,
) -> Vec<DetectedPeak> {
    let threshold = noise_floor + squelch_db;
    let mut peaks = Vec::new();
    let mut i = 0;
    while i < bins.len() {
        if bins[i] < threshold {
            i += 1;
            continue;
        }
        let start = i;
        while i < bins.len() && bins[i] >= threshold {
            i += 1;
        }
        let end = i;
        if end - start < min_bins {
            continue;
        }
        let mut best_bin = start;
        let mut best_val = bins[start];
        for (j, val) in bins[start..end].iter().enumerate() {
            if *val > best_val {
                best_val = *val;
                best_bin = start + j;
            }
        }
        let frequency_hz = frequency_from_bin(best_bin, bins.len(), center_hz, sample_rate);
        if frequency_hz < range.start_hz || frequency_hz > range.end_hz {
            continue;
        }
        peaks.push(DetectedPeak {
            bin: best_bin,
            strength_db: best_val,
            snr_db: best_val - noise_floor,
            frequency_hz,
        });
    }
    peaks.sort_by(|a, b| b.strength_db.partial_cmp(&a.strength_db).unwrap_or(std::cmp::Ordering::Equal));
    peaks
}

fn classify_and_decode_peak(
    iq: &[Complex<f32>],
    peak: &DetectedPeak,
    channel_bw: u32,
    mode: &str,
    range_name: &str,
    cfg: &ScannerConfig,
    db: &Db,
    events_tx: &broadcast::Sender<ScannerEvent>,
    device_center_hz: u64,
    sample_rate: u32,
) -> crate::signal_id::Classification {
    let demod_mode = if cfg.use_arrl_bandplan {
        crate::arrl_bandplan::recommended_mode(peak.frequency_hz, mode)
    } else {
        mode
    };
    let demod_rate = sample_rate.min(500_000).max(8_000);
    let demod_audio = auto_decode::extract_demod_audio(
        iq,
        device_center_hz,
        peak.frequency_hz,
        sample_rate,
        demod_rate,
        demod_mode,
    );
    let classification = signal_id::classify(
        peak.frequency_hz,
        channel_bw,
        demod_mode,
        range_name,
        peak.snr_db,
        Some((&demod_audio, demod_rate as f32)),
    );
    if cfg.auto_decode_all {
        let timestamp = now_ms();
        for decoded in auto_decode::try_decode_signal(
            iq,
            device_center_hz,
            sample_rate,
            peak.frequency_hz,
            channel_bw,
            demod_mode,
            range_name,
            peak.snr_db,
            cfg.auto_decode_threshold,
            timestamp,
            cfg.use_arrl_bandplan,
        ) {
            let _ = db.insert_decoded_message(&decoded);
            let _ = events_tx.send(ScannerEvent::DecodedMessage(decoded));
        }
    }
    classification
}

fn assign_peaks_to_vfos(
    vfos: &mut [VfoState],
    peaks: &[DetectedPeak],
    range: &ScanRange,
    iq: &[Complex<f32>],
    cfg: &ScannerConfig,
    db: &Db,
    events_tx: &broadcast::Sender<ScannerEvent>,
    device_center_hz: u64,
    sample_rate: u32,
    range_name: &str,
    wideband: bool,
) {
    let step = cfg.freq_step_hz.max(1) as u64;
    let mode = range.mode.as_str();
    let channel_bw = range.channel_bw_hz;

    // Release idle slots when the signal has been gone for a while.
    for vfo in vfos.iter_mut() {
        if vfo.locked {
            continue;
        }
        if vfo.role == "signal" && !vfo.squelch_open && vfo.audio_level_db < -42.0 {
            vfo.role = "idle".into();
            vfo.protocol.clear();
            vfo.family.clear();
            vfo.decoder.clear();
            vfo.confidence = 0.0;
            vfo.candidates.clear();
            vfo.segment_label.clear();
        }
    }

    let mut peak_idx = 0;
    for vfo in vfos.iter_mut() {
        if vfo.locked {
            continue;
        }
        // Narrowband: VFO 0 is the hopper — monitor VFOs are id >= 1.
        if !wideband && vfo.id == 0 {
            continue;
        }
        // Keep a parked signal until it drops unless the slot is idle.
        if vfo.role == "signal" && vfo.squelch_open {
            continue;
        }
        while peak_idx < peaks.len() {
            let peak = &peaks[peak_idx];
            peak_idx += 1;
            let dup = vfos.iter().any(|v| {
                v.frequency_hz.abs_diff(peak.frequency_hz) < step / 2 && v.role == "signal"
            });
            if dup {
                continue;
            }
            let classification = classify_and_decode_peak(
                iq,
                peak,
                channel_bw,
                mode,
                range_name,
                cfg,
                db,
                events_tx,
                device_center_hz,
                sample_rate,
            );
            let top = classification.candidates.first();
            vfo.frequency_hz = peak.frequency_hz;
            vfo.role = "signal".into();
            vfo.squelch_open = true;
            let demod_mode = if cfg.use_arrl_bandplan {
                crate::arrl_bandplan::recommended_mode(peak.frequency_hz, mode)
            } else {
                mode
            };
            vfo.mode = demod_mode.to_string();
            if let Some(seg) = crate::arrl_bandplan::segment_at(peak.frequency_hz) {
                vfo.segment_label = seg.label.to_string();
            } else {
                vfo.segment_label.clear();
            }
            vfo.protocol = classification.sub_protocol.clone();
            vfo.family = classification.top_family.clone();
            vfo.confidence = classification.top_confidence;
            vfo.decoder = top
                .map(|c| c.decoder.clone())
                .unwrap_or_else(|| "none".into());
            vfo.candidates = classification.candidates.clone();
            let _ = db.insert_classified_signal_event(
                peak.frequency_hz,
                peak.snr_db,
                channel_bw,
                range_name,
                now_ms(),
                &classification.signal_class,
                &classification.top_family,
                classification.top_confidence,
                &classification.sub_protocol,
                classification.decode_success,
                &classification.decode_protocol,
                &classification.decode_summary,
                classification.likely_proprietary,
                classification.is_novel,
            );
            break;
        }
    }
}


impl ScannerHandle {
    pub fn spawn(
        cfg: ScannerConfig,
        device: Arc<DeviceLayer>,
        db: Db,
        recording: Arc<Mutex<RecordingState>>,
        playback: Arc<Mutex<Option<crate::capture::PlaybackReader>>>,
        audio: Arc<AudioSink>,
        iq_network: IqNetworkSink,
        sidecars: SidecarRegistry,
        events_tx: broadcast::Sender<ScannerEvent>,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(ScannerRuntimeState::default()));
        let capture_ring = IqRing::new("fft", cfg.fft_size.saturating_mul(5).saturating_mul(16));
        let audio_ring = IqRing::new("audio", 2_000_000);
        let handle = ScannerHandle { cmd_tx: cmd_tx.clone(), state: state.clone(), iq_consumers: vec![capture_ring.clone(), audio_ring.clone()] };

        tokio::spawn(scanner_loop(cfg, device, db, recording, playback, audio, iq_network, sidecars, events_tx, cmd_rx, state, capture_ring, audio_ring));
        handle
    }
}

/// Main scanner task — processes commands, runs the FFT loop, emits events.
async fn scanner_loop(
    cfg: ScannerConfig,
    device: Arc<DeviceLayer>,
    db: Db,
    recording: Arc<Mutex<RecordingState>>,
    playback: Arc<Mutex<Option<crate::capture::PlaybackReader>>>,
    audio: Arc<AudioSink>,
    iq_network: IqNetworkSink,
    sidecars: SidecarRegistry,
    events_tx: broadcast::Sender<ScannerEvent>,
    mut cmd_rx: mpsc::UnboundedReceiver<ScannerCommand>,
    state: Arc<Mutex<ScannerRuntimeState>>,
    capture_ring: IqRing,
    audio_ring: IqRing,
) {
    let mut active_range: Option<ScanRange> = None;
    let poll = Duration::from_micros((1_000_000.0 / cfg.update_rate_hz.max(1.0)) as u64);

    // Complex FFT preserves both sides of the SDR's IQ spectrum.
    let mut fft_planner = FftPlanner::<f32>::new();
    let fft = fft_planner.plan_fft_forward(cfg.fft_size);
    let mut spectrum: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); cfg.fft_size];
    let capture_size = cfg.fft_size.saturating_mul(5);
    let _audio_worker = AudioWorker::start(audio_ring.clone(), audio.clone(), device.clone(), state.clone());
    let _capture_worker = CaptureWorker::start(device.clone(), vec![capture_ring.clone(), audio_ring], cfg.fft_size, playback, iq_network);

    // Simple window coefficients (Hanning) — apodize 1.0 exposes `hanning_iter`.
    let window: Vec<f32> = apodize::hanning_iter(cfg.fft_size).map(|x| x as f32).collect();
    let mut last_signal_hit = Instant::now() - Duration::from_secs(2);
    let mut smoothed_noise_floor: Option<f32> = None;
    let mut native_adsb = AdsbDecoder::new(device.status().sample_rate);
    let mut scan_grid: Vec<u64> = Vec::new();
    let mut scan_index: usize = 0;
    let mut last_vfo_hop = Instant::now();

    loop {
        // Drain commands
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                ScannerCommand::Start { range } => {
                    let name = range.name.clone();
                    let center = range.start_hz + range.end_hz.saturating_sub(range.start_hz) / 2;
                    recenter_device_for_frequency(&device, center);
                    let wideband = band_fits_capture_window(&range, device.status().sample_rate);
                    let mut vfos = (0..range.max_vfos.min(cfg.max_vfos as u32))
                        .map(|i| VfoState {
                            id: i,
                            frequency_hz: range.start_hz,
                            mode: range.mode.clone(),
                            muted: true,
                            volume: 0.7,
                            audio_agc: true,
                            squelch_open: false,
                            strength_db: -120.0,
                            audio_level_db: -120.0,
                            role: if i == 0 && !wideband {
                                "scan".into()
                            } else {
                                "idle".into()
                            },
                            protocol: String::new(),
                            family: String::new(),
                            decoder: String::new(),
                            confidence: 0.0,
                            candidates: Vec::new(),
                            locked: false,
                            segment_label: String::new(),
                        })
                        .collect::<Vec<_>>();
                    spread_vfo_frequencies(
                        &mut vfos,
                        device.status().center_freq_hz,
                        device.status().sample_rate,
                    );
                    scan_grid = build_scan_grid(&range, cfg.freq_step_hz);
                    scan_index = 0;
                    last_vfo_hop = Instant::now();
                    if !vfos.is_empty() && !scan_grid.is_empty() {
                        vfos[0].frequency_hz = scan_grid[0];
                    }
                    state.lock().vfo_states = vfos.clone();
                    state.lock().active_range = Some(name.clone());
                    state.lock().running = true;
                    active_range = Some(range);
                    let _ = events_tx.send(ScannerEvent::VfoStates(vfos));
                    tracing::info!(range = %name, vfo_count = vfos.len(), "scanner started");
                }
                ScannerCommand::Stop => {
                    state.lock().running = false;
                    state.lock().active_range = None;
                    state.lock().vfo_states.clear();
                    active_range = None;
                    scan_grid.clear();
                    scan_index = 0;
                    let _ = events_tx.send(ScannerEvent::VfoStates(Vec::new()));
                    tracing::info!("scanner stopped");
                }
                ScannerCommand::SetVfoFrequency { id, frequency_hz } => {
                    if let Some(v) = state.lock().vfo_states.iter_mut().find(|v| v.id == id) {
                        v.frequency_hz = frequency_hz;
                        let center = device.status().center_freq_hz;
                        let half = device.status().sample_rate / 2;
                        if frequency_hz < center.saturating_sub(half as u64)
                            || frequency_hz > center + half as u64
                        {
                            recenter_device_for_frequency(&device, frequency_hz);
                        }
                    }
                    let vfos = state.lock().vfo_states.clone();
                    let _ = events_tx.send(ScannerEvent::VfoStates(vfos));
                }
                ScannerCommand::SetVfoMode { id, mode } => {
                    if let Some(v) = state.lock().vfo_states.iter_mut().find(|v| v.id == id) {
                        v.mode = mode;
                    }
                    let vfos = state.lock().vfo_states.clone();
                    let _ = events_tx.send(ScannerEvent::VfoStates(vfos));
                }
                ScannerCommand::SetVfoMute { id, muted } => {
                    if let Some(v) = state.lock().vfo_states.iter_mut().find(|v| v.id == id) {
                        v.muted = muted;
                    }
                    let vfos = state.lock().vfo_states.clone();
                    let _ = events_tx.send(ScannerEvent::VfoStates(vfos));
                }
                ScannerCommand::SetVfoVolume { id, volume } => {
                    if let Some(v) = state.lock().vfo_states.iter_mut().find(|v| v.id == id) {
                        v.volume = volume.clamp(0.0, 1.0);
                    }
                    let vfos = state.lock().vfo_states.clone();
                    let _ = events_tx.send(ScannerEvent::VfoStates(vfos));
                }
                ScannerCommand::ToggleVfoAgc { id, on } => {
                    if let Some(v) = state.lock().vfo_states.iter_mut().find(|v| v.id == id) {
                        v.audio_agc = on;
                    }
                    let vfos = state.lock().vfo_states.clone();
                    let _ = events_tx.send(ScannerEvent::VfoStates(vfos));
                }
                ScannerCommand::SetVfoLocked { id, locked } => {
                    if let Some(v) = state.lock().vfo_states.iter_mut().find(|v| v.id == id) {
                        v.locked = locked;
                        if locked {
                            v.role = "signal".into();
                        }
                    }
                    let vfos = state.lock().vfo_states.clone();
                    let _ = events_tx.send(ScannerEvent::VfoStates(vfos));
                }
                ScannerCommand::Shutdown => return,
            }
        }

        if !state.lock().running || active_range.is_none() {
            tokio::time::sleep(poll).await;
            continue;
        }

        // Narrowband bands: VFO 0 hops the step grid. Wideband (e.g. 40m): full span
        // is visible — peaks are assigned to all VFO slots each FFT frame.
        let wideband = active_range.as_ref().map(|r| band_fits_capture_window(r, device.status().sample_rate)).unwrap_or(false);
        if !wideband {
            if let Some(range) = active_range.as_ref() {
                let dwell = Duration::from_millis(range.dwell_ms.max(50) as u64);
                if last_vfo_hop.elapsed() >= dwell && !scan_grid.is_empty() {
                    let hold = {
                        let st = state.lock();
                        st.vfo_states
                            .iter()
                            .find(|v| v.id == 0)
                            .map(|v| {
                                cfg.scan_hold_on_audio
                                    && !v.muted
                                    && v.audio_level_db > -35.0
                            })
                            .unwrap_or(false)
                    };
                    if !hold {
                        scan_index = (scan_index + 1) % scan_grid.len();
                        let next_freq = scan_grid[scan_index];
                        let center = device.status().center_freq_hz;
                        let half = device.status().sample_rate / 2;
                        if next_freq < center.saturating_sub(half as u64)
                            || next_freq > center + half as u64
                        {
                            recenter_device_for_frequency(&device, next_freq);
                        }
                        if let Some(v0) = state.lock().vfo_states.iter_mut().find(|v| v.id == 0) {
                            v0.frequency_hz = next_freq;
                            v0.role = "scan".into();
                        }
                        last_vfo_hop = Instant::now();
                    }
                }
            }
        }

        // Pull one live frame from the shared device. Hardware backends will
        // replace DeviceLayer::read_iq; the scanner does not care which source
        // produced the samples.
        let iq = loop {
            if let Some(frame) = capture_ring.take_exact(capture_size) { break frame; }
            if !state.lock().running { tokio::time::sleep(Duration::from_millis(2)).await; }
            else { tokio::time::sleep(Duration::from_millis(1)).await; }
        };
        sidecars.feed_iq(&iq).await;
        recording.lock().write_iq(&iq);

        // Native ADS-B path: only activate on an ADS-B range, so ordinary
        // scanner traffic never pays the Mode S preamble scan cost.
        let native_adsb_active = active_range.as_ref()
            .map(|r| r.name.to_ascii_lowercase().contains("ads-b") || r.name.to_ascii_lowercase().contains("adsb"))
            .unwrap_or(false);
        if native_adsb_active {
            native_adsb.feed_iq(&iq);
            for message in native_adsb.take_messages() {
                let content = message.callsign.clone()
                    .or_else(|| message.altitude_ft.map(|a| format!("{a} ft")))
                    .unwrap_or_default();
                let decoded = crate::db::DecodedMessage {
                    id: None,
                    frequency_hz: 1_090_000_000,
                    protocol: "adsb".into(),
                    message_type: message.message_type.clone(),
                    address: message.icao.clone(),
                    function_code: format!("DF{}", message.df),
                    content: content.clone(),
                    raw: message.raw_hex.clone(),
                    encryption: "none".into(),
                    timestamp_ms: now_ms(),
                };
                let _ = db.insert_decoded_message(&decoded);
                let _ = events_tx.send(ScannerEvent::DecodedMessage(decoded));
            }
        }

        // Window complex IQ in-place, then transform and shift DC to the center.
        for ((dst, sample), w) in spectrum.iter_mut().zip(iq.iter()).zip(window.iter()) {
            *dst = *sample * *w;
        }
        fft.process(&mut spectrum);
        let half = spectrum.len() / 2;
        let bins: Vec<f32> = spectrum[half..].iter().chain(spectrum[..half].iter())
            .map(|c| 10.0 * (c.norm_sqr() + 1e-20).log10()).collect();
        let range_name = state.lock().active_range.clone().unwrap_or_default();
        let mut floor_samples = bins.clone();
        floor_samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let floor = floor_samples[floor_samples.len() / 2];
        let noise_floor = *smoothed_noise_floor.get_or_insert(floor);
        smoothed_noise_floor = Some(noise_floor * 0.92 + floor * 0.08);
        if let Some((bin, peak)) = bins.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal)) {
            let snr = *peak - noise_floor;
            if snr >= cfg.squelch_db && last_signal_hit.elapsed() >= Duration::from_secs(1) {
                let status = device.status();
                let offset = bin as f64 / bins.len() as f64 - 0.5;
                let frequency_hz = (status.center_freq_hz as f64 + offset * status.sample_rate as f64).max(0.0) as u64;
                let bandwidth_hz = (status.sample_rate / bins.len().max(1) as u32).max(1);
                // Prefer the active range's channel BW when available (more accurate than FFT bin width)
                let range_mode = active_range.as_ref().map(|r| r.mode.as_str()).unwrap_or("nfm");
                let demod_mode = if cfg.use_arrl_bandplan {
                    crate::arrl_bandplan::recommended_mode(frequency_hz, range_mode)
                } else {
                    range_mode
                };
                let channel_bw = active_range.as_ref().map(|r| r.channel_bw_hz).unwrap_or(bandwidth_hz);
                let demod_rate = status.sample_rate.min(500_000).max(8_000);
                let demod_audio = auto_decode::extract_demod_audio(
                    &iq,
                    status.center_freq_hz,
                    frequency_hz,
                    status.sample_rate,
                    demod_rate,
                    demod_mode,
                );
                let classification = signal_id::classify(
                    frequency_hz,
                    channel_bw,
                    demod_mode,
                    &range_name,
                    snr,
                    Some((&demod_audio, demod_rate as f32)),
                );
                let top = classification.candidates.first();
                let decoder = top.map(|c| c.decoder.clone()).unwrap_or_else(|| "none".into());
                let _ = db.insert_classified_signal_event(
                    frequency_hz,
                    snr,
                    channel_bw,
                    &range_name,
                    now_ms(),
                    &classification.signal_class,
                    &classification.top_family,
                    classification.top_confidence,
                    &classification.sub_protocol,
                    classification.decode_success,
                    &classification.decode_protocol,
                    &classification.decode_summary,
                    classification.likely_proprietary,
                    classification.is_novel,
                );
                if cfg.auto_decode_all {
                    let timestamp = now_ms();
                    for decoded in auto_decode::try_decode_signal(
                        &iq,
                        status.center_freq_hz,
                        status.sample_rate,
                        frequency_hz,
                        channel_bw,
                        demod_mode,
                        &range_name,
                        snr,
                        cfg.auto_decode_threshold,
                        timestamp,
                        cfg.use_arrl_bandplan,
                    ) {
                        let _ = db.insert_decoded_message(&decoded);
                        let _ = events_tx.send(ScannerEvent::DecodedMessage(decoded));
                    }
                }
                let _ = events_tx.send(ScannerEvent::SignalHit {
                    frequency_hz,
                    strength_db: *peak,
                    snr_db: snr,
                    bandwidth_hz: channel_bw,
                    protocol: classification.sub_protocol.clone(),
                    family: classification.top_family.clone(),
                    confidence: classification.top_confidence,
                    decoder,
                });
                last_signal_hit = Instant::now();
            }
        }

        // 5. Broadcast to WS subscribers and retain the same frame for the
        // HTTP `/spectrum` endpoint.
        {
            let mut runtime = state.lock();
            runtime.latest_spectrum = bins.clone();
            runtime.frames_processed = runtime.frames_processed.saturating_add(1);
            runtime.latest_spectrum_ms = now_ms();
            let center = device.status().center_freq_hz;
            let sample_rate = device.status().sample_rate;
            let peak = bins.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            for vfo in runtime.vfo_states.iter_mut() {
                let strength = vfo_strength_db(&bins, vfo.frequency_hz, center, sample_rate);
                vfo.strength_db = strength;
                vfo.squelch_open = strength - noise_floor >= cfg.squelch_db;
            }
            if let Some(range) = active_range.as_ref() {
                let peaks = find_signal_peaks(
                    &bins,
                    noise_floor,
                    cfg.squelch_db,
                    cfg.min_signal_width_bins.max(2),
                    center,
                    sample_rate,
                    range,
                );
                if !peaks.is_empty() {
                    assign_peaks_to_vfos(
                        &mut runtime.vfo_states,
                        &peaks,
                        range,
                        &iq,
                        &cfg,
                        &db,
                        &events_tx,
                        center,
                        sample_rate,
                        &range_name,
                        wideband,
                    );
                }
            }
        }
        let _ = events_tx.send(ScannerEvent::VfoStates(state.lock().vfo_states.clone()));
        let _ = events_tx.send(ScannerEvent::Spectrum { range: range_name, bins });
        let frame_us = ((capture_size as f64 / device.status().sample_rate.max(1) as f64) * 1_000_000.0).max(500.0) as u64;
        tokio::time::sleep(Duration::from_micros(frame_us)).await;
    }
}

#[allow(dead_code)]
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[allow(dead_code)]
pub fn instant_ms(t: Instant) -> i64 {
    t.elapsed().as_millis() as i64
}

#[cfg(all(test, feature = "soapysdr"))]
mod hardware_tests {
    use super::*;
    #[test]
    fn live_rsp1b_scanner_fft_has_dynamic_range() {
        let _hardware_guard = crate::device::LIVE_HARDWARE_LOCK.lock().unwrap();
        let device = DeviceLayer::new_mock();
        let key = DeviceLayer::discover().into_iter().find(|d| d.driver == "sdrplay").expect("RSP1B missing from discovery").key;
        device.connect(&key).expect("connect RSP1B");
        device.set_sample_rate(2_000_000).expect("set rate");
        device.set_frequency(162_550_000).expect("tune");
        let iq=device.read_iq(4096).expect("read live RSP1B IQ"); assert_eq!(iq.len(),4096,"short IQ frame");
        let mut planner=FftPlanner::<f32>::new(); let fft=planner.plan_fft_forward(4096);
        let mut out=iq.clone();
        let window: Vec<f32>=apodize::hanning_iter(4096).map(|x|x as f32).collect();
        for (sample,w) in out.iter_mut().zip(window.iter()) {*sample*=*w;}
        fft.process(&mut out);
        let half=out.len()/2; let bins: Vec<f32>=out[half..].iter().chain(out[..half].iter()).map(|c|10.0*(c.norm_sqr()+1e-20).log10()).collect();
        let min=bins.iter().copied().fold(f32::INFINITY,f32::min); let max=bins.iter().copied().fold(f32::NEG_INFINITY,f32::max);
        assert!(max-min>6.0,"flat/non-live scanner spectrum: min={min} max={max}");
        eprintln!("live RSP1B scanner FFT bins={} min_db={min:.2} max_db={max:.2} span_db={:.2}",bins.len(),max-min);
        device.disconnect().expect("disconnect RSP1B");
    }
}
