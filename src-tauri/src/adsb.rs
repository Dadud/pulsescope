//! Clean-room Mode S / ADS-B 1090ES decoder (native Rust).
//!
//! Pipeline:
//!   complex IQ → magnitude → preamble correlator (0xD17 timing)
//!   → 112/56-bit PPM slicer → CRC-24 Mode S check → DF17/18 parse
//!
//! No code from dump1090/readsb is used. Protocol facts come from ICAO
//! Annex 10 / publicly documented ADS-B message layouts.
//!
//! Why this beats the sidecar path for PulseScope:
//!   - zero process spawn, no exclusive SDR grab
//!   - runs on the same IQ ring as the scanner
//!   - sub-millisecond per frame at 2 Msps

use serde::{Deserialize, Serialize};

/// Parsed Mode S / ADS-B message.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AdsbMessage {
    pub df: u8,
    pub icao: String,
    pub message_type: String,
    pub callsign: Option<String>,
    pub altitude_ft: Option<i32>,
    pub airborne: Option<bool>,
    pub raw_hex: String,
    pub confidence: f32,
}

/// Streaming ADS-B demodulator operating on complex baseband magnitude.
pub struct AdsbDecoder {
    /// Samples-per-half-bit. At 2 Msps Mode S chip is 0.5 µs → 1 sample/half-bit.
    sphb: usize,
    mag_hist: Vec<f32>,
    /// Output buffer of decoded messages since last drain.
    pub messages: Vec<AdsbMessage>,
    last_preamble_at: usize,
}

impl AdsbDecoder {
    /// Create a decoder for the given IQ sample rate (Hz).
    /// Mode S works best at ≥2 Msps; rates below 1 Msps are rejected.
    pub fn new(sample_rate_hz: u32) -> Option<Self> {
        if sample_rate_hz < 1_000_000 {
            return None;
        }
        // Half-bit duration = 0.5e-6 s → samples per half-bit
        let sphb = ((sample_rate_hz as f64) * 0.5e-6).round().max(1.0) as usize;
        Some(Self {
            sphb,
            mag_hist: Vec::with_capacity(sample_rate_hz as usize / 10),
            messages: Vec::new(),
            last_preamble_at: 0,
        })
    }

    /// Feed complex IQ samples (interleaved I/Q already as Complex, or [re,im,...] f32 pairs handled by caller).
    pub fn feed_iq(&mut self, iq: &[rustfft::num_complex::Complex<f32>]) {
        for s in iq {
            self.mag_hist.push(s.norm());
        }
        self.scan();
        // Keep a trailing window so preambles straddling chunk boundaries survive
        let keep = self.sphb * 240; // preamble + 112 bits with margin
        if self.mag_hist.len() > keep * 4 {
            let drop = self.mag_hist.len() - keep;
            self.mag_hist.drain(0..drop);
            self.last_preamble_at = self.last_preamble_at.saturating_sub(drop);
        }
    }

    /// Feed pre-computed magnitude samples (e.g. from AM demod |z|).
    pub fn feed_magnitude(&mut self, mags: &[f32]) {
        self.mag_hist.extend_from_slice(mags);
        self.scan();
        let keep = self.sphb * 240;
        if self.mag_hist.len() > keep * 4 {
            let drop = self.mag_hist.len() - keep;
            self.mag_hist.drain(0..drop);
            self.last_preamble_at = self.last_preamble_at.saturating_sub(drop);
        }
    }

    /// Drain decoded messages.
    pub fn take_messages(&mut self) -> Vec<AdsbMessage> {
        std::mem::take(&mut self.messages)
    }

    fn scan(&mut self) {
        let sphb = self.sphb;
        // Preamble = 8 µs = 16 half-bits; long frame = 112 bits × 2 half-bits
        let preamble_samples = 16 * sphb;
        let long_frame_samples = 112 * 2 * sphb;
        let need = preamble_samples + long_frame_samples;
        if self.mag_hist.len() < need {
            return;
        }

        let end = self.mag_hist.len() - need;
        let mut i = self.last_preamble_at.min(end);
        while i <= end {
            if self.score_preamble(i) {
                if let Some(msg) = self.try_frame(i + preamble_samples) {
                    self.messages.push(msg);
                    // Skip ahead past this frame to avoid double-hits
                    i += need;
                    self.last_preamble_at = i;
                    continue;
                }
            }
            i += sphb.max(1); // step one half-bit
        }
        self.last_preamble_at = end;
    }

    /// Score Mode S preamble quality at magnitude index `start`.
    fn score_preamble(&self, start: usize) -> bool {
        let sphb = self.sphb;
        // Sample reference points in half-bit units from preamble start
        // High: 0, 2.0µs(=4 hb), 3.5µs(=7), 4.5µs(=9)
        // Low:  1.0, 1.5, 2.5, 3.0, 5.0, 5.5, 6.0, 6.5, 7.0, 7.5
        let highs = [0usize, 4, 7, 9];
        let lows = [2usize, 3, 5, 6, 10, 11, 12, 13, 14, 15];

        let sample = |hb: usize| -> f32 {
            let idx = start + hb * sphb + sphb / 2;
            if idx < self.mag_hist.len() {
                self.mag_hist[idx]
            } else {
                0.0
            }
        };

        let mut high_sum = 0.0f32;
        let mut low_sum = 0.0f32;
        for &h in &highs {
            high_sum += sample(h);
        }
        for &l in &lows {
            low_sum += sample(l);
        }
        let high_avg = high_sum / highs.len() as f32;
        let low_avg = low_sum / lows.len() as f32;
        // Require clear contrast and absolute energy
        high_avg > low_avg * 1.6 && high_avg > 0.02
    }

    /// Demodulate PPM bits starting at first data half-bit after preamble.
    fn try_frame(&self, data_start: usize) -> Option<AdsbMessage> {
        let sphb = self.sphb;
        // Try long frame (112 bits) first, fall back to short (56)
        for bit_len in [112usize, 56] {
            let need = data_start + bit_len * 2 * sphb;
            if need > self.mag_hist.len() {
                continue;
            }
            let mut bits = Vec::with_capacity(bit_len);
            for b in 0..bit_len {
                let first = data_start + b * 2 * sphb + sphb / 2;
                let second = first + sphb;
                if second >= self.mag_hist.len() {
                    break;
                }
                // PPM: first half high → 1, second half high → 0
                let bit = self.mag_hist[first] > self.mag_hist[second];
                bits.push(bit);
            }
            if bits.len() != bit_len {
                continue;
            }
            if let Some(msg) = decode_mode_s_bits(&bits) {
                return Some(msg);
            }
        }
        None
    }
}

/// Mode S CRC-24 polynomial (ICAO Annex 10): 0xFFF409
const CRC_POLY: u32 = 0x00FF_F409;

/// Compute Mode S CRC over message bits (excluding the last 24 CRC bits).
pub fn mode_s_crc(bits: &[bool]) -> u32 {
    let mut crc: u32 = 0;
    for &bit in bits {
        let b = if bit { 1u32 } else { 0 };
        crc ^= b << 23;
        if crc & 0x80_0000 != 0 {
            crc = ((crc << 1) ^ CRC_POLY) & 0xFF_FFFF;
        } else {
            crc = (crc << 1) & 0xFF_FFFF;
        }
    }
    crc
}

/// Decode a Mode S bit vector (56 or 112 bits, MSB first).
pub fn decode_mode_s_bits(bits: &[bool]) -> Option<AdsbMessage> {
    if bits.len() != 56 && bits.len() != 112 {
        return None;
    }
    let bytes = bits_to_bytes(bits);
    let raw_hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();

    // CRC check: last 24 bits vs computed over the rest
    let data_bits = &bits[..bits.len() - 24];
    let mut rx_crc: u32 = 0;
    for &b in &bits[bits.len() - 24..] {
        rx_crc = (rx_crc << 1) | if b { 1 } else { 0 };
    }
    let calc = mode_s_crc(data_bits);
    // Allow exact match only for now (no error correction)
    let confidence = if calc == rx_crc {
        0.98
    } else {
        // Soft accept DF17/18 with near-miss is dangerous; reject
        return None;
    };

    let df = (bytes[0] >> 3) & 0x1F;
    let icao = if bits.len() == 112 {
        format!("{:02X}{:02X}{:02X}", bytes[1], bytes[2], bytes[3])
    } else {
        // Short Mode S: ICAO not always present in the same place
        format!("{:02X}{:02X}{:02X}", bytes[1], bytes[2], bytes[3])
    };

    let (message_type, callsign, altitude_ft, airborne) = match df {
        17 | 18 => parse_adsb_me(&bytes),
        11 => ("All-call reply".into(), None, None, None),
        4 | 5 => {
            let alt = decode_ac13(((bytes[2] as u16) << 8) | bytes[3] as u16);
            ("Surveillance altitude".into(), None, alt, Some(true))
        }
        0 => ("Short air-air".into(), None, None, Some(true)),
        16 => ("Long air-air".into(), None, None, Some(true)),
        20 | 21 => ("Comm-B".into(), None, None, None),
        _ => (format!("DF{df}"), None, None, None),
    };

    Some(AdsbMessage {
        df,
        icao,
        message_type,
        callsign,
        altitude_ft,
        airborne,
        raw_hex,
        confidence,
    })
}

fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bits.len().div_ceil(8));
    for chunk in bits.chunks(8) {
        let mut b = 0u8;
        for (i, &bit) in chunk.iter().enumerate() {
            if bit {
                b |= 1 << (7 - i);
            }
        }
        out.push(b);
    }
    out
}

/// Parse DF17/18 ME field (bytes[4..10]) for common Type Codes.
fn parse_adsb_me(bytes: &[u8]) -> (String, Option<String>, Option<i32>, Option<bool>) {
    if bytes.len() < 11 {
        return ("ADS-B".into(), None, None, None);
    }
    let tc = (bytes[4] >> 3) & 0x1F;
    match tc {
        1..=4 => {
            // Aircraft identification
            let cs = decode_callsign(&bytes[5..11]);
            (format!("Identification TC{tc}"), Some(cs), None, None)
        }
        9..=18 | 20..=22 => {
            // Airborne position
            let alt_bits = ((bytes[5] as u16) << 4) | ((bytes[6] as u16) >> 4);
            let alt = decode_ac12(alt_bits);
            (format!("Airborne position TC{tc}"), None, alt, Some(true))
        }
        5..=8 => ("Surface position".into(), None, Some(0), Some(false)),
        19 => ("Airborne velocity".into(), None, None, Some(true)),
        28 => ("Aircraft status".into(), None, None, None),
        29 => ("Target state".into(), None, None, None),
        31 => ("Operational status".into(), None, None, None),
        _ => (format!("ADS-B TC{tc}"), None, None, None),
    }
}

const AIS_CHARSET: &[u8] = b"#ABCDEFGHIJKLMNOPQRSTUVWXYZ##### ###############0123456789######";

fn decode_callsign(me: &[u8]) -> String {
    // 8 chars × 6 bits packed starting at ME bit 8 (after TC+EC)
    // bytes[0] of me here is already ME byte 1 (first char bits)
    if me.len() < 6 {
        return String::new();
    }
    // Pack 48 bits from 6 bytes
    let mut chars = String::new();
    let mut acc: u64 = 0;
    for &b in &me[..6] {
        acc = (acc << 8) | b as u64;
    }
    // Top of 48 bits: 6 bits each
    for i in 0..8 {
        let shift = 42 - i * 6;
        let idx = ((acc >> shift) & 0x3F) as usize;
        let c = AIS_CHARSET.get(idx).copied().unwrap_or(b'#') as char;
        if c != '#' && c != ' ' || !chars.is_empty() {
            chars.push(if c == '#' { ' ' } else { c });
        }
    }
    chars.trim().to_string()
}

/// Decode 13-bit AC altitude field (Mode S).
fn decode_ac13(ac: u16) -> Option<i32> {
    // M bit (bit 6 of the 13) = 0 → feet with Q bit
    let m = (ac >> 6) & 1;
    if m != 0 {
        return None; // meters — rare
    }
    let q = (ac >> 4) & 1;
    if q == 1 {
        // 25-ft increments
        let n = ((ac & 0x0F80) >> 2) | ((ac & 0x0020) >> 1) | (ac & 0x000F);
        Some(n as i32 * 25 - 1000)
    } else {
        None // Gray-code 100-ft — omit for brevity
    }
}

/// Decode 12-bit AC field used in ADS-B airborne position.
fn decode_ac12(ac: u16) -> Option<i32> {
    let q = (ac >> 4) & 1;
    if q == 1 {
        let n = ((ac & 0x0FE0) >> 1) | (ac & 0x000F);
        Some(n as i32 * 25 - 1000)
    } else {
        None
    }
}

/// Convenience: decode one IQ chunk at the given rate, return all messages found.
pub fn decode_iq_chunk(
    iq: &[rustfft::num_complex::Complex<f32>],
    sample_rate_hz: u32,
) -> Vec<AdsbMessage> {
    let mut dec = match AdsbDecoder::new(sample_rate_hz) {
        Some(d) => d,
        None => return Vec::new(),
    };
    dec.feed_iq(iq);
    dec.take_messages()
}

/// Build synthetic Mode S magnitude samples for tests (ideal PPM at `sphb` samples/half-bit).
pub fn synthesize_mode_s_magnitude(bits: &[bool], sphb: usize) -> Vec<f32> {
    // Preamble: highs at half-bit 0,4,7,9
    let mut mags = vec![0.05f32; 16 * sphb];
    for &hb in &[0usize, 4, 7, 9] {
        let start = hb * sphb;
        for s in &mut mags[start..start + sphb] {
            *s = 1.0;
        }
    }
    // Data bits as PPM
    for &bit in bits {
        let mut first = vec![0.05f32; sphb];
        let mut second = vec![0.05f32; sphb];
        if bit {
            first.fill(1.0);
        } else {
            second.fill(1.0);
        }
        mags.extend(first);
        mags.extend(second);
    }
    mags
}

/// Append Mode S CRC to a 32 or 88-bit payload, returning full 56/112-bit vector.
pub fn append_crc(payload_bits: &[bool]) -> Vec<bool> {
    let crc = mode_s_crc(payload_bits);
    let mut out = payload_bits.to_vec();
    for i in (0..24).rev() {
        out.push(((crc >> i) & 1) == 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits_from_hex(hex: &str) -> Vec<bool> {
        let mut bits = Vec::new();
        for i in (0..hex.len()).step_by(2) {
            let b = u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
            for k in (0..8).rev() {
                bits.push(((b >> k) & 1) == 1);
            }
        }
        bits
    }

    #[test]
    fn crc_roundtrip_on_known_length() {
        // 88-bit payload of zeros + DF17 header nibble pattern
        let mut payload = vec![false; 88];
        // DF = 17 = 10001
        payload[0] = true;
        payload[4] = true;
        let full = append_crc(&payload);
        assert_eq!(full.len(), 112);
        let calc = mode_s_crc(&full[..88]);
        let mut rx = 0u32;
        for &b in &full[88..] {
            rx = (rx << 1) | if b { 1 } else { 0 };
        }
        assert_eq!(calc, rx);
    }

    #[test]
    fn decodes_synthetic_df17_frame() {
        // Build DF17, ICAO = ABCDEF, TC=4 identification-ish zeros body
        let mut payload = vec![false; 88];
        // DF=17 → bits 0..4 = 10001
        payload[0] = true;
        payload[4] = true;
        // ICAO ABCDEF = 1010 1011 1100 1101 1110 1111
        let icao = [0xABu8, 0xCD, 0xEF];
        for (bi, byte) in icao.iter().enumerate() {
            for k in 0..8 {
                payload[8 + bi * 8 + k] = ((byte >> (7 - k)) & 1) == 1;
            }
        }
        // TC = 4 (identification) in ME top 5 bits of byte 4 → bits 32..36
        // TC=4 = 00100
        payload[34] = true;

        let full = append_crc(&payload);
        assert_eq!(full.len(), 112);

        // Direct bit decode
        let msg = decode_mode_s_bits(&full).expect("CRC frame must decode");
        assert_eq!(msg.df, 17);
        assert_eq!(msg.icao, "ABCDEF");
        assert!(msg.confidence > 0.9);

        // Through magnitude demod at 2 Msps (sphb=1)
        let mags = synthesize_mode_s_magnitude(&full, 1);
        let mut dec = AdsbDecoder::new(2_000_000).unwrap();
        dec.feed_magnitude(&mags);
        let msgs = dec.take_messages();
        assert!(!msgs.is_empty(), "preamble+PPM path should recover the frame");
        assert_eq!(msgs[0].icao, "ABCDEF");
        assert_eq!(msgs[0].df, 17);
    }

    #[test]
    fn rejects_corrupt_crc() {
        let bits = bits_from_hex("8DABCDEF00000000000000000000");
        // Force wrong length / garbage — 112 bits of mostly zero DF
        let mut bad = bits;
        if bad.len() >= 112 {
            bad.truncate(112);
            bad[100] = !bad[100]; // flip a CRC bit
            assert!(decode_mode_s_bits(&bad).is_none());
        }
    }

    #[test]
    fn rejects_low_sample_rate() {
        assert!(AdsbDecoder::new(250_000).is_none());
        assert!(AdsbDecoder::new(2_000_000).is_some());
    }

    #[test]
    fn callsign_charset_basic() {
        // Empty ME → empty/short callsign, must not panic
        let cs = decode_callsign(&[0u8; 6]);
        assert!(cs.len() <= 8);
    }
}
