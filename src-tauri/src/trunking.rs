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
/// Group Voice Channel Grant Update, TIA-102.AABC opcode 0x02.
pub const TSBK_GROUP_VOICE_GRANT_UPDT: u8 = 0x02;
/// Identifier Update (VHF/UHF), TIA-102.AABC opcode 0x3D.
pub const TSBK_IDEN_UP: u8 = 0x3D;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TsbkGrant {
    pub opcode: u8,
    pub talkgroup: String,
    pub source: String,
    pub channel: u16,
    pub encrypted: bool,
    pub last_block: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdenUp {
    pub identifier: u8,
    pub base_hz: u64,
    pub spacing_hz: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlChannelObservation {
    pub grants: Vec<TsbkGrant>,
    pub idens: Vec<IdenUp>,
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
    if opcode != TSBK_GROUP_VOICE_GRANT && opcode != TSBK_GROUP_VOICE_GRANT_UPDT {
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

pub fn parse_iden_up(bytes: &[u8]) -> Option<IdenUp> {
    if bytes.len() < 12 {
        return None;
    }
    let opcode = bytes[0] & 0x3F;
    if opcode != TSBK_IDEN_UP {
        return None;
    }
    let identifier = bytes[1] & 0x0F;
    let base_units = u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
    let spacing_hz = u16::from_be_bytes([bytes[6], bytes[7]]) as u32;
    if spacing_hz == 0 || base_units == 0 {
        return None;
    }
    Some(IdenUp {
        identifier,
        base_hz: u64::from(base_units).saturating_mul(5),
        spacing_hz,
    })
}

pub fn encode_iden_up(identifier: u8, base_hz: u64, spacing_hz: u32) -> [u8; 12] {
    let mut bytes = [0u8; 12];
    bytes[0] = TSBK_IDEN_UP;
    bytes[1] = identifier & 0x0F;
    let units = (base_hz / 5) as u32;
    bytes[2..6].copy_from_slice(&units.to_be_bytes());
    bytes[6..8].copy_from_slice(&(spacing_hz as u16).to_be_bytes());
    bytes
}

pub fn voice_hz(iden: &IdenUp, channel: u16) -> u64 {
    iden.base_hz
        .saturating_add(u64::from(channel).saturating_mul(u64::from(iden.spacing_hz)))
}

/// Voice frequency for a recovered grant. Never invents a mapping: IDEN_UP
/// from the same control channel, else an imported voice-channel table.
pub fn follow_frequency(
    observation: &ControlChannelObservation,
    imported_voice_hz: &[u64],
) -> Option<u64> {
    let grant = observation.grants.first()?;
    if let Some(iden) = observation.idens.first() {
        return Some(voice_hz(iden, grant.channel));
    }
    imported_voice_hz.get(usize::from(grant.channel)).copied()
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

fn append_sync(bits: &mut Vec<u8>) {
    for i in (0..48).rev() {
        bits.push(((SYNC >> i) & 1) as u8);
    }
}

pub fn synthesize_tsbk_grant_iq(sample_rate_hz: u32) -> Vec<Complex<f32>> {
    let mut bits = Vec::new();
    append_sync(&mut bits);
    bits.extend(bits_from_bytes(&encode_group_voice_grant(
        1234, 56_789, 0x0A1, false,
    )));
    synthesize_c4fm_iq(&bits, sample_rate_hz)
}

/// Control-channel fixture: Identifier Update then Group Voice Grant.
/// Base 851.0125 MHz, 6.25 kHz spacing, channel 0x0A1 → 852.01875 MHz.
pub fn synthesize_tsbk_control_iq(sample_rate_hz: u32) -> Vec<Complex<f32>> {
    let mut bits = Vec::new();
    append_sync(&mut bits);
    bits.extend(bits_from_bytes(&encode_iden_up(0, 851_012_500, 6_250)));
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

pub fn observe_control_channel(
    iq: &[Complex<f32>],
    sample_rate_hz: u32,
) -> ControlChannelObservation {
    let (filtered, rate) = apply_p25_vfo_fir(iq, sample_rate_hz);
    let mut prev = None;
    let disc = discriminator_samples(&filtered, &mut prev);
    let bits = slice_dibit_bits(&disc, rate);
    let Some(at) = find_sync(&bits) else {
        return ControlChannelObservation::default();
    };
    let mut observation = ControlChannelObservation::default();
    let mut offset = at + 48;
    while offset + 96 <= bits.len() {
        let bytes = bytes_from_bits(&bits[offset..offset + 96]);
        offset += 96;
        if let Some(iden) = parse_iden_up(&bytes) {
            let last = bytes[0] & 0x80 != 0;
            observation.idens.push(iden);
            if last {
                break;
            }
            continue;
        }
        if let Some(grant) = parse_tsbk(&bytes) {
            let last = grant.last_block;
            observation.grants.push(grant);
            if last {
                break;
            }
            continue;
        }
        break;
    }
    observation
}

pub fn decode_tsbk_from_iq(iq: &[Complex<f32>], sample_rate_hz: u32) -> Vec<TsbkGrant> {
    observe_control_channel(iq, sample_rate_hz).grants
}

pub fn grant_to_decoded(grant: &TsbkGrant, frequency_hz: u64) -> crate::db::DecodedMessage {
    crate::db::DecodedMessage {
        id: None,
        frequency_hz,
        protocol: "p25-tsbk".into(),
        message_type: "group_voice_grant".into(),
        address: grant.talkgroup.clone(),
        function_code: grant.source.clone(),
        content: format!(
            "TG {} src {} ch {:#05x} enc={}",
            grant.talkgroup, grant.source, grant.channel, grant.encrypted
        ),
        raw: format!("opcode={:#04x}", grant.opcode),
        encryption: if grant.encrypted {
            "identified".into()
        } else {
            "none".into()
        },
        timestamp_ms: crate::scanner::now_ms(),
    }
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

pub fn grant_is_watched(
    grant: &TsbkGrant,
    system: Option<&str>,
    watched: &[(String, String)],
) -> bool {
    watched.iter().any(|(sys, tg)| {
        tg == &grant.talkgroup && (system.is_none() || system == Some(sys.as_str()))
    })
}

pub fn filter_grants(
    grants: Vec<TsbkGrant>,
    watchlist_only: bool,
    system: Option<&str>,
    watched: &[(String, String)],
) -> Vec<TsbkGrant> {
    if !watchlist_only {
        return grants;
    }
    grants
        .into_iter()
        .filter(|grant| grant_is_watched(grant, system, watched))
        .collect()
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

    #[test]
    fn c4fm_control_fixture_recovers_iden_and_follow_hz() {
        let iq = synthesize_tsbk_control_iq(P25_FIR_RATE_HZ);
        let observation = observe_control_channel(&iq, P25_FIR_RATE_HZ);
        assert!(
            observation
                .grants
                .iter()
                .any(|g| g.talkgroup == "1234" && g.channel == 0x0A1),
            "{observation:?}"
        );
        assert!(!observation.idens.is_empty(), "{observation:?}");
        let hz = follow_frequency(&observation, &[]).expect("follow");
        assert_eq!(hz, voice_hz(&observation.idens[0], 0x0A1));
        assert_eq!(hz, 851_012_500 + 0x0A1 * 6_250);
    }

    #[test]
    fn follow_uses_imported_table_without_iden() {
        let observation = ControlChannelObservation {
            grants: vec![TsbkGrant {
                opcode: TSBK_GROUP_VOICE_GRANT,
                talkgroup: "9".into(),
                source: "1".into(),
                channel: 1,
                encrypted: false,
                last_block: true,
            }],
            idens: Vec::new(),
        };
        assert_eq!(
            follow_frequency(&observation, &[851_000_000, 851_012_500]),
            Some(851_012_500)
        );
        assert_eq!(follow_frequency(&observation, &[]), None);
    }

    #[test]
    fn grant_update_opcode_parses() {
        let mut bytes = encode_group_voice_grant(77, 8, 3, false);
        bytes[0] = TSBK_GROUP_VOICE_GRANT_UPDT | 0x80;
        let grant = parse_tsbk(&bytes).expect("update");
        assert_eq!(grant.opcode, TSBK_GROUP_VOICE_GRANT_UPDT);
        assert_eq!(grant.talkgroup, "77");
    }

    #[test]
    fn filter_grants_honors_watchlist_only_mode() {
        let grants = vec![
            TsbkGrant {
                opcode: TSBK_GROUP_VOICE_GRANT,
                talkgroup: "1234".into(),
                source: "1".into(),
                channel: 1,
                encrypted: false,
                last_block: true,
            },
            TsbkGrant {
                opcode: TSBK_GROUP_VOICE_GRANT,
                talkgroup: "9999".into(),
                source: "2".into(),
                channel: 2,
                encrypted: false,
                last_block: true,
            },
        ];
        let watched = vec![("County".into(), "1234".into())];
        let all = filter_grants(grants.clone(), false, Some("County"), &watched);
        assert_eq!(all.len(), 2);
        let filtered = filter_grants(grants, true, Some("County"), &watched);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].talkgroup, "1234");
    }
}
