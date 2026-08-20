//! Network IQ ingest adapters for registered external sources.
//!
//! Implemented kinds:
//! - `raw_udp`: PulseScope PSIQ framing already emitted by `IqNetworkSink`
//! - `rtl_tcp`: rtl_tcp dongle header plus unsigned 8-bit IQ, with live retune
//! - `spyserver`: Airspy SpyServer / SDR++ 2.x command and IQ messages
//! - `ka9q`: KA9Q radiod RTP v2 or raw s16le IQ over UDP, including multicast
//!
//! KiwiSDR is not implemented. PSIQ and KA9Q are receive-only; center and rate
//! on those links come from packet metadata or operator display state.

use parking_lot::Mutex;
use rustfft::num_complex::Complex;
use serde::Serialize;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
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
        let bind_host = if host.trim().is_empty() {
            "0.0.0.0"
        } else {
            host.trim()
        };
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
                                    guard
                                        .queue
                                        .extend(samples[skip.min(samples.len())..].iter().copied());
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
                        if consecutive_errors >= 8
                            && last_recovery.elapsed() >= Duration::from_secs(1)
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

    pub fn tune(&self, _center: Option<u64>, _rate: Option<u32>) -> anyhow::Result<()> {
        Ok(())
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

pub fn network_kind_supported(kind: &str) -> bool {
    matches!(kind, "raw_udp" | "rtl_tcp" | "spyserver" | "ka9q")
}

pub fn implemented_network_kinds() -> &'static [&'static str] {
    &["raw_udp", "rtl_tcp", "spyserver", "ka9q"]
}

fn wait_for_queued_samples(
    state: &Mutex<IngestState>,
    count: usize,
    what: &str,
) -> anyhow::Result<Vec<Complex<f32>>> {
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        {
            let mut guard = state.lock();
            if guard.queue.len() >= count {
                let out: Vec<_> = guard.queue.drain(..count).collect();
                guard.counters.queued_samples = guard.queue.len();
                return Ok(out);
            }
        }
        if Instant::now() >= deadline {
            anyhow::bail!("{what} timed out waiting for {count} samples");
        }
        thread::sleep(Duration::from_millis(1));
    }
}

struct CounterAtoms<'a> {
    packets_received: &'a AtomicU64,
    packets_dropped: &'a AtomicU64,
    parse_errors: &'a AtomicU64,
    reconnects: &'a AtomicU64,
    last_packet_ms: &'a AtomicI64,
    sample_rate_hz: &'a AtomicU64,
    center_freq_hz: &'a AtomicU64,
}

fn snapshot_counters(state: &Mutex<IngestState>, atoms: CounterAtoms<'_>) -> NetworkIngestCounters {
    let mut guard = state.lock();
    guard.counters.packets_received = atoms.packets_received.load(Ordering::Relaxed);
    guard.counters.packets_dropped = atoms.packets_dropped.load(Ordering::Relaxed);
    guard.counters.parse_errors = atoms.parse_errors.load(Ordering::Relaxed);
    guard.counters.reconnects = atoms.reconnects.load(Ordering::Relaxed);
    guard.counters.last_packet_ms = atoms.last_packet_ms.load(Ordering::Relaxed);
    guard.counters.sample_rate_hz = atoms.sample_rate_hz.load(Ordering::Relaxed) as u32;
    guard.counters.center_freq_hz = atoms.center_freq_hz.load(Ordering::Relaxed);
    guard.counters.queued_samples = guard.queue.len();
    guard.counters.clone()
}

fn require_u32_hz(label: &str, value: u64) -> anyhow::Result<u32> {
    u32::try_from(value).map_err(|_| anyhow::anyhow!("{label} {value} Hz does not fit in 32 bits"))
}

fn push_samples(state: &Mutex<IngestState>, capacity: usize, samples: &[Complex<f32>]) -> u64 {
    let mut guard = state.lock();
    let overflow = guard
        .queue
        .len()
        .saturating_add(samples.len())
        .saturating_sub(capacity);
    if overflow >= guard.queue.len() {
        guard.queue.clear();
        let skip = overflow.saturating_sub(guard.queue.len());
        guard
            .queue
            .extend(samples[skip.min(samples.len())..].iter().copied());
    } else if overflow > 0 {
        guard.queue.drain(..overflow);
        guard.queue.extend(samples.iter().copied());
    } else {
        guard.queue.extend(samples.iter().copied());
    }
    guard.counters.queued_samples = guard.queue.len();
    overflow as u64
}

fn rtl_tcp_sample(i: u8, q: u8) -> Complex<f32> {
    Complex::new(
        (f32::from(i) - 127.5) / 128.0,
        (f32::from(q) - 127.5) / 128.0,
    )
}

pub fn decode_rtl_tcp_iq(bytes: &[u8]) -> Vec<Complex<f32>> {
    let mut samples = Vec::with_capacity(bytes.len() / 2);
    let mut index = 0;
    while index + 1 < bytes.len() {
        samples.push(rtl_tcp_sample(bytes[index], bytes[index + 1]));
        index += 2;
    }
    samples
}

const RTL_TCP_DONGLE_MAGIC: u32 = 0x1234_5678;

fn send_rtl_tcp_u32(stream: &mut TcpStream, command: u8, value: u32) -> std::io::Result<()> {
    stream.write_all(&[command])?;
    stream.write_all(&value.to_le_bytes())
}

fn send_rtl_tcp_tune(
    stream: &mut TcpStream,
    sample_rate_hz: u32,
    center_freq_hz: u32,
) -> std::io::Result<()> {
    send_rtl_tcp_u32(stream, 0x02, sample_rate_hz.max(1))?;
    send_rtl_tcp_u32(stream, 0x01, center_freq_hz)
}

fn read_rtl_tcp_dongle(stream: &mut TcpStream) -> anyhow::Result<()> {
    let mut dongle = [0u8; 12];
    stream.read_exact(&mut dongle)?;
    let magic = u32::from_le_bytes([dongle[0], dongle[1], dongle[2], dongle[3]]);
    if magic != RTL_TCP_DONGLE_MAGIC {
        anyhow::bail!("unexpected RTL-TCP dongle magic {magic:#x}");
    }
    Ok(())
}

pub struct RtlTcpIngest {
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
    tune_epoch: Arc<AtomicU64>,
}

impl RtlTcpIngest {
    pub fn connect(
        host: &str,
        port: u16,
        capacity: usize,
        center_freq_hz: u64,
        sample_rate_hz: u32,
    ) -> anyhow::Result<Arc<Self>> {
        let addr = format!("{}:{}", host.trim(), port);
        let center = require_u32_hz("RTL-TCP frequency", center_freq_hz)?;
        let mut stream = TcpStream::connect(&addr)?;
        read_rtl_tcp_dongle(&mut stream)?;
        send_rtl_tcp_tune(&mut stream, sample_rate_hz.max(1), center)?;
        stream.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(IngestState {
            queue: VecDeque::with_capacity(capacity.min(262_144)),
            sample_rate_hz: sample_rate_hz.max(1),
            center_freq_hz,
            counters: NetworkIngestCounters::default(),
        }));
        let last_packet_ms = Arc::new(AtomicI64::new(0));
        let packets_received = Arc::new(AtomicU64::new(0));
        let packets_dropped = Arc::new(AtomicU64::new(0));
        let parse_errors = Arc::new(AtomicU64::new(0));
        let reconnects = Arc::new(AtomicU64::new(0));
        let sample_rate_hz = Arc::new(AtomicU64::new(sample_rate_hz as u64));
        let center_freq_hz = Arc::new(AtomicU64::new(center_freq_hz));
        let tune_epoch = Arc::new(AtomicU64::new(0));
        let ingest = Arc::new(Self {
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
            tune_epoch: tune_epoch.clone(),
        });
        let stop_thread = stop.clone();
        let reconnect_target = addr.clone();
        let handle = thread::spawn(move || {
            let mut stream = stream;
            let mut buffer = vec![0u8; 8192];
            let mut pending = Vec::<u8>::new();
            let mut consecutive_errors = 0u32;
            let mut last_recovery = Instant::now() - Duration::from_secs(5);
            let mut applied_epoch = 0u64;
            while !stop_thread.load(Ordering::Acquire) {
                let epoch = tune_epoch.load(Ordering::Relaxed);
                if epoch != applied_epoch {
                    if send_rtl_tcp_tune(
                        &mut stream,
                        sample_rate_hz.load(Ordering::Relaxed) as u32,
                        center_freq_hz.load(Ordering::Relaxed) as u32,
                    )
                    .is_ok()
                    {
                        applied_epoch = epoch;
                    } else {
                        consecutive_errors = consecutive_errors.saturating_add(1);
                    }
                }
                match stream.read(&mut buffer) {
                    Ok(0) => {
                        consecutive_errors = consecutive_errors.saturating_add(1);
                    }
                    Ok(len) => {
                        consecutive_errors = 0;
                        pending.extend_from_slice(&buffer[..len]);
                        let usable = pending.len() - (pending.len() % 2);
                        if usable >= 2 {
                            let samples = decode_rtl_tcp_iq(&pending[..usable]);
                            pending.drain(..usable);
                            packets_received.fetch_add(1, Ordering::Relaxed);
                            let now = crate::scanner::now_ms();
                            last_packet_ms.store(now, Ordering::Relaxed);
                            let dropped = push_samples(&state, capacity, &samples);
                            packets_dropped.fetch_add(dropped, Ordering::Relaxed);
                            let mut guard = state.lock();
                            guard.counters.packets_received =
                                packets_received.load(Ordering::Relaxed);
                            guard.counters.packets_dropped =
                                packets_dropped.load(Ordering::Relaxed);
                            guard.counters.last_packet_ms = now;
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
                    }
                }
                if consecutive_errors >= 8 && last_recovery.elapsed() >= Duration::from_secs(1) {
                    reconnects.fetch_add(1, Ordering::Relaxed);
                    consecutive_errors = 0;
                    last_recovery = Instant::now();
                    pending.clear();
                    match TcpStream::connect(&reconnect_target) {
                        Ok(mut next) => {
                            let _ = next.set_read_timeout(Some(Duration::from_millis(500)));
                            if read_rtl_tcp_dongle(&mut next).is_ok() {
                                let _ = send_rtl_tcp_tune(
                                    &mut next,
                                    sample_rate_hz.load(Ordering::Relaxed) as u32,
                                    center_freq_hz.load(Ordering::Relaxed) as u32,
                                );
                            }
                            let _ = next.set_nonblocking(true);
                            stream = next;
                            applied_epoch = tune_epoch.load(Ordering::Relaxed);
                        }
                        Err(_) => thread::sleep(Duration::from_millis(50)),
                    }
                }
            }
        });
        *ingest.thread.lock() = Some(handle);
        Ok(ingest)
    }

    pub fn read(&self, count: usize) -> anyhow::Result<Vec<Complex<f32>>> {
        wait_for_queued_samples(&self.state, count, "RTL-TCP stream")
    }

    pub fn counters(&self) -> NetworkIngestCounters {
        snapshot_counters(
            &self.state,
            CounterAtoms {
                packets_received: &self.packets_received,
                packets_dropped: &self.packets_dropped,
                parse_errors: &self.parse_errors,
                reconnects: &self.reconnects,
                last_packet_ms: &self.last_packet_ms,
                sample_rate_hz: &self.sample_rate_hz,
                center_freq_hz: &self.center_freq_hz,
            },
        )
    }

    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz.load(Ordering::Relaxed) as u32
    }

    pub fn center_freq_hz(&self) -> u64 {
        self.center_freq_hz.load(Ordering::Relaxed)
    }

    pub fn tune(&self, center: Option<u64>, rate: Option<u32>) -> anyhow::Result<()> {
        if let Some(center) = center {
            let _ = require_u32_hz("RTL-TCP frequency", center)?;
            self.center_freq_hz.store(center, Ordering::Relaxed);
        }
        if let Some(rate) = rate {
            if rate == 0 {
                anyhow::bail!("sample rate must be greater than zero");
            }
            self.sample_rate_hz
                .store(u64::from(rate), Ordering::Relaxed);
        }
        self.tune_epoch.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

impl Drop for RtlTcpIngest {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread.lock().take() {
            let _ = handle.join();
        }
    }
}

pub const SPYSERVER_PROTOCOL_VERSION: u32 = (2 << 24) | 1700;
pub const SPY_CMD_HELLO: u32 = 0;
pub const SPY_CMD_SET_SETTING: u32 = 2;
pub const SPY_MSG_DEVICE_INFO: u32 = 0;
pub const SPY_MSG_UINT8_IQ: u32 = 100;
pub const SPY_MSG_INT16_IQ: u32 = 101;
pub const SPY_MSG_INT24_IQ: u32 = 102;
pub const SPY_MSG_FLOAT_IQ: u32 = 103;
pub const SPY_SETTING_STREAMING_MODE: u32 = 0;
pub const SPY_SETTING_STREAMING_ENABLED: u32 = 1;
pub const SPY_SETTING_IQ_FORMAT: u32 = 100;
pub const SPY_SETTING_IQ_FREQUENCY: u32 = 101;
pub const SPY_SETTING_IQ_DECIMATION: u32 = 102;
pub const SPY_STREAM_TYPE_IQ: u32 = 1;
pub const SPY_STREAM_FORMAT_FLOAT: u32 = 4;
const SPYSERVER_HEADER_BYTES: usize = 20;
const SPYSERVER_MAX_BODY: usize = 1_048_576;

fn u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

pub fn parse_spyserver_header(data: &[u8]) -> anyhow::Result<(u32, u32, u32, u32, u32)> {
    if data.len() < SPYSERVER_HEADER_BYTES {
        anyhow::bail!("SpyServer header truncated");
    }
    Ok((
        u32_le(data, 0),
        u32_le(data, 4),
        u32_le(data, 8),
        u32_le(data, 12),
        u32_le(data, 16),
    ))
}

pub fn decode_s16le_iq(bytes: &[u8]) -> Vec<Complex<f32>> {
    bytes
        .chunks_exact(4)
        .map(|chunk| {
            let i = i16::from_le_bytes([chunk[0], chunk[1]]);
            let q = i16::from_le_bytes([chunk[2], chunk[3]]);
            Complex::new(f32::from(i) / 32768.0, f32::from(q) / 32768.0)
        })
        .collect()
}

pub fn decode_f32le_iq(bytes: &[u8]) -> Vec<Complex<f32>> {
    bytes
        .chunks_exact(8)
        .map(|chunk| {
            Complex::new(
                f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
                f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]),
            )
        })
        .collect()
}

pub fn decode_s24le_iq(bytes: &[u8]) -> Vec<Complex<f32>> {
    bytes
        .chunks_exact(6)
        .map(|chunk| {
            let sample = |lo: u8, mid: u8, hi: u8| {
                let mut raw = u32::from(lo) | (u32::from(mid) << 8) | (u32::from(hi) << 16);
                if raw & 0x80_0000 != 0 {
                    raw |= 0xFF00_0000;
                }
                (raw as i32 as f32) / 8_388_608.0
            };
            Complex::new(
                sample(chunk[0], chunk[1], chunk[2]),
                sample(chunk[3], chunk[4], chunk[5]),
            )
        })
        .collect()
}

pub fn decode_spyserver_iq(message_type: u32, body: &[u8]) -> anyhow::Result<Vec<Complex<f32>>> {
    match message_type {
        SPY_MSG_UINT8_IQ => Ok(decode_rtl_tcp_iq(body)),
        SPY_MSG_INT16_IQ => Ok(decode_s16le_iq(body)),
        SPY_MSG_INT24_IQ => Ok(decode_s24le_iq(body)),
        SPY_MSG_FLOAT_IQ => Ok(decode_f32le_iq(body)),
        _ => anyhow::bail!("SpyServer message {message_type} is not IQ"),
    }
}

pub fn spy_decimation_for_rate(max_rate: u32, stages: u32, requested: u32) -> Option<u32> {
    if max_rate == 0 || requested == 0 {
        return None;
    }
    for stage in 0..=stages.min(16) {
        let divisor = 1u32.checked_shl(stage)?;
        if max_rate / divisor == requested {
            return Some(stage);
        }
    }
    None
}

fn send_spy_command(stream: &mut TcpStream, command: u32, body: &[u8]) -> std::io::Result<()> {
    stream.write_all(&command.to_le_bytes())?;
    stream.write_all(&(body.len() as u32).to_le_bytes())?;
    stream.write_all(body)
}

fn send_spy_setting(stream: &mut TcpStream, setting: u32, value: u32) -> std::io::Result<()> {
    let mut body = [0u8; 8];
    body[..4].copy_from_slice(&setting.to_le_bytes());
    body[4..].copy_from_slice(&value.to_le_bytes());
    send_spy_command(stream, SPY_CMD_SET_SETTING, &body)
}

fn send_spy_hello(stream: &mut TcpStream) -> std::io::Result<()> {
    let mut body = Vec::from(SPYSERVER_PROTOCOL_VERSION.to_le_bytes());
    body.extend_from_slice(b"PulseScope");
    send_spy_command(stream, SPY_CMD_HELLO, &body)
}

fn spy_handshake(stream: &mut TcpStream, center: u32) -> std::io::Result<()> {
    let _ = stream.set_nodelay(true);
    send_spy_hello(stream)?;
    send_spy_setting(stream, SPY_SETTING_STREAMING_MODE, SPY_STREAM_TYPE_IQ)?;
    send_spy_setting(stream, SPY_SETTING_IQ_FORMAT, SPY_STREAM_FORMAT_FLOAT)?;
    send_spy_setting(stream, SPY_SETTING_IQ_FREQUENCY, center)?;
    send_spy_setting(stream, SPY_SETTING_STREAMING_ENABLED, 1)
}

fn apply_spyserver_device_info(body: &[u8], max_sample_rate: &AtomicU64, stages: &AtomicU64) {
    if body.len() >= 20 {
        max_sample_rate.store(u64::from(u32_le(body, 8)), Ordering::Relaxed);
        stages.store(u64::from(u32_le(body, 16)), Ordering::Relaxed);
    }
}

pub struct SpyServerIngest {
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
    tune_epoch: Arc<AtomicU64>,
    max_sample_rate: Arc<AtomicU64>,
    decimation_stages: Arc<AtomicU64>,
}

impl SpyServerIngest {
    pub fn connect(
        host: &str,
        port: u16,
        capacity: usize,
        center_freq_hz: u64,
        sample_rate_hz: u32,
    ) -> anyhow::Result<Arc<Self>> {
        let addr = format!("{}:{}", host.trim(), port);
        let center = require_u32_hz("SpyServer frequency", center_freq_hz)?;
        let mut stream = TcpStream::connect(&addr)?;
        spy_handshake(&mut stream, center)?;
        stream.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(IngestState {
            queue: VecDeque::with_capacity(capacity.min(262_144)),
            sample_rate_hz: sample_rate_hz.max(1),
            center_freq_hz,
            counters: NetworkIngestCounters::default(),
        }));
        let last_packet_ms = Arc::new(AtomicI64::new(0));
        let packets_received = Arc::new(AtomicU64::new(0));
        let packets_dropped = Arc::new(AtomicU64::new(0));
        let parse_errors = Arc::new(AtomicU64::new(0));
        let reconnects = Arc::new(AtomicU64::new(0));
        let sample_rate_hz = Arc::new(AtomicU64::new(u64::from(sample_rate_hz)));
        let center_freq_hz = Arc::new(AtomicU64::new(center_freq_hz));
        let tune_epoch = Arc::new(AtomicU64::new(0));
        let max_sample_rate = Arc::new(AtomicU64::new(0));
        let decimation_stages = Arc::new(AtomicU64::new(0));
        let ingest = Arc::new(Self {
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
            tune_epoch: tune_epoch.clone(),
            max_sample_rate: max_sample_rate.clone(),
            decimation_stages: decimation_stages.clone(),
        });
        let stop_thread = stop.clone();
        let reconnect_target = addr;
        let handle = thread::spawn(move || {
            let mut stream = stream;
            let mut buffer = vec![0u8; 8192];
            let mut pending = Vec::<u8>::new();
            let mut consecutive_errors = 0u32;
            let mut last_recovery = Instant::now() - Duration::from_secs(5);
            let mut applied_epoch = 0u64;
            while !stop_thread.load(Ordering::Acquire) {
                let epoch = tune_epoch.load(Ordering::Relaxed);
                if epoch != applied_epoch {
                    let center = center_freq_hz.load(Ordering::Relaxed) as u32;
                    let rate = sample_rate_hz.load(Ordering::Relaxed) as u32;
                    let max = max_sample_rate.load(Ordering::Relaxed) as u32;
                    let stages = decimation_stages.load(Ordering::Relaxed) as u32;
                    let mut ok =
                        send_spy_setting(&mut stream, SPY_SETTING_IQ_FREQUENCY, center).is_ok();
                    if ok {
                        if let Some(stage) = spy_decimation_for_rate(max, stages, rate) {
                            ok = send_spy_setting(&mut stream, SPY_SETTING_IQ_DECIMATION, stage)
                                .is_ok();
                        }
                    }
                    if ok {
                        applied_epoch = epoch;
                    } else {
                        consecutive_errors = consecutive_errors.saturating_add(1);
                    }
                }
                match stream.read(&mut buffer) {
                    Ok(0) => consecutive_errors = consecutive_errors.saturating_add(1),
                    Ok(len) => {
                        consecutive_errors = 0;
                        pending.extend_from_slice(&buffer[..len]);
                        loop {
                            if pending.len() < SPYSERVER_HEADER_BYTES {
                                break;
                            }
                            let Ok((_, message_type, _, _, body_size)) =
                                parse_spyserver_header(&pending)
                            else {
                                pending.clear();
                                parse_errors.fetch_add(1, Ordering::Relaxed);
                                break;
                            };
                            let body_size = body_size as usize;
                            if body_size > SPYSERVER_MAX_BODY {
                                pending.clear();
                                parse_errors.fetch_add(1, Ordering::Relaxed);
                                break;
                            }
                            let total = SPYSERVER_HEADER_BYTES + body_size;
                            if pending.len() < total {
                                break;
                            }
                            let body = pending[SPYSERVER_HEADER_BYTES..total].to_vec();
                            pending.drain(..total);
                            if message_type == SPY_MSG_DEVICE_INFO {
                                apply_spyserver_device_info(
                                    &body,
                                    &max_sample_rate,
                                    &decimation_stages,
                                );
                                continue;
                            }
                            match decode_spyserver_iq(message_type, &body) {
                                Ok(samples) if !samples.is_empty() => {
                                    packets_received.fetch_add(1, Ordering::Relaxed);
                                    let now = crate::scanner::now_ms();
                                    last_packet_ms.store(now, Ordering::Relaxed);
                                    let dropped = push_samples(&state, capacity, &samples);
                                    packets_dropped.fetch_add(dropped, Ordering::Relaxed);
                                }
                                Ok(_) => {}
                                Err(_) => {
                                    parse_errors.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            || error.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(_) => consecutive_errors = consecutive_errors.saturating_add(1),
                }
                if consecutive_errors >= 8 && last_recovery.elapsed() >= Duration::from_secs(1) {
                    reconnects.fetch_add(1, Ordering::Relaxed);
                    consecutive_errors = 0;
                    last_recovery = Instant::now();
                    pending.clear();
                    match TcpStream::connect(&reconnect_target) {
                        Ok(mut next) => {
                            let _ = next.set_read_timeout(Some(Duration::from_millis(500)));
                            if spy_handshake(
                                &mut next,
                                center_freq_hz.load(Ordering::Relaxed) as u32,
                            )
                            .is_ok()
                            {
                                let _ = next.set_nonblocking(true);
                                stream = next;
                                applied_epoch = tune_epoch.load(Ordering::Relaxed);
                            }
                        }
                        Err(_) => thread::sleep(Duration::from_millis(50)),
                    }
                }
            }
        });
        *ingest.thread.lock() = Some(handle);
        Ok(ingest)
    }

    pub fn read(&self, count: usize) -> anyhow::Result<Vec<Complex<f32>>> {
        wait_for_queued_samples(&self.state, count, "SpyServer stream")
    }

    pub fn counters(&self) -> NetworkIngestCounters {
        snapshot_counters(
            &self.state,
            CounterAtoms {
                packets_received: &self.packets_received,
                packets_dropped: &self.packets_dropped,
                parse_errors: &self.parse_errors,
                reconnects: &self.reconnects,
                last_packet_ms: &self.last_packet_ms,
                sample_rate_hz: &self.sample_rate_hz,
                center_freq_hz: &self.center_freq_hz,
            },
        )
    }

    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz.load(Ordering::Relaxed) as u32
    }

    pub fn center_freq_hz(&self) -> u64 {
        self.center_freq_hz.load(Ordering::Relaxed)
    }

    pub fn tune(&self, center: Option<u64>, rate: Option<u32>) -> anyhow::Result<()> {
        if let Some(center) = center {
            let _ = require_u32_hz("SpyServer frequency", center)?;
            self.center_freq_hz.store(center, Ordering::Relaxed);
        }
        if let Some(rate) = rate {
            if rate == 0 {
                anyhow::bail!("sample rate must be greater than zero");
            }
            let max = self.max_sample_rate.load(Ordering::Relaxed) as u32;
            let stages = self.decimation_stages.load(Ordering::Relaxed) as u32;
            if max == 0 {
                anyhow::bail!(
                    "SpyServer has not reported MaximumSampleRate; sample rate cannot be commanded yet"
                );
            }
            if spy_decimation_for_rate(max, stages, rate).is_none() {
                anyhow::bail!(
                    "SpyServer cannot provide {rate} Hz; rate must equal MaximumSampleRate / 2^n"
                );
            }
            self.sample_rate_hz
                .store(u64::from(rate), Ordering::Relaxed);
        }
        self.tune_epoch.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

impl Drop for SpyServerIngest {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread.lock().take() {
            let _ = handle.join();
        }
    }
}

pub fn is_udp_multicast_host(host: &str) -> bool {
    host.parse::<IpAddr>()
        .map(|ip| ip.is_multicast())
        .unwrap_or(false)
}

pub fn parse_rtp_s16_iq(data: &[u8]) -> anyhow::Result<(u16, Vec<Complex<f32>>)> {
    if data.len() < 12 {
        anyhow::bail!("RTP header truncated");
    }
    if data[0] >> 6 != 2 {
        anyhow::bail!("RTP version is not 2");
    }
    let padding = data[0] & 0x20 != 0;
    let extension = data[0] & 0x10 != 0;
    let cc = usize::from(data[0] & 0x0f);
    let seq = u16::from_be_bytes([data[2], data[3]]);
    let mut offset = 12usize
        .checked_add(cc.saturating_mul(4))
        .ok_or_else(|| anyhow::anyhow!("RTP CSRC overflow"))?;
    if data.len() < offset {
        anyhow::bail!("RTP CSRC truncated");
    }
    if extension {
        if data.len() < offset + 4 {
            anyhow::bail!("RTP extension truncated");
        }
        let ext_words = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
        offset = offset
            .checked_add(4)
            .and_then(|value| value.checked_add(ext_words.saturating_mul(4)))
            .ok_or_else(|| anyhow::anyhow!("RTP extension overflow"))?;
        if data.len() < offset {
            anyhow::bail!("RTP extension payload truncated");
        }
    }
    let mut payload = &data[offset..];
    if padding {
        if payload.is_empty() {
            anyhow::bail!("RTP padding truncated");
        }
        let pad = usize::from(payload[payload.len() - 1]);
        if pad == 0 || pad > payload.len() {
            anyhow::bail!("invalid RTP padding");
        }
        payload = &payload[..payload.len() - pad];
    }
    Ok((seq, decode_s16le_iq(payload)))
}

pub fn parse_ka9q_udp(data: &[u8]) -> anyhow::Result<(Option<u16>, Vec<Complex<f32>>)> {
    if data.len() >= 12 && data[0] >> 6 == 2 {
        let (seq, samples) = parse_rtp_s16_iq(data)?;
        if samples.is_empty() {
            anyhow::bail!("RTP IQ payload empty");
        }
        Ok((Some(seq), samples))
    } else {
        let samples = decode_s16le_iq(data);
        if samples.is_empty() {
            anyhow::bail!("KA9Q UDP payload too short");
        }
        Ok((None, samples))
    }
}

pub struct Ka9qUdpIngest {
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

impl Ka9qUdpIngest {
    pub fn start(
        host: &str,
        port: u16,
        capacity: usize,
        center_freq_hz: u64,
        sample_rate_hz: u32,
    ) -> anyhow::Result<Arc<Self>> {
        let bind_addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
        let socket = UdpSocket::bind(bind_addr)?;
        socket.set_nonblocking(true)?;
        if is_udp_multicast_host(host) {
            match host.parse::<IpAddr>()? {
                IpAddr::V4(group) => {
                    socket.join_multicast_v4(&group, &Ipv4Addr::UNSPECIFIED)?;
                }
                IpAddr::V6(group) => {
                    socket.join_multicast_v6(&group, 0)?;
                }
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(IngestState {
            queue: VecDeque::with_capacity(capacity.min(262_144)),
            sample_rate_hz: sample_rate_hz.max(1),
            center_freq_hz,
            counters: NetworkIngestCounters::default(),
        }));
        let last_packet_ms = Arc::new(AtomicI64::new(0));
        let packets_received = Arc::new(AtomicU64::new(0));
        let packets_dropped = Arc::new(AtomicU64::new(0));
        let parse_errors = Arc::new(AtomicU64::new(0));
        let reconnects = Arc::new(AtomicU64::new(0));
        let sample_rate_hz = Arc::new(AtomicU64::new(u64::from(sample_rate_hz.max(1))));
        let center_freq_hz = Arc::new(AtomicU64::new(center_freq_hz));
        let ingest = Arc::new(Self {
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
            let mut last_seq: Option<u16> = None;
            let mut consecutive_errors = 0u32;
            let mut last_recovery = Instant::now() - Duration::from_secs(5);
            while !stop_thread.load(Ordering::Acquire) {
                match socket.recv_from(&mut buffer) {
                    Ok((len, _peer)) => {
                        consecutive_errors = 0;
                        match parse_ka9q_udp(&buffer[..len]) {
                            Ok((seq, samples)) => {
                                if let Some(seq) = seq {
                                    if let Some(previous) = last_seq {
                                        let expected = previous.wrapping_add(1);
                                        if seq != expected {
                                            let gap = u64::from(seq.wrapping_sub(expected)).max(1);
                                            packets_dropped.fetch_add(gap, Ordering::Relaxed);
                                        }
                                    }
                                    last_seq = Some(seq);
                                }
                                packets_received.fetch_add(1, Ordering::Relaxed);
                                let now = crate::scanner::now_ms();
                                last_packet_ms.store(now, Ordering::Relaxed);
                                let overflow = push_samples(&state, capacity, &samples);
                                packets_dropped.fetch_add(overflow, Ordering::Relaxed);
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
                        if consecutive_errors >= 8
                            && last_recovery.elapsed() >= Duration::from_secs(1)
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
        wait_for_queued_samples(&self.state, count, "KA9Q stream")
    }

    pub fn counters(&self) -> NetworkIngestCounters {
        snapshot_counters(
            &self.state,
            CounterAtoms {
                packets_received: &self.packets_received,
                packets_dropped: &self.packets_dropped,
                parse_errors: &self.parse_errors,
                reconnects: &self.reconnects,
                last_packet_ms: &self.last_packet_ms,
                sample_rate_hz: &self.sample_rate_hz,
                center_freq_hz: &self.center_freq_hz,
            },
        )
    }

    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz.load(Ordering::Relaxed) as u32
    }

    pub fn center_freq_hz(&self) -> u64 {
        self.center_freq_hz.load(Ordering::Relaxed)
    }

    pub fn tune(&self, center: Option<u64>, rate: Option<u32>) -> anyhow::Result<()> {
        if let Some(center) = center {
            self.center_freq_hz.store(center, Ordering::Relaxed);
        }
        if let Some(rate) = rate {
            if rate == 0 {
                anyhow::bail!("sample rate must be greater than zero");
            }
            self.sample_rate_hz
                .store(u64::from(rate), Ordering::Relaxed);
        }
        Ok(())
    }
}

impl Drop for Ka9qUdpIngest {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread.lock().take() {
            let _ = handle.join();
        }
    }
}

pub enum NetworkIqIngest {
    PsiqUdp(Arc<PsiqUdpIngest>),
    RtlTcp(Arc<RtlTcpIngest>),
    SpyServer(Arc<SpyServerIngest>),
    Ka9q(Arc<Ka9qUdpIngest>),
}

impl NetworkIqIngest {
    pub fn connect(
        kind: &str,
        host: &str,
        port: u16,
        capacity: usize,
        center_freq_hz: u64,
        sample_rate_hz: u32,
    ) -> anyhow::Result<Self> {
        match kind {
            "raw_udp" => Ok(Self::PsiqUdp(PsiqUdpIngest::start(host, port, capacity)?)),
            "rtl_tcp" => Ok(Self::RtlTcp(RtlTcpIngest::connect(
                host,
                port,
                capacity,
                center_freq_hz,
                sample_rate_hz,
            )?)),
            "spyserver" => Ok(Self::SpyServer(SpyServerIngest::connect(
                host,
                port,
                capacity,
                center_freq_hz,
                sample_rate_hz,
            )?)),
            "ka9q" => Ok(Self::Ka9q(Ka9qUdpIngest::start(
                host,
                port,
                capacity,
                center_freq_hz,
                sample_rate_hz,
            )?)),
            other => anyhow::bail!(
                "network kind {other} is registered but ingest adapter is not available yet"
            ),
        }
    }

    pub fn read(&self, count: usize) -> anyhow::Result<Vec<Complex<f32>>> {
        match self {
            Self::PsiqUdp(ingest) => ingest.read(count),
            Self::RtlTcp(ingest) => ingest.read(count),
            Self::SpyServer(ingest) => ingest.read(count),
            Self::Ka9q(ingest) => ingest.read(count),
        }
    }

    pub fn counters(&self) -> NetworkIngestCounters {
        match self {
            Self::PsiqUdp(ingest) => ingest.counters(),
            Self::RtlTcp(ingest) => ingest.counters(),
            Self::SpyServer(ingest) => ingest.counters(),
            Self::Ka9q(ingest) => ingest.counters(),
        }
    }

    pub fn sample_rate_hz(&self) -> u32 {
        match self {
            Self::PsiqUdp(ingest) => ingest.sample_rate_hz(),
            Self::RtlTcp(ingest) => ingest.sample_rate_hz(),
            Self::SpyServer(ingest) => ingest.sample_rate_hz(),
            Self::Ka9q(ingest) => ingest.sample_rate_hz(),
        }
    }

    pub fn center_freq_hz(&self) -> u64 {
        match self {
            Self::PsiqUdp(ingest) => ingest.center_freq_hz(),
            Self::RtlTcp(ingest) => ingest.center_freq_hz(),
            Self::SpyServer(ingest) => ingest.center_freq_hz(),
            Self::Ka9q(ingest) => ingest.center_freq_hz(),
        }
    }

    pub fn tune(&self, center: Option<u64>, rate: Option<u32>) -> anyhow::Result<()> {
        match self {
            Self::PsiqUdp(ingest) => ingest.tune(center, rate),
            Self::RtlTcp(ingest) => ingest.tune(center, rate),
            Self::SpyServer(ingest) => ingest.tune(center, rate),
            Self::Ka9q(ingest) => ingest.tune(center, rate),
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

    #[test]
    fn decode_rtl_tcp_iq_normalizes_unsigned_bytes() {
        let samples = decode_rtl_tcp_iq(&[128, 127, 0, 255]);
        assert!((samples[0].re - 0.0).abs() < 0.01);
        assert!((samples[1].re + 1.0).abs() < 0.02);
    }

    #[test]
    fn rtl_tcp_mock_server_stream() {
        use std::io::Write;
        use std::net::TcpListener;
        use std::sync::mpsc;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (ready_tx, ready_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .write_all(&RTL_TCP_DONGLE_MAGIC.to_le_bytes())
                .unwrap();
            stream.write_all(&0u32.to_le_bytes()).unwrap();
            stream.write_all(&28u32.to_le_bytes()).unwrap();
            let mut tune = [0u8; 10];
            stream.read_exact(&mut tune).unwrap();
            let iq: Vec<u8> = (0..8192)
                .flat_map(|index| [128u8.wrapping_add(index as u8), 127])
                .collect();
            stream.write_all(&iq).unwrap();
        });
        ready_rx.recv().unwrap();
        let ingest =
            RtlTcpIngest::connect("127.0.0.1", port, 65_536, 146_000_000, 2_048_000).unwrap();
        let samples = ingest.read(4096).expect("rtl_tcp read");
        assert_eq!(samples.len(), 4096);
        assert!(ingest.counters().packets_received >= 1);
        server.join().unwrap();
    }

    fn rtl_tcp_iq_bytes(count: usize) -> Vec<u8> {
        (0..count)
            .flat_map(|index| [128u8.wrapping_add(index as u8), 127])
            .collect()
    }

    #[test]
    fn rtl_tcp_live_retune_sends_frequency_command() {
        use std::net::TcpListener;
        use std::sync::mpsc;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (tune_tx, tune_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .write_all(&RTL_TCP_DONGLE_MAGIC.to_le_bytes())
                .unwrap();
            stream.write_all(&0u32.to_le_bytes()).unwrap();
            stream.write_all(&28u32.to_le_bytes()).unwrap();
            let mut first = [0u8; 10];
            stream.read_exact(&mut first).unwrap();
            stream.write_all(&rtl_tcp_iq_bytes(8192)).unwrap();
            let mut second = [0u8; 10];
            stream.read_exact(&mut second).unwrap();
            let command = second[5];
            let value = u32::from_le_bytes([second[6], second[7], second[8], second[9]]);
            tune_tx.send((command, value)).unwrap();
            stream.write_all(&rtl_tcp_iq_bytes(8192)).unwrap();
        });
        ready_rx.recv().unwrap();
        let ingest =
            RtlTcpIngest::connect("127.0.0.1", port, 65_536, 146_000_000, 2_048_000).unwrap();
        assert_eq!(ingest.read(4096).expect("initial rtl_tcp read").len(), 4096);
        ingest.tune(Some(162_000_000), None).unwrap();
        let (command, value) = tune_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("live retune command");
        assert_eq!(command, 0x01);
        assert_eq!(value, 162_000_000);
        assert_eq!(ingest.center_freq_hz(), 162_000_000);
        server.join().unwrap();
    }

    fn spy_message(message_type: u32, body: &[u8]) -> Vec<u8> {
        let mut packet = Vec::new();
        packet.extend_from_slice(&SPYSERVER_PROTOCOL_VERSION.to_le_bytes());
        packet.extend_from_slice(&message_type.to_le_bytes());
        packet.extend_from_slice(&SPY_STREAM_TYPE_IQ.to_le_bytes());
        packet.extend_from_slice(&1u32.to_le_bytes());
        packet.extend_from_slice(&(body.len() as u32).to_le_bytes());
        packet.extend_from_slice(body);
        packet
    }

    fn spy_device_info_body(max_rate: u32, stages: u32) -> Vec<u8> {
        let mut body = vec![0u8; 48];
        body[8..12].copy_from_slice(&max_rate.to_le_bytes());
        body[16..20].copy_from_slice(&stages.to_le_bytes());
        body
    }

    fn spy_float_iq_body(count: usize) -> Vec<u8> {
        let mut body = Vec::with_capacity(count * 8);
        for index in 0..count {
            body.extend_from_slice(&(index as f32 / 1024.0).to_le_bytes());
            body.extend_from_slice(&0.25f32.to_le_bytes());
        }
        body
    }

    #[test]
    fn network_kind_supported_covers_implemented_adapters_only() {
        for kind in implemented_network_kinds() {
            assert!(network_kind_supported(kind));
        }
        assert!(!network_kind_supported("kiwisdr"));
        assert!(!network_kind_supported("soapysdr"));
    }

    #[test]
    fn spy_decimation_rejects_rates_that_are_not_exact_powers_of_two() {
        assert_eq!(spy_decimation_for_rate(2_048_000, 4, 2_048_000), Some(0));
        assert_eq!(spy_decimation_for_rate(2_048_000, 4, 1_024_000), Some(1));
        assert_eq!(spy_decimation_for_rate(2_048_000, 4, 1_000_000), None);
    }

    #[test]
    fn decode_spyserver_float_and_int16_iq() {
        let float_body = spy_float_iq_body(2);
        let float_samples = decode_spyserver_iq(SPY_MSG_FLOAT_IQ, &float_body).unwrap();
        assert!((float_samples[1].im - 0.25).abs() < f32::EPSILON);
        let mut int16 = Vec::new();
        int16.extend_from_slice(&32767i16.to_le_bytes());
        int16.extend_from_slice(&(-32768i16).to_le_bytes());
        let samples = decode_spyserver_iq(SPY_MSG_INT16_IQ, &int16).unwrap();
        assert!(samples[0].re > 0.99);
        assert!(samples[0].im < -0.99);
    }

    #[test]
    fn spyserver_mock_server_streams_and_retunes() {
        use std::net::TcpListener;
        use std::sync::mpsc;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (freq_tx, freq_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            let (stream, _) = listener.accept().unwrap();
            let mut reader = stream.try_clone().unwrap();
            let mut writer = stream;
            let mut header = [0u8; 8];
            reader.read_exact(&mut header).unwrap();
            let body_size =
                u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
            let mut body = vec![0u8; body_size];
            reader.read_exact(&mut body).unwrap();
            writer
                .write_all(&spy_message(
                    SPY_MSG_DEVICE_INFO,
                    &spy_device_info_body(2_048_000, 4),
                ))
                .unwrap();
            writer
                .write_all(&spy_message(SPY_MSG_FLOAT_IQ, &spy_float_iq_body(1024)))
                .unwrap();
            loop {
                let mut next = [0u8; 8];
                if reader.read_exact(&mut next).is_err() {
                    break;
                }
                let command = u32::from_le_bytes([next[0], next[1], next[2], next[3]]);
                let size = u32::from_le_bytes([next[4], next[5], next[6], next[7]]) as usize;
                let mut payload = vec![0u8; size];
                if reader.read_exact(&mut payload).is_err() {
                    break;
                }
                if command == SPY_CMD_SET_SETTING && payload.len() >= 8 {
                    let setting = u32_le(&payload, 0);
                    let value = u32_le(&payload, 4);
                    if setting == SPY_SETTING_IQ_FREQUENCY {
                        let _ = freq_tx.send(value);
                        let _ = writer
                            .write_all(&spy_message(SPY_MSG_FLOAT_IQ, &spy_float_iq_body(256)));
                    }
                }
            }
        });
        ready_rx.recv().unwrap();
        let ingest =
            SpyServerIngest::connect("127.0.0.1", port, 65_536, 100_000_000, 2_048_000).unwrap();
        let samples = ingest.read(1024).expect("spyserver read");
        assert_eq!(samples.len(), 1024);
        assert!(ingest.counters().packets_received >= 1);
        ingest.tune(Some(162_550_000), Some(1_024_000)).unwrap();
        let mut frequency = 0u32;
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Ok(value) = freq_rx.recv_timeout(Duration::from_millis(200)) {
                frequency = value;
                if frequency == 162_550_000 {
                    break;
                }
            }
        }
        assert_eq!(frequency, 162_550_000);
        assert_eq!(ingest.center_freq_hz(), 162_550_000);
        assert_eq!(ingest.sample_rate_hz(), 1_024_000);
        drop(ingest);
        server.join().unwrap();
    }

    fn rtp_s16_packet(seq: u16, samples: &[Complex<f32>]) -> Vec<u8> {
        let mut packet = vec![0x80, 0x60];
        packet.extend_from_slice(&seq.to_be_bytes());
        packet.extend_from_slice(&0u32.to_be_bytes());
        packet.extend_from_slice(&0x11223344u32.to_be_bytes());
        for sample in samples {
            let i = (sample.re.clamp(-1.0, 1.0) * 32767.0) as i16;
            let q = (sample.im.clamp(-1.0, 1.0) * 32767.0) as i16;
            packet.extend_from_slice(&i.to_le_bytes());
            packet.extend_from_slice(&q.to_le_bytes());
        }
        packet
    }

    #[test]
    fn parse_ka9q_rtp_and_raw_s16le() {
        let tone = vec![Complex::new(0.5, -0.25), Complex::new(-0.5, 0.25)];
        let packet = rtp_s16_packet(7, &tone);
        let (seq, parsed) = parse_ka9q_udp(&packet).unwrap();
        assert_eq!(seq, Some(7));
        assert_eq!(parsed.len(), 2);
        assert!((parsed[0].re - 0.5).abs() < 0.01);
        let mut raw = Vec::new();
        raw.extend_from_slice(&16384i16.to_le_bytes());
        raw.extend_from_slice(&(-8192i16).to_le_bytes());
        let (raw_seq, raw_samples) = parse_ka9q_udp(&raw).unwrap();
        assert_eq!(raw_seq, None);
        assert!(raw_samples[0].re > 0.4);
        assert!(is_udp_multicast_host("239.1.2.3"));
        assert!(!is_udp_multicast_host("127.0.0.1"));
    }

    #[test]
    fn ka9q_udp_loopback_counts_sequence_gaps() {
        let port = free_udp_port();
        let ingest = Ka9qUdpIngest::start("127.0.0.1", port, 65_536, 146_000_000, 48_000).unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let tone: Vec<_> = (0..256)
            .map(|index| Complex::new((index as f32).sin() * 0.2, 0.1))
            .collect();
        let dest: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        sender.send_to(&rtp_s16_packet(10, &tone), dest).unwrap();
        assert_eq!(ingest.read(256).expect("ka9q read").len(), 256);
        sender.send_to(&rtp_s16_packet(14, &tone), dest).unwrap();
        assert_eq!(ingest.read(256).expect("ka9q gap read").len(), 256);
        let counters = ingest.counters();
        assert!(counters.packets_received >= 2);
        assert!(counters.packets_dropped >= 3);
        assert_eq!(ingest.center_freq_hz(), 146_000_000);
        ingest.tune(Some(162_000_000), Some(96_000)).unwrap();
        assert_eq!(ingest.center_freq_hz(), 162_000_000);
        assert_eq!(ingest.sample_rate_hz(), 96_000);
    }
}
