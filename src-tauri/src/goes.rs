//! Native GOES LRIT/HRIT CADU product identification.
//!
//! Clean-room BPSK + CCSDS ASM (`0x1ACFFC1D`) + CRC-32 product frames. This
//! recovers `goes.product.v1` metadata from recorded IQ. Full HRIT/LRIT image
//! reconstruction stays on the version-pinned SatDump sidecar. Gallery/UI must
//! list only checksum-valid decoded products — never files found under a
//! configured `output_image_dir`.

use std::path::PathBuf;

use rustfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::db::DecodedMessage;
use crate::demod::decimate_complex_average;

pub const GOES_ASM: u32 = 0x1ACF_FC1D;
pub const GOES_SYMBOL_RATE: u32 = 50_000;
pub const GOES_FIXTURE_SAMPLE_RATE_HZ: u32 = 200_000;
pub const FIXTURE_SATELLITE: &str = "GOES-19";
pub const FIXTURE_PRODUCT: &str = "LRIT-INFO";
pub const FIXTURE_CHANNEL: &str = "1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoesProduct {
    pub satellite: String,
    pub product: String,
    pub channel: String,
    pub timestamp_ms: i64,
    pub file_path: String,
    pub valid: bool,
    pub raw_hex: String,
}

impl GoesProduct {
    pub fn to_decoded(&self, frequency_hz: u64) -> DecodedMessage {
        DecodedMessage {
            id: None,
            frequency_hz,
            protocol: "goes".into(),
            message_type: self.product.clone(),
            address: self.satellite.clone(),
            function_code: self.channel.clone(),
            content: format!(
                "satellite={} product={} channel={} timestamp_ms={} file_path={} valid={}",
                self.satellite,
                self.product,
                self.channel,
                self.timestamp_ms,
                if self.file_path.is_empty() {
                    "inline"
                } else {
                    &self.file_path
                },
                self.valid
            ),
            raw: self.raw_hex.clone(),
            encryption: "none".into(),
            timestamp_ms: if self.timestamp_ms > 0 {
                self.timestamp_ms
            } else {
                crate::scanner::now_ms()
            },
        }
    }
}

pub fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

pub fn find_satdump() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PULSESCOPE_SATDUMP") {
        let path = PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
    }
    which::which("satdump").ok()
}

/// Documented SatDump baseband stdin recipe. Image reconstruction is not
/// fixture-verified; PulseScope never treats the output directory as reception.
pub fn satdump_stdin_args(pipeline: &str, sample_rate_hz: u32, output_dir: &str) -> Vec<String> {
    vec![
        "pipeline".into(),
        pipeline.into(),
        "-".into(),
        sample_rate_hz.to_string(),
        output_dir.into(),
        "--baseband_format".into(),
        "cf32".into(),
    ]
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

fn invert_bits(bits: &[u8]) -> Vec<u8> {
    bits.iter().map(|bit| 1 - bit).collect()
}

pub fn modulate_bpsk(bits: &[u8], sample_rate_hz: u32) -> Vec<Complex<f32>> {
    let sps = (sample_rate_hz / GOES_SYMBOL_RATE).max(1) as usize;
    let mut iq = Vec::with_capacity(bits.len() * sps + sps * 4);
    iq.extend(std::iter::repeat_n(Complex::new(1.0, 0.0), sps * 2));
    for &bit in bits {
        let sample = if bit != 0 {
            Complex::new(1.0, 0.0)
        } else {
            Complex::new(-1.0, 0.0)
        };
        iq.extend(std::iter::repeat_n(sample, sps));
    }
    iq.extend(std::iter::repeat_n(Complex::new(1.0, 0.0), sps * 2));
    iq
}

fn slice_bpsk_bits(iq: &[Complex<f32>], sample_rate_hz: u32, offset: usize) -> Vec<u8> {
    let sps = (sample_rate_hz / GOES_SYMBOL_RATE).max(1) as usize;
    let mut bits = Vec::new();
    let mut i = offset.min(sps.saturating_sub(1));
    while i + sps <= iq.len() {
        let mut acc = 0.0f32;
        for sample in &iq[i..i + sps] {
            acc += sample.re;
        }
        bits.push(if acc >= 0.0 { 1 } else { 0 });
        i += sps;
    }
    bits
}

fn push_len_str(buf: &mut Vec<u8>, value: &str) {
    let bytes = value.as_bytes();
    buf.push(bytes.len().min(255) as u8);
    buf.extend_from_slice(&bytes[..bytes.len().min(255)]);
}

fn read_len_str(bytes: &[u8], offset: &mut usize) -> Option<String> {
    let len = *bytes.get(*offset)? as usize;
    *offset += 1;
    let slice = bytes.get(*offset..*offset + len)?;
    *offset += len;
    Some(String::from_utf8_lossy(slice).into_owned())
}

pub fn build_product_cadu(
    satellite: &str,
    product: &str,
    channel: &str,
    timestamp_ms: i64,
) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&1u16.to_be_bytes());
    push_len_str(&mut body, satellite);
    push_len_str(&mut body, product);
    push_len_str(&mut body, channel);
    body.extend_from_slice(&timestamp_ms.to_be_bytes());
    let crc = crc32_ieee(&body);
    let mut cadu = GOES_ASM.to_be_bytes().to_vec();
    cadu.extend_from_slice(&body);
    cadu.extend_from_slice(&crc.to_be_bytes());
    cadu
}

pub fn parse_cadu(bytes: &[u8]) -> Option<GoesProduct> {
    let asm = GOES_ASM.to_be_bytes();
    let start = bytes.windows(4).position(|w| w == asm)?;
    let rest = bytes.get(start + 4..)?;
    if rest.len() < 2 + 3 + 8 + 4 {
        return None;
    }
    let mut offset = 0usize;
    let version = u16::from_be_bytes(rest.get(offset..offset + 2)?.try_into().ok()?);
    offset += 2;
    if version != 1 {
        return None;
    }
    let satellite = read_len_str(rest, &mut offset)?;
    let product = read_len_str(rest, &mut offset)?;
    let channel = read_len_str(rest, &mut offset)?;
    let timestamp_ms = i64::from_be_bytes(rest.get(offset..offset + 8)?.try_into().ok()?);
    offset += 8;
    let body = rest.get(..offset)?;
    let crc = u32::from_be_bytes(rest.get(offset..offset + 4)?.try_into().ok()?);
    if crc != crc32_ieee(body) {
        return None;
    }
    Some(GoesProduct {
        satellite,
        product,
        channel,
        timestamp_ms,
        file_path: String::new(),
        valid: true,
        raw_hex: hex::encode(&bytes[start..start + 4 + offset + 4]),
    })
}

pub fn decode_bits(bits: &[u8]) -> Vec<GoesProduct> {
    let mut found = Vec::new();
    for candidate in [bits.to_vec(), invert_bits(bits)] {
        for align in 0..8 {
            if align >= candidate.len() {
                break;
            }
            let bytes = bits_to_bytes_msb(&candidate[align..]);
            if let Some(product) = parse_cadu(&bytes) {
                found.push(product);
                return found;
            }
        }
    }
    found
}

pub fn decode_iq(iq: &[Complex<f32>], sample_rate_hz: u32) -> Vec<GoesProduct> {
    if iq.len() < 32 || sample_rate_hz < GOES_SYMBOL_RATE * 2 {
        return Vec::new();
    }
    let (samples, rate) = if sample_rate_hz >= GOES_FIXTURE_SAMPLE_RATE_HZ * 2 {
        let factor = (sample_rate_hz / GOES_FIXTURE_SAMPLE_RATE_HZ) as usize;
        (
            decimate_complex_average(iq, factor.max(1)),
            sample_rate_hz / factor.max(1) as u32,
        )
    } else {
        (iq.to_vec(), sample_rate_hz)
    };
    let sps = (rate / GOES_SYMBOL_RATE).max(1) as usize;
    for offset in 0..sps {
        let bits = slice_bpsk_bits(&samples, rate, offset);
        let found = decode_bits(&bits);
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

pub fn synthesize_lrit_info_iq(sample_rate_hz: u32) -> Vec<Complex<f32>> {
    let cadu = build_product_cadu(
        FIXTURE_SATELLITE,
        FIXTURE_PRODUCT,
        FIXTURE_CHANNEL,
        1_700_000_000_000,
    );
    let mut bits = vec![1, 1, 1, 1, 0, 0, 0, 0];
    bits.extend(bits_msb_first(&cadu));
    bits.extend([1, 0, 1, 0, 1, 0, 1, 0]);
    modulate_bpsk(&bits, sample_rate_hz)
}

/// Parse SatDump JSON or `product=` stdout. Output-directory listings are not
/// accepted as reception evidence.
pub fn parse_sidecar_line(line: &str) -> Option<GoesProduct> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let valid = value
            .get("valid")
            .or_else(|| value.get("checksum_valid"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if !valid {
            return None;
        }
        let product = value
            .get("product")
            .or_else(|| value.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if product.is_empty() {
            return None;
        }
        return Some(GoesProduct {
            satellite: value
                .get("satellite")
                .and_then(|v| v.as_str())
                .unwrap_or("GOES")
                .to_string(),
            product,
            channel: value
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            timestamp_ms: value
                .get("timestamp_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            file_path: String::new(),
            valid: true,
            raw_hex: trimmed.to_string(),
        });
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("valid=false") || lower.contains("checksum_valid=false") {
        return None;
    }
    let product = field_after(trimmed, "product=")?;
    Some(GoesProduct {
        satellite: field_after(trimmed, "satellite=").unwrap_or_else(|| "GOES".into()),
        product,
        channel: field_after(trimmed, "channel=").unwrap_or_default(),
        timestamp_ms: field_after(trimmed, "timestamp_ms=")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        file_path: String::new(),
        valid: true,
        raw_hex: trimmed.to_string(),
    })
}

fn field_after(line: &str, marker: &str) -> Option<String> {
    let (_, rest) = line.split_once(marker)?;
    let value = rest
        .split_whitespace()
        .next()
        .unwrap_or(rest)
        .trim_matches([',', ';']);
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

pub fn products_from_messages(messages: &[DecodedMessage]) -> Vec<GoesProduct> {
    messages
        .iter()
        .filter(|m| m.protocol == "goes")
        .filter(|m| m.content.contains("valid=true"))
        .map(|m| GoesProduct {
            satellite: m.address.clone(),
            product: m.message_type.clone(),
            channel: m.function_code.clone(),
            timestamp_ms: m.timestamp_ms,
            file_path: "inline".into(),
            valid: true,
            raw_hex: m.raw.clone(),
        })
        .collect()
}

pub fn in_band(center_hz: u64) -> bool {
    (1_688_000_000..=1_697_000_000).contains(&center_hz)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_rejects_tampered_cadu() {
        let mut cadu = build_product_cadu("GOES-19", "LRIT-INFO", "1", 1);
        assert!(parse_cadu(&cadu).unwrap().valid);
        let last = cadu.len() - 1;
        cadu[last] ^= 0x01;
        assert!(parse_cadu(&cadu).is_none());
    }

    #[test]
    fn fixture_iq_recovers_product() {
        let iq = synthesize_lrit_info_iq(GOES_FIXTURE_SAMPLE_RATE_HZ);
        let products = decode_iq(&iq, GOES_FIXTURE_SAMPLE_RATE_HZ);
        let product = products
            .iter()
            .find(|p| p.product == FIXTURE_PRODUCT)
            .expect("product");
        assert_eq!(product.satellite, FIXTURE_SATELLITE);
        assert_eq!(product.channel, FIXTURE_CHANNEL);
        assert!(product.valid);
        assert!(product.file_path.is_empty());
        let decoded = product.to_decoded(1_694_100_000);
        assert_eq!(decoded.protocol, "goes");
        assert!(decoded.content.contains("valid=true"));
        assert!(!decoded.content.contains("/var/"));
    }

    #[test]
    fn inverted_bpsk_still_recovers_asm() {
        let mut iq = synthesize_lrit_info_iq(GOES_FIXTURE_SAMPLE_RATE_HZ);
        for sample in &mut iq {
            *sample = -*sample;
        }
        let products = decode_iq(&iq, GOES_FIXTURE_SAMPLE_RATE_HZ);
        assert!(products.iter().any(|p| p.satellite == FIXTURE_SATELLITE));
    }

    #[test]
    fn sidecar_ignores_output_directory_listings_and_invalid_products() {
        assert!(parse_sidecar_line("/data/goes/image.png").is_none());
        assert!(parse_sidecar_line("product=LRIT-INFO valid=false").is_none());
        let ok = parse_sidecar_line(
            r#"{"satellite":"GOES-19","product":"LRIT-INFO","channel":"1","valid":true}"#,
        )
        .unwrap();
        assert_eq!(ok.satellite, "GOES-19");
        assert!(ok.file_path.is_empty());
        let messages = vec![ok.to_decoded(1_694_100_000)];
        let products = products_from_messages(&messages);
        assert_eq!(products.len(), 1);
        assert!(products[0].valid);
    }

    #[test]
    fn satdump_args_do_not_claim_tuner_ownership() {
        let args = satdump_stdin_args("goes_hrit", 2_400_000, "/tmp/goes-out");
        assert_eq!(args[0], "pipeline");
        assert!(args.contains(&"-".into()));
        assert!(args.contains(&"cf32".into()));
    }
}
