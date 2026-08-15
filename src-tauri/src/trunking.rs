//! P25 trunking observer: FIR-prepared C4FM dibits and TSBK parse.
//!
//! Encrypted calls are labeled only. This is not a voice decoder and does not
//! invent talkgroups when no control-channel bytes are recovered.

use rustfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::demod::{
    apply_fir_complex, decimate_complex_average, discriminator_samples, hamming_sinc_lowpass,
};
use crate::state::TrunkingCall;

pub const P25_FIR_TAPS: usize = 63;
pub const P25_FIR_CUTOFF_HZ: f32 = 6_000.0;
pub const P25_FIR_RATE_HZ: u32 = 48_000;
pub const P25_C4FM_SYMBOL_RATE: u32 = 4_800;

/// Group Voice Channel Grant (explicit), TIA-102.AABC opcode 0x00.
pub const TSBK_GROUP_VOICE_GRANT: u8 = 0x00;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TsbkGrant {
    pub opcode: u8,
    pub talkgroup: String,
    pub source: String,
    pub channel: u16,
    pub encrypted: bool,
    pub last_block: bool,
}

pub fn p25_fir_taps(sample_rate_hz: u32) -> Vec<f32> {
    hamming_sinc_lowpass(P25_FIR_TAPS, P25_FIR_CUTOFF_HZ, sample_rate_hz.max(1))
}

/// Mix-independent P25 channel filter: decimate toward 48 kHz, then 63-tap 6 kHz FIR.
pub fn apply_p25_vfo_fir(iq: &[Complex<f32>], sample_rate_hz: u32) -> (Vec<Complex<f32>>, u32) {
    if iq.is_empty() || sample_rate_hz == 0 {
        return (Vec::new(), sample_rate_hz);
    }
    let factor = (sample_rate_hz / P25_FIR_RATE_HZ).max(1) as usize;
    let decimated = decimate_complex_average(iq, factor);
    let rate = sample_rate_hz / factor as u32;
    let taps = p25_fir_taps(rate.max(1));
    (apply_fir_complex(&decimated, &taps), rate)
}

pub fn parse_tsbk(bytes: &[u8]) -> Option<TsbkGrant> {
    if bytes.len() < 12 {
        return None;
    }
    let opcode_byte = bytes[0];
    let last_block = opcode_byte & 0x80 != 0;
    let protected = opcode_byte & 0x40 != 0;
    let opcode = opcode_byte & 0x3F;
    if opcode != TSBK_GROUP_VOICE_GRANT {
        return None;
    }
    let service = bytes[1];
    let encrypted = protected || (service & 0x40) != 0;
    let talkgroup = u16::from_be_bytes([bytes[2], bytes[3]]);
    let source = u32::from_be_bytes([0, bytes[4], bytes[5], bytes[6]]);
    let channel = u16::from_be_bytes([bytes[7], bytes[8]]) & 0x0FFF;
    Some(TsbkGrant {
        opcode,
        talkgroup: format!("{talkgroup}"),
        source: format!("{source}"),
        channel,
        encrypted,
        last_block,
    })
}

pub fn encode_group_voice_grant(
    talkgroup: u16,
    source: u32,
    channel: u16,
    encrypted: bool,
) -> [u8; 12] {
    let mut bytes = [0u8; 12];
    bytes[0] = TSBK_GROUP_VOICE_GRANT | 0x80;
    if encrypted {
        bytes[0] |= 0x40;
        bytes[1] |= 0x40;
    }
    bytes[2] = (talkgroup >> 8) as u8;
    bytes[3] = talkgroup as u8;
    bytes[4] = ((source >> 16) & 0xFF) as u8;
    bytes[5] = ((source >> 8) & 0xFF) as u8;
    bytes[6] = source as u8;
    bytes[7] = ((channel >> 8) & 0x0F) as u8;
    bytes[8] = channel as u8;
    bytes
}

const SYNC: u64 = 0x0000_5575_F5FF_77FF;

fn bits_from_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut bits = Vec::with_capacity(bytes.len() * 8);
    for &b in bytes {
        for i in (0..8).rev() {
            bits.push((b >> i) & 1);
        }
    }
    bits
}

fn bytes_from_bits(bits: &[u8]) -> Vec<u8> {
    bits.chunks(8)
        .filter(|c| c.len() == 8)
        .map(|c| {
            let mut v = 0u8;
            for bit in c {
                v = (v << 1) | bit;
            }
            v
        })
        .collect()
}

/// 4-level C4FM mapping used by the observer fixture (not a full P25 vocoder).
fn dibit_deviation(dibit: u8) -> f32 {
    match dibit & 0x03 {
        0 => 600.0,
        1 => 1_800.0,
        2 => -600.0,
        _ => -1_800.0,
    }
}

pub fn synthesize_c4fm_iq(bits: &[u8], sample_rate_hz: u32) -> Vec<Complex<f32>> {
    let sps = (sample_rate_hz / P25_C4FM_SYMBOL_RATE).max(1);
    let mut phase = 0.0f64;
    let mut iq = Vec::new();
    for pair in bits.chunks(2) {
        if pair.len() < 2 {
            break;
        }
        let dibit = (pair[0] << 1) | pair[1];
        let freq = f64::from(dibit_deviation(dibit));
        let step = std::f64::consts::TAU * freq / f64::from(sample_rate_hz.max(1));
        for _ in 0..sps {
            iq.push(Complex::new(phase.cos() as f32, phase.sin() as f32));
            phase += step;
        }
    }
    iq
}

pub fn synthesize_tsbk_grant_iq(sample_rate_hz: u32) -> Vec<Complex<f32>> {
    let mut bits = Vec::new();
    for i in (0..48).rev() {
        bits.push(((SYNC >> i) & 1) as u8);
    }
    bits.extend(bits_from_bytes(&encode_group_voice_grant(
        1234, 56_789, 0x0A1, false,
    )));
    synthesize_c4fm_iq(&bits, sample_rate_hz)
}

fn slice_dibit_bits(disc: &[f32], sample_rate_hz: u32) -> Vec<u8> {
    let sps = (sample_rate_hz / P25_C4FM_SYMBOL_RATE).max(1) as usize;
    let mut bits = Vec::new();
    let mut i = sps / 2;
    while i < disc.len() {
        let v = disc[i];
        let dibit = if v > 1_200.0 * std::f32::consts::TAU / sample_rate_hz as f32 {
            0b01
        } else if v > 0.0 {
            0b00
        } else if v > -1_200.0 * std::f32::consts::TAU / sample_rate_hz as f32 {
            0b10
        } else {
            0b11
        };
        bits.push((dibit >> 1) & 1);
        bits.push(dibit & 1);
        i += sps;
    }
    bits
}

fn find_sync(bits: &[u8]) -> Option<usize> {
    let mut pattern = Vec::with_capacity(48);
    for i in (0..48).rev() {
        pattern.push(((SYNC >> i) & 1) as u8);
    }
    bits.windows(48).position(|w| w == pattern.as_slice())
}

pub fn decode_tsbk_from_iq(iq: &[Complex<f32>], sample_rate_hz: u32) -> Vec<TsbkGrant> {
    let (filtered, rate) = apply_p25_vfo_fir(iq, sample_rate_hz);
    let mut prev = None;
    let disc = discriminator_samples(&filtered, &mut prev);
    let bits = slice_dibit_bits(&disc, rate);
    let Some(at) = find_sync(&bits) else {
        return Vec::new();
    };
    let payload_bits = &bits[at + 48..];
    if payload_bits.len() < 96 {
        return Vec::new();
    }
    let bytes = bytes_from_bits(&payload_bits[..96]);
    parse_tsbk(&bytes).into_iter().collect()
}

pub fn grant_to_call(grant: &TsbkGrant, frequency_hz: u64) -> TrunkingCall {
    TrunkingCall {
        timestamp_ms: crate::scanner::now_ms(),
        talkgroup: grant.talkgroup.clone(),
        frequency_hz,
        duration_ms: 0,
        encrypted: grant.encrypted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsbk_parse_round_trip() {
        let bytes = encode_group_voice_grant(1234, 56_789, 0x0A1, false);
        let grant = parse_tsbk(&bytes).expect("grant");
        assert_eq!(grant.talkgroup, "1234");
        assert_eq!(grant.source, "56789");
        assert!(!grant.encrypted);
        let enc = encode_group_voice_grant(9, 1, 1, true);
        assert!(parse_tsbk(&enc).expect("enc").encrypted);
    }

    #[test]
    fn fir_dc_gain_near_unity_and_rejects_high_bin() {
        let taps = p25_fir_taps(P25_FIR_RATE_HZ);
        assert_eq!(taps.len(), P25_FIR_TAPS);
        let dc: f32 = taps.iter().sum();
        assert!((dc - 1.0).abs() < 0.05, "dc gain {dc}");
        let n = 4_800usize;
        let tone: Vec<_> = (0..n)
            .map(|i| {
                let phase = std::f32::consts::TAU * 18_000.0 * i as f32 / P25_FIR_RATE_HZ as f32;
                Complex::from_polar(1.0, phase)
            })
            .collect();
        let filtered = apply_fir_complex(&tone, &taps);
        let in_rms = (tone.iter().map(|c| c.norm_sqr()).sum::<f32>() / n as f32).sqrt();
        let out_rms = (filtered[taps.len()..]
            .iter()
            .map(|c| c.norm_sqr())
            .sum::<f32>()
            / (n - taps.len()) as f32)
            .sqrt();
        assert!(
            out_rms < in_rms * 0.35,
            "high-frequency leakage {out_rms} vs {in_rms}"
        );
    }

    #[test]
    fn c4fm_fixture_recovers_group_grant() {
        let iq = synthesize_tsbk_grant_iq(P25_FIR_RATE_HZ);
        let grants = decode_tsbk_from_iq(&iq, P25_FIR_RATE_HZ);
        assert!(
            grants
                .iter()
                .any(|g| g.talkgroup == "1234" && g.source == "56789"),
            "{grants:?}"
        );
    }
}
