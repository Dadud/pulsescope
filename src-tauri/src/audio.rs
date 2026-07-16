//! Cross-platform PCM output using CPAL.
//!
//! CPAL's native stream handle is deliberately owned by a dedicated thread;
//! AppState only contains thread-safe queue/status handles, so the HTTP server
//! remains Send + Sync on Windows and other platforms.

use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, atomic::{AtomicU32, AtomicU64, Ordering}};
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;

const MAX_QUEUE_SAMPLES: usize = 48_000 * 4;

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
        }
    }

    pub fn start(&self) {
        let mut started = self.started.lock();
        if *started { return; }
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
                Err(e) => { *error.lock() = Some(format!("Audio config: {e}")); return; }
            };
            *sample_rate.lock() = config.sample_rate().0;
            let channels = config.channels() as usize;
            let err_state = error.clone();
            let err_fn = move |e| { *err_state.lock() = Some(format!("Audio stream: {e}")); };

            let result = match config.sample_format() {
                cpal::SampleFormat::F32 => device.build_output_stream(
                    &config.clone().into(),
                    move |data: &mut [f32], _| fill_f32(data, channels, &queue, &callback_frames, &underrun_samples, &output_peak_bits, &nonzero_samples, &network, &network_packets, &network_errors),
                    err_fn, None,
                ),
                cpal::SampleFormat::I16 => device.build_output_stream(
                    &config.clone().into(),
                    move |data: &mut [i16], _| fill_i16(data, channels, &queue, &callback_frames, &underrun_samples, &output_peak_bits, &nonzero_samples, &network, &network_packets, &network_errors),
                    err_fn, None,
                ),
                cpal::SampleFormat::U16 => device.build_output_stream(
                    &config.clone().into(),
                    move |data: &mut [u16], _| fill_u16(data, channels, &queue, &callback_frames, &underrun_samples, &output_peak_bits, &nonzero_samples, &network, &network_packets, &network_errors),
                    err_fn, None,
                ),
                other => {
                    *error.lock() = Some(format!("Unsupported audio format: {other:?}"));
                    return;
                }
            };
            let stream = match result {
                Ok(v) => v,
                Err(e) => { *error.lock() = Some(format!("Audio stream build: {e}")); return; }
            };
            if let Err(e) = stream.play() {
                *error.lock() = Some(format!("Audio play: {e}"));
                return;
            }
            *running.lock() = true;
            loop { thread::park(); }
        });
    }

    pub fn sample_rate(&self) -> u32 { *self.sample_rate.lock() }

    pub fn push(&self, samples: &[f32], volume: f32) {
        self.start();
        let mut q = self.queue.lock();
        let gain = volume.clamp(0.0, 1.0);
        for &sample in samples {
            if q.len() >= MAX_QUEUE_SAMPLES { q.pop_front(); }
            q.push_back((sample * gain).clamp(-1.0, 1.0));
        }
        self.pushed_samples.fetch_add(samples.len() as u64, Ordering::Relaxed);
    }

    pub fn start_network(&self, target: SocketAddr) -> std::io::Result<()> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;
        *self.network.lock() = Some((socket, target));
        Ok(())
    }

    pub fn stop_network(&self) { *self.network.lock() = None; }

    pub fn network_status(&self) -> serde_json::Value {
        let target = self.network.lock().as_ref().map(|(_, target)| target.to_string());
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
    if magnitude > 1.0e-6 { nonzero.fetch_add(1, Ordering::Relaxed); }
    let mut old = peak.load(Ordering::Relaxed);
    let bits = magnitude.to_bits();
    while bits > old {
        match peak.compare_exchange_weak(old, bits, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(current) => old = current,
        }
    }
}
fn next_sample(queue: &Arc<Mutex<VecDeque<f32>>>, underruns: &AtomicU64, peak: &AtomicU32, nonzero: &AtomicU64) -> f32 {
    let sample = queue.lock().pop_front().unwrap_or_else(|| { underruns.fetch_add(1, Ordering::Relaxed); 0.0 });
    observe_sample(sample, peak, nonzero);
    sample
}
fn fill_f32(data: &mut [f32], channels: usize, queue: &Arc<Mutex<VecDeque<f32>>>, callbacks: &AtomicU64, underruns: &AtomicU64, peak: &AtomicU32, nonzero: &AtomicU64, network: &Arc<Mutex<Option<(UdpSocket, SocketAddr)>>>, packets: &AtomicU64, errors: &AtomicU64) {
    callbacks.fetch_add((data.len() / channels.max(1)) as u64, Ordering::Relaxed);
    for frame in data.chunks_mut(channels) { let s = next_sample(queue, underruns, peak, nonzero); for out in frame { *out = s; } }
    let samples: Vec<f32> = data.chunks(channels.max(1)).map(|frame| frame[0]).collect();
    send_network(&samples, network, packets, errors);
}
fn fill_i16(data: &mut [i16], channels: usize, queue: &Arc<Mutex<VecDeque<f32>>>, callbacks: &AtomicU64, underruns: &AtomicU64, peak: &AtomicU32, nonzero: &AtomicU64, network: &Arc<Mutex<Option<(UdpSocket, SocketAddr)>>>, packets: &AtomicU64, errors: &AtomicU64) {
    callbacks.fetch_add((data.len() / channels.max(1)) as u64, Ordering::Relaxed);
    for frame in data.chunks_mut(channels) { let s = next_sample(queue, underruns, peak, nonzero); let value = (s * i16::MAX as f32) as i16; for out in frame { *out = value; } }
    let samples: Vec<f32> = data.chunks(channels.max(1)).map(|frame| frame[0] as f32 / i16::MAX as f32).collect();
    send_network(&samples, network, packets, errors);
}
fn fill_u16(data: &mut [u16], channels: usize, queue: &Arc<Mutex<VecDeque<f32>>>, callbacks: &AtomicU64, underruns: &AtomicU64, peak: &AtomicU32, nonzero: &AtomicU64, network: &Arc<Mutex<Option<(UdpSocket, SocketAddr)>>>, packets: &AtomicU64, errors: &AtomicU64) {
    callbacks.fetch_add((data.len() / channels.max(1)) as u64, Ordering::Relaxed);
    for frame in data.chunks_mut(channels) { let s = next_sample(queue, underruns, peak, nonzero); let value = ((s * 0.5 + 0.5) * u16::MAX as f32) as u16; for out in frame { *out = value; } }
    let samples: Vec<f32> = data.chunks(channels.max(1)).map(|frame| (frame[0] as f32 / u16::MAX as f32 - 0.5) * 2.0).collect();
    send_network(&samples, network, packets, errors);
}

fn send_network(samples: &[f32], network: &Arc<Mutex<Option<(UdpSocket, SocketAddr)>>>, packets: &AtomicU64, errors: &AtomicU64) {
    let mut packet = Vec::with_capacity(16 + samples.len() * 4);
    packet.extend_from_slice(b"PSAU");
    packet.extend_from_slice(&1u16.to_le_bytes());
    packet.extend_from_slice(&96_000u32.to_le_bytes());
    packet.extend_from_slice(&(samples.len() as u16).to_le_bytes());
    packet.extend_from_slice(&0u16.to_le_bytes());
    packet.extend_from_slice(&0u16.to_le_bytes());
    for sample in samples { packet.extend_from_slice(&sample.to_le_bytes()); }
    let binding = network.lock();
    let Some((socket, target)) = binding.as_ref() else { return; };
    match socket.send_to(&packet, target) { Ok(_) => { packets.fetch_add(1, Ordering::Relaxed); }, Err(_) => { errors.fetch_add(1, Ordering::Relaxed); } }
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
}
