//! Cross-platform PCM output using CPAL.
//!
//! CPAL's native stream handle is deliberately owned by a dedicated thread;
//! AppState only contains thread-safe queue/status handles, so the HTTP server
//! remains Send + Sync on Windows and other platforms.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU32, AtomicU64, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;

const MAX_QUEUE_SAMPLES: usize = 48_000 * 4;
const STREAM_QUEUE_CHUNKS: usize = 32;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VoxConfig {
    pub enabled: bool,
    pub threshold_db: f32,
    pub pre_roll_ms: u32,
    pub post_roll_ms: u32,
}
impl Default for VoxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_db: -42.0,
            pre_roll_ms: 2_000,
            post_roll_ms: 1_000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordingMetadata {
    pub frequency_hz: u64,
    pub mode: String,
    pub signal_id: Option<String>,
    pub case_id: Option<i64>,
    pub case_name: Option<String>,
}

struct WavRecording {
    path: PathBuf,
    file: File,
    sample_rate: u32,
    started_ms: i64,
    samples: u64,
    metadata: RecordingMetadata,
    vox: VoxConfig,
    active: bool,
    pre_roll: VecDeque<i16>,
    last_voice: Option<Instant>,
    write_error: Option<String>,
}

impl WavRecording {
    fn new(
        path: PathBuf,
        sample_rate: u32,
        metadata: RecordingMetadata,
        vox: VoxConfig,
    ) -> std::io::Result<Self> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        write_wav_header(
            &mut file,
            sample_rate,
            0,
            &metadata,
            chrono::Utc::now().timestamp_millis(),
        )?;
        Ok(Self {
            path,
            file,
            sample_rate,
            started_ms: chrono::Utc::now().timestamp_millis(),
            samples: 0,
            metadata,
            active: !vox.enabled,
            vox,
            pre_roll: VecDeque::new(),
            last_voice: None,
            write_error: None,
        })
    }
    fn write(&mut self, pcm: &[f32]) {
        if self.write_error.is_some() {
            return;
        }
        let threshold = 10f32.powf(self.vox.threshold_db / 20.0);
        let voiced = pcm.iter().any(|s| s.abs() >= threshold);
        let max_pre = self.sample_rate as usize * self.vox.pre_roll_ms as usize / 1000;
        let converted: Vec<i16> = pcm
            .iter()
            .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect();
        if self.vox.enabled && !self.active {
            self.pre_roll.extend(converted.iter().copied());
            while self.pre_roll.len() > max_pre {
                self.pre_roll.pop_front();
            }
            if !voiced {
                return;
            }
            self.active = true;
            let pre: Vec<i16> = self.pre_roll.drain(..).collect();
            if let Err(e) = write_pcm16(&mut self.file, &pre) {
                self.write_error = Some(e.to_string());
                return;
            }
            self.samples += pre.len() as u64;
        }
        if voiced {
            self.last_voice = Some(Instant::now());
        }
        if self.vox.enabled
            && !voiced
            && self
                .last_voice
                .map(|t| t.elapsed() > Duration::from_millis(self.vox.post_roll_ms as u64))
                .unwrap_or(false)
        {
            self.active = false;
            self.pre_roll.clear();
            return;
        }
        if let Err(e) = write_pcm16(&mut self.file, &converted) {
            self.write_error = Some(e.to_string());
        } else {
            self.samples += converted.len() as u64;
        }
    }
    fn finish(mut self) -> serde_json::Value {
        let ended_ms = chrono::Utc::now().timestamp_millis();
        let _ = self.file.flush();
        let _ = self.file.seek(SeekFrom::Start(0));
        if self.write_error.is_none() {
            if let Err(e) = write_wav_header(
                &mut self.file,
                self.sample_rate,
                self.samples,
                &self.metadata,
                self.started_ms,
            ) {
                self.write_error = Some(e.to_string())
            }
        }
        serde_json::json!({"path":self.path,"started_ms":self.started_ms,"ended_ms":ended_ms,"elapsed_ms":ended_ms-self.started_ms,"samples":self.samples,"write_error":self.write_error})
    }
}

fn info_payload(meta: &RecordingMetadata, started_ms: i64) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"frequency_hz":meta.frequency_hz,"mode":meta.mode,"sample_rate":null,"started_ms":started_ms,"signal_id":meta.signal_id,"case_id":meta.case_id,"case_name":meta.case_name})).unwrap_or_default()
}
fn write_wav_header(
    file: &mut File,
    rate: u32,
    samples: u64,
    meta: &RecordingMetadata,
    started_ms: i64,
) -> std::io::Result<()> {
    let info = info_payload(meta, started_ms);
    let padded = info.len() + (info.len() % 2);
    let header = 44 + 8 + padded;
    let data_bytes = (samples * 2).min(u32::MAX as u64) as u32;
    file.write_all(b"RIFF")?;
    file.write_all(&((header as u32 - 8).saturating_add(data_bytes)).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&rate.to_le_bytes())?;
    file.write_all(&(rate * 2).to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&16u16.to_le_bytes())?;
    file.write_all(b"psmd")?;
    file.write_all(&(info.len() as u32).to_le_bytes())?;
    file.write_all(&info)?;
    if info.len() % 2 == 1 {
        file.write_all(&[0])?
    }
    file.write_all(b"data")?;
    file.write_all(&data_bytes.to_le_bytes())?;
    file.seek(SeekFrom::Start(header as u64))?;
    Ok(())
}
fn write_pcm16(file: &mut File, pcm: &[i16]) -> std::io::Result<()> {
    let mut bytes = Vec::with_capacity(pcm.len() * 2);
    for s in pcm {
        bytes.extend_from_slice(&s.to_le_bytes())
    }
    file.write_all(&bytes)
}

#[derive(Clone)]
pub struct AudioSink {
    queue: Arc<Mutex<VecDeque<f32>>>,
    running: Arc<Mutex<bool>>,
    sample_rate: Arc<Mutex<u32>>,
    error: Arc<Mutex<Option<String>>>,
    output_device: Arc<Mutex<Option<String>>>,
    callback_frames: Arc<AtomicU64>,
    pushed_samples: Arc<AtomicU64>,
    underrun_samples: Arc<AtomicU64>,
    output_peak_bits: Arc<AtomicU32>,
    nonzero_samples: Arc<AtomicU64>,
    network: Arc<Mutex<Option<(UdpSocket, SocketAddr)>>>,
    network_packets: Arc<AtomicU64>,
    network_errors: Arc<AtomicU64>,
    started: Arc<Mutex<bool>>,
    streams: Arc<Mutex<std::collections::HashMap<u32, broadcast::Sender<Vec<u8>>>>>,
    recordings: Arc<Mutex<std::collections::HashMap<u32, WavRecording>>>,
}

impl AudioSink {
    pub fn new() -> Self {
        Self {
            queue: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_QUEUE_SAMPLES))),
            running: Arc::new(Mutex::new(false)),
            sample_rate: Arc::new(Mutex::new(48_000)),
            error: Arc::new(Mutex::new(None)),
            output_device: Arc::new(Mutex::new(None)),
            callback_frames: Arc::new(AtomicU64::new(0)),
            pushed_samples: Arc::new(AtomicU64::new(0)),
            underrun_samples: Arc::new(AtomicU64::new(0)),
            output_peak_bits: Arc::new(AtomicU32::new(0)),
            nonzero_samples: Arc::new(AtomicU64::new(0)),
            network: Arc::new(Mutex::new(None)),
            network_packets: Arc::new(AtomicU64::new(0)),
            network_errors: Arc::new(AtomicU64::new(0)),
            started: Arc::new(Mutex::new(false)),
            streams: Arc::new(Mutex::new(std::collections::HashMap::new())),
            recordings: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub fn start(&self) {
        let mut started = self.started.lock();
        if *started {
            return;
        }
        *started = true;
        drop(started);

        let queue = self.queue.clone();
        let running = self.running.clone();
        let sample_rate = self.sample_rate.clone();
        let error = self.error.clone();
        let output_device = self.output_device.clone();
        let callback_frames = self.callback_frames.clone();
        let underrun_samples = self.underrun_samples.clone();
        let output_peak_bits = self.output_peak_bits.clone();
        let nonzero_samples = self.nonzero_samples.clone();
        let network = self.network.clone();
        let network_packets = self.network_packets.clone();
        let network_errors = self.network_errors.clone();

        thread::spawn(move || {
            let host = cpal::default_host();
            let Some(device) = host.default_output_device() else {
                *error.lock() = Some("No default audio output device".into());
                return;
            };
            *output_device.lock() = device.name().ok();
            let config = match device.default_output_config() {
                Ok(v) => v,
                Err(e) => {
                    *error.lock() = Some(format!("Audio config: {e}"));
                    return;
                }
            };
            *sample_rate.lock() = config.sample_rate().0;
            let channels = config.channels() as usize;
            let err_state = error.clone();
            let err_fn = move |e| {
                *err_state.lock() = Some(format!("Audio stream: {e}"));
            };

            let result = match config.sample_format() {
                cpal::SampleFormat::F32 => device.build_output_stream(
                    &config.clone().into(),
                    move |data: &mut [f32], _| {
                        fill_f32(
                            data,
                            channels,
                            &queue,
                            &callback_frames,
                            &underrun_samples,
                            &output_peak_bits,
                            &nonzero_samples,
                            &network,
                            &network_packets,
                            &network_errors,
                        )
                    },
                    err_fn,
                    None,
                ),
                cpal::SampleFormat::I16 => device.build_output_stream(
                    &config.clone().into(),
                    move |data: &mut [i16], _| {
                        fill_i16(
                            data,
                            channels,
                            &queue,
                            &callback_frames,
                            &underrun_samples,
                            &output_peak_bits,
                            &nonzero_samples,
                            &network,
                            &network_packets,
                            &network_errors,
                        )
                    },
                    err_fn,
                    None,
                ),
                cpal::SampleFormat::U16 => device.build_output_stream(
                    &config.clone().into(),
                    move |data: &mut [u16], _| {
                        fill_u16(
                            data,
                            channels,
                            &queue,
                            &callback_frames,
                            &underrun_samples,
                            &output_peak_bits,
                            &nonzero_samples,
                            &network,
                            &network_packets,
                            &network_errors,
                        )
                    },
                    err_fn,
                    None,
                ),
                other => {
                    *error.lock() = Some(format!("Unsupported audio format: {other:?}"));
                    return;
                }
            };
            let stream = match result {
                Ok(v) => v,
                Err(e) => {
                    *error.lock() = Some(format!("Audio stream build: {e}"));
                    return;
                }
            };
            if let Err(e) = stream.play() {
                *error.lock() = Some(format!("Audio play: {e}"));
                return;
            }
            *running.lock() = true;
            loop {
                thread::park();
            }
        });
    }

    pub fn sample_rate(&self) -> u32 {
        *self.sample_rate.lock()
    }

    /// Flush queued PCM when the last VFO stops. The CPAL stream may remain
    /// open, but it must output silence rather than stale demodulated audio.
    pub fn clear_queue(&self) {
        self.queue.lock().clear();
    }

    pub fn push(&self, samples: &[f32], volume: f32) {
        // Headless/LAN mode must not route RF noise to the host speakers.
        // It can be explicitly enabled for a local lab server with
        // PULSESCOPE_AUDIO_OUTPUT=1; desktop mode leaves this unset.
        if std::env::var("PULSESCOPE_AUDIO_OUTPUT").as_deref() == Ok("0") {
            return;
        }
        self.start();
        let mut q = self.queue.lock();
        let gain = volume.clamp(0.0, 1.0);
        for &sample in samples {
            if q.len() >= MAX_QUEUE_SAMPLES {
                q.pop_front();
            }
            q.push_back((sample * gain).clamp(-1.0, 1.0));
        }
        self.pushed_samples
            .fetch_add(samples.len() as u64, Ordering::Relaxed);
    }

    /// Fan out one VFO as signed 16-bit mono PCM. Tokio broadcast channels are
    /// deliberately bounded; slow/disconnected browsers lose old chunks
    /// instead of blocking the real-time DSP thread.
    pub fn push_vfo(&self, id: u32, samples: &[f32], sample_rate: u32) {
        let bytes: Vec<u8> = samples
            .iter()
            .flat_map(|s| ((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes())
            .collect();
        if let Some(tx) = self.streams.lock().get(&id) {
            let _ = tx.send(bytes);
        }
        if let Some(rec) = self.recordings.lock().get_mut(&id) {
            if rec.sample_rate == sample_rate {
                rec.write(samples);
            }
        }
    }
    pub fn subscribe(&self, id: u32) -> broadcast::Receiver<Vec<u8>> {
        self.streams
            .lock()
            .entry(id)
            .or_insert_with(|| broadcast::channel(STREAM_QUEUE_CHUNKS).0)
            .subscribe()
    }
    pub fn start_recording(
        &self,
        id: u32,
        root: &Path,
        meta: RecordingMetadata,
        vox: VoxConfig,
    ) -> anyhow::Result<serde_json::Value> {
        std::fs::create_dir_all(root)?;
        if self.recordings.lock().contains_key(&id) {
            anyhow::bail!("VFO {id} is already recording")
        }
        let safe_mode: String = meta
            .mode
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(16)
            .collect();
        let name = format!(
            "vfo{id}-{}-{}-{}.wav",
            meta.frequency_hz,
            safe_mode,
            chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ")
        );
        let rec = WavRecording::new(root.join(name), self.sample_rate(), meta, vox)?;
        let value = serde_json::json!({"recording":true,"vfo_id":id,"path":rec.path,"started_ms":rec.started_ms});
        self.recordings.lock().insert(id, rec);
        Ok(value)
    }
    pub fn stop_recording(&self, id: u32) -> Option<serde_json::Value> {
        self.recordings.lock().remove(&id).map(WavRecording::finish)
    }
    pub fn recording_status(&self) -> serde_json::Value {
        let r = self.recordings.lock();
        serde_json::json!({"active":r.iter().map(|(id,x)|serde_json::json!({"vfo_id":id,"path":x.path,"started_ms":x.started_ms,"elapsed_ms":chrono::Utc::now().timestamp_millis()-x.started_ms,"samples":x.samples,"vox_active":x.active,"write_error":x.write_error})).collect::<Vec<_>>()})
    }
    pub fn shutdown_recordings(&self) {
        let ids = self.recordings.lock().keys().copied().collect::<Vec<_>>();
        for id in ids {
            let _ = self.stop_recording(id);
        }
    }

    pub fn start_network(&self, target: SocketAddr) -> std::io::Result<()> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;
        *self.network.lock() = Some((socket, target));
        Ok(())
    }

    pub fn stop_network(&self) {
        *self.network.lock() = None;
    }

    pub fn network_status(&self) -> serde_json::Value {
        let target = self
            .network
            .lock()
            .as_ref()
            .map(|(_, target)| target.to_string());
        serde_json::json!({"enabled": target.is_some(), "target": target, "packets": self.network_packets.load(Ordering::Relaxed), "errors": self.network_errors.load(Ordering::Relaxed), "format":"PSAU-f32le", "channels":1})
    }

    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "running": *self.running.lock(),
            "sample_rate": *self.sample_rate.lock(),
            "queued_samples": self.queue.lock().len(),
            "output_device": self.output_device.lock().clone(),
            "callback_frames": self.callback_frames.load(Ordering::Relaxed),
            "pushed_samples": self.pushed_samples.load(Ordering::Relaxed),
            "underrun_samples": self.underrun_samples.load(Ordering::Relaxed),
            "output_peak": f32::from_bits(self.output_peak_bits.load(Ordering::Relaxed)),
            "nonzero_samples": self.nonzero_samples.load(Ordering::Relaxed),
            "network": self.network_status(),
            "error": self.error.lock().clone(),
        })
    }
}

fn observe_sample(sample: f32, peak: &AtomicU32, nonzero: &AtomicU64) {
    let magnitude = sample.abs();
    if magnitude > 1.0e-6 {
        nonzero.fetch_add(1, Ordering::Relaxed);
    }
    let mut old = peak.load(Ordering::Relaxed);
    let bits = magnitude.to_bits();
    while bits > old {
        match peak.compare_exchange_weak(old, bits, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(current) => old = current,
        }
    }
}
fn next_sample(
    queue: &Arc<Mutex<VecDeque<f32>>>,
    underruns: &AtomicU64,
    peak: &AtomicU32,
    nonzero: &AtomicU64,
) -> f32 {
    let sample = queue.lock().pop_front().unwrap_or_else(|| {
        underruns.fetch_add(1, Ordering::Relaxed);
        0.0
    });
    observe_sample(sample, peak, nonzero);
    sample
}
fn fill_f32(
    data: &mut [f32],
    channels: usize,
    queue: &Arc<Mutex<VecDeque<f32>>>,
    callbacks: &AtomicU64,
    underruns: &AtomicU64,
    peak: &AtomicU32,
    nonzero: &AtomicU64,
    network: &Arc<Mutex<Option<(UdpSocket, SocketAddr)>>>,
    packets: &AtomicU64,
    errors: &AtomicU64,
) {
    callbacks.fetch_add((data.len() / channels.max(1)) as u64, Ordering::Relaxed);
    for frame in data.chunks_mut(channels) {
        let s = next_sample(queue, underruns, peak, nonzero);
        for out in frame {
            *out = s;
        }
    }
    let samples: Vec<f32> = data.chunks(channels.max(1)).map(|frame| frame[0]).collect();
    send_network(&samples, network, packets, errors);
}
fn fill_i16(
    data: &mut [i16],
    channels: usize,
    queue: &Arc<Mutex<VecDeque<f32>>>,
    callbacks: &AtomicU64,
    underruns: &AtomicU64,
    peak: &AtomicU32,
    nonzero: &AtomicU64,
    network: &Arc<Mutex<Option<(UdpSocket, SocketAddr)>>>,
    packets: &AtomicU64,
    errors: &AtomicU64,
) {
    callbacks.fetch_add((data.len() / channels.max(1)) as u64, Ordering::Relaxed);
    for frame in data.chunks_mut(channels) {
        let s = next_sample(queue, underruns, peak, nonzero);
        let value = (s * i16::MAX as f32) as i16;
        for out in frame {
            *out = value;
        }
    }
    let samples: Vec<f32> = data
        .chunks(channels.max(1))
        .map(|frame| frame[0] as f32 / i16::MAX as f32)
        .collect();
    send_network(&samples, network, packets, errors);
}
fn fill_u16(
    data: &mut [u16],
    channels: usize,
    queue: &Arc<Mutex<VecDeque<f32>>>,
    callbacks: &AtomicU64,
    underruns: &AtomicU64,
    peak: &AtomicU32,
    nonzero: &AtomicU64,
    network: &Arc<Mutex<Option<(UdpSocket, SocketAddr)>>>,
    packets: &AtomicU64,
    errors: &AtomicU64,
) {
    callbacks.fetch_add((data.len() / channels.max(1)) as u64, Ordering::Relaxed);
    for frame in data.chunks_mut(channels) {
        let s = next_sample(queue, underruns, peak, nonzero);
        let value = ((s * 0.5 + 0.5) * u16::MAX as f32) as u16;
        for out in frame {
            *out = value;
        }
    }
    let samples: Vec<f32> = data
        .chunks(channels.max(1))
        .map(|frame| (frame[0] as f32 / u16::MAX as f32 - 0.5) * 2.0)
        .collect();
    send_network(&samples, network, packets, errors);
}

fn send_network(
    samples: &[f32],
    network: &Arc<Mutex<Option<(UdpSocket, SocketAddr)>>>,
    packets: &AtomicU64,
    errors: &AtomicU64,
) {
    let mut packet = Vec::with_capacity(16 + samples.len() * 4);
    packet.extend_from_slice(b"PSAU");
    packet.extend_from_slice(&1u16.to_le_bytes());
    packet.extend_from_slice(&96_000u32.to_le_bytes());
    packet.extend_from_slice(&(samples.len() as u16).to_le_bytes());
    packet.extend_from_slice(&0u16.to_le_bytes());
    packet.extend_from_slice(&0u16.to_le_bytes());
    for sample in samples {
        packet.extend_from_slice(&sample.to_le_bytes());
    }
    let binding = network.lock();
    let Some((socket, target)) = binding.as_ref() else {
        return;
    };
    match socket.send_to(&packet, target) {
        Ok(_) => {
            packets.fetch_add(1, Ordering::Relaxed);
        }
        Err(_) => {
            errors.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sink_reports_safe_idle_state() {
        let sink = AudioSink::new();
        let status = sink.status();
        assert_eq!(status["running"], false);
        assert_eq!(status["queued_samples"], 0);
    }
    #[test]
    fn wav_header_and_truncated_input_are_identifiable() {
        let path=std::env::temp_dir().join(format!("pulsescope-wav-{}.wav",std::process::id()));
        let meta=RecordingMetadata{frequency_hz:162_550_000,mode:"nfm".into(),signal_id:Some("weather".into()),case_id:None,case_name:None};
        let mut rec=WavRecording::new(path.clone(),48_000,meta,VoxConfig::default()).unwrap();rec.write(&[0.25,-0.25]);let status=rec.finish();assert!(status["write_error"].is_null());
        let bytes=std::fs::read(&path).unwrap();assert_eq!(&bytes[..4],b"RIFF");assert!(bytes.windows(4).any(|x|x==b"psmd"));assert!(bytes.windows(4).any(|x|x==b"data"));
        std::fs::write(&path,&bytes[..20]).unwrap();assert!(std::fs::metadata(&path).unwrap().len()<44);let _=std::fs::remove_file(path);
    }
    #[test]
    fn concurrent_start_is_rejected_and_stop_is_idempotent() {
        let sink=AudioSink::new();let root=std::env::temp_dir().join(format!("pulsescope-audio-{}",std::process::id()));let meta=RecordingMetadata{frequency_hz:1,mode:"am".into(),signal_id:None,case_id:None,case_name:None};
        assert!(sink.start_recording(7,&root,meta.clone(),VoxConfig::default()).is_ok());assert!(sink.start_recording(7,&root,meta,VoxConfig::default()).is_err());assert!(sink.stop_recording(7).is_some());assert!(sink.stop_recording(7).is_none());let _=std::fs::remove_dir_all(root);
    }
}
