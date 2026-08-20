//! Bounded IQ buffering and the dedicated hardware capture worker.
use crate::device::DeviceLayer;
use parking_lot::Mutex;
use rustfft::num_complex::Complex;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::net::{SocketAddr, UdpSocket};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct IqNetworkSink {
    target: Arc<Mutex<Option<(UdpSocket, SocketAddr)>>>,
    packets: Arc<AtomicU64>,
    errors: Arc<AtomicU64>,
}
impl Default for IqNetworkSink {
    fn default() -> Self {
        Self::new()
    }
}

impl IqNetworkSink {
    pub fn new() -> Self {
        Self {
            target: Arc::new(Mutex::new(None)),
            packets: Arc::new(AtomicU64::new(0)),
            errors: Arc::new(AtomicU64::new(0)),
        }
    }
    pub fn start(&self, target: SocketAddr) -> std::io::Result<()> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;
        *self.target.lock() = Some((socket, target));
        Ok(())
    }
    pub fn stop(&self) {
        *self.target.lock() = None;
    }
    pub fn status(&self) -> serde_json::Value {
        let target = self.target.lock().as_ref().map(|(_, a)| a.to_string());
        serde_json::json!({"enabled":target.is_some(),"target":target,"packets":self.packets.load(Ordering::Relaxed),"errors":self.errors.load(Ordering::Relaxed),"format":"PSIQ-cf32-le","header_bytes":24})
    }
    pub fn send(&self, samples: &[Complex<f32>], rate: u32, center: u64) {
        let guard = self.target.lock();
        let Some((socket, target)) = guard.as_ref() else {
            return;
        };
        const HEADER_BYTES: usize = 24;
        const SAMPLE_BYTES: usize = 8;
        const MAX_PAYLOAD: usize = 1400;
        let max_samples = (MAX_PAYLOAD - HEADER_BYTES) / SAMPLE_BYTES;
        if max_samples == 0 {
            return;
        }
        let mut offset = 0;
        while offset < samples.len() {
            let chunk = samples[offset..].len().min(max_samples);
            let slice = &samples[offset..offset + chunk];
            offset += chunk;
            let mut packet = Vec::with_capacity(HEADER_BYTES + slice.len() * SAMPLE_BYTES);
            packet.extend_from_slice(b"PSIQ");
            packet.extend_from_slice(&1u16.to_le_bytes());
            packet.extend_from_slice(&0u16.to_le_bytes());
            packet.extend_from_slice(&rate.to_le_bytes());
            packet.extend_from_slice(&center.to_le_bytes());
            packet.extend_from_slice(&(slice.len() as u32).to_le_bytes());
            for s in slice {
                packet.extend_from_slice(&s.re.to_le_bytes());
                packet.extend_from_slice(&s.im.to_le_bytes());
            }
            match socket.send_to(&packet, target) {
                Ok(_) => {
                    self.packets.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    self.errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct IqRing {
    inner: Arc<Mutex<VecDeque<Complex<f32>>>>,
    capacity: usize,
    name: Arc<str>,
    pushed: Arc<AtomicU64>,
    taken: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    skipped: Arc<AtomicU64>,
    latest_only: bool,
}

impl IqRing {
    pub fn new(name: impl Into<Arc<str>>, capacity: usize) -> Self {
        assert!(capacity > 0, "IQ ring capacity must be nonzero");
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
            name: name.into(),
            pushed: Arc::new(AtomicU64::new(0)),
            taken: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
            skipped: Arc::new(AtomicU64::new(0)),
            latest_only: false,
        }
    }
    pub fn new_latest(name: impl Into<Arc<str>>, capacity: usize) -> Self {
        Self {
            latest_only: true,
            ..Self::new(name, capacity)
        }
    }
    pub fn push(&self, samples: &[Complex<f32>]) {
        let mut q = self.inner.lock();
        let queued = q.len();
        let overflow = queued
            .saturating_add(samples.len())
            .saturating_sub(self.capacity);
        if overflow >= queued {
            q.clear();
            let incoming_skip = overflow - queued;
            q.extend(samples[incoming_skip.min(samples.len())..].iter().copied());
        } else if overflow > 0 {
            q.drain(..overflow);
            q.extend(samples.iter().copied());
        } else {
            q.extend(samples.iter().copied());
        }
        self.pushed
            .fetch_add(samples.len() as u64, Ordering::Relaxed);
        if self.latest_only {
            self.skipped.fetch_add(overflow as u64, Ordering::Relaxed);
        } else {
            self.dropped.fetch_add(overflow as u64, Ordering::Relaxed);
        }
    }
    pub fn take_exact(&self, count: usize) -> Option<Vec<Complex<f32>>> {
        let mut q = self.inner.lock();
        if q.len() < count {
            return None;
        }
        let out = (0..count)
            .map(|_| q.pop_front().expect("length checked"))
            .collect();
        self.taken.fetch_add(count as u64, Ordering::Relaxed);
        Some(out)
    }
    /// Return the newest complete frame and discard obsolete queued history.
    /// Spectrum consumers need current RF state, not every intermediate IQ
    /// sample; audio and decoder consumers continue to use `take_exact`.
    pub fn take_latest_exact(&self, count: usize) -> Option<Vec<Complex<f32>>> {
        let mut q = self.inner.lock();
        if q.len() < count {
            return None;
        }
        let skip = q.len() - count;
        if skip > 0 {
            q.drain(..skip);
            self.skipped.fetch_add(skip as u64, Ordering::Relaxed);
        }
        let out = q.drain(..count).collect();
        self.taken.fetch_add(count as u64, Ordering::Relaxed);
        Some(out)
    }
    /// Copy the newest samples without consuming them. Identify / RDS / CTCSS
    /// / APRS endpoints must not steal IQ from the capture worker.
    pub fn copy_latest(&self, count: usize) -> Option<Vec<Complex<f32>>> {
        if count == 0 {
            return None;
        }
        let q = self.inner.lock();
        if q.is_empty() {
            return None;
        }
        let take = count.min(q.len());
        let skip = q.len() - take;
        Some(q.iter().skip(skip).copied().collect())
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Drop samples captured under an earlier tuning/sample-rate contract.
    /// Keeping them would briefly render and demodulate the old band after a
    /// retune, which looks like a frozen VFO and produces a burst of bad audio.
    pub fn clear(&self) {
        self.inner.lock().clear();
    }
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({"name":self.name.as_ref(),"capacity_samples":self.capacity,"queued_samples":self.len(),"pushed_samples":self.pushed.load(Ordering::Relaxed),"taken_samples":self.taken.load(Ordering::Relaxed),"dropped_samples":self.dropped.load(Ordering::Relaxed),"skipped_samples":self.skipped.load(Ordering::Relaxed)})
    }
}

pub struct PlaybackReader {
    file: File,
    pub path: std::path::PathBuf,
    pub samples_read: u64,
    pub total_samples: u64,
    pub eof: bool,
}
impl PlaybackReader {
    pub fn open(path: std::path::PathBuf) -> anyhow::Result<Self> {
        let mut file = File::open(&path)?;
        let total_samples = file.metadata()?.len() / 8;
        file.seek(SeekFrom::Start(0))?;
        Ok(Self {
            file,
            path,
            samples_read: 0,
            total_samples,
            eof: false,
        })
    }
    pub fn seek_samples(&mut self, offset: u64) -> anyhow::Result<()> {
        let clamped = offset.min(self.total_samples);
        self.file.seek(SeekFrom::Start(clamped * 8))?;
        self.samples_read = clamped;
        self.eof = clamped >= self.total_samples;
        Ok(())
    }
    pub fn read_samples(&mut self, count: usize) -> anyhow::Result<Vec<Complex<f32>>> {
        let mut bytes = vec![0u8; count * 8];
        let n = self.file.read(&mut bytes)?;
        let usable = n - (n % 8);
        if usable == 0 {
            self.eof = true;
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(usable / 8);
        for chunk in bytes[..usable].chunks_exact(8) {
            out.push(Complex::new(
                f32::from_le_bytes(chunk[0..4].try_into().unwrap()),
                f32::from_le_bytes(chunk[4..8].try_into().unwrap()),
            ));
        }
        self.samples_read += out.len() as u64;
        Ok(out)
    }
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "playing": !self.eof,
            "path": self.path,
            "samples_read": self.samples_read,
            "total_samples": self.total_samples,
            "progress": if self.total_samples == 0 { 0.0 } else { self.samples_read as f64 / self.total_samples as f64 },
            "format": "cf32-le",
            "eof": self.eof
        })
    }
}

pub struct CaptureWorker {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl CaptureWorker {
    pub fn start(
        device: Arc<DeviceLayer>,
        rings: Vec<IqRing>,
        chunk_size: usize,
        playback: Arc<Mutex<Option<PlaybackReader>>>,
        iq_network: IqNetworkSink,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let stream_mtu = device.stream_mtu();
        let chunk_size = if stream_mtu > 0 {
            stream_mtu.clamp(512, 262_144)
        } else {
            chunk_size
        };
        let thread = thread::spawn(move || {
            let mut consecutive_errors = 0u32;
            let mut last_recovery = Instant::now() - Duration::from_secs(5);
            while !stop_thread.load(Ordering::Acquire) {
                let (result, playback_active) = {
                    let mut selected = playback.lock();
                    let active = selected.is_some();
                    let result = if let Some(reader) = selected.as_mut() {
                        reader.read_samples(chunk_size)
                    } else {
                        device.read_iq(chunk_size)
                    };
                    (result, active)
                };
                match result {
                    Ok(samples) if !samples.is_empty() => {
                        consecutive_errors = 0;
                        let status = device.status();
                        iq_network.send(&samples, status.sample_rate, status.center_freq_hz);
                        for ring in &rings {
                            ring.push(&samples);
                        }
                        if playback_active || device.status().driver == "mock" {
                            let rate = device.status().sample_rate.max(1) as f64;
                            let seconds = samples.len() as f64 / rate;
                            if seconds > 0.0 {
                                thread::sleep(Duration::from_secs_f64(seconds));
                            }
                        }
                    }
                    Ok(_) => thread::sleep(Duration::from_millis(1)),
                    Err(_) => {
                        consecutive_errors = consecutive_errors.saturating_add(1);
                        if !playback_active
                            && consecutive_errors >= 8
                            && last_recovery.elapsed() >= Duration::from_secs(1)
                        {
                            let _ = device.recover();
                            last_recovery = Instant::now();
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                }
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for CaptureWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_order_and_requires_complete_frames() {
        let ring = IqRing::new("test", 8);
        ring.push(&[Complex::new(1.0, 0.0), Complex::new(2.0, 0.0)]);
        assert!(ring.take_exact(3).is_none());
        let frame = ring.take_exact(2).unwrap();
        assert_eq!(frame[0].re, 1.0);
        assert_eq!(frame[1].re, 2.0);
    }
    #[test]
    fn playback_reader_decodes_cf32_le_and_reports_eof() {
        let path =
            std::env::temp_dir().join(format!("pulsescope-playback-{}.cf32", std::process::id()));
        let mut bytes = Vec::new();
        for (re, im) in [(1.0f32, -2.0f32), (0.25, 0.5)] {
            bytes.extend_from_slice(&re.to_le_bytes());
            bytes.extend_from_slice(&im.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        let mut reader = PlaybackReader::open(path.clone()).unwrap();
        let samples = reader.read_samples(2).unwrap();
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0], Complex::new(1.0, -2.0));
        assert_eq!(samples[1], Complex::new(0.25, 0.5));
        assert_eq!(reader.samples_read, 2);
        assert!(reader.read_samples(1).unwrap().is_empty());
        assert!(reader.eof);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn playback_reader_seek_samples_updates_progress() {
        let path = std::env::temp_dir().join(format!(
            "pulsescope-playback-seek-{}.cf32",
            std::process::id()
        ));
        let mut bytes = Vec::new();
        for value in [1.0f32, 2.0, 3.0, 4.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
            bytes.extend_from_slice(&0.0f32.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        let mut reader = PlaybackReader::open(path.clone()).unwrap();
        assert_eq!(reader.total_samples, 4);
        reader.seek_samples(2).unwrap();
        let samples = reader.read_samples(2).unwrap();
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].re, 3.0);
        reader.seek_samples(99).unwrap();
        assert_eq!(reader.samples_read, 4);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn drops_oldest_when_capture_outpaces_consumer() {
        let ring = IqRing::new("test", 3);
        ring.push(&[
            Complex::new(1.0, 0.0),
            Complex::new(2.0, 0.0),
            Complex::new(3.0, 0.0),
            Complex::new(4.0, 0.0),
        ]);
        let frame = ring.take_exact(3).unwrap();
        assert_eq!(
            frame.iter().map(|x| x.re).collect::<Vec<_>>(),
            vec![2.0, 3.0, 4.0]
        );
    }
    #[test]
    fn latest_frame_skips_obsolete_history_without_overflow() {
        let ring = IqRing::new_latest("fft", 8);
        ring.push(
            &(1..=8)
                .map(|value| Complex::new(value as f32, 0.0))
                .collect::<Vec<_>>(),
        );
        let frame = ring.take_latest_exact(3).unwrap();
        assert_eq!(
            frame.iter().map(|sample| sample.re).collect::<Vec<_>>(),
            vec![6.0, 7.0, 8.0]
        );
        let status = ring.status();
        assert_eq!(status["dropped_samples"], 0);
        assert_eq!(status["skipped_samples"], 5);
    }
    #[test]
    fn latest_ring_classifies_capacity_discard_as_skip() {
        let ring = IqRing::new_latest("fft", 3);
        ring.push(
            &(1..=5)
                .map(|value| Complex::new(value as f32, 0.0))
                .collect::<Vec<_>>(),
        );
        let status = ring.status();
        assert_eq!(status["dropped_samples"], 0);
        assert_eq!(status["skipped_samples"], 2);
    }

    #[test]
    fn copy_latest_does_not_consume_queued_samples() {
        let ring = IqRing::new_latest("snapshot", 8);
        ring.push(
            &(1..=6)
                .map(|value| Complex::new(value as f32, 0.0))
                .collect::<Vec<_>>(),
        );
        let copied = ring.copy_latest(3).unwrap();
        assert_eq!(
            copied.iter().map(|sample| sample.re).collect::<Vec<_>>(),
            vec![4.0, 5.0, 6.0]
        );
        assert_eq!(ring.len(), 6);
        assert_eq!(ring.status()["taken_samples"], 0);
        let taken = ring.take_latest_exact(3).unwrap();
        assert_eq!(
            taken.iter().map(|sample| sample.re).collect::<Vec<_>>(),
            vec![4.0, 5.0, 6.0]
        );
    }
}
