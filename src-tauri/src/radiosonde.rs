//! Native RS41-class GFSK radiosonde telemetry decoder.
//!
//! Clean-room 4800 baud GFSK with a published-style 0x10 0xB6 sync, CRC-16, and
//! `radiosonde.telemetry.v1` events. Reed-Solomon FEC used by live Vaisala RS41
//! frames is **not** claimed here; that path stays on the `rs41mod` sidecar.
//! Map/table consumers must ignore frames with `checksum_valid = false`.

use std::path::PathBuf;

use rustfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::db::DecodedMessage;
use crate::demod::decimate_complex_average;

pub const RADIOSONDE_SYMBOL_RATE: u32 = 4_800;
pub const RADIOSONDE_FIXTURE_SAMPLE_RATE_HZ: u32 = 192_000;
pub const RADIOSONDE_SYNC: [u8; 2] = [0x10, 0xB6];
pub const RADIOSONDE_PREAMBLE: u8 = 0xAA;
pub const FIXTURE_SERIAL: &str = "PULSE001";
pub const FIXTURE_MODEL: &str = "RS41";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RadiosondeTelemetry {
    pub model: String,
    pub serial: String,
    pub frame: u16,
    pub lat: f64,
    pub lon: f64,
    pub altitude_m: f64,
    pub temperature_c: f64,
    pub checksum_valid: bool,
    pub raw_hex: String,
}

impl RadiosondeTelemetry {
    pub fn to_decoded(&self, frequency_hz: u64) -> DecodedMessage {
        DecodedMessage {
            id: None,
            frequency_hz,
            protocol: "radiosonde".into(),
            message_type: "telemetry".into(),
            address: self.serial.clone(),
            function_code: format!("{}#{}", self.model, self.frame),
            content: format!(
                "model={} serial={} frame={} lat={:.5} lon={:.5} altitude_m={:.1} temperature_c={:.2} checksum_valid={}",
                self.model,
                self.serial,
                self.frame,
                self.lat,
                self.lon,
                self.altitude_m,
                self.temperature_c,
                self.checksum_valid
            ),
            raw: self.raw_hex.clone(),
            encryption: "none".into(),
            timestamp_ms: crate::scanner::now_ms(),
        }
    }
}

pub fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

pub fn find_rs41mod() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PULSESCOPE_RS41MOD") {
        let path = PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
    }
    which::which("rs41mod").ok()
}

pub fn rs41mod_stdin_args() -> Vec<String> {
    Vec::new()
}

fn bits_msb_first(bytes: &[u8]) -> Vec<u8> {
    let mut bits = Vec::with_capacity(bytes.len() * 8);
    for &byte in bytes {
        for i in (0..8).rev() {
            bits.push((byte >> i) & 1);
        }
    }
    bits
}

fn bits_to_bytes_msb(bits: &[u8]) -> Vec<u8> {
    bits.chunks(8)
        .filter(|chunk| chunk.len() == 8)
        .map(|chunk| {
            let mut value = 0u8;
            for (i, bit) in chunk.iter().enumerate() {
                value |= bit << (7 - i);
            }
            value
        })
        .collect()
}

fn gaussian_pulse(samples_per_symbol: usize) -> Vec<f32> {
    let n = samples_per_symbol * 3;
    let alpha = 1.5f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / samples_per_symbol as f32 - 1.5;
            f32::exp(-(alpha * t).powi(2))
        })
        .collect()
}

pub fn modulate_gfsk(bits: &[u8], sample_rate_hz: u32) -> Vec<Complex<f32>> {
    let sps = (sample_rate_hz / RADIOSONDE_SYMBOL_RATE).max(1) as usize;
    let pulse = gaussian_pulse(sps);
    let mut nrz = vec![0.0f32; bits.len() * sps + pulse.len()];
    for (i, &bit) in bits.iter().enumerate() {
        let v = if bit != 0 { 1.0 } else { -1.0 };
        for (p, &w) in pulse.iter().enumerate() {
            nrz[i * sps + p] += v * w;
        }
    }
    let deviation = 2_400.0_f64;
    let mut phase = 0.0f64;
    let step = std::f64::consts::TAU * deviation / f64::from(sample_rate_hz.max(1));
    nrz.iter()
        .map(|&s| {
            phase += step * f64::from(s);
            Complex::new(phase.cos() as f32, phase.sin() as f32)
        })
        .collect()
}

fn discriminator(iq: &[Complex<f32>]) -> Vec<f32> {
    let mut out = Vec::with_capacity(iq.len());
    let mut prev = iq.first().copied().unwrap_or(Complex::new(1.0, 0.0));
    for &sample in iq {
        let cross = sample.im * prev.re - sample.re * prev.im;
        let dot = sample.re * prev.re + sample.im * prev.im;
        out.push(cross.atan2(dot));
        prev = sample;
    }
    out
}

fn slice_bits(disc: &[f32], sample_rate_hz: u32, offset: usize) -> Vec<u8> {
    let sps = (sample_rate_hz / RADIOSONDE_SYMBOL_RATE).max(1) as usize;
    let mut bits = Vec::new();
    let mut i = offset.min(sps.saturating_sub(1));
    while i < disc.len() {
        bits.push(if disc[i] >= 0.0 { 1 } else { 0 });
        i += sps;
    }
    bits
}

fn encode_i32_be(value: i32) -> [u8; 4] {
    value.to_be_bytes()
}

fn encode_i16_be(value: i16) -> [u8; 2] {
    value.to_be_bytes()
}

fn padded_ascii(value: &str, len: usize) -> Vec<u8> {
    let mut out = vec![b' '; len];
    let bytes = value.as_bytes();
    let copy = bytes.len().min(len);
    out[..copy].copy_from_slice(&bytes[..copy]);
    out
}

pub fn build_telemetry_frame(
    model: &str,
    serial: &str,
    frame: u16,
    lat: f64,
    lon: f64,
    altitude_m: f64,
    temperature_c: f64,
) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&padded_ascii(model, 4));
    payload.extend_from_slice(&padded_ascii(serial, 8));
    payload.extend_from_slice(&frame.to_be_bytes());
    payload.extend_from_slice(&encode_i32_be((lat * 1.0e7).round() as i32));
    payload.extend_from_slice(&encode_i32_be((lon * 1.0e7).round() as i32));
    payload.extend_from_slice(&encode_i32_be((altitude_m * 100.0).round() as i32));
    payload.extend_from_slice(&encode_i16_be((temperature_c * 100.0).round() as i16));
    let mut body = Vec::with_capacity(payload.len() + 1);
    body.push(payload.len() as u8);
    body.extend_from_slice(&payload);
    let crc = crc16_ccitt(&body);
    body.extend_from_slice(&crc.to_be_bytes());
    let mut framed = vec![RADIOSONDE_PREAMBLE; 8];
    framed.extend_from_slice(&RADIOSONDE_SYNC);
    framed.extend_from_slice(&body);
    framed
}

pub fn parse_frame(bytes: &[u8]) -> Option<RadiosondeTelemetry> {
    let sync_at = bytes.windows(2).position(|w| w == RADIOSONDE_SYNC)?;
    let body = bytes.get(sync_at + 2..)?;
    if body.len() < 4 {
        return None;
    }
    let payload_len = body[0] as usize;
    if body.len() < 1 + payload_len + 2 {
        return None;
    }
    let payload = &body[1..1 + payload_len];
    let crc_bytes = &body[1 + payload_len..1 + payload_len + 2];
    let crc = u16::from_be_bytes([crc_bytes[0], crc_bytes[1]]);
    let checksum_valid = crc == crc16_ccitt(&body[..1 + payload_len]);
    if !checksum_valid || payload.len() < 28 {
        return None;
    }
    let model = String::from_utf8_lossy(&payload[0..4]).trim().to_string();
    let serial = String::from_utf8_lossy(&payload[4..12]).trim().to_string();
    let frame = u16::from_be_bytes([payload[12], payload[13]]);
    let lat = f64::from(i32::from_be_bytes(payload[14..18].try_into().ok()?)) / 1.0e7;
    let lon = f64::from(i32::from_be_bytes(payload[18..22].try_into().ok()?)) / 1.0e7;
    let altitude_m = f64::from(i32::from_be_bytes(payload[22..26].try_into().ok()?)) / 100.0;
    let temperature_c = f64::from(i16::from_be_bytes(payload[26..28].try_into().ok()?)) / 100.0;
    Some(RadiosondeTelemetry {
        model,
        serial,
        frame,
        lat,
        lon,
        altitude_m,
        temperature_c,
        checksum_valid: true,
        raw_hex: hex::encode(&body[..1 + payload_len + 2]),
    })
}

pub fn decode_bits(bits: &[u8]) -> Vec<RadiosondeTelemetry> {
    let mut found = Vec::new();
    for align in 0..8 {
        if align >= bits.len() {
            break;
        }
        let bytes = bits_to_bytes_msb(&bits[align..]);
        if let Some(telemetry) = parse_frame(&bytes) {
            found.push(telemetry);
            break;
        }
    }
    found
}

pub fn decode_iq(iq: &[Complex<f32>], sample_rate_hz: u32) -> Vec<RadiosondeTelemetry> {
    if iq.len() < 64 || sample_rate_hz < RADIOSONDE_SYMBOL_RATE * 8 {
        return Vec::new();
    }
    let (samples, rate) = if sample_rate_hz >= RADIOSONDE_FIXTURE_SAMPLE_RATE_HZ * 2 {
        let factor = (sample_rate_hz / RADIOSONDE_FIXTURE_SAMPLE_RATE_HZ) as usize;
        (
            decimate_complex_average(iq, factor.max(1)),
            sample_rate_hz / factor.max(1) as u32,
        )
    } else {
        (iq.to_vec(), sample_rate_hz)
    };
    let disc = discriminator(&samples);
    let sps = (rate / RADIOSONDE_SYMBOL_RATE).max(1) as usize;
    for offset in 0..sps {
        let bits = slice_bits(&disc, rate, offset);
        let found = decode_bits(&bits);
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

pub fn synthesize_pulse_rs41_iq(sample_rate_hz: u32) -> Vec<Complex<f32>> {
    let frame = build_telemetry_frame(
        FIXTURE_MODEL,
        FIXTURE_SERIAL,
        42,
        39.739_15,
        -104.984_70,
        12_500.0,
        -45.25,
    );
    let mut bits = vec![1, 0, 1, 0, 1, 0, 1, 0];
    bits.extend(bits_msb_first(&frame));
    bits.extend([0, 1, 0, 1, 0, 1, 0, 1]);
    modulate_gfsk(&bits, sample_rate_hz)
}

/// Parse documented rs41mod / sonde stdout. Checksum-invalid lines are dropped.
pub fn parse_sidecar_line(line: &str) -> Option<RadiosondeTelemetry> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let checksum_valid = value
            .get("checksum_valid")
            .or_else(|| value.get("crc_ok"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if !checksum_valid {
            return None;
        }
        let serial = value
            .get("serial")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if serial.is_empty() {
            return None;
        }
        return Some(RadiosondeTelemetry {
            model: value
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("RS41")
                .to_string(),
            serial,
            frame: value.get("frame").and_then(|v| v.as_u64()).unwrap_or(0) as u16,
            lat: value.get("lat").and_then(|v| v.as_f64()).unwrap_or(0.0),
            lon: value.get("lon").and_then(|v| v.as_f64()).unwrap_or(0.0),
            altitude_m: value
                .get("alt")
                .or_else(|| value.get("altitude_m"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            temperature_c: value
                .get("temp")
                .or_else(|| value.get("temperature_c"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            checksum_valid: true,
            raw_hex: trimmed.to_string(),
        });
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("crc=fail")
        || lower.contains("checksum_valid=false")
        || lower.contains("crc_ok=false")
    {
        return None;
    }
    let serial = field_after(trimmed, "serial=")?;
    Some(RadiosondeTelemetry {
        model: field_after(trimmed, "model=").unwrap_or_else(|| "RS41".into()),
        serial,
        frame: field_after(trimmed, "frame=")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        lat: field_after(trimmed, "lat=")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0),
        lon: field_after(trimmed, "lon=")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0),
        altitude_m: field_after(trimmed, "alt=")
            .or_else(|| field_after(trimmed, "altitude_m="))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0),
        temperature_c: field_after(trimmed, "temp=")
            .or_else(|| field_after(trimmed, "tes="))
            .or_else(|| field_after(trimmed, "temperature_c="))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0),
        checksum_valid: true,
        raw_hex: trimmed.to_string(),
    })
}

fn field_after(line: &str, marker: &str) -> Option<String> {
    let (_, rest) = line.split_once(marker)?;
    let mut value = rest.trim();
    for next in [
        " serial=",
        " model=",
        " frame=",
        " lat=",
        " lon=",
        " alt=",
        " tes=",
        " temp=",
        " crc=",
        " checksum_valid=",
    ] {
        if let Some((head, _)) = value.split_once(next.trim_start()) {
            value = head.trim();
        }
        let spaced = format!(" {next}");
        if let Some((head, _)) = value.split_once(spaced.trim_start()) {
            value = head.trim();
        }
    }
    let value = value
        .split_whitespace()
        .next()
        .unwrap_or(value)
        .trim_matches([',', ';', ':']);
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

pub fn in_band(center_hz: u64) -> bool {
    (400_000_000..=406_500_000).contains(&center_hz)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc16_detects_payload_tampering() {
        let mut frame = build_telemetry_frame("RS41", "PULSE001", 1, 1.0, 2.0, 100.0, 12.0);
        let original = parse_frame(&frame).unwrap();
        assert!(original.checksum_valid);
        let last = frame.len() - 1;
        frame[last] ^= 0xFF;
        assert!(parse_frame(&frame).is_none());
    }

    #[test]
    fn fixture_iq_recovers_telemetry() {
        let iq = synthesize_pulse_rs41_iq(RADIOSONDE_FIXTURE_SAMPLE_RATE_HZ);
        let messages = decode_iq(&iq, RADIOSONDE_FIXTURE_SAMPLE_RATE_HZ);
        let tel = messages
            .iter()
            .find(|m| m.serial == FIXTURE_SERIAL)
            .expect("serial");
        assert_eq!(tel.model, FIXTURE_MODEL);
        assert_eq!(tel.frame, 42);
        assert!((tel.lat - 39.739_15).abs() < 1e-4);
        assert!((tel.lon + 104.984_70).abs() < 1e-4);
        assert!((tel.altitude_m - 12_500.0).abs() < 1.0);
        assert!(tel.checksum_valid);
        let decoded = tel.to_decoded(402_500_000);
        assert_eq!(decoded.protocol, "radiosonde");
        assert!(decoded.content.contains("checksum_valid=true"));
    }

    #[test]
    fn sidecar_drops_failed_checksum_lines() {
        assert!(parse_sidecar_line("serial=RS41-001 lat=39.7 crc=fail").is_none());
        let ok = parse_sidecar_line(
            "model=RS41 serial=RS41-001 frame=9 lat=39.7 lon=-104.9 alt=1000 temp=-12 crc=ok",
        )
        .unwrap();
        assert_eq!(ok.serial, "RS41-001");
        assert_eq!(ok.frame, 9);
        let json = parse_sidecar_line(
            r#"{"serial":"T1234567","lat":51.5,"lon":-0.12,"alt":12345.0,"checksum_valid":true}"#,
        )
        .unwrap();
        assert_eq!(json.serial, "T1234567");
        assert!(parse_sidecar_line(r#"{"serial":"X","checksum_valid":false}"#).is_none());
    }
}
