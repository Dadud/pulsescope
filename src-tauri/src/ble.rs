//! Native BLE advertising GFSK decoder (channels 37/38/39).
//!
//! Clean-room 1 Msym/s GFSK, access-address correlator, PDU parse, and CRC-24.
//! Encrypted / private payloads are identified only.

use rustfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::db::DecodedMessage;
use crate::demod::decimate_complex_average;

pub const BLE_ACCESS_ADDRESS: u32 = 0x8E89_BED6;
pub const BLE_FIXTURE_SAMPLE_RATE_HZ: u32 = 4_000_000;
pub const BLE_SYMBOL_RATE: u32 = 1_000_000;
pub const BLE_CRC_POLY: u32 = 0x0000_065B;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BleAdvertisement {
    pub address: String,
    pub address_type: String,
    pub pdu_type: String,
    pub channel: u8,
    pub name: Option<String>,
    pub ad_structures: Vec<String>,
    pub crc_valid: bool,
    pub rssi_dbm: i16,
    pub raw_hex: String,
}

impl BleAdvertisement {
    pub fn to_decoded(&self, frequency_hz: u64) -> DecodedMessage {
        DecodedMessage {
            id: None,
            frequency_hz,
            protocol: "ble".into(),
            message_type: self.pdu_type.clone(),
            address: self.address.clone(),
            function_code: format!("{} rssi={} dBm", self.address_type, self.rssi_dbm),
            content: self
                .name
                .clone()
                .unwrap_or_else(|| self.ad_structures.join("; ")),
            raw: self.raw_hex.clone(),
            encryption: "none".into(),
            timestamp_ms: crate::scanner::now_ms(),
        }
    }
}

pub fn crc24(pdu: &[u8]) -> [u8; 3] {
    let mut crc = 0x555_555u32;
    for &byte in pdu {
        for bit in 0..8 {
            let data_bit = u32::from((byte >> bit) & 1);
            let msb = (crc >> 23) & 1;
            crc = (crc << 1) & 0xFF_FFFF;
            if (msb ^ data_bit) != 0 {
                crc ^= BLE_CRC_POLY;
            }
        }
    }
    [
        (crc & 0xFF) as u8,
        ((crc >> 8) & 0xFF) as u8,
        ((crc >> 16) & 0xFF) as u8,
    ]
}

/// BLE whitening LFSR (x^7 + x^4 + 1), initialized with channel | 0x40.
pub fn whiten(data: &[u8], channel: u8) -> Vec<u8> {
    let mut lfsr = channel | 0x40;
    let mut out = vec![0u8; data.len()];
    for (i, &byte) in data.iter().enumerate() {
        let mut whitened = 0u8;
        for bit in 0..8 {
            let lsb = lfsr & 1;
            whitened |= lsb << bit;
            let feedback = lsb ^ ((lfsr >> 4) & 1);
            lfsr >>= 1;
            if feedback != 0 {
                lfsr |= 0x40;
            }
        }
        out[i] = byte ^ whitened;
    }
    out
}

fn pdu_type_name(t: u8) -> &'static str {
    match t & 0x0F {
        0 => "ADV_IND",
        1 => "ADV_DIRECT_IND",
        2 => "ADV_NONCONN_IND",
        3 => "SCAN_REQ",
        4 => "SCAN_RSP",
        5 => "CONNECT_IND",
        6 => "ADV_SCAN_IND",
        _ => "ADV",
    }
}

pub fn parse_pdu(pdu: &[u8], channel: u8, crc_valid: bool) -> Option<BleAdvertisement> {
    if pdu.len() < 8 {
        return None;
    }
    let header = u16::from_le_bytes([pdu[0], pdu[1]]);
    let pdu_type = (header & 0x0F) as u8;
    let tx_add_random = (header & 0x40) != 0;
    let length = ((header >> 8) & 0xFF) as usize;
    if pdu.len() < 2 + length {
        return None;
    }
    let body = &pdu[2..2 + length];
    if body.len() < 6 {
        return None;
    }
    let addr_bytes = &body[..6];
    let address = format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        addr_bytes[5], addr_bytes[4], addr_bytes[3], addr_bytes[2], addr_bytes[1], addr_bytes[0]
    );
    let mut name = None;
    let mut ad_structures = Vec::new();
    let mut i = 6usize;
    while i + 1 < body.len() {
        let ad_len = body[i] as usize;
        if ad_len == 0 || i + 1 + ad_len > body.len() {
            break;
        }
        let ad_type = body[i + 1];
        let data = &body[i + 2..i + 1 + ad_len];
        match ad_type {
            0x08 | 0x09 => {
                let text = String::from_utf8_lossy(data).into_owned();
                name = Some(text.clone());
                ad_structures.push(format!("name={text}"));
            }
            0x01 => ad_structures.push(format!("flags={:02x}", data.first().copied().unwrap_or(0))),
            _ => ad_structures.push(format!("ad_{ad_type:02x}")),
        }
        i += 1 + ad_len;
    }
    Some(BleAdvertisement {
        address,
        address_type: if tx_add_random { "random" } else { "public" }.into(),
        pdu_type: pdu_type_name(pdu_type).into(),
        channel,
        name,
        ad_structures,
        crc_valid,
        rssi_dbm: -128,
        raw_hex: hex::encode(pdu),
    })
}

pub fn mean_rssi_dbm(iq: &[Complex<f32>]) -> i16 {
    if iq.is_empty() {
        return -128;
    }
    let power = iq.iter().map(|c| c.norm_sqr()).sum::<f32>() / iq.len() as f32;
    (10.0 * power.max(1.0e-12).log10()).round() as i16
}

pub fn build_adv_ind(address: [u8; 6], name: &str, channel: u8) -> Vec<u8> {
    let mut ad = vec![0x02, 0x01, 0x06];
    let name_bytes = name.as_bytes();
    ad.push((1 + name_bytes.len()) as u8);
    ad.push(0x09);
    ad.extend_from_slice(name_bytes);
    let mut body = Vec::new();
    body.extend_from_slice(&address);
    body.extend_from_slice(&ad);
    let mut header = 0u16;
    header |= 0x00; // ADV_IND
    header |= 0x40; // random TxAdd
    header |= (body.len() as u16) << 8;
    let mut pdu = Vec::new();
    pdu.extend_from_slice(&header.to_le_bytes());
    pdu.extend_from_slice(&body);
    let crc = crc24(&pdu);
    pdu.extend_from_slice(&crc);
    whiten(&pdu, channel)
}

fn bits_lsb_first(bytes: &[u8]) -> Vec<u8> {
    let mut bits = Vec::with_capacity(bytes.len() * 8);
    for &byte in bytes {
        for i in 0..8 {
            bits.push((byte >> i) & 1);
        }
    }
    bits
}

fn gaussian_pulse(samples_per_symbol: usize) -> Vec<f32> {
    let n = samples_per_symbol * 3;
    let bt = 0.5f32;
    let t_sym = 1.0;
    let alpha = f32::sqrt(2.0 * std::f32::consts::LN_2) / (bt * t_sym);
    (0..n)
        .map(|i| {
            let t = i as f32 / samples_per_symbol as f32 - 1.5;
            f32::exp(-(alpha * t).powi(2))
        })
        .collect()
}

pub fn modulate_gfsk(bits: &[u8], sample_rate_hz: u32) -> Vec<Complex<f32>> {
    let sps = (sample_rate_hz / BLE_SYMBOL_RATE).max(1) as usize;
    let pulse = gaussian_pulse(sps);
    let mut nrz = vec![0.0f32; bits.len() * sps + pulse.len()];
    for (i, &bit) in bits.iter().enumerate() {
        let v = if bit != 0 { 1.0 } else { -1.0 };
        for (p, &w) in pulse.iter().enumerate() {
            nrz[i * sps + p] += v * w;
        }
    }
    let deviation = 250_000.0_f64;
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
    for &s in iq {
        let cross = s.im * prev.re - s.re * prev.im;
        let dot = s.re * prev.re + s.im * prev.im;
        out.push(cross.atan2(dot));
        prev = s;
    }
    out
}

fn slice_bits(disc: &[f32], sample_rate_hz: u32) -> Vec<u8> {
    let sps = (sample_rate_hz / BLE_SYMBOL_RATE).max(1) as usize;
    let mut bits = Vec::new();
    let mut i = sps / 2;
    while i < disc.len() {
        bits.push(if disc[i] >= 0.0 { 1 } else { 0 });
        i += sps;
    }
    bits
}

fn bits_to_bytes_lsb(bits: &[u8]) -> Vec<u8> {
    bits.chunks(8)
        .filter(|c| c.len() == 8)
        .map(|c| {
            let mut v = 0u8;
            for (i, bit) in c.iter().enumerate() {
                v |= bit << i;
            }
            v
        })
        .collect()
}

fn find_aa(bits: &[u8]) -> Option<usize> {
    let aa: Vec<u8> = (0..32)
        .map(|i| ((BLE_ACCESS_ADDRESS >> i) & 1) as u8)
        .collect();
    bits.windows(32).position(|w| w == aa.as_slice())
}

pub fn decode_bits(bits: &[u8], channel: u8) -> Vec<BleAdvertisement> {
    let Some(aa_at) = find_aa(bits) else {
        return Vec::new();
    };
    let after = &bits[aa_at + 32..];
    let whitened = bits_to_bytes_lsb(after);
    if whitened.len() < 11 {
        return Vec::new();
    }
    let pdu_crc = whiten(&whitened, channel);
    if pdu_crc.len() < 11 {
        return Vec::new();
    }
    let length = pdu_crc[1] as usize;
    let frame_len = 2 + length + 3;
    if pdu_crc.len() < frame_len {
        return Vec::new();
    }
    let pdu = &pdu_crc[..frame_len - 3];
    let crc = &pdu_crc[frame_len - 3..frame_len];
    let valid = crc == crc24(pdu);
    parse_pdu(pdu, channel, valid).into_iter().collect()
}

pub fn decode_iq(iq: &[Complex<f32>], sample_rate_hz: u32) -> Vec<BleAdvertisement> {
    if iq.len() < 64 || sample_rate_hz < BLE_SYMBOL_RATE {
        return Vec::new();
    }
    let (samples, rate) = if sample_rate_hz >= BLE_FIXTURE_SAMPLE_RATE_HZ * 2 {
        let factor = (sample_rate_hz / BLE_FIXTURE_SAMPLE_RATE_HZ) as usize;
        (
            decimate_complex_average(iq, factor),
            sample_rate_hz / factor as u32,
        )
    } else {
        (iq.to_vec(), sample_rate_hz)
    };
    let disc = discriminator(&samples);
    let bits = slice_bits(&disc, rate);
    let rssi = mean_rssi_dbm(&samples);
    let mut found = decode_bits(&bits, 37);
    if found.is_empty() {
        found = decode_bits(&bits, 38);
    }
    if found.is_empty() {
        found = decode_bits(&bits, 39);
    }
    for advert in &mut found {
        advert.rssi_dbm = rssi;
    }
    found
}

pub fn synthesize_pulse_advert_iq(sample_rate_hz: u32) -> Vec<Complex<f32>> {
    let channel = 37u8;
    let pdu = build_adv_ind([0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA], "PULSE", channel);
    let mut bits = vec![0, 1, 0, 1, 0, 1, 0, 1]; // preamble 0x55 (AA LSB=0 → 0x55)
    bits.extend((0..32).map(|i| ((BLE_ACCESS_ADDRESS >> i) & 1) as u8));
    bits.extend(bits_lsb_first(&pdu));
    modulate_gfsk(&bits, sample_rate_hz)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_whiten_round_trip() {
        let pdu = vec![
            0x40, 0x0C, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x02, 0x01, 0x06, 0x03, 0x09, b'H',
        ];
        let crc = crc24(&pdu);
        let mut framed = pdu.clone();
        framed.extend_from_slice(&crc);
        let whitened = whiten(&framed, 37);
        let restored = whiten(&whitened, 37);
        assert_eq!(restored, framed);
        assert_eq!(&restored[restored.len() - 3..], &crc);
    }

    #[test]
    fn iq_fixture_recovers_pulse_name() {
        let iq = synthesize_pulse_advert_iq(BLE_FIXTURE_SAMPLE_RATE_HZ);
        let ads = decode_iq(&iq, BLE_FIXTURE_SAMPLE_RATE_HZ);
        assert!(
            ads.iter()
                .any(|a| a.name.as_deref() == Some("PULSE") && a.crc_valid),
            "{ads:?}"
        );
        assert!(ads.iter().any(|a| a.address == "AA:BB:CC:DD:EE:FF"));
        assert!(ads.iter().any(|a| a.rssi_dbm > -80));
    }

    #[test]
    fn parse_rejects_truncated_pdu() {
        assert!(parse_pdu(&[0x00, 0x10], 37, false).is_none());
    }

    #[test]
    fn scan_rsp_pdu_type_is_named() {
        let mut pdu = vec![0x04, 0x06, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];
        pdu.extend_from_slice(&crc24(&pdu));
        let parsed = parse_pdu(&pdu[..pdu.len() - 3], 37, true).expect("pdu");
        assert_eq!(parsed.pdu_type, "SCAN_RSP");
    }
}
