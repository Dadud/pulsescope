// scanner.rs — clean-room FFT-based scanner core.
//
// No code from any proprietary scanner is used here. The DSP is built from
// public primitives: windowed complex-IQ FFT (rustfft), Hann apodization,
// power-spectrum averaging, hysteresis squelch. The VFO bank
// runs multiple virtual channels inside one captured I/Q span — the same
// ergonomics used across the SDR scanner category, implemented originally.

use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, watch};

use crate::adsb::AdsbDecoder;
use crate::audio::AudioSink;
use crate::capture::{CaptureWorker, IqNetworkSink, IqRing};
use crate::config::{ScanRange, ScannerConfig};
use crate::db::Db;
use crate::demod::{
    channelize_iq, dc_block, decimate_complex_average, decode_wfm_stereo, deemphasis, demodulate,
    discriminator_samples, low_pass_complex, low_pass_real, mix_down, Mode, SincResampler,
    WfmStereoState,
};
use crate::device::DeviceLayer;
use crate::sidecar::SidecarRegistry;
use crate::signal_id;
use crate::state::{RecordingState, ScannerEvent, SpectrumFrame};

/// Handle shared between the API and the UI. Cloning is cheap.
#[derive(Clone)]
pub struct ScannerHandle {
    pub cmd_tx: mpsc::UnboundedSender<ScannerCommand>,
    pub state: Arc<Mutex<ScannerRuntimeState>>,
    pub iq_consumers: Vec<IqRing>,
    task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

#[derive(Clone)]
pub struct ScannerDependencies {
    pub device: Arc<DeviceLayer>,
    pub db: Db,
    pub recording: Arc<Mutex<RecordingState>>,
    pub playback: Arc<Mutex<Option<crate::capture::PlaybackReader>>>,
    pub audio: Arc<AudioSink>,
    pub iq_network: IqNetworkSink,
    pub sidecars: SidecarRegistry,
    pub events_tx: broadcast::Sender<ScannerEvent>,
    pub spectrum_tx: watch::Sender<SpectrumFrame>,
    pub wfm_deemphasis_us: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScannerRuntimeState {
    pub active_range: Option<String>,
    pub running: bool,
    pub started_ms: i64,
    pub vfo_states: Vec<VfoState>,
    pub latest_spectrum: Vec<f32>,
    pub frames_processed: u64,
    /// Backend capture timestamp for the currently retained FFT frame.
    pub latest_spectrum_ms: i64,
    pub scan_locked: bool,
    /// True while delay/hang-time is holding the window on a confirmed hit.
    #[serde(default)]
    pub holding: bool,
    /// Smoothed median noise floor for the current FFT frame (dBFS).
    #[serde(default)]
    pub noise_floor_db: f32,
    /// Live WFM de-emphasis applied by the audio worker (µs).
    #[serde(default)]
    pub wfm_deemphasis_us: u32,
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
    /// True while the scanner is holding this VFO on a detected signal (or
    /// the operator has manually parked it).
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub last_hit_ms: i64,
    /// Signal above noise floor in dB (local peak minus smoothed floor).
    #[serde(default)]
    pub snr_db: f32,
    /// Smoothed noise floor used for squelch decisions (dBFS).
    #[serde(default)]
    pub noise_floor_db: f32,
}

/// Prefer the VFO the operator is listening to, then a scanner lock, then VFO 0.
pub fn selected_vfo(vfos: &[VfoState]) -> Option<&VfoState> {
    vfos.iter()
        .find(|vfo| !vfo.muted)
        .or_else(|| vfos.iter().find(|vfo| vfo.locked))
        .or(vfos.first())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrunkingState {
    pub system: String,
    pub active_talkgroup: Option<String>,
    pub control_channel_hz: u64,
    pub voice_channels: Vec<u64>,
}

pub enum ScannerCommand {
    Start {
        range: ScanRange,
        /// Extra banks visited after the current window wraps. Empty means
        /// search only `range`.
        cycle: Vec<ScanRange>,
    },
    Stop,
    /// Skip the current hit and resume. Temporary entries drop on blacklist clear.
    Skip {
        temporary: bool,
    },
    /// Release operator hold and resume sweeping after the current delay.
    Resume,
    SetRangeSquelch {
        squelch_db: f32,
    },
    SetVfoFrequency {
        id: u32,
        frequency_hz: u64,
    },
    SetVfoMode {
        id: u32,
        mode: String,
    },
    SetVfoMute {
        id: u32,
        muted: bool,
    },
    SetVfoVolume {
        id: u32,
        volume: f32,
    },
    ToggleVfoAgc {
        id: u32,
        on: bool,
    },
    Shutdown,
}

/// `POST /channels/scan/start` name that visits every `enabled` bank.
pub const SCAN_ENABLED_BANKS: &str = "enabled";
/// `POST /channels/scan/start` name that dwells on saved bookmark channels.
pub const SCAN_BOOKMARKS: &str = "Bookmarks";

#[derive(Clone, Debug, PartialEq)]
pub enum ScanStep {
    Stay,
    Retune(u64),
    SwitchRange(ScanRange),
}

pub fn frequency_is_locked_out(
    frequency_hz: u64,
    entries: &[crate::db::BlacklistEntry],
    channel_bw_hz: u32,
) -> bool {
    let radius = (channel_bw_hz as u64 / 2).max(1);
    entries
        .iter()
        .any(|entry| entry.frequency_hz.abs_diff(frequency_hz) <= radius)
}

pub fn peak_is_dc_rejected(
    detected_hz: u64,
    center_hz: u64,
    sample_rate_hz: u32,
    fft_size: usize,
    dc_reject_hz: u32,
) -> bool {
    let bin_hz = (sample_rate_hz as f64 / fft_size.max(1) as f64).max(1.0);
    let reject_hz = (dc_reject_hz as f64).max(bin_hz);
    (detected_hz.abs_diff(center_hz) as f64) < reject_hz
}

pub fn candidate_is_confirmed(
    candidate_hz: Option<u64>,
    detected_hz: u64,
    channel_bw_hz: u32,
    present_for: Duration,
    confirm_ms: u64,
) -> bool {
    let radius = (channel_bw_hz as u64 / 2).max(1);
    let Some(candidate) = candidate_hz else {
        return confirm_ms == 0;
    };
    if candidate.abs_diff(detected_hz) > radius {
        return false;
    }
    present_for >= Duration::from_millis(confirm_ms)
}

pub fn next_cycle_range<'a>(current: &str, cycle: &'a [ScanRange]) -> Option<&'a ScanRange> {
    if cycle.len() < 2 {
        return None;
    }
    let index = cycle.iter().position(|range| range.name == current)?;
    Some(&cycle[(index + 1) % cycle.len()])
}

fn search_window_wraps(range: &ScanRange, center_hz: u64, usable_span_hz: u64) -> bool {
    let half = usable_span_hz / 2;
    center_hz
        .saturating_add(usable_span_hz)
        .saturating_add(half)
        > range.end_hz
}

/// Conventional search hop, or the next enabled bank / bookmark when the
/// current span is finished. Delay/hold is applied by the caller.
pub fn next_scan_step(
    range: &ScanRange,
    cycle: &[ScanRange],
    center_hz: u64,
    usable_span_hz: u64,
) -> ScanStep {
    let usable = usable_span_hz.max(1);
    let span = range.end_hz.saturating_sub(range.start_hz);
    let wraps = search_window_wraps(range, center_hz, usable);
    if span <= usable || wraps {
        if let Some(next) = next_cycle_range(&range.name, cycle) {
            return ScanStep::SwitchRange(next.clone());
        }
        if wraps && span > usable {
            return ScanStep::Retune(initial_scan_center(range, usable as u32));
        }
        return ScanStep::Stay;
    }
    ScanStep::Retune(center_hz.saturating_add(usable))
}

fn settle_dwell(range: &ScanRange) -> Duration {
    Duration::from_millis(range.dwell_ms.max(750) as u64)
}

fn bookmark_lockout_entry(
    frequency_hz: u64,
    temporary: bool,
    reason: &str,
) -> crate::db::BlacklistEntry {
    crate::db::BlacklistEntry {
        frequency_hz,
        reason: reason.into(),
        temporary,
        created_ms: now_ms(),
    }
}

fn starter_vfos(range: &ScanRange, center_hz: u64, max_vfos: usize) -> Vec<VfoState> {
    if range.max_vfos == 0 || max_vfos == 0 {
        return Vec::new();
    }
    vec![VfoState {
        id: 0,
        frequency_hz: center_hz,
        mode: range.mode.clone(),
        // Monitoring is opt-in. The SSTV automation is the
        // narrow exception: it needs post-demod audio, but
        // it remains silent until a browser subscribes.
        muted: !(range.name.starts_with("SSTV ")
            || range.name.starts_with("FT8 ")
            || range.name.starts_with("WSPR ")
            || range.name.starts_with("RTTY ")
            || range.name.starts_with("NAVTEX ")
            || range.name.starts_with("CW ")),
        volume: 0.7,
        audio_agc: true,
        squelch_open: false,
        strength_db: -120.0,
        audio_level_db: -120.0,
        locked: false,
        last_hit_ms: 0,
        snr_db: 0.0,
        noise_floor_db: -120.0,
    }]
}

pub fn initial_scan_center(range: &ScanRange, usable_span_hz: u32) -> u64 {
    range
        .start_hz
        .saturating_add(usable_span_hz as u64 / 2)
        .min(range.end_hz)
}

/// Convert a noisy FFT-bin peak into a stable channel frequency. Broadcast FM
/// channels are centered half a channel above the stored lower band edge
/// (88.1, 88.3, ... MHz in the North American preset). Other services use the
/// configured scanner raster while staying inside the selected range.
fn stable_channel_frequency(range: &ScanRange, detected_hz: u64, fallback_step_hz: u32) -> u64 {
    let (origin, step) = if range.mode.eq_ignore_ascii_case("wfm") && range.channel_bw_hz > 0 {
        (
            range
                .start_hz
                .saturating_add(range.channel_bw_hz as u64 / 2),
            range.channel_bw_hz as u64,
        )
    } else {
        (range.start_hz, fallback_step_hz.max(1) as u64)
    };
    let steps = detected_hz.saturating_sub(origin).saturating_add(step / 2) / step;
    origin
        .saturating_add(steps.saturating_mul(step))
        .clamp(range.start_hz, range.end_hz)
}

pub fn occupancy_from_spectrum(
    bins: &[f32],
    center_hz: u64,
    sample_rate_hz: u32,
    noise_floor_db: f32,
    now_ms: i64,
) -> Vec<crate::db::SpectrumOccupancy> {
    if bins.is_empty() || sample_rate_hz == 0 {
        return Vec::new();
    }
    let bucket_count = bins.len().clamp(1, 64);
    let chunk = (bins.len() / bucket_count).max(1);
    let start_hz = center_hz.saturating_sub(sample_rate_hz as u64 / 2);
    bins.chunks(chunk)
        .enumerate()
        .map(|(index, chunk_bins)| {
            let avg = chunk_bins.iter().sum::<f32>() / chunk_bins.len() as f32;
            let peak = chunk_bins.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let freq = start_hz.saturating_add(
                (sample_rate_hz as u64).saturating_mul(index as u64) / bucket_count as u64,
            );
            crate::db::SpectrumOccupancy {
                frequency_bucket_hz: freq,
                time_bucket_15min: now_ms / 900_000,
                avg_power_db: avg,
                peak_power_db: peak,
                avg_above_floor_db: avg - noise_floor_db,
                sample_count: chunk_bins.len() as i64,
                noise_floor_db,
            }
        })
        .collect()
}

pub fn occupancy_fraction(row: &crate::db::SpectrumOccupancy) -> f32 {
    ((row.avg_power_db - row.noise_floor_db) / 40.0).clamp(0.0, 1.0)
}

/// Dedicated audio consumer; keeps demodulation and CPAL feeding off the FFT loop.
struct AudioWorker {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl AudioWorker {
    fn start(
        ring: IqRing,
        audio: Arc<AudioSink>,
        device: Arc<DeviceLayer>,
        state: Arc<Mutex<ScannerRuntimeState>>,
        wfm_deemphasis_us: u32,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut previous = Vec::<Option<Complex<f32>>>::new();
            let mut phases = Vec::<f64>::new();
            let mut filter_states = Vec::<Complex<f32>>::new();
            let mut audio_filter_states = Vec::<f32>::new();
            let mut deemphasis_states = Vec::<f32>::new();
            let mut dc_states = Vec::<f32>::new();
            let mut agc_gains = Vec::<f32>::new();
            let mut sam_phases = Vec::<f64>::new();
            let mut cw_phases = Vec::<f64>::new();
            let mut resampler = SincResampler::default();
            let mut stereo_left_resampler = SincResampler::default();
            let mut stereo_right_resampler = SincResampler::default();
            let mut stereo_states = Vec::<WfmStereoState>::new();
            while !stop_thread.load(Ordering::Acquire) {
                // Consume approximately 20 ms per DSP block at every device
                // rate. The former fixed 131072-sample block added more than
                // half a second of latency at 250 kS/s.
                let sample_rate = device.status().sample_rate.max(1);
                let audio_iq_chunk = ((sample_rate as usize) / 50).clamp(1_024, 262_144);
                let Some(iq) = ring.take_exact(audio_iq_chunk) else {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                };
                let (vfos, deemph_us) = {
                    let runtime = state.lock();
                    let us = if runtime.wfm_deemphasis_us == 0 {
                        wfm_deemphasis_us
                    } else {
                        runtime.wfm_deemphasis_us
                    };
                    (runtime.vfo_states.clone(), us)
                };
                if previous.len() != vfos.len() {
                    previous = vec![None; vfos.len()];
                    phases = vec![0.0; vfos.len()];
                    filter_states = vec![Complex::new(0.0, 0.0); vfos.len()];
                    audio_filter_states = vec![0.0; vfos.len()];
                    deemphasis_states = vec![0.0; vfos.len()];
                    dc_states = vec![0.0; vfos.len()];
                    agc_gains = vec![1.0; vfos.len()];
                    sam_phases = vec![0.0; vfos.len()];
                    cw_phases = vec![0.0; vfos.len()];
                    stereo_states = vec![WfmStereoState::default(); vfos.len()];
                }
                let predecimation = (sample_rate / 500_000).max(1) as usize;
                let effective_rate = (sample_rate / predecimation as u32).max(1);
                let mut mixed = Vec::<f32>::new();
                let mut active = 0usize;
                let mut stereo_candidate: Option<Vec<[f32; 2]>> = None;
                let mut stereo_gain = 1.0f32;
                for (idx, vfo) in vfos.iter().enumerate() {
                    if vfo.muted {
                        continue;
                    }
                    let offset = vfo.frequency_hz as f64 - device.status().center_freq_hz as f64;
                    // Translate before reducing the wide RF sample rate. Doing
                    // this in the opposite order aliases offset VFOs.
                    let baseband = mix_down(&iq, offset, sample_rate, &mut phases[idx]);
                    let mode = Mode::parse(&vfo.mode);
                    let cutoff_hz = match mode {
                        Mode::Wfm => 100_000.0,
                        Mode::Nfm => 12_500.0,
                        Mode::Cw => 800.0,
                        _ => 5_000.0,
                    };
                    let baseband = low_pass_complex(
                        &baseband,
                        cutoff_hz,
                        sample_rate,
                        &mut filter_states[idx],
                    );
                    let baseband = decimate_complex_average(&baseband, predecimation);
                    let multiplex = match mode {
                        Mode::Sam => crate::demod::demodulate_sam(&baseband, &mut sam_phases[idx]),
                        Mode::Cw => crate::demod::demodulate_cw(
                            &baseband,
                            effective_rate,
                            700.0,
                            &mut cw_phases[idx],
                        ),
                        other => demodulate(other, &baseband, &mut previous[idx]),
                    };
                    if mode == Mode::Wfm {
                        stereo_candidate = Some(decode_wfm_stereo(
                            &multiplex,
                            effective_rate,
                            deemph_us as f32,
                            &mut stereo_states[idx],
                        ));
                    }
                    let mut pcm = multiplex;
                    let audio_cutoff_hz = match mode {
                        Mode::Wfm => 15_000.0,
                        Mode::Nfm => 5_000.0,
                        Mode::Cw => 1_200.0,
                        _ => 3_400.0,
                    };
                    pcm = low_pass_real(
                        &pcm,
                        audio_cutoff_hz,
                        effective_rate,
                        &mut audio_filter_states[idx],
                    );
                    if mode == Mode::Wfm {
                        deemphasis(
                            &mut pcm,
                            effective_rate,
                            deemph_us as f32,
                            &mut deemphasis_states[idx],
                        );
                    }
                    dc_block(&mut pcm, &mut dc_states[idx], 0.995);
                    if mixed.len() < pcm.len() {
                        mixed.resize(pcm.len(), 0.0);
                    }
                    let rms =
                        (pcm.iter().map(|v| v * v).sum::<f32>() / pcm.len().max(1) as f32).sqrt();
                    if vfo.audio_agc {
                        let target = (0.16 / rms.max(1e-5)).clamp(0.1, 12.0);
                        let smoothing = if target < agc_gains[idx] { 0.35 } else { 0.04 };
                        agc_gains[idx] += (target - agc_gains[idx]) * smoothing;
                    } else {
                        agc_gains[idx] = 1.0;
                    }
                    if mode == Mode::Wfm {
                        stereo_gain = agc_gains[idx] * vfo.volume;
                    }
                    if let Some(current) =
                        state.lock().vfo_states.iter_mut().find(|x| x.id == vfo.id)
                    {
                        current.audio_level_db = 20.0 * rms.max(1e-6).log10() + 60.0;
                    }
                    for (dst, sample) in mixed.iter_mut().zip(pcm.iter()) {
                        *dst += (*sample * agc_gains[idx]).clamp(-1.0, 1.0) * vfo.volume;
                    }
                    active += 1;
                }
                if active == 0 {
                    continue;
                }
                if active > 1 {
                    for sample in &mut mixed {
                        *sample /= active as f32;
                    }
                }
                if active == 1 {
                    if let Some(stereo) = stereo_candidate.filter(|frames| !frames.is_empty()) {
                        let left = stereo.iter().map(|frame| frame[0]).collect::<Vec<_>>();
                        let right = stereo.iter().map(|frame| frame[1]).collect::<Vec<_>>();
                        let left = stereo_left_resampler.process(
                            &left,
                            effective_rate,
                            audio.sample_rate(),
                        );
                        let right = stereo_right_resampler.process(
                            &right,
                            effective_rate,
                            audio.sample_rate(),
                        );
                        let mut interleaved = Vec::with_capacity(left.len().min(right.len()) * 2);
                        for (left, right) in left.into_iter().zip(right) {
                            interleaved.extend_from_slice(&[left, right]);
                        }
                        audio.push_interleaved(&interleaved, 2, stereo_gain);
                        continue;
                    }
                }
                let output = resampler.process(&mixed, effective_rate, audio.sample_rate());
                audio.push(&output, 1.0);
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for AudioWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl ScannerHandle {
    pub fn flush_iq(&self) {
        for ring in &self.iq_consumers {
            ring.clear();
        }
    }

    /// Copy the newest captured IQ without draining FFT or audio consumers.
    pub fn snapshot_iq(&self, count: usize) -> Option<Vec<Complex<f32>>> {
        self.iq_consumers
            .iter()
            .find(|ring| ring.name() == "snapshot")
            .or_else(|| self.iq_consumers.first())
            .and_then(|ring| ring.copy_latest(count))
    }

    pub fn abort(&self) {
        if let Some(task) = self.task.lock().take() {
            task.abort();
        }
    }

    pub fn spawn(cfg: ScannerConfig, dependencies: ScannerDependencies) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(ScannerRuntimeState::default()));
        // FFT is a latest-frame consumer. Two complete FFT windows absorb
        // driver burstiness without accumulating stale waterfall history.
        let capture_ring =
            IqRing::new_latest("fft", cfg.fft_size.saturating_mul(5).saturating_mul(2));
        let audio_ring = IqRing::new("audio", 2_000_000);
        // ~1 s at 2.4 MS/s. Identify / RDS / CTCSS copy from here instead of
        // calling DeviceLayer::read_iq, which would steal hardware samples.
        let snapshot_ring = IqRing::new_latest("snapshot", 2_400_000);
        let task = tokio::spawn(scanner_loop(
            cfg,
            dependencies,
            cmd_rx,
            state.clone(),
            capture_ring.clone(),
            audio_ring.clone(),
            snapshot_ring.clone(),
        ));

        ScannerHandle {
            cmd_tx: cmd_tx.clone(),
            state,
            iq_consumers: vec![capture_ring, audio_ring, snapshot_ring],
            task: Arc::new(Mutex::new(Some(task))),
        }
    }
}

/// Main scanner task — processes commands, runs the FFT loop, emits events.
async fn scanner_loop(
    cfg: ScannerConfig,
    dependencies: ScannerDependencies,
    mut cmd_rx: mpsc::UnboundedReceiver<ScannerCommand>,
    state: Arc<Mutex<ScannerRuntimeState>>,
    capture_ring: IqRing,
    audio_ring: IqRing,
    snapshot_ring: IqRing,
) {
    let ScannerDependencies {
        device,
        db,
        recording,
        playback,
        audio,
        iq_network,
        sidecars,
        events_tx,
        spectrum_tx,
        wfm_deemphasis_us,
    } = dependencies;
    let mut active_range: Option<ScanRange> = None;
    let poll = Duration::from_micros((1_000_000.0 / cfg.update_rate_hz.max(1.0)) as u64);

    // Complex FFT preserves both sides of the SDR's IQ spectrum.
    let mut fft_planner = FftPlanner::<f32>::new();
    let fft = fft_planner.plan_fft_forward(cfg.fft_size);
    let mut spectrum: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); cfg.fft_size];
    let capture_size = cfg.fft_size.saturating_mul(5);
    let _audio_worker = AudioWorker::start(
        audio_ring.clone(),
        audio.clone(),
        device.clone(),
        state.clone(),
        wfm_deemphasis_us,
    );
    let _capture_worker = CaptureWorker::start(
        device.clone(),
        vec![
            capture_ring.clone(),
            audio_ring.clone(),
            snapshot_ring.clone(),
        ],
        cfg.fft_size,
        playback,
        iq_network,
    );

    // Simple window coefficients (Hanning) — apodize 1.0 exposes `hanning_iter`.
    let window: Vec<f32> = apodize::hanning_iter(cfg.fft_size)
        .map(|x| x as f32)
        .collect();
    let mut smoothed_noise_floor: Option<f32> = None;
    let mut native_decoders = NativeRangeDecoders::new(device.status().sample_rate);
    let mut discriminator_prev = None;
    let mut next_sweep_at = Instant::now();
    let mut signal_hold_started: Option<Instant> = None;
    let mut signal_hold_until: Option<Instant> = None;
    let mut logged_channels = HashSet::<u64>::new();
    let mut cycle_ranges: Vec<ScanRange> = Vec::new();
    let mut candidate_hz: Option<u64> = None;
    let mut candidate_since: Option<Instant> = None;
    let mut blacklist: Vec<crate::db::BlacklistEntry> = Vec::new();
    let mut blacklist_loaded_at = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    let mut force_sweep = false;
    let mut last_occupancy_at = Instant::now()
        .checked_sub(Duration::from_secs(2))
        .unwrap_or_else(Instant::now);
    state.lock().wfm_deemphasis_us = wfm_deemphasis_us;

    loop {
        // Drain commands
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                ScannerCommand::Start { range, cycle } => {
                    // Hardware configuration happens before this command. Do
                    // not process IQ captured under the previous contract.
                    capture_ring.clear();
                    audio_ring.clear();
                    snapshot_ring.clear();
                    audio.clear_queue();
                    let name = range.name.clone();
                    let vfos = starter_vfos(&range, device.status().center_freq_hz, cfg.max_vfos);
                    {
                        let mut runtime = state.lock();
                        runtime.vfo_states = vfos.clone();
                        runtime.active_range = Some(name.clone());
                        runtime.running = true;
                        runtime.scan_locked = false;
                        runtime.holding = false;
                        runtime.started_ms = now_ms();
                        runtime.wfm_deemphasis_us = wfm_deemphasis_us;
                    }
                    next_sweep_at = Instant::now() + settle_dwell(&range);
                    active_range = Some(range);
                    cycle_ranges = cycle;
                    native_decoders.reset(device.status().sample_rate);
                    signal_hold_started = None;
                    signal_hold_until = None;
                    candidate_hz = None;
                    candidate_since = None;
                    logged_channels.clear();
                    let _ = events_tx.send(ScannerEvent::VfoStates(vfos));
                    tracing::info!(range = %name, cycle = cycle_ranges.len(), "scanner started");
                }
                ScannerCommand::SetRangeSquelch { squelch_db } => {
                    if let Some(range) = active_range.as_mut() {
                        range.squelch_db = squelch_db;
                    }
                }
                ScannerCommand::Skip { temporary } => {
                    let frequency_hz = {
                        let runtime = state.lock();
                        runtime
                            .vfo_states
                            .iter()
                            .find(|vfo| vfo.locked)
                            .or_else(|| runtime.vfo_states.first())
                            .map(|vfo| vfo.frequency_hz)
                    };
                    if let Some(frequency_hz) = frequency_hz {
                        let reason = if temporary {
                            "scan skip"
                        } else {
                            "scan lockout"
                        };
                        let _ = db.add_blacklist(&bookmark_lockout_entry(
                            frequency_hz,
                            temporary,
                            reason,
                        ));
                        blacklist = db.list_blacklist().unwrap_or_default();
                        blacklist_loaded_at = Instant::now();
                    }
                    let mut runtime = state.lock();
                    runtime.scan_locked = false;
                    runtime.holding = false;
                    for vfo in &mut runtime.vfo_states {
                        vfo.locked = false;
                        vfo.squelch_open = false;
                    }
                    signal_hold_started = None;
                    signal_hold_until = None;
                    candidate_hz = None;
                    candidate_since = None;
                    next_sweep_at = Instant::now();
                    force_sweep = true;
                }
                ScannerCommand::Resume => {
                    let mut runtime = state.lock();
                    runtime.scan_locked = false;
                    runtime.holding = false;
                    for vfo in &mut runtime.vfo_states {
                        vfo.locked = false;
                    }
                    signal_hold_started = None;
                    signal_hold_until = None;
                    candidate_hz = None;
                    candidate_since = None;
                    next_sweep_at = Instant::now();
                    force_sweep = true;
                }
                ScannerCommand::Stop => {
                    {
                        let mut runtime = state.lock();
                        runtime.running = false;
                        runtime.active_range = None;
                        runtime.vfo_states.clear();
                        runtime.noise_floor_db = -120.0;
                        runtime.holding = false;
                        runtime.scan_locked = false;
                    }
                    active_range = None;
                    cycle_ranges.clear();
                    signal_hold_started = None;
                    signal_hold_until = None;
                    candidate_hz = None;
                    candidate_since = None;
                    logged_channels.clear();
                    let _ = events_tx.send(ScannerEvent::VfoStates(Vec::new()));
                    tracing::info!("scanner stopped");
                }
                ScannerCommand::SetVfoFrequency { id, frequency_hz } => {
                    let mut runtime = state.lock();
                    runtime.scan_locked = true;
                    if let Some(v) = runtime.vfo_states.iter_mut().find(|v| v.id == id) {
                        v.frequency_hz = frequency_hz;
                        v.locked = true;
                        v.last_hit_ms = now_ms();
                    }
                }
                ScannerCommand::SetVfoMode { id, mode } => {
                    if let Some(v) = state.lock().vfo_states.iter_mut().find(|v| v.id == id) {
                        v.mode = mode;
                    }
                }
                ScannerCommand::SetVfoMute { id, muted } => {
                    if let Some(v) = state.lock().vfo_states.iter_mut().find(|v| v.id == id) {
                        v.muted = muted;
                    }
                }
                ScannerCommand::SetVfoVolume { id, volume } => {
                    if let Some(v) = state.lock().vfo_states.iter_mut().find(|v| v.id == id) {
                        v.volume = volume;
                    }
                }
                ScannerCommand::ToggleVfoAgc { id, on } => {
                    if let Some(v) = state.lock().vfo_states.iter_mut().find(|v| v.id == id) {
                        v.audio_agc = on;
                    }
                }
                ScannerCommand::Shutdown => return,
            }
        }

        if !state.lock().running || active_range.is_none() {
            tokio::time::sleep(poll).await;
            continue;
        }

        // Pull one live frame from the shared device. Hardware backends will
        // replace DeviceLayer::read_iq; the scanner does not care which source
        // produced the samples.
        let iq = loop {
            if let Some(frame) = capture_ring.take_latest_exact(capture_size) {
                break frame;
            }
            if !state.lock().running {
                tokio::time::sleep(Duration::from_millis(2)).await;
            } else {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        };
        sidecars.feed_iq(&iq).await;
        let discriminator = discriminator_samples(&iq, &mut discriminator_prev);
        sidecars
            .feed_audio(&discriminator, device.status().sample_rate)
            .await;
        recording.lock().write_iq(&iq);

        if let Some(range) = active_range.as_ref() {
            let tune_hz = selected_vfo(&state.lock().vfo_states)
                .map(|vfo| vfo.frequency_hz)
                .unwrap_or_else(|| device.status().center_freq_hz);
            native_decoders.feed(
                range,
                &iq,
                &discriminator,
                device.status().sample_rate,
                device.status().center_freq_hz,
                tune_hz,
                &db,
                &events_tx,
            );
        }

        // Window complex IQ in-place, then transform and shift DC to the center.
        for ((dst, sample), w) in spectrum.iter_mut().zip(iq.iter()).zip(window.iter()) {
            *dst = *sample * *w;
        }
        fft.process(&mut spectrum);
        let half = spectrum.len() / 2;
        let normalization = window.iter().sum::<f32>().max(1.0);
        let bins: Vec<f32> = spectrum[half..]
            .iter()
            .chain(spectrum[..half].iter())
            .map(|c| 20.0 * (c.norm() / normalization + 1e-12).log10())
            .collect();
        let range_name = state.lock().active_range.clone().unwrap_or_default();
        let mut floor_samples = bins.clone();
        floor_samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let floor = floor_samples[floor_samples.len() / 2];
        let noise_floor = *smoothed_noise_floor.get_or_insert(floor);
        smoothed_noise_floor = Some(noise_floor * 0.92 + floor * 0.08);
        if blacklist_loaded_at.elapsed() >= Duration::from_millis(500) {
            blacklist = db.list_blacklist().unwrap_or_default();
            blacklist_loaded_at = Instant::now();
        }
        if let Some((bin, peak)) = bins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        {
            let snr = *peak - noise_floor;
            let hit_squelch = active_range
                .as_ref()
                .map(|r| r.squelch_db)
                .unwrap_or(cfg.squelch_db);
            if snr >= hit_squelch {
                let status = device.status();
                let offset = bin as f64 / bins.len() as f64 - 0.5;
                let detected_frequency_hz = (status.center_freq_hz as f64
                    + offset * status.sample_rate as f64)
                    .max(0.0) as u64;
                let bandwidth_hz = (status.sample_rate / bins.len().max(1) as u32).max(1);
                // Prefer the active range's channel BW when available (more accurate than FFT bin width)
                let mode = active_range
                    .as_ref()
                    .map(|r| r.mode.as_str())
                    .unwrap_or("nfm");
                let channel_bw = active_range
                    .as_ref()
                    .map(|r| r.channel_bw_hz)
                    .unwrap_or(bandwidth_hz);
                let frequency_hz = active_range
                    .as_ref()
                    .map(|range| {
                        stable_channel_frequency(range, detected_frequency_hz, cfg.freq_step_hz)
                    })
                    .unwrap_or(detected_frequency_hz);
                let dc_rejected = peak_is_dc_rejected(
                    detected_frequency_hz,
                    status.center_freq_hz,
                    status.sample_rate,
                    bins.len(),
                    cfg.dc_reject_hz,
                );
                let locked_out = frequency_is_locked_out(frequency_hz, &blacklist, channel_bw);
                let radius = (channel_bw as u64 / 2).max(1);
                let same_candidate = candidate_hz
                    .is_some_and(|candidate| candidate.abs_diff(frequency_hz) <= radius);
                if dc_rejected || locked_out {
                    candidate_hz = None;
                    candidate_since = None;
                } else if !same_candidate {
                    candidate_hz = Some(frequency_hz);
                    candidate_since = Some(Instant::now());
                }
                let present_for = candidate_since
                    .map(|started| started.elapsed())
                    .unwrap_or_default();
                let confirmed = !dc_rejected
                    && !locked_out
                    && candidate_is_confirmed(
                        candidate_hz,
                        frequency_hz,
                        channel_bw,
                        present_for,
                        cfg.confirm_ms,
                    );
                if confirmed {
                    let classification = signal_id::classify(
                        frequency_hz,
                        channel_bw,
                        mode,
                        &range_name,
                        snr,
                        None, // audio analysis happens on VFO identify / auto-decode path
                    );
                    let top = classification.candidates.first();
                    let decoder = top
                        .map(|c| c.decoder.clone())
                        .unwrap_or_else(|| "none".into());
                    let first_observation = logged_channels.insert(frequency_hz);
                    if first_observation {
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
                    }
                    // A real scanner must surface a hit through a VFO, not merely
                    // write an invisible database row. Reuse an unlocked slot or
                    // allocate another one up to the range/device limit. Slots are
                    // muted until the browser's explicit Listen gesture.
                    let max_vfos = active_range
                        .as_ref()
                        .map(|range| (range.max_vfos as usize).min(cfg.max_vfos))
                        .unwrap_or(0);
                    if max_vfos > 0 {
                        let now = now_ms();
                        let mut runtime = state.lock();
                        let tolerance = (channel_bw as u64).max(bandwidth_hz as u64);
                        let existing = runtime
                            .vfo_states
                            .iter()
                            .position(|vfo| vfo.frequency_hz.abs_diff(frequency_hz) <= tolerance);
                        let reusable = runtime.vfo_states.iter().position(|vfo| !vfo.locked);
                        let index = existing.or(reusable).or_else(|| {
                            if runtime.vfo_states.len() < max_vfos {
                                let id = runtime
                                    .vfo_states
                                    .iter()
                                    .map(|vfo| vfo.id)
                                    .max()
                                    .unwrap_or(0)
                                    .saturating_add(1);
                                runtime.vfo_states.push(VfoState {
                                    id,
                                    frequency_hz,
                                    mode: mode.to_string(),
                                    muted: true,
                                    volume: 0.7,
                                    audio_agc: true,
                                    squelch_open: true,
                                    strength_db: *peak,
                                    audio_level_db: -120.0,
                                    locked: true,
                                    last_hit_ms: now,
                                    snr_db: snr,
                                    noise_floor_db: noise_floor,
                                });
                                Some(runtime.vfo_states.len() - 1)
                            } else {
                                None
                            }
                        });
                        if let Some(index) = index {
                            let vfo = &mut runtime.vfo_states[index];
                            vfo.frequency_hz = frequency_hz;
                            vfo.mode = mode.to_string();
                            vfo.strength_db = *peak;
                            vfo.squelch_open = true;
                            vfo.locked = true;
                            vfo.last_hit_ms = now;
                        }
                    }
                    if active_range.as_ref().is_some_and(|range| range.hold_ms > 0) {
                        let now = Instant::now();
                        let started = signal_hold_started.get_or_insert(now);
                        let maximum = *started + Duration::from_millis(cfg.scan_hold_max_ms.max(1));
                        signal_hold_until = Some(
                            (now + Duration::from_millis(
                                active_range
                                    .as_ref()
                                    .map(|range| range.hold_ms)
                                    .unwrap_or(0) as u64,
                            ))
                            .min(maximum),
                        );
                    }
                    if first_observation {
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
                    }
                }
            } else {
                candidate_hz = None;
                candidate_since = None;
            }
        }

        // 5. Broadcast to WS subscribers and retain the same frame for the
        // HTTP `/spectrum` endpoint.
        let (frame_sequence, frame_timestamp_ms) = {
            let mut runtime = state.lock();
            runtime.latest_spectrum = bins.clone();
            runtime.frames_processed = runtime.frames_processed.saturating_add(1);
            runtime.latest_spectrum_ms = now_ms();
            runtime.noise_floor_db = noise_floor;
            let squelch_threshold = active_range
                .as_ref()
                .map(|r| r.squelch_db)
                .unwrap_or(cfg.squelch_db);
            let squelch_close = squelch_threshold - 4.0;
            for vfo in runtime.vfo_states.iter_mut() {
                let status = device.status();
                let normalized = (vfo.frequency_hz as f64 - status.center_freq_hz as f64)
                    / status.sample_rate.max(1) as f64
                    + 0.5;
                let center_bin = (normalized * bins.len() as f64).round() as isize;
                let channel_bw = active_range
                    .as_ref()
                    .map(|r| r.channel_bw_hz)
                    .unwrap_or(12_500);
                let radius = ((channel_bw as f64 / status.sample_rate.max(1) as f64)
                    * bins.len() as f64
                    / 2.0)
                    .ceil()
                    .max(2.0) as isize;
                let start = (center_bin - radius).clamp(0, bins.len() as isize - 1) as usize;
                let end = (center_bin + radius + 1).clamp(1, bins.len() as isize) as usize;
                let local_peak = if start < end {
                    bins[start..end]
                        .iter()
                        .copied()
                        .fold(f32::NEG_INFINITY, f32::max)
                } else {
                    f32::NEG_INFINITY
                };
                let margin = local_peak - noise_floor;
                vfo.strength_db = local_peak;
                vfo.noise_floor_db = noise_floor;
                vfo.snr_db = margin;
                if margin >= squelch_threshold {
                    vfo.squelch_open = true;
                } else if margin < squelch_close {
                    vfo.squelch_open = false;
                }
                if !vfo.muted && cfg.scan_hold_on_audio && vfo.squelch_open {
                    vfo.squelch_open = vfo.audio_level_db >= cfg.voice_audio_min_db;
                }
            }
            let signal_present = runtime
                .vfo_states
                .iter()
                .any(|vfo| vfo.locked && vfo.squelch_open);
            if signal_present {
                let now = Instant::now();
                let started = signal_hold_started.get_or_insert(now);
                let hold_ms = active_range
                    .as_ref()
                    .map(|range| range.hold_ms)
                    .unwrap_or(0);
                if hold_ms > 0 {
                    let maximum = *started + Duration::from_millis(cfg.scan_hold_max_ms.max(1));
                    signal_hold_until =
                        Some((now + Duration::from_millis(hold_ms as u64)).min(maximum));
                }
            }
            let holding_signal = signal_hold_until.is_some_and(|until| Instant::now() < until);
            if !signal_present && !holding_signal {
                signal_hold_started = None;
                signal_hold_until = None;
            }
            runtime.holding = holding_signal;
            (runtime.frames_processed, runtime.latest_spectrum_ms)
        };
        let status = device.status();
        spectrum_tx.send_replace(SpectrumFrame {
            sequence: frame_sequence,
            captured_ms: frame_timestamp_ms,
            center_freq_hz: status.center_freq_hz,
            sample_rate_hz: status.sample_rate,
            usable_span_hz: status
                .bandwidth_hz
                .min((status.sample_rate as f64 * 0.9) as u32),
            bins_dbfs: bins.clone(),
        });
        let _ = events_tx.send(ScannerEvent::VfoStates(state.lock().vfo_states.clone()));
        if last_occupancy_at.elapsed() >= Duration::from_secs(2) {
            let rows = occupancy_from_spectrum(
                &bins,
                status.center_freq_hz,
                status.sample_rate,
                noise_floor,
                now_ms(),
            );
            for row in &rows {
                let _ = db.upsert_occupancy(row);
            }
            last_occupancy_at = Instant::now();
        }
        let _ = events_tx.send(ScannerEvent::Spectrum {
            range: range_name,
            bins,
        });
        let pending_step = active_range.as_ref().and_then(|range| {
            let status = device.status();
            let usable = (status.bandwidth_hz as u64)
                .min((status.sample_rate as u64 * 90) / 100)
                .max(1);
            let (scan_locked, holding_signal) = {
                let runtime = state.lock();
                (runtime.scan_locked, runtime.holding)
            };
            let dwell_elapsed = Instant::now() >= next_sweep_at;
            let should_sweep = (force_sweep
                || (!scan_locked
                    && !holding_signal
                    && dwell_elapsed
                    && (range.end_hz.saturating_sub(range.start_hz) > usable
                        || cycle_ranges.len() > 1)))
                && !scan_locked
                && !holding_signal;
            should_sweep
                .then(|| next_scan_step(range, &cycle_ranges, status.center_freq_hz, usable))
        });
        if let Some(step) = pending_step {
            match step {
                ScanStep::Stay => {}
                ScanStep::Retune(next) => {
                    if device.set_frequency(next).is_ok() {
                        capture_ring.clear();
                        audio_ring.clear();
                        snapshot_ring.clear();
                        audio.clear_queue();
                        smoothed_noise_floor = None;
                        signal_hold_started = None;
                        signal_hold_until = None;
                        candidate_hz = None;
                        candidate_since = None;
                        logged_channels.clear();
                        let mut runtime = state.lock();
                        runtime.holding = false;
                        for vfo in &mut runtime.vfo_states {
                            vfo.locked = false;
                            vfo.squelch_open = false;
                        }
                    }
                    if let Some(range) = active_range.as_ref() {
                        next_sweep_at = Instant::now() + settle_dwell(range);
                    }
                }
                ScanStep::SwitchRange(next_range) => {
                    let requested_rate = device.status().sample_rate.max(next_range.sample_rate_hz);
                    let _ = device.set_sample_contract(requested_rate);
                    let status = device.status();
                    let usable = (status.bandwidth_hz as u64)
                        .min((status.sample_rate as u64 * 90) / 100)
                        .max(1);
                    let center = initial_scan_center(&next_range, usable as u32);
                    if device.set_frequency(center).is_ok() {
                        capture_ring.clear();
                        audio_ring.clear();
                        snapshot_ring.clear();
                        audio.clear_queue();
                        smoothed_noise_floor = None;
                        native_decoders.reset(status.sample_rate);
                        signal_hold_started = None;
                        signal_hold_until = None;
                        candidate_hz = None;
                        candidate_since = None;
                        logged_channels.clear();
                        let vfos = starter_vfos(&next_range, center, cfg.max_vfos);
                        {
                            let mut runtime = state.lock();
                            runtime.active_range = Some(next_range.name.clone());
                            runtime.vfo_states = vfos.clone();
                            runtime.holding = false;
                            runtime.scan_locked = false;
                        }
                        let _ = events_tx.send(ScannerEvent::VfoStates(vfos));
                        next_sweep_at = Instant::now() + settle_dwell(&next_range);
                        tracing::info!(range = %next_range.name, "scanner cycled to next bank");
                        active_range = Some(next_range);
                    }
                }
            }
        }
        force_sweep = false;
        let frame_us = ((capture_size as f64 / device.status().sample_rate.max(1) as f64)
            * 1_000_000.0)
            .max(500.0) as u64;
        tokio::time::sleep(Duration::from_micros(frame_us)).await;
    }
}

fn range_name_matches(name: &str, needles: &[&str]) -> bool {
    let lower = name.to_ascii_lowercase();
    needles.iter().any(|needle| lower.contains(needle))
}

fn publish_decoded(
    db: &Db,
    events_tx: &broadcast::Sender<ScannerEvent>,
    decoded: crate::db::DecodedMessage,
) {
    let _ = db.insert_decoded_message(&decoded);
    let _ = events_tx.send(ScannerEvent::DecodedMessage(decoded));
}

struct NativeRangeDecoders {
    adsb: Option<AdsbDecoder>,
    pocsag_1200: crate::pocsag::PocsagDecoder,
    ais: Option<crate::ais::IqDecoder>,
    aprs: Option<crate::aprs::AprsDecoder>,
    uat: Option<crate::aviation::UatIqDecoder>,
    acars: Option<crate::aviation::AcarsIqDecoder>,
    vdl2: Option<crate::aviation::Vdl2IqDecoder>,
    hf_audio: Vec<f32>,
    last_hf_text: String,
    rds_mpx: Vec<f32>,
    last_rds_pi: Option<u16>,
    sample_rate: u32,
}

impl NativeRangeDecoders {
    fn new(sample_rate: u32) -> Self {
        Self {
            adsb: AdsbDecoder::new(sample_rate),
            pocsag_1200: crate::pocsag::PocsagDecoder::new(
                24_000,
                crate::pocsag::PocsagBaud::Baud1200,
            ),
            ais: crate::ais::IqDecoder::new(sample_rate as f64).ok(),
            aprs: Some(crate::aprs::AprsDecoder::new(24_000.0)),
            uat: Some(crate::aviation::UatIqDecoder::new(sample_rate)),
            acars: Some(crate::aviation::AcarsIqDecoder::new(
                sample_rate,
                crate::aviation::BitOrder::LsbFirst,
                false,
            )),
            vdl2: Some(crate::aviation::Vdl2IqDecoder::new(sample_rate)),
            hf_audio: Vec::new(),
            last_hf_text: String::new(),
            rds_mpx: Vec::new(),
            last_rds_pi: None,
            sample_rate,
        }
    }

    fn reset(&mut self, sample_rate: u32) {
        *self = Self::new(sample_rate);
    }

    fn ensure_rate(&mut self, sample_rate: u32) {
        if self.sample_rate != sample_rate {
            self.reset(sample_rate);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn feed(
        &mut self,
        range: &ScanRange,
        iq: &[Complex<f32>],
        discriminator: &[f32],
        sample_rate: u32,
        center_hz: u64,
        tune_hz: u64,
        db: &Db,
        events_tx: &broadcast::Sender<ScannerEvent>,
    ) {
        self.ensure_rate(sample_rate);
        let name = range.name.as_str();
        let mode_s = range_name_matches(name, &["1090"])
            || ((range_name_matches(name, &["ads-b", "adsb"]))
                && !range_name_matches(name, &["uat", "978"]));
        let uat = range_name_matches(name, &["uat", "978"]);
        let ais = range_name_matches(name, &["ais"]);
        let aprs = range_name_matches(name, &["aprs"]);
        let acars = range_name_matches(name, &["acars"]);
        let vdl2 = range_name_matches(name, &["vdl"]);
        let pocsag = range_name_matches(name, &["pocsag", "pager"]);
        let rtty = range_name_matches(name, &["rtty"]);
        let navtex = range_name_matches(name, &["navtex"]);
        let cw = range_name_matches(name, &["cw"]);
        let rds =
            range.mode.eq_ignore_ascii_case("wfm") || range_name_matches(name, &["fm broadcast"]);
        let offset_hz = tune_hz as f64 - center_hz as f64;

        if mode_s {
            if self.adsb.is_none() {
                self.adsb = AdsbDecoder::new(sample_rate);
            }
            if let Some(decoder) = self.adsb.as_mut() {
                decoder.feed_iq(iq);
                for message in decoder.take_messages() {
                    let content = message
                        .callsign
                        .clone()
                        .or_else(|| message.altitude_ft.map(|a| format!("{a} ft")))
                        .unwrap_or_default();
                    publish_decoded(
                        db,
                        events_tx,
                        crate::db::DecodedMessage {
                            id: None,
                            frequency_hz: 1_090_000_000,
                            protocol: "adsb".into(),
                            message_type: message.message_type.clone(),
                            address: message.icao.clone(),
                            function_code: format!("DF{}", message.df),
                            content,
                            raw: message.raw_hex.clone(),
                            encryption: "none".into(),
                            timestamp_ms: now_ms(),
                        },
                    );
                }
            }
        }

        if pocsag {
            let channel = channelize_iq(iq, offset_hz, sample_rate, Mode::Nfm);
            let mut previous = None;
            let audio = discriminator_samples(&channel, &mut previous);
            let audio_24k = crate::sidecar::resample_audio(&audio, sample_rate, 24_000);
            for message in self.pocsag_1200.push_audio(&audio_24k) {
                publish_decoded(
                    db,
                    events_tx,
                    crate::db::DecodedMessage {
                        id: None,
                        frequency_hz: tune_hz,
                        protocol: "pocsag".into(),
                        message_type: "pager".into(),
                        address: message.ric.to_string(),
                        function_code: message.function.to_string(),
                        content: message.text.clone(),
                        raw: message.text,
                        encryption: "none".into(),
                        timestamp_ms: now_ms(),
                    },
                );
            }
        }

        if ais || uat || acars || vdl2 {
            let pairs: Vec<(f32, f32)> = iq.iter().map(|c| (c.re, c.im)).collect();
            if ais {
                if self.ais.is_none() {
                    self.ais = crate::ais::IqDecoder::new(sample_rate as f64).ok();
                }
                if let Some(decoder) = self.ais.as_mut() {
                    for message in decoder.push_iq(&pairs).into_iter().filter_map(Result::ok) {
                        publish_decoded(
                            db,
                            events_tx,
                            crate::db::DecodedMessage {
                                id: None,
                                frequency_hz: center_hz,
                                protocol: "ais".into(),
                                message_type: format!("type_{}", message.message_type()),
                                address: message.mmsi().to_string(),
                                function_code: String::new(),
                                content: serde_json::to_string(&message).unwrap_or_default(),
                                raw: String::new(),
                                encryption: "none".into(),
                                timestamp_ms: now_ms(),
                            },
                        );
                    }
                }
            }
            if uat {
                if self.uat.is_none() {
                    self.uat = Some(crate::aviation::UatIqDecoder::new(sample_rate));
                }
                if let Some(decoder) = self.uat.as_mut() {
                    decoder.push_iq(&pairs);
                    for message in decoder.take_messages() {
                        publish_decoded(
                            db,
                            events_tx,
                            crate::db::DecodedMessage {
                                id: None,
                                frequency_hz: 978_000_000,
                                protocol: "uat".into(),
                                message_type: format!("{:?}", message.frame_kind)
                                    .to_ascii_lowercase(),
                                address: message.address_hex.clone().unwrap_or_default(),
                                function_code: message
                                    .message_code
                                    .map(|code| format!("MC{code}"))
                                    .unwrap_or_default(),
                                content: message
                                    .payload
                                    .iter()
                                    .filter(|b| (0x20..=0x7e).contains(*b))
                                    .map(|b| *b as char)
                                    .collect(),
                                raw: hex::encode(&message.raw_codeword),
                                encryption: "none".into(),
                                timestamp_ms: now_ms(),
                            },
                        );
                    }
                }
            }
            if acars {
                if self.acars.is_none() {
                    self.acars = Some(crate::aviation::AcarsIqDecoder::new(
                        sample_rate,
                        crate::aviation::BitOrder::LsbFirst,
                        false,
                    ));
                }
                if let Some(decoder) = self.acars.as_mut() {
                    decoder.push_iq(&pairs);
                    for message in decoder.take_messages().into_iter().filter(|m| m.crc_valid) {
                        publish_decoded(
                            db,
                            events_tx,
                            crate::db::DecodedMessage {
                                id: None,
                                frequency_hz: center_hz,
                                protocol: "acars".into(),
                                message_type: message
                                    .label
                                    .clone()
                                    .unwrap_or_else(|| "acars".into()),
                                address: message.registration.clone().unwrap_or_default(),
                                function_code: message
                                    .block_id
                                    .map(|c| c.to_string())
                                    .unwrap_or_default(),
                                content: message.text.clone(),
                                raw: hex::encode(&message.raw_bytes),
                                encryption: "none".into(),
                                timestamp_ms: now_ms(),
                            },
                        );
                    }
                }
            }
            if vdl2 {
                if self.vdl2.is_none() {
                    self.vdl2 = Some(crate::aviation::Vdl2IqDecoder::new(sample_rate));
                }
                if let Some(decoder) = self.vdl2.as_mut() {
                    decoder.push_iq(&pairs);
                    for message in decoder.take_messages().into_iter().filter(|m| m.fcs_valid) {
                        publish_decoded(
                            db,
                            events_tx,
                            crate::db::DecodedMessage {
                                id: None,
                                frequency_hz: center_hz,
                                protocol: "vdl2".into(),
                                message_type: "avlc".into(),
                                address: String::new(),
                                function_code: String::new(),
                                content: message
                                    .payload
                                    .iter()
                                    .filter(|b| (0x20..=0x7e).contains(*b))
                                    .map(|b| *b as char)
                                    .collect(),
                                raw: hex::encode(&message.raw_frame),
                                encryption: "none".into(),
                                timestamp_ms: now_ms(),
                            },
                        );
                    }
                }
            }
        }

        if aprs {
            if self.aprs.is_none() {
                self.aprs = Some(crate::aprs::AprsDecoder::new(24_000.0));
            }
            if let Some(decoder) = self.aprs.as_mut() {
                let channel = channelize_iq(iq, offset_hz, sample_rate, Mode::Nfm);
                let mut previous = None;
                let audio = discriminator_samples(&channel, &mut previous);
                let audio_24k = crate::sidecar::resample_audio(&audio, sample_rate, 24_000);
                for sample in audio_24k {
                    decoder.feed(sample);
                }
                for frame in std::mem::take(&mut decoder.frames) {
                    publish_decoded(
                        db,
                        events_tx,
                        crate::db::DecodedMessage {
                            id: None,
                            frequency_hz: tune_hz,
                            protocol: "aprs".into(),
                            message_type: "ax25".into(),
                            address: frame.source,
                            function_code: frame.dest,
                            content: frame.info.clone(),
                            raw: frame.info,
                            encryption: "none".into(),
                            timestamp_ms: now_ms(),
                        },
                    );
                }
            }
        }

        if rds {
            let channel = channelize_iq(iq, offset_hz, sample_rate, Mode::Wfm);
            let mut previous = None;
            let mpx = discriminator_samples(&channel, &mut previous);
            let mpx = crate::sidecar::resample_audio(&mpx, sample_rate, 190_000);
            self.rds_mpx.extend_from_slice(&mpx);
            const RDS_WINDOW: usize = 76_000;
            if self.rds_mpx.len() >= RDS_WINDOW {
                if let Some(result) = crate::demod::decode_rds(&self.rds_mpx, 190_000.0) {
                    if result.groups_found > 0 {
                        if let Some(pi) = result.pi_code {
                            if self.last_rds_pi != Some(pi) {
                                self.last_rds_pi = Some(pi);
                                let ps = result.program_service.clone().unwrap_or_default();
                                publish_decoded(
                                    db,
                                    events_tx,
                                    crate::db::DecodedMessage {
                                        id: None,
                                        frequency_hz: tune_hz,
                                        protocol: "rds".into(),
                                        message_type: "group".into(),
                                        address: format!("{pi:04X}"),
                                        function_code: result
                                            .pty
                                            .map(|pty| format!("PTY{pty}"))
                                            .unwrap_or_default(),
                                        content: ps.clone(),
                                        raw: ps,
                                        encryption: "none".into(),
                                        timestamp_ms: now_ms(),
                                    },
                                );
                            }
                        }
                    }
                }
                let drain = self.rds_mpx.len() / 2;
                self.rds_mpx.drain(..drain);
            }
        } else {
            self.rds_mpx.clear();
            self.last_rds_pi = None;
        }

        if rtty || navtex || cw {
            let audio_8k = crate::sidecar::resample_audio(discriminator, sample_rate, 8_000);
            self.hf_audio.extend_from_slice(&audio_8k);
            const HF_WINDOW: usize = 16_000;
            if self.hf_audio.len() >= HF_WINDOW {
                if rtty {
                    if let Some(text) =
                        crate::demod::decode_rtty(&self.hf_audio, 8_000.0, 2125.0, 1955.0, 50.0)
                    {
                        if !text.is_empty() && text != self.last_hf_text {
                            self.last_hf_text = text.clone();
                            publish_decoded(
                                db,
                                events_tx,
                                crate::db::DecodedMessage {
                                    id: None,
                                    frequency_hz: center_hz,
                                    protocol: "rtty".into(),
                                    message_type: "text".into(),
                                    address: String::new(),
                                    function_code: String::new(),
                                    content: text.clone(),
                                    raw: text,
                                    encryption: "none".into(),
                                    timestamp_ms: now_ms(),
                                },
                            );
                        }
                    }
                }
                if navtex {
                    if let Some(text) = crate::demod::decode_navtex(&self.hf_audio, 8_000.0) {
                        if !text.is_empty() && text != self.last_hf_text {
                            self.last_hf_text = text.clone();
                            publish_decoded(
                                db,
                                events_tx,
                                crate::db::DecodedMessage {
                                    id: None,
                                    frequency_hz: center_hz,
                                    protocol: "navtex".into(),
                                    message_type: "text".into(),
                                    address: String::new(),
                                    function_code: String::new(),
                                    content: text.clone(),
                                    raw: text,
                                    encryption: "none".into(),
                                    timestamp_ms: now_ms(),
                                },
                            );
                        }
                    }
                }
                if cw {
                    if let Some(text) = crate::demod::decode_cw(&self.hf_audio, 8_000.0, 700.0) {
                        if !text.is_empty() && text != self.last_hf_text {
                            self.last_hf_text = text.clone();
                            publish_decoded(
                                db,
                                events_tx,
                                crate::db::DecodedMessage {
                                    id: None,
                                    frequency_hz: center_hz,
                                    protocol: "cw".into(),
                                    message_type: "morse".into(),
                                    address: String::new(),
                                    function_code: String::new(),
                                    content: text.clone(),
                                    raw: text,
                                    encryption: "none".into(),
                                    timestamp_ms: now_ms(),
                                },
                            );
                        }
                    }
                }
                let drain = self.hf_audio.len() / 2;
                self.hf_audio.drain(..drain);
            }
        } else {
            self.hf_audio.clear();
        }
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

#[cfg(test)]
mod scan_window_tests {
    use super::*;

    #[test]
    fn initial_center_reserves_both_window_edges() {
        let range = ScanRange {
            start_hz: 88_000_000,
            end_hz: 108_000_000,
            ..Default::default()
        };
        assert_eq!(initial_scan_center(&range, 1_800_000), 88_900_000);
    }

    #[test]
    fn narrow_range_centers_at_its_upper_edge_without_overflow() {
        let range = ScanRange {
            start_hz: 144_000_000,
            end_hz: 144_100_000,
            ..Default::default()
        };
        assert_eq!(initial_scan_center(&range, 2_000_000), 144_100_000);
    }

    #[test]
    fn broadcast_fm_peaks_snap_to_stable_channel_centers() {
        let range = ScanRange {
            start_hz: 88_000_000,
            end_hz: 108_000_000,
            mode: "wfm".into(),
            channel_bw_hz: 200_000,
            ..Default::default()
        };
        assert_eq!(
            stable_channel_frequency(&range, 90_283_980, 5_000),
            90_300_000
        );
        assert_eq!(
            stable_channel_frequency(&range, 92_714_511, 5_000),
            92_700_000
        );
    }

    #[test]
    fn lockout_matches_within_half_channel() {
        let entries = vec![crate::db::BlacklistEntry {
            frequency_hz: 162_550_000,
            reason: "skip".into(),
            temporary: true,
            created_ms: 1,
        }];
        assert!(frequency_is_locked_out(162_551_000, &entries, 25_000));
        assert!(!frequency_is_locked_out(162_400_000, &entries, 25_000));
    }

    #[test]
    fn selected_vfo_prefers_unmuted_then_locked() {
        let muted_locked = VfoState {
            id: 0,
            frequency_hz: 162_400_000,
            mode: "nfm".into(),
            muted: true,
            volume: 0.7,
            audio_agc: true,
            squelch_open: true,
            strength_db: -40.0,
            audio_level_db: -20.0,
            locked: true,
            last_hit_ms: 1,
            snr_db: 12.0,
            noise_floor_db: -90.0,
        };
        let unmuted = VfoState {
            id: 1,
            frequency_hz: 162_550_000,
            muted: false,
            locked: false,
            ..muted_locked.clone()
        };
        assert_eq!(
            selected_vfo(&[muted_locked.clone(), unmuted.clone()]).map(|vfo| vfo.id),
            Some(1)
        );
        assert_eq!(
            selected_vfo(std::slice::from_ref(&muted_locked)).map(|vfo| vfo.frequency_hz),
            Some(162_400_000)
        );
    }

    #[test]
    fn dc_bin_is_rejected_even_when_config_is_zero() {
        assert!(peak_is_dc_rejected(
            100_000_000,
            100_000_000,
            2_000_000,
            4096,
            0
        ));
        assert!(!peak_is_dc_rejected(
            100_050_000,
            100_000_000,
            2_000_000,
            4096,
            0
        ));
    }

    #[test]
    fn confirm_requires_dwell_on_the_same_channel() {
        assert!(!candidate_is_confirmed(
            Some(146_520_000),
            146_520_000,
            12_500,
            Duration::from_millis(100),
            300,
        ));
        assert!(candidate_is_confirmed(
            Some(146_520_000),
            146_522_000,
            12_500,
            Duration::from_millis(300),
            300,
        ));
        assert!(candidate_is_confirmed(
            Some(146_520_000),
            146_520_000,
            12_500,
            Duration::from_millis(0),
            0,
        ));
    }

    #[test]
    fn search_hops_then_wraps_inside_one_bank() {
        let range = ScanRange {
            start_hz: 144_000_000,
            end_hz: 148_000_000,
            ..Default::default()
        };
        assert_eq!(
            next_scan_step(&range, &[], 144_900_000, 1_800_000),
            ScanStep::Retune(146_700_000)
        );
        assert_eq!(
            next_scan_step(&range, &[], 147_500_000, 1_800_000),
            ScanStep::Retune(initial_scan_center(&range, 1_800_000))
        );
    }

    #[test]
    fn wrapping_a_bank_advances_the_enabled_cycle() {
        let two_meters = ScanRange {
            name: "2m Amateur".into(),
            start_hz: 144_000_000,
            end_hz: 148_000_000,
            ..Default::default()
        };
        let weather = ScanRange {
            name: "NOAA Weather".into(),
            start_hz: 162_400_000,
            end_hz: 162_550_000,
            ..Default::default()
        };
        let cycle = vec![two_meters.clone(), weather.clone()];
        match next_scan_step(&two_meters, &cycle, 147_500_000, 1_800_000) {
            ScanStep::SwitchRange(next) => assert_eq!(next.name, "NOAA Weather"),
            other => panic!("expected next bank, got {other:?}"),
        }
        match next_scan_step(&weather, &cycle, 162_475_000, 1_800_000) {
            ScanStep::SwitchRange(next) => assert_eq!(next.name, "2m Amateur"),
            other => panic!("expected wrap to 2m, got {other:?}"),
        }
    }

    #[test]
    fn narrow_bookmark_without_a_cycle_stays_put() {
        let bookmark = ScanRange {
            name: "Bookmark NOAA".into(),
            start_hz: 162_550_000,
            end_hz: 162_550_000,
            ..Default::default()
        };
        assert_eq!(
            next_scan_step(&bookmark, &[], 162_550_000, 1_800_000),
            ScanStep::Stay
        );
    }

    #[test]
    fn occupancy_buckets_use_the_capture_window_not_the_whole_bank() {
        let bins = vec![-90.0f32; 64];
        let rows = occupancy_from_spectrum(&bins, 100_000_000, 2_000_000, -100.0, 1_800_000);
        assert!(!rows.is_empty());
        assert!(rows[0].frequency_bucket_hz >= 99_000_000);
        assert!(rows[0].frequency_bucket_hz < 100_000_000);
        assert!((occupancy_fraction(&rows[0]) - 0.25).abs() < 0.01);
    }

    #[test]
    fn api_documents_scan_operator_routes() {
        let docs = include_str!("../../docs/API.md");
        for route in [
            "/channels/scan/start",
            "/scan/status",
            "/scan/lock",
            "/scan/unlock",
            "/scan/skip",
            "/scan/lockout",
            "/vfo/:id/rds",
            "/scan/ctcss",
            "/scan/aprs",
        ] {
            assert!(docs.contains(route), "API.md must document {route}");
        }
        assert!(docs.contains("`enabled`"));
        assert!(docs.contains("Bookmarks"));
        assert!(
            docs.contains("without parking scanner Hold"),
            "profile apply must not park Hold"
        );
    }

    #[test]
    fn decoder_range_names_select_the_matching_native_path() {
        assert!(range_name_matches("ADS-B 1090", &["1090"]));
        assert!(!range_name_matches("ADS-B UAT", &["1090"]));
        assert!(range_name_matches("ADS-B UAT", &["uat", "978"]));
        assert!(range_name_matches("AIS", &["ais"]));
        assert!(range_name_matches("APRS 2m", &["aprs"]));
        assert!(range_name_matches("ACARS", &["acars"]));
        assert!(range_name_matches("VDL2", &["vdl"]));
        assert!(range_name_matches("NAVTEX 518", &["navtex"]));
        assert!(range_name_matches("RTTY 20m", &["rtty"]));
        assert!(range_name_matches("CW 20m", &["cw"]));
        assert!(range_name_matches("FM Broadcast", &["fm broadcast"]));
        assert!(range_name_matches("Pagers", &["pocsag", "pager"]));
        assert!(!range_name_matches("Aircraft AM", &["ais", "acars", "vdl"]));
        assert!(!range_name_matches("2m Amateur", &["aprs"]));
    }
}

#[cfg(all(test, feature = "soapysdr"))]
mod hardware_tests {
    use super::*;
    #[test]
    fn live_rsp1b_scanner_fft_has_dynamic_range() {
        let _hardware_guard = crate::device::LIVE_HARDWARE_LOCK.lock().unwrap();
        let device = DeviceLayer::new_mock();
        let key = DeviceLayer::discover()
            .into_iter()
            .find(|d| d.driver == "sdrplay")
            .expect("RSP1B missing from discovery")
            .key;
        device.connect(&key).expect("connect RSP1B");
        device.set_sample_rate(2_000_000).expect("set rate");
        device.set_frequency(162_550_000).expect("tune");
        let iq = device.read_iq(4096).expect("read live RSP1B IQ");
        assert_eq!(iq.len(), 4096, "short IQ frame");
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(4096);
        let mut out = iq.clone();
        let window: Vec<f32> = apodize::hanning_iter(4096).map(|x| x as f32).collect();
        for (sample, w) in out.iter_mut().zip(window.iter()) {
            *sample *= *w;
        }
        fft.process(&mut out);
        let half = out.len() / 2;
        let bins: Vec<f32> = out[half..]
            .iter()
            .chain(out[..half].iter())
            .map(|c| 10.0 * (c.norm_sqr() + 1e-20).log10())
            .collect();
        let min = bins.iter().copied().fold(f32::INFINITY, f32::min);
        let max = bins.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(
            max - min > 6.0,
            "flat/non-live scanner spectrum: min={min} max={max}"
        );
        eprintln!(
            "live RSP1B scanner FFT bins={} min_db={min:.2} max_db={max:.2} span_db={:.2}",
            bins.len(),
            max - min
        );
        device.disconnect().expect("disconnect RSP1B");
    }
}
