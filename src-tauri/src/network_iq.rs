//! Network IQ ingest adapters for registered external sources.
//!
//! `raw_udp` uses the PulseScope PSIQ framing already emitted by `IqNetworkSink`.

use parking_lot::Mutex;
use rustfft::num_complex::Complex;
use serde::Serialize;
use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub const PSIQ_MAGIC: &[u8; 4] = b"PSIQ";
pub const PSIQ_HEADER_BYTES: usize = 24;

#[derive(Clone, Debug, Default, Serialize)]
pub struct NetworkIngestCounters {
    pub packets_received: u64,
    pub packets_dropped: u64,
    pub parse_errors: u64,
    pub reconnects: u64,
    pub last_packet_ms: i64,
    pub sample_rate_hz: u32,
    pub center_freq_hz: u64,
    pub queued_samples: usize,
}

pub fn parse_psiq_packet(data: &[u8]) -> anyhow::Result<(u32, u64, Vec<Complex<f32>>)> {
    if data.len() < PSIQ_HEADER_BYTES {
        anyhow::bail!("PSIQ packet too short");
    }
    if &data[0..4] != PSIQ_MAGIC {
        anyhow::bail!("invalid PSIQ magic");
    }
    let version = u16::from_le_bytes([data[4], data[5]]);
    if version != 1 {
        anyhow::bail!("unsupported PSIQ version {version}");
    }
    let sample_rate_hz = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let center_freq_hz = u64::from_le_bytes([
        data[12], data[13], data[14], data[15], data[16], data[17], data[18], data[19],
    ]);
    let sample_count = u32::from_le_bytes([data[20], data[21], data[22], data[23]]) as usize;
    let payload_bytes = sample_count
        .checked_mul(8)
        .ok_or_else(|| anyhow::anyhow!("PSIQ sample count overflow"))?;
    if data.len() < PSIQ_HEADER_BYTES + payload_bytes {
        anyhow::bail!("PSIQ payload truncated");
    }
    let mut samples = Vec::with_capacity(sample_count);
    let mut offset = PSIQ_HEADER_BYTES;
    for _ in 0..sample_count {
        let re = f32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        offset += 4;
        let im = f32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        offset += 4;
        samples.push(Complex::new(re, im));
    }
    Ok((sample_rate_hz, center_freq_hz, samples))
}

struct IngestState {
    queue: VecDeque<Complex<f32>>,
    sample_rate_hz: u32,
    center_freq_hz: u64,
    counters: NetworkIngestCounters,
}

pub struct PsiqUdpIngest {
    _bind_addr: SocketAddr,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<IngestState>>,
    _capacity: usize,
    thread: Mutex<Option<thread::JoinHandle<()>>>,
    last_packet_ms: Arc<AtomicI64>,
    packets_received: Arc<AtomicU64>,
    packets_dropped: Arc<AtomicU64>,
    parse_errors: Arc<AtomicU64>,
    reconnects: Arc<AtomicU64>,
    sample_rate_hz: Arc<AtomicU64>,
    center_freq_hz: Arc<AtomicU64>,
}

impl PsiqUdpIngest {
    pub fn start(host: &str, port: u16, capacity: usize) -> anyhow::Result<Arc<Self>> {
        let bind_host = if host.trim().is_empty() { "0.0.0.0" } else { host.trim() };
        let bind_addr: SocketAddr = format!("{bind_host}:{port}").parse()?;
        let socket = UdpSocket::bind(bind_addr)?;
        socket.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(IngestState {
            queue: VecDeque::with_capacity(capacity.min(262_144)),
            sample_rate_hz: 2_000_000,
            center_freq_hz: 100_000_000,
            counters: NetworkIngestCounters::default(),
        }));
        let last_packet_ms = Arc::new(AtomicI64::new(0));
        let packets_received = Arc::new(AtomicU64::new(0));
        let packets_dropped = Arc::new(AtomicU64::new(0));
        let parse_errors = Arc::new(AtomicU64::new(0));
        let reconnects = Arc::new(AtomicU64::new(0));
        let sample_rate_hz = Arc::new(AtomicU64::new(2_000_000));
        let center_freq_hz = Arc::new(AtomicU64::new(100_000_000));
        let ingest = Arc::new(Self {
            _bind_addr: bind_addr,
            stop: stop.clone(),
            state: state.clone(),
            _capacity: capacity,
            thread: Mutex::new(None),
            last_packet_ms: last_packet_ms.clone(),
            packets_received: packets_received.clone(),
            packets_dropped: packets_dropped.clone(),
            parse_errors: parse_errors.clone(),
            reconnects: reconnects.clone(),
            sample_rate_hz: sample_rate_hz.clone(),
            center_freq_hz: center_freq_hz.clone(),
        });
        let stop_thread = stop.clone();
        let handle = thread::spawn(move || {
            let mut buffer = vec![0u8; 8192];
            let mut consecutive_errors = 0u32;
            let mut last_recovery = Instant::now() - Duration::from_secs(5);
            while !stop_thread.load(Ordering::Acquire) {
                match socket.recv_from(&mut buffer) {
                    Ok((len, _peer)) => {
                        consecutive_errors = 0;
                        match parse_psiq_packet(&buffer[..len]) {
                            Ok((rate, center, samples)) => {
                                packets_received.fetch_add(1, Ordering::Relaxed);
                                let now = crate::scanner::now_ms();
                                last_packet_ms.store(now, Ordering::Relaxed);
                                sample_rate_hz.store(rate as u64, Ordering::Relaxed);
                                center_freq_hz.store(center, Ordering::Relaxed);
                                let mut guard = state.lock();
                                guard.sample_rate_hz = rate;
                                guard.center_freq_hz = center;
                                guard.counters.last_packet_ms = now;
                                guard.counters.sample_rate_hz = rate;
                                guard.counters.center_freq_hz = center;
                                let overflow = guard
                                    .queue
                                    .len()
                                    .saturating_add(samples.len())
                                    .saturating_sub(capacity);
                                if overflow >= guard.queue.len() {
                                    guard.queue.clear();
                                    let skip = overflow.saturating_sub(guard.queue.len());
                                    guard.queue.extend(
                                        samples[skip.min(samples.len())..].iter().copied(),
                                    );
                                } else if overflow > 0 {
                                    guard.queue.drain(..overflow);
                                    guard.queue.extend(samples.iter().copied());
                                } else {
                                    guard.queue.extend(samples.iter().copied());
                                }
                                packets_dropped.fetch_add(overflow as u64, Ordering::Relaxed);
                                guard.counters.packets_received =
                                    packets_received.load(Ordering::Relaxed);
                                guard.counters.packets_dropped =
                                    packets_dropped.load(Ordering::Relaxed);
                                guard.counters.queued_samples = guard.queue.len();
                            }
                            Err(_) => {
                                parse_errors.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            || error.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(_) => {
                        consecutive_errors = consecutive_errors.saturating_add(1);
                        if consecutive_errors >= 8 && last_recovery.elapsed() >= Duration::from_secs(1)
                        {
                            reconnects.fetch_add(1, Ordering::Relaxed);
                            consecutive_errors = 0;
                            last_recovery = Instant::now();
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                }
            }
        });
        *ingest.thread.lock() = Some(handle);
        Ok(ingest)
    }

    pub fn read(&self, count: usize) -> anyhow::Result<Vec<Complex<f32>>> {
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            {
                let mut guard = self.state.lock();
                if guard.queue.len() >= count {
                    let out: Vec<_> = guard.queue.drain(..count).collect();
                    guard.counters.queued_samples = guard.queue.len();
                    return Ok(out);
                }
            }
            if Instant::now() >= deadline {
                anyhow::bail!("network IQ stream timed out waiting for {count} samples");
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn counters(&self) -> NetworkIngestCounters {
        let mut guard = self.state.lock();
        guard.counters.packets_received = self.packets_received.load(Ordering::Relaxed);
        guard.counters.packets_dropped = self.packets_dropped.load(Ordering::Relaxed);
        guard.counters.parse_errors = self.parse_errors.load(Ordering::Relaxed);
        guard.counters.reconnects = self.reconnects.load(Ordering::Relaxed);
        guard.counters.last_packet_ms = self.last_packet_ms.load(Ordering::Relaxed);
        guard.counters.sample_rate_hz = self.sample_rate_hz.load(Ordering::Relaxed) as u32;
        guard.counters.center_freq_hz = self.center_freq_hz.load(Ordering::Relaxed);
        guard.counters.queued_samples = guard.queue.len();
        guard.counters.clone()
    }

    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz.load(Ordering::Relaxed) as u32
    }

    pub fn center_freq_hz(&self) -> u64 {
        self.center_freq_hz.load(Ordering::Relaxed)
    }
}

impl Drop for PsiqUdpIngest {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread.lock().take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::IqNetworkSink;

    fn free_udp_port() -> u16 {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.local_addr().unwrap().port()
    }

    #[test]
    fn parse_psiq_packet_round_trip() {
        let samples = vec![
            Complex::new(1.0_f32, -2.0_f32),
            Complex::new(0.5_f32, 0.25_f32),
        ];
        let mut packet = Vec::new();
        packet.extend_from_slice(PSIQ_MAGIC);
        packet.extend_from_slice(&1u16.to_le_bytes());
        packet.extend_from_slice(&0u16.to_le_bytes());
        packet.extend_from_slice(&2_000_000u32.to_le_bytes());
        packet.extend_from_slice(&915_000_000u64.to_le_bytes());
        packet.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        for sample in &samples {
            packet.extend_from_slice(&sample.re.to_le_bytes());
            packet.extend_from_slice(&sample.im.to_le_bytes());
        }
        let (rate, center, parsed) = parse_psiq_packet(&packet).unwrap();
        assert_eq!(rate, 2_000_000);
        assert_eq!(center, 915_000_000);
        assert_eq!(parsed, samples);
    }

    #[test]
    fn psiq_udp_loopback_preserves_timestamps_and_recovers() {
        let port = free_udp_port();
        let ingest = PsiqUdpIngest::start("127.0.0.1", port, 65_536).unwrap();
        let sink = IqNetworkSink::new();
        sink.start(format!("127.0.0.1:{port}").parse().unwrap())
            .unwrap();
        let samples: Vec<_> = (0..4096)
            .map(|index| Complex::new((index as f32).sin(), (index as f32).cos()))
            .collect();
        sink.send(&samples, 2_000_000, 146_000_000);
        let received = ingest.read(4096).expect("loopback read");
        assert_eq!(received.len(), 4096);
        assert_eq!(ingest.sample_rate_hz(), 2_000_000);
        assert_eq!(ingest.center_freq_hz(), 146_000_000);
        let counters = ingest.counters();
        assert!(counters.packets_received >= 1);
        assert!(counters.last_packet_ms > 0);

        sink.stop();
        thread::sleep(Duration::from_millis(20));
        sink.start(format!("127.0.0.1:{port}").parse().unwrap())
            .unwrap();
        sink.send(&samples[..1024], 2_000_000, 146_500_000);
        let second = ingest.read(1024).expect("recovered read");
        assert_eq!(second.len(), 1024);
        assert_eq!(ingest.center_freq_hz(), 146_500_000);
        let _counters = ingest.counters();
    }
}
