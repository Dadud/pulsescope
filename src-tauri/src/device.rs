// device.rs — SDR device layer. The mock source stays available for development;
// enabling `soapysdr` adds an owned SoapySDR RX backend for installed modules.
use std::f32::consts::TAU;
#[cfg(feature = "soapysdr")]
use std::process::Command;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
pub static LIVE_HARDWARE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
use parking_lot::Mutex;
use rustfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    pub driver: String,
    pub label: String,
    pub key: String,
    pub hardware_key: String,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceLifecycle {
    Detected,
    Probing,
    Configuring,
    Streaming,
    Degraded,
    Recovering,
    #[default]
    Disconnected,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StreamCountersSnapshot {
    pub read_calls: u64,
    pub samples_received: u64,
    pub short_reads: u64,
    pub source_errors: u64,
    pub consecutive_errors: u64,
    pub retunes: u64,
    pub restarts: u64,
    pub last_sample_ms: i64,
    pub last_error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceStatus {
    pub connected: bool,
    pub lifecycle: DeviceLifecycle,
    pub driver: String,
    pub label: String,
    pub sample_rate: u32,
    /// Analog RF bandwidth actually requested from the device. This is not
    /// interchangeable with sample rate (for example, an RSP1B may stream at
    /// 10 MSPS while using its 8 MHz frontend filter).
    pub bandwidth_hz: u32,
    pub center_freq_hz: u64,
    pub ppm_correction: f32,
    pub gain: String,
    pub bias_tee_on: bool,
    pub saturation: bool,
    pub stream: StreamCountersSnapshot,
}

/// Capability report comes from the currently opened SoapySDR RX chain. UI
/// renders these dynamically; there is no driver-name-specific knob list.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GainStage {
    pub name: String,
    pub value_db: f64,
    pub min_db: f64,
    pub max_db: f64,
    pub step_db: f64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DeviceSetting {
    pub key: String,
    pub name: String,
    pub value: String,
    pub kind: String,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub options: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NumericRange {
    pub minimum: f64,
    pub maximum: f64,
    pub step: f64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub stable_id: String,
    pub driver: String,
    pub label: String,
    pub serial: Option<String>,
    pub connection: String,
    pub hardware_key: String,
    pub firmware_version: Option<String>,
    pub api_version: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DeviceCapabilities {
    pub contract_version: u16,
    pub identity: DeviceIdentity,
    pub connected: bool,
    pub agc_supported: bool,
    pub agc_enabled: bool,
    pub rf_ranges_hz: Vec<NumericRange>,
    pub sample_rate_ranges_hz: Vec<NumericRange>,
    pub supported_sample_rates_hz: Vec<u32>,
    pub bandwidth_ranges_hz: Vec<NumericRange>,
    pub sample_formats: Vec<String>,
    pub stream_mtu: usize,
    pub total_bandwidth_hz: u32,
    pub usable_bandwidth_hz: u32,
    pub tuner_count: u32,
    pub full_duplex: bool,
    pub gain_stages: Vec<GainStage>,
    pub settings: Vec<DeviceSetting>,
    pub antennas: Vec<String>,
    pub antenna: String,
    pub dc_offset_auto_supported: bool,
    pub dc_offset_auto: bool,
    pub iq_balance_auto_supported: bool,
    pub iq_balance_auto: bool,
    pub frequency_correction_supported: bool,
    pub frequency_correction_ppm: f64,
}

/// Versioned hardware boundary used by capture, allocation, API, and future
/// native/network adapters. `DeviceLayer` is the first implementation and
/// continues to own the proven Soapy hot path while callers migrate from the
/// concrete type. Adapters must report capabilities; model-name guessing is
/// deliberately not part of this contract.
pub trait RadioDevice: Send + Sync {
    fn contract_version(&self) -> u16 {
        2
    }
    fn status(&self) -> DeviceStatus;
    fn capabilities(&self) -> DeviceCapabilities;
    fn connect(&self, key: &str) -> anyhow::Result<()>;
    fn disconnect(&self) -> anyhow::Result<()>;
    fn recover(&self) -> anyhow::Result<()>;
    fn set_frequency(&self, frequency_hz: u64) -> anyhow::Result<()>;
    fn set_sample_contract(&self, sample_rate_hz: u32) -> anyhow::Result<u32>;
    fn set_gain(&self, gain: String) -> anyhow::Result<()>;
    fn set_control(&self, control: &str, value: &str) -> anyhow::Result<()>;
    fn read_iq(&self, sample_count: usize) -> anyhow::Result<Vec<Complex<f32>>>;
}

#[derive(Default)]
struct StreamCounters {
    read_calls: AtomicU64,
    samples_received: AtomicU64,
    short_reads: AtomicU64,
    source_errors: AtomicU64,
    consecutive_errors: AtomicU64,
    retunes: AtomicU64,
    restarts: AtomicU64,
    last_sample_ms: AtomicU64,
    last_error: Mutex<Option<String>>,
}

impl StreamCounters {
    fn snapshot(&self) -> StreamCountersSnapshot {
        StreamCountersSnapshot {
            read_calls: self.read_calls.load(Ordering::Relaxed),
            samples_received: self.samples_received.load(Ordering::Relaxed),
            short_reads: self.short_reads.load(Ordering::Relaxed),
            source_errors: self.source_errors.load(Ordering::Relaxed),
            consecutive_errors: self.consecutive_errors.load(Ordering::Relaxed),
            retunes: self.retunes.load(Ordering::Relaxed),
            restarts: self.restarts.load(Ordering::Relaxed),
            last_sample_ms: self.last_sample_ms.load(Ordering::Relaxed) as i64,
            last_error: self.last_error.lock().clone(),
        }
    }
}

#[cfg(feature = "soapysdr")]
mod soapy {
    use super::*;
    use soapysdr_sys as s;
    use std::{
        ffi::{CStr, CString},
        os::raw::{c_char, c_void},
        ptr,
    };
    pub struct Hardware {
        device: *mut s::SoapySDRDevice,
        stream: *mut s::SoapySDRStream,
        stream_mtu: usize,
    }
    unsafe impl Send for Hardware {}
    fn err(op: &str, rc: i32) -> anyhow::Error {
        unsafe {
            let p = s::SoapySDRDevice_lastError();
            let e = if p.is_null() {
                "unknown SoapySDR error".into()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            };
            anyhow::anyhow!("{op} failed ({rc}): {e}")
        }
    }
    fn check(op: &str, rc: i32) -> anyhow::Result<()> {
        if rc == 0 {
            Ok(())
        } else {
            Err(err(op, rc))
        }
    }
    fn string(p: *mut c_char) -> String {
        unsafe {
            if p.is_null() {
                return String::new();
            }
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
    unsafe fn numeric_ranges(p: *mut s::SoapySDRRange, length: usize) -> Vec<NumericRange> {
        if p.is_null() {
            return Vec::new();
        }
        let values = std::slice::from_raw_parts(p, length)
            .iter()
            .map(|range| NumericRange {
                minimum: range.minimum,
                maximum: range.maximum,
                step: range.step,
            })
            .collect();
        s::SoapySDR_free(p.cast());
        values
    }
    impl Hardware {
        pub fn open(key: &str, rate: u32, freq: u64) -> anyhow::Result<Self> {
            unsafe {
                let key = CString::new(key)?;
                let device = s::SoapySDRDevice_makeStrArgs(key.as_ptr());
                if device.is_null() {
                    return Err(err("SoapySDRDevice_make", -1));
                };
                let mut stream = ptr::null_mut();
                let result = (|| {
                    check(
                        "set sample rate",
                        s::SoapySDRDevice_setSampleRate(
                            device,
                            s::SOAPY_SDR_RX as i32,
                            0,
                            rate as f64,
                        ),
                    )?;
                    check(
                        "set frequency",
                        s::SoapySDRDevice_setFrequency(
                            device,
                            s::SOAPY_SDR_RX as i32,
                            0,
                            freq as f64,
                            ptr::null(),
                        ),
                    )?;
                    check(
                        "enable AGC",
                        s::SoapySDRDevice_setGainMode(device, s::SOAPY_SDR_RX as i32, 0, true),
                    )?;
                    check(
                        "setup CF32 RX stream",
                        s::SoapySDRDevice_setupStream(
                            device,
                            &mut stream,
                            s::SOAPY_SDR_RX as i32,
                            s::SOAPY_SDR_CF32.as_ptr().cast(),
                            ptr::null(),
                            0,
                            ptr::null(),
                        ),
                    )?;
                    check(
                        "activate RX stream",
                        s::SoapySDRDevice_activateStream(device, stream, 0, 0, 0),
                    )?;
                    let stream_mtu = s::SoapySDRDevice_getStreamMTU(device, stream);
                    Ok(Self {
                        device,
                        stream,
                        stream_mtu,
                    })
                })();
                if result.is_err() {
                    if !stream.is_null() {
                        let _ = s::SoapySDRDevice_closeStream(device, stream);
                    }
                    let _ = s::SoapySDRDevice_unmake(device);
                }
                result
            }
        }
        pub fn read(&mut self, count: usize) -> anyhow::Result<Vec<Complex<f32>>> {
            for _ in 0..8 {
                let mut out = vec![Complex::new(0.0, 0.0); count];
                let mut buffer = out.as_mut_ptr().cast::<c_void>();
                let mut flags = 0i32;
                let mut time = 0i64;
                let n = unsafe {
                    s::SoapySDRDevice_readStream(
                        self.device,
                        self.stream,
                        &mut buffer,
                        count,
                        &mut flags,
                        &mut time,
                        250000,
                    )
                };
                if n > 0 {
                    out.truncate(n as usize);
                    return Ok(out);
                }
                if n == s::SOAPY_SDR_TIMEOUT || n == s::SOAPY_SDR_OVERFLOW {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                }
                return Err(err("read RX stream", n));
            }
            Err(anyhow::anyhow!(
                "read RX stream exhausted retries after timeout/overflow"
            ))
        }
        pub fn set_frequency(&mut self, freq: u64) -> anyhow::Result<()> {
            unsafe {
                if !self.stream.is_null() {
                    check(
                        "deactivate RX stream",
                        s::SoapySDRDevice_deactivateStream(self.device, self.stream, 0, 0),
                    )?;
                    check(
                        "close RX stream",
                        s::SoapySDRDevice_closeStream(self.device, self.stream),
                    )?;
                    self.stream = ptr::null_mut();
                }
                for attempt in 0..2 {
                    check(
                        "set frequency",
                        s::SoapySDRDevice_setFrequency(
                            self.device,
                            s::SOAPY_SDR_RX as i32,
                            0,
                            freq as f64,
                            ptr::null(),
                        ),
                    )?;
                    std::thread::sleep(std::time::Duration::from_millis(75));
                    let actual =
                        s::SoapySDRDevice_getFrequency(self.device, s::SOAPY_SDR_RX as i32, 0);
                    if (actual - freq as f64).abs() <= 2_000.0 {
                        check(
                            "setup CF32 RX stream",
                            s::SoapySDRDevice_setupStream(
                                self.device,
                                &mut self.stream,
                                s::SOAPY_SDR_RX as i32,
                                s::SOAPY_SDR_CF32.as_ptr().cast(),
                                ptr::null(),
                                0,
                                ptr::null(),
                            ),
                        )?;
                        if let Err(e) = check(
                            "activate RX stream",
                            s::SoapySDRDevice_activateStream(self.device, self.stream, 0, 0, 0),
                        ) {
                            let _ = s::SoapySDRDevice_closeStream(self.device, self.stream);
                            self.stream = ptr::null_mut();
                            return Err(e);
                        }
                        return Ok(());
                    }
                    if attempt == 1 {
                        return Err(anyhow::anyhow!(
                            "frequency readback mismatch: requested {freq} Hz, got {actual:.0} Hz"
                        ));
                    }
                }
                unreachable!()
            }
        }
        pub fn set_rate(&mut self, rate: u32) -> anyhow::Result<()> {
            unsafe {
                if !self.stream.is_null() {
                    check(
                        "deactivate RX stream",
                        s::SoapySDRDevice_deactivateStream(self.device, self.stream, 0, 0),
                    )?;
                    check(
                        "close RX stream",
                        s::SoapySDRDevice_closeStream(self.device, self.stream),
                    )?;
                    self.stream = ptr::null_mut();
                }
                check(
                    "set sample rate",
                    s::SoapySDRDevice_setSampleRate(
                        self.device,
                        s::SOAPY_SDR_RX as i32,
                        0,
                        rate as f64,
                    ),
                )?;
                check(
                    "setup CF32 RX stream",
                    s::SoapySDRDevice_setupStream(
                        self.device,
                        &mut self.stream,
                        s::SOAPY_SDR_RX as i32,
                        s::SOAPY_SDR_CF32.as_ptr().cast(),
                        ptr::null(),
                        0,
                        ptr::null(),
                    ),
                )?;
                if let Err(e) = check(
                    "activate RX stream",
                    s::SoapySDRDevice_activateStream(self.device, self.stream, 0, 0, 0),
                ) {
                    let _ = s::SoapySDRDevice_closeStream(self.device, self.stream);
                    self.stream = ptr::null_mut();
                    return Err(e);
                }
                Ok(())
            }
        }
        pub fn set_bandwidth(&mut self, bw: u32) -> anyhow::Result<()> {
            check("set bandwidth", unsafe {
                s::SoapySDRDevice_setBandwidth(self.device, s::SOAPY_SDR_RX as i32, 0, bw as f64)
            })
        }
        pub fn set_gain(&mut self, gain: f64) -> anyhow::Result<()> {
            check("set gain", unsafe {
                s::SoapySDRDevice_setGainMode(self.device, s::SOAPY_SDR_RX as i32, 0, false)
            })?;
            check("set gain", unsafe {
                s::SoapySDRDevice_setGain(self.device, s::SOAPY_SDR_RX as i32, 0, gain)
            })
        }
        pub fn apply_safe_defaults(&mut self) -> anyhow::Result<()> {
            unsafe {
                let dir = s::SOAPY_SDR_RX as i32;
                if s::SoapySDRDevice_hasGainMode(self.device, dir, 0) {
                    check(
                        "enable AGC",
                        s::SoapySDRDevice_setGainMode(self.device, dir, 0, true),
                    )?;
                }
                if s::SoapySDRDevice_hasDCOffsetMode(self.device, dir, 0) {
                    check(
                        "enable DC offset auto",
                        s::SoapySDRDevice_setDCOffsetMode(self.device, dir, 0, true),
                    )?;
                }
                if s::SoapySDRDevice_hasIQBalanceMode(self.device, dir, 0) {
                    check(
                        "enable IQ balance auto",
                        s::SoapySDRDevice_setIQBalanceMode(self.device, dir, 0, true),
                    )?;
                }
                // SDRplay exposes these as Soapy vendor settings. Apply only when
                // present; other hardware retains its own driver defaults.
                for (key, value) in [
                    ("iqcorr_ctrl", "true"),
                    ("agc_setpoint", "-30"),
                    ("rfgain_sel", "1"),
                ] {
                    let key_c = CString::new(key).unwrap();
                    if !s::SoapySDRDevice_readSetting(self.device, key_c.as_ptr()).is_null() {
                        let value_c = CString::new(value).unwrap();
                        check(
                            "apply receiver default",
                            s::SoapySDRDevice_writeSetting(
                                self.device,
                                key_c.as_ptr(),
                                value_c.as_ptr(),
                            ),
                        )?;
                    }
                }
                Ok(())
            }
        }
        pub fn capabilities(&self) -> DeviceCapabilities {
            unsafe {
                let mut c = DeviceCapabilities {
                    contract_version: 2,
                    connected: true,
                    stream_mtu: self.stream_mtu,
                    sample_formats: vec!["CF32".into()],
                    tuner_count: 1,
                    ..Default::default()
                };
                let dir = s::SOAPY_SDR_RX as i32;
                c.tuner_count = s::SoapySDRDevice_getNumChannels(self.device, dir) as u32;
                c.full_duplex =
                    c.tuner_count > 0 && s::SoapySDRDevice_getFullDuplex(self.device, dir, 0);
                let mut n = 0usize;
                c.rf_ranges_hz = numeric_ranges(
                    s::SoapySDRDevice_getFrequencyRange(self.device, dir, 0, &mut n),
                    n,
                );
                let mut n = 0usize;
                c.sample_rate_ranges_hz = numeric_ranges(
                    s::SoapySDRDevice_getSampleRateRange(self.device, dir, 0, &mut n),
                    n,
                );
                c.supported_sample_rates_hz = c
                    .sample_rate_ranges_hz
                    .iter()
                    .filter(|range| {
                        range.minimum == range.maximum
                            && range.minimum > 0.0
                            && range.minimum <= u32::MAX as f64
                    })
                    .map(|range| range.minimum as u32)
                    .collect();
                let mut n = 0usize;
                c.bandwidth_ranges_hz = numeric_ranges(
                    s::SoapySDRDevice_getBandwidthRange(self.device, dir, 0, &mut n),
                    n,
                );
                let mut n = 0usize;
                let formats = s::SoapySDRDevice_getStreamFormats(self.device, dir, 0, &mut n);
                if !formats.is_null() {
                    c.sample_formats = (0..n)
                        .map(|i| {
                            CStr::from_ptr(*formats.add(i))
                                .to_string_lossy()
                                .into_owned()
                        })
                        .collect();
                    s::SoapySDRStrings_clear(&mut (formats as *mut *mut c_char), n);
                }
                let current_rate = s::SoapySDRDevice_getSampleRate(self.device, dir, 0)
                    .max(0.0)
                    .min(u32::MAX as f64) as u32;
                c.total_bandwidth_hz = current_rate;
                c.usable_bandwidth_hz = (current_rate as f64 * 0.9) as u32;
                c.agc_supported = s::SoapySDRDevice_hasGainMode(self.device, dir, 0);
                c.agc_enabled =
                    c.agc_supported && s::SoapySDRDevice_getGainMode(self.device, dir, 0);
                let mut n = 0usize;
                let p = s::SoapySDRDevice_listGains(self.device, dir, 0, &mut n);
                for i in 0..n {
                    let name = CStr::from_ptr(*p.add(i)).to_string_lossy().into_owned();
                    let key = CString::new(name.as_str()).unwrap();
                    let r =
                        s::SoapySDRDevice_getGainElementRange(self.device, dir, 0, key.as_ptr());
                    c.gain_stages.push(GainStage {
                        name,
                        value_db: s::SoapySDRDevice_getGainElement(
                            self.device,
                            dir,
                            0,
                            key.as_ptr(),
                        ),
                        min_db: r.minimum,
                        max_db: r.maximum,
                        step_db: r.step,
                    });
                }
                if !p.is_null() {
                    s::SoapySDRStrings_clear(&mut (p as *mut *mut c_char), n);
                }
                let mut n = 0usize;
                let p = s::SoapySDRDevice_listAntennas(self.device, dir, 0, &mut n);
                for i in 0..n {
                    c.antennas
                        .push(CStr::from_ptr(*p.add(i)).to_string_lossy().into_owned());
                }
                if !p.is_null() {
                    s::SoapySDRStrings_clear(&mut (p as *mut *mut c_char), n);
                }
                c.antenna = string(s::SoapySDRDevice_getAntenna(self.device, dir, 0));
                let mut sn = 0usize;
                let sp = s::SoapySDRDevice_getSettingInfo(self.device, &mut sn);
                for i in 0..sn {
                    let a = &*sp.add(i);
                    let key = string(a.key);
                    let mut options = Vec::new();
                    for j in 0..a.numOptions {
                        options.push(string(*a.options.add(j)));
                    }
                    let kind = match a.type_ {
                        0 => "bool",
                        1 => "int",
                        2 => "float",
                        _ => "string",
                    }
                    .to_string();
                    c.settings.push(DeviceSetting {
                        key: key.clone(),
                        name: {
                            let n = string(a.name);
                            if n.is_empty() {
                                key.clone()
                            } else {
                                n
                            }
                        },
                        value: string(s::SoapySDRDevice_readSetting(
                            self.device,
                            CString::new(key).unwrap().as_ptr(),
                        )),
                        kind,
                        min: a.range.minimum,
                        max: a.range.maximum,
                        step: a.range.step,
                        options,
                    });
                }
                if !sp.is_null() {
                    s::SoapySDRArgInfoList_clear(sp, sn);
                }
                c.dc_offset_auto_supported = s::SoapySDRDevice_hasDCOffsetMode(self.device, dir, 0);
                c.dc_offset_auto = c.dc_offset_auto_supported
                    && s::SoapySDRDevice_getDCOffsetMode(self.device, dir, 0);
                c.iq_balance_auto_supported =
                    s::SoapySDRDevice_hasIQBalanceMode(self.device, dir, 0);
                c.iq_balance_auto = c.iq_balance_auto_supported
                    && s::SoapySDRDevice_getIQBalanceMode(self.device, dir, 0);
                c.frequency_correction_supported =
                    s::SoapySDRDevice_hasFrequencyCorrection(self.device, dir, 0);
                if c.frequency_correction_supported {
                    c.frequency_correction_ppm =
                        s::SoapySDRDevice_getFrequencyCorrection(self.device, dir, 0);
                }
                c
            }
        }
        pub fn set_control(&mut self, control: &str, value: &str) -> anyhow::Result<()> {
            unsafe {
                let dir = s::SOAPY_SDR_RX as i32;
                match control {
                    "agc" => check(
                        "set AGC",
                        s::SoapySDRDevice_setGainMode(self.device, dir, 0, value == "true"),
                    ),
                    "dc_offset_auto" => check(
                        "set DC auto",
                        s::SoapySDRDevice_setDCOffsetMode(self.device, dir, 0, value == "true"),
                    ),
                    "iq_balance_auto" => check(
                        "set IQ auto",
                        s::SoapySDRDevice_setIQBalanceMode(self.device, dir, 0, value == "true"),
                    ),
                    "frequency_correction_ppm" => check(
                        "set frequency correction",
                        s::SoapySDRDevice_setFrequencyCorrection(
                            self.device,
                            dir,
                            0,
                            value.parse()?,
                        ),
                    ),
                    "antenna" => {
                        let v = CString::new(value)?;
                        check(
                            "set antenna",
                            s::SoapySDRDevice_setAntenna(self.device, dir, 0, v.as_ptr()),
                        )
                    }
                    _ if control.starts_with("gain:") => {
                        let n = CString::new(&control[5..])?;
                        check(
                            "disable AGC",
                            s::SoapySDRDevice_setGainMode(self.device, dir, 0, false),
                        )?;
                        check(
                            "set gain stage",
                            s::SoapySDRDevice_setGainElement(
                                self.device,
                                dir,
                                0,
                                n.as_ptr(),
                                value.parse()?,
                            ),
                        )
                    }
                    _ if control.starts_with("setting:") => {
                        let k = CString::new(&control[8..])?;
                        let v = CString::new(value)?;
                        check(
                            "write device setting",
                            s::SoapySDRDevice_writeSetting(self.device, k.as_ptr(), v.as_ptr()),
                        )
                    }
                    _ => Err(anyhow::anyhow!("unsupported control: {control}")),
                }
            }
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn live_sdrplay_rsp1b_cf32_iq() {
            let _hardware_guard = crate::device::LIVE_HARDWARE_LOCK.lock().unwrap();
            eprintln!("stage=enumerate");
            let found = super::super::DeviceLayer::discover();
            assert!(
                found.iter().any(|d| d.driver == "sdrplay"),
                "RSP1B missing from discovery: {found:?}"
            );
            let key = super::super::DeviceLayer::discover()
                .into_iter()
                .find(|d| d.driver == "sdrplay")
                .expect("RSP1B missing from discovery")
                .key;
            eprintln!("stage=open");
            let mut dev =
                Hardware::open(&key, 2_000_000, 162_550_000).expect("open RSP1B CF32 stream");
            eprintln!("stage=read");
            let iq = dev.read(16384).expect("read RSP1B IQ");
            eprintln!("stage=read_done count={}", iq.len());
            assert!(iq.len() >= 1024, "insufficient IQ: {}", iq.len());
            let power = iq.iter().map(|x| x.re * x.re + x.im * x.im).sum::<f32>() / iq.len() as f32;
            eprintln!("stage=power value={power}");
            assert!(
                power.is_finite() && power > 1e-12,
                "zero RSP1B IQ power: {power}"
            );
            eprintln!("stage=drop");
        }
    }
    #[cfg(test)]
    mod lifecycle_tests {
        use super::Hardware;
        #[test]
        fn live_rsp1b_retune_rate_reconnect() {
            let _hardware_guard = crate::device::LIVE_HARDWARE_LOCK.lock().unwrap();
            let key = super::super::DeviceLayer::discover()
                .into_iter()
                .find(|d| d.driver == "sdrplay")
                .expect("RSP1B missing from discovery")
                .key;
            let mut dev = Hardware::open(&key, 2_000_000, 162_550_000).expect("open RSP1B");
            assert!(dev.read(4096).expect("initial read").len() == 4096);
            dev.set_frequency(162_500_000).expect("retune RSP1B");
            assert!(dev.read(4096).expect("post-retune read").len() == 4096);
            dev.set_rate(1_000_000).expect("change sample rate");
            assert!(dev.read(4096).expect("post-rate read").len() == 4096);
            drop(dev);
            let mut reopened =
                Hardware::open(&key, 2_000_000, 162_550_000).expect("reconnect RSP1B");
            assert!(reopened.read(4096).expect("reconnect read").len() == 4096);
        }
    }
    impl Drop for Hardware {
        fn drop(&mut self) {
            unsafe {
                if !self.stream.is_null() {
                    let _ = s::SoapySDRDevice_deactivateStream(self.device, self.stream, 0, 0);
                    let _ = s::SoapySDRDevice_closeStream(self.device, self.stream);
                }
                if !self.device.is_null() {
                    let _ = s::SoapySDRDevice_unmake(self.device);
                }
            }
        }
    }
}

/// Cross-platform candidate path list for `SoapySDRUtil`.
///
/// Resolution order:
///   1. `PULSESCOPE_SOAPY_UTIL` env override (absolute path or bare name)
///   2. `SOAPY_SDR_ROOT` env var, with a `bin/` subdirectory appended
///   3. `SOAPYSDR_HOME` env var, with a `bin/` subdirectory appended
///   4. PothosSDR default install location on Windows
///   5. Standard Linux/macOS paths (`/usr/local/bin`, `/usr/bin`, `/usr/lib/SoapySDR/bin`)
///   6. Bare name `SoapySDRUtil{,.exe}` resolved via PATH lookup
///
/// Only paths that actually exist on the current filesystem are returned.
pub fn build_soapy_discovery_paths() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;

    let mut out: Vec<PathBuf> = Vec::new();

    if let Ok(path) = std::env::var("PULSESCOPE_SOAPY_UTIL") {
        if !path.trim().is_empty() {
            out.push(PathBuf::from(path.trim()));
        }
    }
    let bin_under = |root: &str| -> PathBuf {
        let mut p = PathBuf::from(root);
        // PothosSDR uses `bin\` on Windows; *nix users set SOAPY_SDR_ROOT directly to the `bin/` dir,
        // so do not double-append.
        if !p.ends_with("bin") && !p.ends_with("bin\\") {
            p.push("bin");
        }
        p.push(soapy_util_exe());
        p
    };
    if let Ok(root) = std::env::var("SOAPY_SDR_ROOT") {
        out.push(bin_under(&root));
    }
    if let Ok(home) = std::env::var("SOAPYSDR_HOME") {
        out.push(bin_under(&home));
    }

    if cfg!(windows) {
        out.push(PathBuf::from(
            r"C:\Program Files\PothosSDR\bin\SoapySDRUtil.exe",
        ));
        out.push(PathBuf::from(
            r"C:\Program Files (x86)\PothosSDR\bin\SoapySDRUtil.exe",
        ));
    } else {
        out.push(PathBuf::from("/usr/local/bin/SoapySDRUtil"));
        out.push(PathBuf::from("/usr/bin/SoapySDRUtil"));
        out.push(PathBuf::from("/opt/homebrew/bin/SoapySDRUtil"));
    }

    // Final fallback: bare name. `Command::new` will resolve via PATH lookup.
    out.push(PathBuf::from(soapy_util_exe()));

    out
}

/// The plausible executable name for the current OS.
fn soapy_util_exe() -> &'static str {
    if cfg!(windows) {
        "SoapySDRUtil.exe"
    } else {
        "SoapySDRUtil"
    }
}

#[cfg(feature = "soapysdr")]
fn discover_soapy_util() -> Vec<DiscoveredDevice> {
    // Cross-platform SoapySDR discovery search list. Order:
    //   1. Explicit override via PULSESCOPE_SOAPY_UTIL
    //   2. SOAPY_SDR_ROOT environment variable (`bin/` subdirectory)
    //   3. SOAPYSDR_HOME if set
    //   4. Platform conventions: PothosSDR (Windows), /usr/local/lib/SoapySDR (Linux/macOS via BrewPkg dir), /usr/bin
    //   5. Bare name `SoapySDRUtil{,exe}` resolved via PATH
    let candidates: Vec<std::path::PathBuf> = build_soapy_discovery_paths();
    let mut rows = Vec::new();
    for exe in candidates {
        let Ok(out) = Command::new(&exe).arg("--find").output() else {
            continue;
        };
        // Soapy warnings/info can be emitted on stderr; device records are
        // normally stdout, but combine both so discovery is runtime independent.
        let mut combined = out.stdout;
        combined.extend_from_slice(&out.stderr);
        let text = String::from_utf8_lossy(&combined);
        let mut props = std::collections::BTreeMap::new();
        let push = |p: &mut std::collections::BTreeMap<String, String>,
                    rows: &mut Vec<DiscoveredDevice>| {
            if let Some(driver) = p.get("driver").cloned() {
                if driver != "audio" {
                    let mut kv = vec![format!("driver={driver}")];
                    if let Some(serial) = p.get("serial") {
                        kv.push(format!("serial={serial}"));
                    } else if let Some(id) = p.get("device_id") {
                        kv.push(format!("device_id={id}"));
                    }
                    let key = kv.join(",");
                    if !rows.iter().any(|d| d.key == key) {
                        let label = p.get("label").cloned().unwrap_or_else(|| driver.clone());
                        rows.push(DiscoveredDevice {
                            driver: driver.clone(),
                            label,
                            key,
                            hardware_key: driver,
                        });
                    }
                }
            }
            p.clear();
        };
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("Found device ") {
                push(&mut props, &mut rows);
            } else if let Some((k, v)) = line.split_once(" = ") {
                props.insert(k.to_string(), v.to_string());
            }
        }
        push(&mut props, &mut rows);
    }
    rows
}

pub struct DeviceLayer {
    state: Arc<Mutex<DeviceStatus>>,
    phase: Arc<Mutex<f32>>,
    counters: Arc<StreamCounters>,
    last_key: Arc<Mutex<Option<String>>>,
    #[cfg(feature = "soapysdr")]
    hardware: Arc<Mutex<Option<soapy::Hardware>>>,
}

impl DeviceLayer {
    pub fn new_mock() -> Self {
        Self {
            state: Arc::new(Mutex::new(DeviceStatus {
                connected: false,
                lifecycle: DeviceLifecycle::Disconnected,
                driver: "mock".into(),
                label: "Mock Source (Test Tones)".into(),
                sample_rate: 10_000_000,
                bandwidth_hz: 10_000_000,
                center_freq_hz: 100_000_000,
                ppm_correction: 0.0,
                gain: "auto".into(),
                bias_tee_on: false,
                saturation: false,
                stream: StreamCountersSnapshot::default(),
            })),
            phase: Arc::new(Mutex::new(0.0)),
            counters: Arc::new(StreamCounters::default()),
            last_key: Arc::new(Mutex::new(None)),
            #[cfg(feature = "soapysdr")]
            hardware: Arc::new(Mutex::new(None)),
        }
    }

    pub fn status(&self) -> DeviceStatus {
        let mut status = self.state.lock().clone();
        status.stream = self.counters.snapshot();
        status
    }

    pub fn discover() -> Vec<DiscoveredDevice> {
        #[allow(unused_mut)]
        let mut devices = vec![DiscoveredDevice {
            driver: "mock".into(),
            label: "Mock Source (Test Tones)".into(),
            key: "driver=mock".into(),
            hardware_key: "mock".into(),
        }];
        #[cfg(feature = "soapysdr")]
        devices.extend(discover_soapy_util());
        devices
    }

    pub fn auto_connect(&self, preferred: Option<&str>) -> anyhow::Result<()> {
        // A vendor runtime may expose a usable device through its native
        // driver while SoapySDRUtil is absent from PATH. Do not discard a
        // persisted physical key merely because discovery could not list it.
        if let Some(key) = preferred.filter(|key| !key.trim().is_empty() && *key != "driver=mock") {
            if self.connect(key).is_ok() {
                return Ok(());
            }
        }
        let devices = Self::discover();
        let candidate = devices
            .into_iter()
            .find(|device| device.driver != "mock")
            .map(|device| device.key);
        self.connect(candidate.as_deref().unwrap_or("driver=mock"))
    }

    pub fn connect(&self, key: &str) -> anyhow::Result<()> {
        self.state.lock().lifecycle = DeviceLifecycle::Probing;
        *self.last_key.lock() = Some(key.to_owned());
        if key != "driver=mock" {
            #[cfg(feature = "soapysdr")]
            {
                self.state.lock().lifecycle = DeviceLifecycle::Configuring;
                let rate = 2_000_000;
                let freq = 100_000_000;
                let mut hardware = match soapy::Hardware::open(key, rate, freq) {
                    Ok(hardware) => hardware,
                    Err(error) => {
                        let mut status = self.state.lock();
                        status.connected = false;
                        status.lifecycle = DeviceLifecycle::Degraded;
                        *self.counters.last_error.lock() = Some(error.to_string());
                        return Err(error);
                    }
                };
                hardware.apply_safe_defaults()?;
                *self.hardware.lock() = Some(hardware);
                let mut status = self.state.lock();
                status.connected = true;
                status.lifecycle = DeviceLifecycle::Streaming;
                status.sample_rate = rate;
                status.bandwidth_hz = rate;
                status.center_freq_hz = freq;
                status.driver = key
                    .split(',')
                    .find_map(|part| part.trim().strip_prefix("driver="))
                    .unwrap_or("soapy")
                    .to_string();
                status.label = key.to_string();
                return Ok(());
            }
            #[cfg(not(feature = "soapysdr"))]
            {
                self.state.lock().lifecycle = DeviceLifecycle::Degraded;
                return Err(anyhow::anyhow!(
                    "SoapySDR support is not present in this build"
                ));
            }
        }
        let mut status = self.state.lock();
        status.connected = true;
        status.lifecycle = DeviceLifecycle::Streaming;
        status.driver = "mock".into();
        status.label = "Mock Source (Test Tones)".into();
        Ok(())
    }

    pub fn disconnect(&self) -> anyhow::Result<()> {
        #[cfg(feature = "soapysdr")]
        self.hardware.lock().take();
        let mut status = self.state.lock();
        status.connected = false;
        status.lifecycle = DeviceLifecycle::Disconnected;
        Ok(())
    }

    pub fn recover(&self) -> anyhow::Result<()> {
        let key = self
            .last_key
            .lock()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no previous device to recover"))?;
        if key == "driver=mock" {
            return Ok(());
        }
        self.state.lock().lifecycle = DeviceLifecycle::Recovering;
        self.counters.restarts.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "soapysdr")]
        self.hardware.lock().take();
        self.connect(&key)
    }

    pub fn set_frequency(&self, freq: u64) -> anyhow::Result<()> {
        let caps = self.capabilities();
        if !Self::frequency_in_range(&caps, freq) {
            return Err(anyhow::anyhow!(
                "frequency {} Hz is outside supported RF ranges",
                freq
            ));
        }
        #[cfg(feature = "soapysdr")]
        if let Some(hardware) = self.hardware.lock().as_mut() {
            hardware.set_frequency(freq)?;
        }
        self.state.lock().center_freq_hz = freq;
        self.counters.retunes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub fn frequency_in_range(capabilities: &DeviceCapabilities, freq: u64) -> bool {
        if capabilities.rf_ranges_hz.is_empty() {
            return true;
        }
        capabilities
            .rf_ranges_hz
            .iter()
            .any(|range| freq >= range.minimum as u64 && freq <= range.maximum as u64)
    }

    pub fn set_sample_rate(&self, rate: u32) -> anyhow::Result<()> {
        if rate == 0 {
            return Err(anyhow::anyhow!("sample rate must be greater than zero"));
        }
        #[cfg(feature = "soapysdr")]
        if let Some(hardware) = self.hardware.lock().as_mut() {
            hardware.set_rate(rate)?;
        }
        self.state.lock().sample_rate = rate;
        Ok(())
    }

    /// Apply a coherent sampled-spectrum contract. SDR channel bandwidth is
    /// the analog frontend window, not an individual demodulator passband.
    pub fn set_sample_contract(&self, rate: u32) -> anyhow::Result<u32> {
        self.set_sample_rate(rate)?;
        let capabilities = self.capabilities();
        let bandwidth = capabilities
            .bandwidth_ranges_hz
            .iter()
            .filter_map(|range| {
                let candidate = (rate as f64).clamp(range.minimum, range.maximum);
                (candidate > 0.0 && candidate <= rate as f64).then_some(candidate as u32)
            })
            .max()
            .ok_or_else(|| {
                anyhow::anyhow!("no analog bandwidth range supports sample rate {} Hz", rate)
            })?;
        self.set_bandwidth(bandwidth)?;
        Ok(bandwidth)
    }

    pub fn set_bandwidth(&self, bandwidth: u32) -> anyhow::Result<()> {
        if bandwidth == 0 {
            return Err(anyhow::anyhow!("bandwidth must be greater than zero"));
        }
        #[cfg(feature = "soapysdr")]
        if let Some(hardware) = self.hardware.lock().as_mut() {
            hardware.set_bandwidth(bandwidth)?;
        }
        self.state.lock().bandwidth_hz = bandwidth;
        Ok(())
    }

    pub fn set_gain(&self, gain: String) -> anyhow::Result<()> {
        if gain.eq_ignore_ascii_case("auto") {
            self.state.lock().gain = gain;
            return Ok(());
        }
        let value = gain
            .parse::<f64>()
            .map_err(|_| anyhow::anyhow!("gain must be numeric dB or 'auto'"))?;
        #[cfg(feature = "soapysdr")]
        if let Some(hardware) = self.hardware.lock().as_mut() {
            hardware.set_gain(value)?;
        }
        #[cfg(not(feature = "soapysdr"))]
        let _ = value;
        self.state.lock().gain = gain;
        Ok(())
    }

    pub fn capabilities(&self) -> DeviceCapabilities {
        let status = self.status();
        #[cfg(feature = "soapysdr")]
        if let Some(hardware) = self.hardware.lock().as_ref() {
            let mut capabilities = hardware.capabilities();
            capabilities.contract_version = 2;
            capabilities.connected = status.connected;
            capabilities.total_bandwidth_hz = status.bandwidth_hz;
            capabilities.usable_bandwidth_hz = status
                .bandwidth_hz
                .min((status.sample_rate as f64 * 0.9) as u32);
            if capabilities.supported_sample_rates_hz.is_empty() {
                capabilities
                    .supported_sample_rates_hz
                    .push(status.sample_rate);
            }
            capabilities.identity = self.identity(&status);
            return capabilities;
        }
        DeviceCapabilities {
            contract_version: 2,
            identity: self.identity(&status),
            connected: status.connected,
            rf_ranges_hz: vec![NumericRange {
                minimum: 0.0,
                maximum: 6_000_000_000.0,
                step: 1.0,
            }],
            sample_rate_ranges_hz: vec![NumericRange {
                minimum: 48_000.0,
                maximum: 20_000_000.0,
                step: 1.0,
            }],
            supported_sample_rates_hz: vec![2_000_000, 2_400_000, 5_000_000, 10_000_000],
            bandwidth_ranges_hz: vec![NumericRange {
                minimum: 12_000.0,
                maximum: 10_000_000.0,
                step: 1.0,
            }],
            sample_formats: vec!["CF32".into()],
            stream_mtu: 16_384,
            total_bandwidth_hz: status.bandwidth_hz,
            usable_bandwidth_hz: status
                .bandwidth_hz
                .min((status.sample_rate as f64 * 0.9) as u32),
            tuner_count: 1,
            ..Default::default()
        }
    }

    fn identity(&self, status: &DeviceStatus) -> DeviceIdentity {
        let key = self
            .last_key
            .lock()
            .clone()
            .unwrap_or_else(|| format!("driver={}", status.driver));
        let serial = key
            .split(',')
            .find_map(|part| part.trim().strip_prefix("serial="))
            .map(str::to_owned);
        DeviceIdentity {
            stable_id: serial
                .as_ref()
                .map(|serial| format!("{}:{serial}", status.driver))
                .unwrap_or_else(|| key.clone()),
            driver: status.driver.clone(),
            label: status.label.clone(),
            serial,
            connection: if status.driver == "mock" {
                "virtual".into()
            } else {
                "usb_or_network".into()
            },
            hardware_key: key,
            firmware_version: None,
            api_version: None,
        }
    }

    pub fn stream_mtu(&self) -> usize {
        self.capabilities().stream_mtu
    }

    pub fn set_control(&self, control: &str, value: &str) -> anyhow::Result<()> {
        #[cfg(feature = "soapysdr")]
        if let Some(hardware) = self.hardware.lock().as_mut() {
            hardware.set_control(control, value)?;
            return Ok(());
        }
        #[cfg(not(feature = "soapysdr"))]
        let _ = (control, value);
        Err(anyhow::anyhow!(
            "connect a hardware SDR before changing controls"
        ))
    }

    fn observe_read(
        &self,
        requested: usize,
        result: anyhow::Result<Vec<Complex<f32>>>,
    ) -> anyhow::Result<Vec<Complex<f32>>> {
        self.counters.read_calls.fetch_add(1, Ordering::Relaxed);
        match result {
            Ok(samples) => {
                self.counters
                    .samples_received
                    .fetch_add(samples.len() as u64, Ordering::Relaxed);
                if samples.len() < requested {
                    self.counters.short_reads.fetch_add(1, Ordering::Relaxed);
                }
                self.counters.consecutive_errors.store(0, Ordering::Relaxed);
                self.counters
                    .last_sample_ms
                    .store(crate::scanner::now_ms().max(0) as u64, Ordering::Relaxed);
                *self.counters.last_error.lock() = None;
                self.state.lock().lifecycle = DeviceLifecycle::Streaming;
                Ok(samples)
            }
            Err(error) => {
                self.counters.source_errors.fetch_add(1, Ordering::Relaxed);
                self.counters
                    .consecutive_errors
                    .fetch_add(1, Ordering::Relaxed);
                *self.counters.last_error.lock() = Some(error.to_string());
                self.state.lock().lifecycle = DeviceLifecycle::Degraded;
                Err(error)
            }
        }
    }

    pub fn read_iq(&self, count: usize) -> anyhow::Result<Vec<Complex<f32>>> {
        #[cfg(feature = "soapysdr")]
        {
            let mut hardware = self.hardware.lock();
            if let Some(device) = hardware.as_mut() {
                let result = device.read(count);
                drop(hardware);
                return self.observe_read(count, result);
            }
        }
        let status = self.status();
        if !status.connected || status.driver != "mock" {
            return self.observe_read(
                count,
                Err(anyhow::anyhow!("hardware IQ stream unavailable")),
            );
        }
        let mut phase = self.phase.lock();
        let mut samples = Vec::with_capacity(count);
        for i in 0..count {
            let time = *phase + i as f32;
            let a = Complex::from_polar(0.18, TAU * time / 97.0);
            let b = Complex::from_polar(0.08, TAU * time / 211.0);
            let noise = (((i as f32 * 12.9898).sin() * 43_758.547).fract() - 0.5) * 0.012;
            samples.push(a + b + Complex::new(noise, -noise * 0.7));
        }
        *phase += count as f32;
        drop(phase);
        self.observe_read(count, Ok(samples))
    }
}

impl RadioDevice for DeviceLayer {
    fn status(&self) -> DeviceStatus {
        DeviceLayer::status(self)
    }
    fn capabilities(&self) -> DeviceCapabilities {
        DeviceLayer::capabilities(self)
    }
    fn connect(&self, key: &str) -> anyhow::Result<()> {
        DeviceLayer::connect(self, key)
    }
    fn disconnect(&self) -> anyhow::Result<()> {
        DeviceLayer::disconnect(self)
    }
    fn recover(&self) -> anyhow::Result<()> {
        DeviceLayer::recover(self)
    }
    fn set_frequency(&self, frequency_hz: u64) -> anyhow::Result<()> {
        DeviceLayer::set_frequency(self, frequency_hz)
    }
    fn set_sample_contract(&self, sample_rate_hz: u32) -> anyhow::Result<u32> {
        DeviceLayer::set_sample_contract(self, sample_rate_hz)
    }
    fn set_gain(&self, gain: String) -> anyhow::Result<()> {
        DeviceLayer::set_gain(self, gain)
    }
    fn set_control(&self, control: &str, value: &str) -> anyhow::Result<()> {
        DeviceLayer::set_control(self, control, value)
    }
    fn read_iq(&self, sample_count: usize) -> anyhow::Result<Vec<Complex<f32>>> {
        DeviceLayer::read_iq(self, sample_count)
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn mock_publishes_versioned_capabilities_and_stream_mtu() {
        let device = DeviceLayer::new_mock();
        device.connect("driver=mock").unwrap();
        let capabilities = device.capabilities();
        assert_eq!(capabilities.contract_version, 2);
        assert_eq!(capabilities.identity.connection, "virtual");
        assert!(capabilities.stream_mtu >= 512);
        assert!(capabilities.usable_bandwidth_hz < capabilities.total_bandwidth_hz);
        assert!(capabilities.supported_sample_rates_hz.contains(&2_400_000));
    }

    #[test]
    fn stream_status_counts_reads_samples_and_retunes() {
        let device = DeviceLayer::new_mock();
        device.connect("driver=mock").unwrap();
        device.set_frequency(101_100_000).unwrap();
        let samples = device.read_iq(4_096).unwrap();
        let status = device.status();
        assert_eq!(samples.len(), 4_096);
        assert_eq!(status.lifecycle, DeviceLifecycle::Streaming);
        assert_eq!(status.stream.read_calls, 1);
        assert_eq!(status.stream.samples_received, 4_096);
        assert_eq!(status.stream.retunes, 1);
        assert!(status.stream.last_sample_ms > 0);
    }
}
