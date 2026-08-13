//! Bounded IQ buffering and the dedicated hardware capture worker.
use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::thread;
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use rustfft::num_complex::Complex;
use crate::device::DeviceLayer;

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
    pub fn new() -> Self { Self { target: Arc::new(Mutex::new(None)), packets: Arc::new(AtomicU64::new(0)), errors: Arc::new(AtomicU64::new(0)) } }
    pub fn start(&self, target: SocketAddr) -> std::io::Result<()> { let socket=UdpSocket::bind("0.0.0.0:0")?; socket.set_nonblocking(true)?; *self.target.lock()=Some((socket,target)); Ok(()) }
    pub fn stop(&self) { *self.target.lock()=None; }
    pub fn status(&self) -> serde_json::Value { let target=self.target.lock().as_ref().map(|(_,a)|a.to_string()); serde_json::json!({"enabled":target.is_some(),"target":target,"packets":self.packets.load(Ordering::Relaxed),"errors":self.errors.load(Ordering::Relaxed),"format":"PSIQ-cf32-le","header_bytes":24}) }
    pub fn send(&self, samples: &[Complex<f32>], rate: u32, center: u64) {
        let guard=self.target.lock(); let Some((socket,target))=guard.as_ref() else{return;};
        let mut packet=Vec::with_capacity(24+samples.len()*8); packet.extend_from_slice(b"PSIQ"); packet.extend_from_slice(&1u16.to_le_bytes()); packet.extend_from_slice(&0u16.to_le_bytes()); packet.extend_from_slice(&rate.to_le_bytes()); packet.extend_from_slice(&center.to_le_bytes()); packet.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        for s in samples { packet.extend_from_slice(&s.re.to_le_bytes()); packet.extend_from_slice(&s.im.to_le_bytes()); }
        match socket.send_to(&packet,target){Ok(_)=>{self.packets.fetch_add(1,Ordering::Relaxed);},Err(_)=>{self.errors.fetch_add(1,Ordering::Relaxed);}}
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
}

impl IqRing {
    pub fn new(name: impl Into<Arc<str>>, capacity: usize) -> Self {
        assert!(capacity > 0, "IQ ring capacity must be nonzero");
        Self { inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))), capacity, name: name.into(), pushed: Arc::new(AtomicU64::new(0)), taken: Arc::new(AtomicU64::new(0)), dropped: Arc::new(AtomicU64::new(0)) }
    }
    pub fn push(&self, samples: &[Complex<f32>]) {
        let mut q = self.inner.lock();
        for &sample in samples {
            if q.len() == self.capacity { q.pop_front(); self.dropped.fetch_add(1, Ordering::Relaxed); }
            q.push_back(sample); self.pushed.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn take_exact(&self, count: usize) -> Option<Vec<Complex<f32>>> {
        let mut q = self.inner.lock();
        if q.len() < count { return None; }
        let out = (0..count).map(|_| q.pop_front().expect("length checked")).collect();
        self.taken.fetch_add(count as u64, Ordering::Relaxed);
        Some(out)
    }
    pub fn len(&self) -> usize { self.inner.lock().len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
    /// Drop samples captured under an earlier tuning/sample-rate contract.
    /// Keeping them would briefly render and demodulate the old band after a
    /// retune, which looks like a frozen VFO and produces a burst of bad audio.
    pub fn clear(&self) { self.inner.lock().clear(); }
    pub fn status(&self) -> serde_json::Value { serde_json::json!({"name":self.name.as_ref(),"capacity_samples":self.capacity,"queued_samples":self.len(),"pushed_samples":self.pushed.load(Ordering::Relaxed),"taken_samples":self.taken.load(Ordering::Relaxed),"dropped_samples":self.dropped.load(Ordering::Relaxed)}) }
}

pub struct PlaybackReader {
    file: File,
    pub path: std::path::PathBuf,
    pub samples_read: u64,
    pub eof: bool,
}
impl PlaybackReader {
    pub fn open(path: std::path::PathBuf) -> anyhow::Result<Self> {
        let mut file = File::open(&path)?;
        file.seek(SeekFrom::Start(0))?;
        Ok(Self { file, path, samples_read: 0, eof: false })
    }
    pub fn read_samples(&mut self, count: usize) -> anyhow::Result<Vec<Complex<f32>>> {
        let mut bytes = vec![0u8; count * 8];
        let n = self.file.read(&mut bytes)?;
        let usable = n - (n % 8);
        if usable == 0 { self.eof = true; return Ok(Vec::new()); }
        let mut out = Vec::with_capacity(usable / 8);
        for chunk in bytes[..usable].chunks_exact(8) {
            out.push(Complex::new(f32::from_le_bytes(chunk[0..4].try_into().unwrap()), f32::from_le_bytes(chunk[4..8].try_into().unwrap())));
        }
        self.samples_read += out.len() as u64;
        Ok(out)
    }
    pub fn status(&self) -> serde_json::Value { serde_json::json!({"playing": !self.eof, "path": self.path, "samples_read": self.samples_read, "format": "cf32-le", "eof": self.eof}) }
}

pub struct CaptureWorker {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl CaptureWorker {
    pub fn start(device: Arc<DeviceLayer>, rings: Vec<IqRing>, chunk_size: usize, playback: Arc<Mutex<Option<PlaybackReader>>>, iq_network: IqNetworkSink) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let stream_mtu = device.stream_mtu();
        let chunk_size = if stream_mtu > 0 { stream_mtu.clamp(512, 262_144) } else { chunk_size };
        let thread = thread::spawn(move || {
            let mut consecutive_errors = 0u32;
            let mut last_recovery = Instant::now() - Duration::from_secs(5);
            while !stop_thread.load(Ordering::Acquire) {
                let (result, playback_active) = {
                    let mut selected = playback.lock();
                    let active = selected.is_some();
                    let result = if let Some(reader) = selected.as_mut() { reader.read_samples(chunk_size) }
                    else { device.read_iq(chunk_size) };
                    (result, active)
                };
                match result {
                    Ok(samples) if !samples.is_empty() => {
                        consecutive_errors = 0;
                        let status = device.status();
                        iq_network.send(&samples, status.sample_rate, status.center_freq_hz);
                        for ring in &rings { ring.push(&samples); }
                        if playback_active || device.status().driver == "mock" {
                            let rate = device.status().sample_rate.max(1) as f64;
                            let seconds = samples.len() as f64 / rate;
                            if seconds > 0.0 { thread::sleep(Duration::from_secs_f64(seconds)); }
                        }
                    }
                    Ok(_) => thread::sleep(Duration::from_millis(1)),
                    Err(_) => {
                        consecutive_errors = consecutive_errors.saturating_add(1);
                        if !playback_active && consecutive_errors >= 8 && last_recovery.elapsed() >= Duration::from_secs(1) {
                            let _ = device.recover();
                            last_recovery = Instant::now();
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                }
            }
        });
        Self { stop, thread: Some(thread) }
    }
}

impl Drop for CaptureWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
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
        let path = std::env::temp_dir().join(format!("pulsescope-playback-{}.cf32", std::process::id()));
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
    fn drops_oldest_when_capture_outpaces_consumer() {
        let ring = IqRing::new("test", 3);
        ring.push(&[Complex::new(1.0, 0.0), Complex::new(2.0, 0.0), Complex::new(3.0, 0.0), Complex::new(4.0, 0.0)]);
        let frame = ring.take_exact(3).unwrap();
        assert_eq!(frame.iter().map(|x| x.re).collect::<Vec<_>>(), vec![2.0, 3.0, 4.0]);
    }
}
