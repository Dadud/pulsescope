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
    dc_block, decimate_complex_average, decode_wfm_stereo, deemphasis, demodulate,
    low_pass_complex, low_pass_real, mix_down, Mode, SincResampler, WfmStereoState,
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
    Shutdown,
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
                let vfos = state.lock().vfo_states.clone();
                if previous.len() != vfos.len() {
                    previous = vec![None; vfos.len()];
                    phases = vec![0.0; vfos.len()];
                    filter_states = vec![Complex::new(0.0, 0.0); vfos.len()];
                    audio_filter_states = vec![0.0; vfos.len()];
                    deemphasis_states = vec![0.0; vfos.len()];
                    dc_states = vec![0.0; vfos.len()];
                    agc_gains = vec![1.0; vfos.len()];
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
                        _ => 5_000.0,
                    };
                    let baseband = low_pass_complex(
                        &baseband,
                        cutoff_hz,
                        sample_rate,
                        &mut filter_states[idx],
                    );
                    let baseband = decimate_complex_average(&baseband, predecimation);
                    let multiplex = demodulate(mode, &baseband, &mut previous[idx]);
                    if mode == Mode::Wfm {
                        stereo_candidate = Some(decode_wfm_stereo(
                            &multiplex,
                            effective_rate,
                            wfm_deemphasis_us as f32,
                            &mut stereo_states[idx],
                        ));
                    }
                    let mut pcm = multiplex;
                    let audio_cutoff_hz = match mode {
                        Mode::Wfm => 15_000.0,
                        Mode::Nfm => 5_000.0,
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
                            wfm_deemphasis_us as f32,
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
        let task = tokio::spawn(scanner_loop(
            cfg,
            dependencies,
            cmd_rx,
            state.clone(),
            capture_ring.clone(),
            audio_ring.clone(),
        ));

        ScannerHandle {
            cmd_tx: cmd_tx.clone(),
            state,
            iq_consumers: vec![capture_ring, audio_ring],
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
        vec![capture_ring.clone(), audio_ring.clone()],
        cfg.fft_size,
        playback,
        iq_network,
    );

    // Simple window coefficients (Hanning) — apodize 1.0 exposes `hanning_iter`.
    let window: Vec<f32> = apodize::hanning_iter(cfg.fft_size)
        .map(|x| x as f32)
        .collect();
    let mut last_signal_hit = Instant::now() - Duration::from_secs(2);
    let mut smoothed_noise_floor: Option<f32> = None;
    let mut native_adsb = AdsbDecoder::new(device.status().sample_rate);
    let mut next_sweep_at = Instant::now();
    let mut signal_hold_started: Option<Instant> = None;
    let mut signal_hold_until: Option<Instant> = None;
    let mut logged_channels = HashSet::<u64>::new();

    loop {
        // Drain commands
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                ScannerCommand::Start { range } => {
                    // Hardware configuration happens before this command. Do
                    // not process IQ captured under the previous contract.
                    capture_ring.clear();
                    audio_ring.clear();
                    audio.clear_queue();
                    let name = range.name.clone();
                    let vfos = if range.max_vfos == 0 || cfg.max_vfos == 0 {
                        Vec::new()
                    } else {
                        vec![VfoState {
                            id: 0,
                            frequency_hz: device.status().center_freq_hz,
                            mode: range.mode.clone(),
                            // Monitoring is opt-in. The SSTV automation is the
                            // narrow exception: it needs post-demod audio, but
                            // it remains silent until a browser subscribes.
                            muted: !(range.name.starts_with("SSTV ") || range.name.starts_with("FT8 ")),
                            volume: 0.7,
                            audio_agc: true,
                            squelch_open: false,
                            strength_db: -120.0,
                            audio_level_db: -120.0,
                            locked: false,
                            last_hit_ms: 0,
                        }]
                    };
                    state.lock().vfo_states = vfos.clone();
                    state.lock().active_range = Some(name.clone());
                    state.lock().running = true;
                    state.lock().scan_locked = false;
                    state.lock().started_ms = now_ms();
                    active_range = Some(range);
                    next_sweep_at = Instant::now()
                        + Duration::from_millis(
                            active_range
                                .as_ref()
                                .map(|r| r.dwell_ms.max(750) as u64)
                                .unwrap_or(750),
                        );
                    signal_hold_started = None;
                    signal_hold_until = None;
                    logged_channels.clear();
                    let _ = events_tx.send(ScannerEvent::VfoStates(vfos));
                    tracing::info!(range = %name, "scanner started");
                }
                ScannerCommand::Stop => {
                    state.lock().running = false;
                    state.lock().active_range = None;
                    state.lock().vfo_states.clear();
                    active_range = None;
                    signal_hold_started = None;
                    signal_hold_until = None;
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
        recording.lock().write_iq(&iq);

        // Native ADS-B path: only activate on an ADS-B range, so ordinary
        // scanner traffic never pays the Mode S preamble scan cost.
        let native_adsb_active = active_range
            .as_ref()
            .map(|r| {
                r.name.to_ascii_lowercase().contains("ads-b")
                    || r.name.to_ascii_lowercase().contains("adsb")
            })
            .unwrap_or(false);
        if native_adsb_active {
            if native_adsb.is_none() {
                native_adsb = AdsbDecoder::new(device.status().sample_rate);
            }
            if let Some(decoder) = native_adsb.as_mut() {
                decoder.feed_iq(&iq);
                for message in decoder.take_messages() {
                    let content = message
                        .callsign
                        .clone()
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
        if let Some((bin, peak)) = bins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        {
            let snr = *peak - noise_floor;
            if snr >= 12.0 && last_signal_hit.elapsed() >= Duration::from_secs(1) {
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
                last_signal_hit = Instant::now();
            }
        }

        // 5. Broadcast to WS subscribers and retain the same frame for the
        // HTTP `/spectrum` endpoint.
        let (frame_sequence, frame_timestamp_ms) = {
            let mut runtime = state.lock();
            runtime.latest_spectrum = bins.clone();
            runtime.frames_processed = runtime.frames_processed.saturating_add(1);
            runtime.latest_spectrum_ms = now_ms();
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
                vfo.strength_db = local_peak;
                vfo.squelch_open = local_peak - noise_floor
                    >= active_range.as_ref().map(|r| r.squelch_db).unwrap_or(12.0);
            }
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
        let _ = events_tx.send(ScannerEvent::Spectrum {
            range: range_name,
            bins,
        });
        if let Some(range) = active_range.as_ref() {
            let status = device.status();
            let usable = (status.bandwidth_hz as u64)
                .min((status.sample_rate as u64 * 90) / 100)
                .max(1);
            let runtime = state.lock();
            let holding_signal = signal_hold_until.is_some_and(|until| Instant::now() < until);
            let should_sweep = range.end_hz.saturating_sub(range.start_hz) > usable
                && !runtime.scan_locked
                && !holding_signal
                && !runtime.vfo_states.iter().any(|vfo| !vfo.muted)
                && Instant::now() >= next_sweep_at;
            drop(runtime);
            if should_sweep {
                let half = usable / 2;
                let next = if status
                    .center_freq_hz
                    .saturating_add(usable)
                    .saturating_add(half)
                    > range.end_hz
                {
                    initial_scan_center(range, usable as u32)
                } else {
                    status.center_freq_hz.saturating_add(usable)
                };
                if device.set_frequency(next).is_ok() {
                    capture_ring.clear();
                    audio_ring.clear();
                    audio.clear_queue();
                    smoothed_noise_floor = None;
                    signal_hold_started = None;
                    signal_hold_until = None;
                    logged_channels.clear();
                    let mut runtime = state.lock();
                    for vfo in &mut runtime.vfo_states {
                        vfo.locked = false;
                        vfo.squelch_open = false;
                    }
                }
                next_sweep_at =
                    Instant::now() + Duration::from_millis(range.dwell_ms.max(750) as u64);
            }
        }
        let frame_us = ((capture_size as f64 / device.status().sample_rate.max(1) as f64)
            * 1_000_000.0)
            .max(500.0) as u64;
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
