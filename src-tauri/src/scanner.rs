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

use crate::audio::AudioSink;
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
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScannerRuntimeState {
    pub active_range: Option<String>,
    pub running: bool,
    pub vfo_states: Vec<VfoState>,
    pub latest_spectrum: Vec<f32>,
    pub frames_processed: u64,
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
                    let pcm = demodulate(mode, &baseband, &mut previous[idx]);
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
        let handle = ScannerHandle { cmd_tx: cmd_tx.clone(), state: state.clone() };

        tokio::spawn(scanner_loop(cfg, device, db, recording, playback, audio, iq_network, sidecars, events_tx, cmd_rx, state));
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
) {
    let mut active_range: Option<ScanRange> = None;
    let poll = Duration::from_micros((1_000_000.0 / cfg.update_rate_hz.max(1.0)) as u64);

    // Complex FFT preserves both sides of the SDR's IQ spectrum.
    let mut fft_planner = FftPlanner::<f32>::new();
    let fft = fft_planner.plan_fft_forward(cfg.fft_size);
    let mut spectrum: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); cfg.fft_size];
    let capture_size = cfg.fft_size.saturating_mul(5);
    let capture_ring = IqRing::new(capture_size.saturating_mul(16));
    let audio_ring = IqRing::new(2_000_000);
    let _audio_worker = AudioWorker::start(audio_ring.clone(), audio.clone(), device.clone(), state.clone());
    let _capture_worker = CaptureWorker::start(device.clone(), vec![capture_ring.clone(), audio_ring], cfg.fft_size, playback, iq_network);

    // Simple window coefficients (Hanning) — apodize 1.0 exposes `hanning_iter`.
    let window: Vec<f32> = apodize::hanning_iter(cfg.fft_size).map(|x| x as f32).collect();
    let mut last_signal_hit = Instant::now() - Duration::from_secs(2);
    let mut smoothed_noise_floor: Option<f32> = None;
    let mut native_adsb = AdsbDecoder::new(device.status().sample_rate);

    loop {
        // Drain commands
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                ScannerCommand::Start { range } => {
                    let name = range.name.clone();
                    let vfos = (0..range.max_vfos.min(cfg.max_vfos as u32))
                        .map(|i| VfoState {
                            id: i,
                            frequency_hz: range.start_hz,
                            mode: range.mode.clone(),
                            // Monitoring is opt-in. Scans must never turn into
                            // speaker static merely because the app launched.
                            muted: true,
                            volume: 0.7,
                            audio_agc: true,
                            squelch_open: false,
                            strength_db: -120.0,
                            audio_level_db: -120.0,
                        })
                        .collect::<Vec<_>>();
                    state.lock().vfo_states = vfos.clone();
                    state.lock().active_range = Some(name.clone());
                    state.lock().running = true;
                    active_range = Some(range);
                    let _ = events_tx.send(ScannerEvent::VfoStates(vfos));
                    tracing::info!(range = %name, "scanner started");
                }
                ScannerCommand::Stop => {
                    state.lock().running = false;
                    state.lock().active_range = None;
                    state.lock().vfo_states.clear();
                    active_range = None;
                    let _ = events_tx.send(ScannerEvent::VfoStates(Vec::new()));
                    tracing::info!("scanner stopped");
                }
                ScannerCommand::SetVfoFrequency { id, frequency_hz } => {
                    if let Some(v) = state.lock().vfo_states.iter_mut().find(|v| v.id == id) {
                        v.frequency_hz = frequency_hz;
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
            if native_adsb.is_none() {
                native_adsb = AdsbDecoder::new(device.status().sample_rate);
            }
            if let Some(decoder) = native_adsb.as_mut() {
                decoder.feed_iq(&iq);
                for message in decoder.take_messages() {
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
            if snr >= 12.0 && last_signal_hit.elapsed() >= Duration::from_secs(1) {
                let status = device.status();
                let offset = bin as f64 / bins.len() as f64 - 0.5;
                let frequency_hz = (status.center_freq_hz as f64 + offset * status.sample_rate as f64).max(0.0) as u64;
                let bandwidth_hz = (status.sample_rate / bins.len().max(1) as u32).max(1);
                // Prefer the active range's channel BW when available (more accurate than FFT bin width)
                let mode = active_range.as_ref().map(|r| r.mode.as_str()).unwrap_or("nfm");
                let channel_bw = active_range.as_ref().map(|r| r.channel_bw_hz).unwrap_or(bandwidth_hz);
                let classification = signal_id::classify(
                    frequency_hz,
                    channel_bw,
                    mode,
                    &range_name,
                    snr,
                    None, // audio analysis happens on VFO identify / auto-decode path
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
            let peak = bins.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            for vfo in runtime.vfo_states.iter_mut() {
                vfo.strength_db = peak;
                vfo.squelch_open = peak - noise_floor >= 12.0;
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
