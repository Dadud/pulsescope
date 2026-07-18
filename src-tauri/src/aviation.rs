//! Native aviation data-link framing and validation.
//!
//! This module deliberately starts at protocol-appropriate hard decisions:
//!
//! * UAT input is one boolean per recovered 1.041667 Mbit/s CPFSK symbol. UAT is
//!   **not PPM**; carrier filtering, FM/CPFSK discrimination, symbol timing and
//!   optional Reed-Solomon error *correction* are outside this module. The
//!   decoder detects the 36-bit UAT sync words and validates the transmitted
//!   Reed-Solomon parity by evaluating all syndromes.
//! * ACARS input is recovered 2400 bit/s MSK discriminator bits. MSK filtering,
//!   clock recovery and polarity selection are outside this module.
//! * VDL Mode 2 uses D8PSK, not FSK. `Vdl2Decoder::feed_bits` therefore accepts
//!   an already recovered/decoded HDLC bit stream; D8PSK carrier recovery,
//!   differential decoding, deinterleaving and physical-layer FEC are not
//!   implemented here. `feed_nrzi_levels` is provided only for sources which
//!   expose HDLC NRZI levels after those stages.
//!
//! No decoded output is synthesized: a UAT frame is emitted only with zero RS
//! syndromes, while ACARS and VDL2 completed frames carry explicit CRC/FCS and
//! parity validity.

use serde::{Deserialize, Serialize};

const UAT_SYNC_BITS: usize = 36;
const UAT_DOWNLINK_SYNC: u64 = 0xEACD_DA4E2;
const UAT_UPLINK_SYNC: u64 = 0x1532_25B1D;
const UAT_SHORT_BYTES: usize = 30; // 18 data + 12 RS parity
const UAT_LONG_BYTES: usize = 48; // 34 data + 14 RS parity
const UAT_UPLINK_BLOCK_BYTES: usize = 92; // 72 data + 20 RS parity
const UAT_UPLINK_BLOCKS: usize = 6;

/// Raw-IQ front end for UAT CPFSK. It performs phase discrimination and a
/// fixed-clock hard slicer at the UAT symbol rate; higher-quality timing loops
/// can feed `UatDecoder::feed_bits` directly.
pub struct UatIqDecoder {
    decoder: UatDecoder,
    sample_rate: u32,
    clock: u64,
    sum: f64,
    previous: Option<(f32, f32)>,
}

impl UatIqDecoder {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            decoder: UatDecoder::new(),
            sample_rate,
            clock: 0,
            sum: 0.0,
            previous: None,
        }
    }
    pub fn push_iq(&mut self, samples: &[(f32, f32)]) {
        for &(i, q) in samples {
            if let Some((pi, pq)) = self.previous {
                self.sum += (q * pi - i * pq).atan2(i * pi + q * pq) as f64;
                self.clock += 1_041_667;
                if self.clock >= self.sample_rate as u64 {
                    self.clock -= self.sample_rate as u64;
                    self.decoder.feed_bits(&[self.sum >= 0.0]);
                    self.sum = 0.0;
                }
            }
            self.previous = Some((i, q));
        }
    }
    pub fn take_messages(&mut self) -> Vec<UatMessage> {
        self.decoder.take_messages()
    }
}

/// Raw-IQ front end for ACARS MSK. The output is handed to the native ACARS
/// framing decoder; callers may use `AcarsDecoder::feed_bits` for recovered
/// clocks when the capture has substantial sample-rate error.
pub struct AcarsIqDecoder {
    decoder: AcarsDecoder,
    sample_rate: u32,
    clock: u64,
    sum: f64,
    previous: Option<(f32, f32)>,
}

impl AcarsIqDecoder {
    pub fn new(sample_rate: u32, order: BitOrder, invert: bool) -> Self {
        Self {
            decoder: AcarsDecoder::new(order, invert),
            sample_rate,
            clock: 0,
            sum: 0.0,
            previous: None,
        }
    }
    pub fn push_iq(&mut self, samples: &[(f32, f32)]) {
        for &(i, q) in samples {
            if let Some((pi, pq)) = self.previous {
                self.sum += (q * pi - i * pq).atan2(i * pi + q * pq) as f64;
                self.clock += 2400;
                if self.clock >= self.sample_rate as u64 {
                    self.clock -= self.sample_rate as u64;
                    self.decoder.feed_bits(&[self.sum >= 0.0]);
                    self.sum = 0.0;
                }
            }
            self.previous = Some((i, q));
        }
    }
    pub fn take_messages(&mut self) -> Vec<AcarsMessage> {
        self.decoder.take_messages()
    }
}

/// The on-air UAT frame family selected by its sync word and downlink type.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UatFrameKind {
    DownlinkShort,
    DownlinkLong,
    Uplink,
}

/// A validated UAT frame. `payload` excludes Reed-Solomon parity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UatMessage {
    pub frame_kind: UatFrameKind,
    /// Downlink payload type (top five bits of the first payload octet).
    pub message_code: Option<u8>,
    /// Downlink address qualifier (low three bits of the first octet).
    pub address_qualifier: Option<u8>,
    /// Downlink 24-bit address, without assuming that every qualifier means ICAO.
    pub address_hex: Option<String>,
    pub payload: Vec<u8>,
    /// Complete transmitted codeword(s), including RS parity.
    pub raw_codeword: Vec<u8>,
    pub fec_valid: bool,
}

/// Streaming UAT sync detector and Reed-Solomon validator.
pub struct UatDecoder {
    bits: Vec<bool>,
    scan_at: usize,
    max_sync_errors: u8,
    messages: Vec<UatMessage>,
}

impl Default for UatDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl UatDecoder {
    /// Exact sync matching is the conservative default.
    pub fn new() -> Self {
        Self {
            bits: Vec::new(),
            scan_at: 0,
            max_sync_errors: 0,
            messages: Vec::new(),
        }
    }

    /// Permit up to `errors` hard-bit errors in the 36-bit sync word.
    /// Values above three are rejected to keep accidental candidates bounded.
    pub fn with_sync_tolerance(errors: u8) -> Option<Self> {
        if errors > 3 {
            return None;
        }
        let mut decoder = Self::new();
        decoder.max_sync_errors = errors;
        Some(decoder)
    }

    /// Feed symbol decisions, MSB first in each over-the-air octet.
    pub fn feed_bits(&mut self, bits: &[bool]) {
        self.bits.extend_from_slice(bits);
        self.scan();
        self.compact();
    }

    pub fn take_messages(&mut self) -> Vec<UatMessage> {
        std::mem::take(&mut self.messages)
    }

    fn scan(&mut self) {
        while self.scan_at + UAT_SYNC_BITS <= self.bits.len() {
            let at = self.scan_at;
            let down_errors = sync_distance(&self.bits[at..at + UAT_SYNC_BITS], UAT_DOWNLINK_SYNC);
            let up_errors = sync_distance(&self.bits[at..at + UAT_SYNC_BITS], UAT_UPLINK_SYNC);

            if down_errors <= self.max_sync_errors {
                if self.bits.len() < at + UAT_SYNC_BITS + 8 {
                    break;
                }
                let first =
                    bits_to_bytes_msb(&self.bits[at + UAT_SYNC_BITS..at + UAT_SYNC_BITS + 8])[0];
                let bytes = if first >> 3 == 0 {
                    UAT_SHORT_BYTES
                } else {
                    UAT_LONG_BYTES
                };
                let end = at + UAT_SYNC_BITS + bytes * 8;
                if self.bits.len() < end {
                    break;
                }
                let codeword = bits_to_bytes_msb(&self.bits[at + UAT_SYNC_BITS..end]);
                if let Some(message) = parse_uat_downlink(&codeword) {
                    self.messages.push(message);
                    self.scan_at = end;
                    continue;
                }
            } else if up_errors <= self.max_sync_errors {
                let encoded_bytes = UAT_UPLINK_BLOCK_BYTES * UAT_UPLINK_BLOCKS;
                let end = at + UAT_SYNC_BITS + encoded_bytes * 8;
                if self.bits.len() < end {
                    break;
                }
                let codeword = bits_to_bytes_msb(&self.bits[at + UAT_SYNC_BITS..end]);
                if let Some(message) = parse_uat_uplink(&codeword) {
                    self.messages.push(message);
                    self.scan_at = end;
                    continue;
                }
            }
            self.scan_at += 1;
        }
    }

    fn compact(&mut self) {
        // Retain a possible partial sync/candidate while bounding long noise runs.
        if self.scan_at > 8192 {
            let drain = self.scan_at.saturating_sub(UAT_SYNC_BITS - 1);
            self.bits.drain(..drain);
            self.scan_at -= drain;
        }
    }
}

fn parse_uat_downlink(codeword: &[u8]) -> Option<UatMessage> {
    let (kind, data_len, roots) = match codeword.len() {
        UAT_SHORT_BYTES => (UatFrameKind::DownlinkShort, 18, 12),
        UAT_LONG_BYTES => (UatFrameKind::DownlinkLong, 34, 14),
        _ => return None,
    };
    if !uat_rs_valid(codeword, roots) {
        return None;
    }
    let payload = codeword[..data_len].to_vec();
    let message_code = payload[0] >> 3;
    // Type zero is the short format; all non-zero downlink types use long format.
    if (message_code == 0) != (kind == UatFrameKind::DownlinkShort) {
        return None;
    }
    Some(UatMessage {
        frame_kind: kind,
        message_code: Some(message_code),
        address_qualifier: Some(payload[0] & 0x07),
        address_hex: Some(format!(
            "{:02X}{:02X}{:02X}",
            payload[1], payload[2], payload[3]
        )),
        payload,
        raw_codeword: codeword.to_vec(),
        fec_valid: true,
    })
}

fn parse_uat_uplink(codeword: &[u8]) -> Option<UatMessage> {
    if codeword.len() != UAT_UPLINK_BLOCK_BYTES * UAT_UPLINK_BLOCKS {
        return None;
    }
    let mut payload = Vec::with_capacity(72 * UAT_UPLINK_BLOCKS);
    for block in codeword.chunks_exact(UAT_UPLINK_BLOCK_BYTES) {
        if !uat_rs_valid(block, 20) {
            return None;
        }
        payload.extend_from_slice(&block[..72]);
    }
    Some(UatMessage {
        frame_kind: UatFrameKind::Uplink,
        message_code: None,
        address_qualifier: None,
        address_hex: None,
        payload,
        raw_codeword: codeword.to_vec(),
        fec_valid: true,
    })
}

/// Validate a UAT shortened RS(255, k) codeword without attempting correction.
///
/// UAT uses GF(256), primitive polynomial `0x187`, first consecutive root 120,
/// primitive element step 1, with 12/14/20 roots depending on frame/block type.
pub fn uat_rs_valid(codeword: &[u8], roots: usize) -> bool {
    if !matches!((codeword.len(), roots), (30, 12) | (48, 14) | (92, 20)) {
        return false;
    }
    (0..roots).all(|i| {
        let root = gf_pow_alpha(120 + i);
        codeword
            .iter()
            .fold(0u8, |acc, &octet| gf_mul(acc, root) ^ octet)
            == 0
    })
}

fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut product = 0u8;
    while b != 0 {
        if b & 1 != 0 {
            product ^= a;
        }
        b >>= 1;
        let carry = a & 0x80 != 0;
        a <<= 1;
        if carry {
            // x^8 + x^7 + x^2 + x + 1 (0x187, with x^8 omitted here)
            a ^= 0x87;
        }
    }
    product
}

fn gf_pow_alpha(power: usize) -> u8 {
    let mut value = 1u8;
    for _ in 0..(power % 255) {
        value = gf_mul(value, 2);
    }
    value
}

fn sync_distance(bits: &[bool], word: u64) -> u8 {
    bits.iter()
        .enumerate()
        .filter(|(i, bit)| **bit != (((word >> (UAT_SYNC_BITS - 1 - i)) & 1) != 0))
        .count() as u8
}

fn bits_to_bytes_msb(bits: &[bool]) -> Vec<u8> {
    bits.chunks_exact(8)
        .map(|chunk| {
            chunk
                .iter()
                .fold(0u8, |value, bit| (value << 1) | u8::from(*bit))
        })
        .collect()
}

/// Bit packing used by asynchronous aviation links.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BitOrder {
    LsbFirst,
    MsbFirst,
}

/// Raw-IQ VDL2 front end. VDL Mode 2 uses differential 8-PSK at 31.5 ksym/s;
/// this performs phase differencing, sector slicing and Gray-symbol expansion.
/// The HDLC/FCS decoder remains responsible for rejecting bad frames.
pub struct Vdl2IqDecoder {
    decoder: Vdl2Decoder,
    sample_rate: u32,
    clock: u64,
    phase_sum: f64,
    previous: Option<(f32, f32)>,
}

impl Vdl2IqDecoder {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            decoder: Vdl2Decoder::new(),
            sample_rate,
            clock: 0,
            phase_sum: 0.0,
            previous: None,
        }
    }
    pub fn push_iq(&mut self, samples: &[(f32, f32)]) {
        for &(i, q) in samples {
            if let Some((pi, pq)) = self.previous {
                self.phase_sum += (q * pi - i * pq).atan2(i * pi + q * pq) as f64;
                self.clock += 31_500;
                if self.clock >= self.sample_rate as u64 {
                    self.clock -= self.sample_rate as u64;
                    let sector = ((self.phase_sum / (std::f64::consts::TAU / 8.0)).round() as i32)
                        .rem_euclid(8) as usize;
                    // Differential 8-PSK Gray ordering: adjacent sectors differ by one bit.
                    const GRAY: [u8; 8] = [0, 1, 3, 2, 6, 7, 5, 4];
                    let symbol = GRAY[sector];
                    self.decoder.feed_bits(&[
                        (symbol & 4) != 0,
                        (symbol & 2) != 0,
                        (symbol & 1) != 0,
                    ]);
                    self.phase_sum = 0.0;
                }
            }
            self.previous = Some((i, q));
        }
    }
    pub fn take_messages(&mut self) -> Vec<Vdl2Message> {
        self.decoder.take_messages()
    }
}

/// A complete ACARS block, including its integrity result.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcarsMessage {
    pub mode: Option<char>,
    pub registration: Option<String>,
    pub acknowledgement: Option<char>,
    pub label: Option<String>,
    pub block_id: Option<char>,
    pub text: String,
    pub end_of_block: bool,
    pub crc_valid: bool,
    pub parity_errors: usize,
    /// Bytes from mode through terminator, followed by the two BCS octets.
    pub raw_bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AcarsState {
    FirstSync,
    SecondSync,
    Soh,
    Body,
    Fcs1,
    Fcs2,
}

/// Streaming ACARS character/framing decoder for recovered MSK bits.
pub struct AcarsDecoder {
    order: BitOrder,
    invert: bool,
    byte: u8,
    bit_count: u8,
    state: AcarsState,
    body: Vec<u8>,
    fcs: [u8; 2],
    messages: Vec<AcarsMessage>,
    max_body_bytes: usize,
}

impl Default for AcarsDecoder {
    fn default() -> Self {
        Self::new(BitOrder::LsbFirst, false)
    }
}

impl AcarsDecoder {
    pub fn new(order: BitOrder, invert: bool) -> Self {
        Self {
            order,
            invert,
            byte: 0,
            bit_count: 0,
            state: AcarsState::FirstSync,
            body: Vec::new(),
            fcs: [0; 2],
            messages: Vec::new(),
            max_body_bytes: 240,
        }
    }

    pub fn feed_bits(&mut self, bits: &[bool]) {
        for &input in bits {
            let bit = input ^ self.invert;
            let position = match self.order {
                BitOrder::LsbFirst => self.bit_count,
                BitOrder::MsbFirst => 7 - self.bit_count,
            };
            if bit {
                self.byte |= 1 << position;
            }
            self.bit_count += 1;
            if self.bit_count == 8 {
                let byte = self.byte;
                self.byte = 0;
                self.bit_count = 0;
                self.feed_byte(byte);
            }
        }
    }

    pub fn take_messages(&mut self) -> Vec<AcarsMessage> {
        std::mem::take(&mut self.messages)
    }

    fn reset(&mut self) {
        self.state = AcarsState::FirstSync;
        self.body.clear();
    }

    fn feed_byte(&mut self, byte: u8) {
        const SYN: u8 = 0x16;
        const SOH: u8 = 0x01;
        const ETX: u8 = 0x83;
        const ETB: u8 = 0x97;
        match self.state {
            AcarsState::FirstSync => {
                if byte == SYN {
                    self.state = AcarsState::SecondSync;
                }
            }
            AcarsState::SecondSync => {
                self.state = if byte == SYN {
                    AcarsState::Soh
                } else {
                    AcarsState::FirstSync
                };
            }
            AcarsState::Soh => {
                if byte == SOH {
                    self.body.clear();
                    self.state = AcarsState::Body;
                } else if byte != SYN {
                    self.reset();
                }
            }
            AcarsState::Body => {
                self.body.push(byte);
                if byte == ETX || byte == ETB {
                    self.state = AcarsState::Fcs1;
                } else if self.body.len() >= self.max_body_bytes {
                    self.reset();
                }
            }
            AcarsState::Fcs1 => {
                self.fcs[0] = byte;
                self.state = AcarsState::Fcs2;
            }
            AcarsState::Fcs2 => {
                self.fcs[1] = byte;
                let message = parse_acars_block(&self.body, self.fcs);
                self.messages.push(message);
                self.reset();
            }
        }
    }
}

fn parse_acars_block(body: &[u8], fcs: [u8; 2]) -> AcarsMessage {
    let seven = |index: usize| body.get(index).map(|byte| byte & 0x7f);
    let field = |range: std::ops::Range<usize>| -> Option<String> {
        if range.end > body.len() {
            return None;
        }
        Some(
            body[range]
                .iter()
                .map(|byte| (byte & 0x7f) as char)
                .collect::<String>()
                .trim()
                .to_string(),
        )
    };
    let terminator_at = body.len().saturating_sub(1);
    // Standard header: mode, 7-char address, ACK, 2-char label, block id, STX.
    let text_start = if body.get(12).map(|b| b & 0x7f) == Some(0x02) {
        13
    } else {
        12
    };
    let text = if text_start <= terminator_at {
        body[text_start..terminator_at]
            .iter()
            .map(|byte| {
                let c = byte & 0x7f;
                if c == b'\r' || c == b'\n' || c == b'\t' || (0x20..=0x7e).contains(&c) {
                    c as char
                } else {
                    '\u{fffd}'
                }
            })
            .collect()
    } else {
        String::new()
    };

    let mut checked = body.to_vec();
    checked.extend_from_slice(&fcs);
    let parity_errors = body
        .iter()
        .filter(|byte| byte.count_ones() % 2 == 0)
        .count();
    let mut raw_bytes = body.to_vec();
    raw_bytes.extend_from_slice(&fcs);
    AcarsMessage {
        mode: seven(0).map(char::from),
        registration: field(1..8),
        acknowledgement: seven(8).map(char::from),
        label: field(9..11),
        block_id: seven(11).map(char::from),
        text,
        end_of_block: body.last().copied() == Some(0x97),
        crc_valid: acars_crc16(&checked) == 0,
        parity_errors,
        raw_bytes,
    }
}

/// Reflected CRC-16-CCITT used for the ACARS BCS (initial value zero).
pub fn acars_crc16(bytes: &[u8]) -> u16 {
    crc16_reflected(bytes, 0)
}

/// A delimited VDL2 AVLC/HDLC frame. `payload` excludes its two FCS octets.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Vdl2Message {
    pub payload: Vec<u8>,
    pub raw_frame: Vec<u8>,
    pub fcs_valid: bool,
}

/// Streaming HDLC deframer for a post-physical-layer VDL2 bit stream.
pub struct Vdl2Decoder {
    since_flag: Vec<bool>,
    messages: Vec<Vdl2Message>,
    previous_nrzi_level: Option<bool>,
    max_frame_bits: usize,
}

impl Default for Vdl2Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Vdl2Decoder {
    pub fn new() -> Self {
        Self {
            since_flag: Vec::new(),
            messages: Vec::new(),
            previous_nrzi_level: None,
            max_frame_bits: 16_384,
        }
    }

    /// Feed HDLC bits after VDL2 physical decoding, in over-the-air order.
    pub fn feed_bits(&mut self, bits: &[bool]) {
        const FLAG: [bool; 8] = [false, true, true, true, true, true, true, false];
        for &bit in bits {
            self.since_flag.push(bit);
            if self.since_flag.ends_with(&FLAG) {
                let frame_len = self.since_flag.len() - FLAG.len();
                if frame_len > 0 {
                    let raw = self.since_flag[..frame_len].to_vec();
                    if let Some(frame) = decode_hdlc_frame(&raw) {
                        self.messages.push(frame);
                    }
                }
                self.since_flag.clear();
            } else if self.since_flag.len() > self.max_frame_bits {
                // Keep enough tail to still recognize a split flag.
                let keep = FLAG.len() - 1;
                let drain = self.since_flag.len() - keep;
                self.since_flag.drain(..drain);
            }
        }
    }

    /// Convert NRZI levels (zero causes a transition) before HDLC deframing.
    pub fn feed_nrzi_levels(&mut self, levels: &[bool]) {
        let mut decoded = Vec::with_capacity(levels.len());
        for &level in levels {
            if let Some(previous) = self.previous_nrzi_level {
                decoded.push(level == previous);
            }
            self.previous_nrzi_level = Some(level);
        }
        self.feed_bits(&decoded);
    }

    pub fn take_messages(&mut self) -> Vec<Vdl2Message> {
        std::mem::take(&mut self.messages)
    }
}

fn decode_hdlc_frame(stuffed: &[bool]) -> Option<Vdl2Message> {
    let mut bits = Vec::with_capacity(stuffed.len());
    let mut ones = 0u8;
    let mut i = 0usize;
    while i < stuffed.len() {
        let bit = stuffed[i];
        if bit {
            ones += 1;
            if ones > 5 {
                return None;
            }
            bits.push(true);
        } else {
            if ones == 5 {
                ones = 0;
                i += 1;
                continue;
            }
            ones = 0;
            bits.push(false);
        }
        i += 1;
    }
    if bits.len() < 24 || bits.len() % 8 != 0 {
        return None;
    }
    let raw_frame: Vec<u8> = bits
        .chunks_exact(8)
        .map(|chunk| {
            chunk.iter().enumerate().fold(0u8, |byte, (position, bit)| {
                byte | (u8::from(*bit) << position)
            })
        })
        .collect();
    let fcs_valid = vdl2_fcs_valid(&raw_frame);
    let payload = raw_frame[..raw_frame.len() - 2].to_vec();
    Some(Vdl2Message {
        payload,
        raw_frame,
        fcs_valid,
    })
}

/// Validate an HDLC/AVLC FCS using CRC-16/X-25's on-wire good residue.
pub fn vdl2_fcs_valid(frame_with_fcs: &[u8]) -> bool {
    frame_with_fcs.len() >= 3 && crc16_reflected(frame_with_fcs, 0xffff) == 0xf0b8
}

fn crc16_reflected(bytes: &[u8], initial: u16) -> u16 {
    let mut crc = initial;
    for &byte in bytes {
        crc ^= byte as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x8408
            } else {
                crc >> 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits_of_word(word: u64, count: usize) -> Vec<bool> {
        (0..count)
            .map(|i| ((word >> (count - 1 - i)) & 1) != 0)
            .collect()
    }

    fn bytes_to_bits_msb(bytes: &[u8]) -> Vec<bool> {
        bytes
            .iter()
            .flat_map(|byte| (0..8).rev().map(move |bit| ((byte >> bit) & 1) != 0))
            .collect()
    }

    fn bytes_to_bits_lsb(bytes: &[u8]) -> Vec<bool> {
        bytes
            .iter()
            .flat_map(|byte| (0..8).map(move |bit| ((byte >> bit) & 1) != 0))
            .collect()
    }

    fn rs_encode(data: &[u8], roots: usize) -> Vec<u8> {
        let mut generator = vec![1u8];
        for i in 0..roots {
            let root = gf_pow_alpha(120 + i);
            let mut next = vec![0u8; generator.len() + 1];
            for (j, &coefficient) in generator.iter().enumerate() {
                next[j] ^= coefficient;
                next[j + 1] ^= gf_mul(coefficient, root);
            }
            generator = next;
        }
        let mut work = data.to_vec();
        work.resize(data.len() + roots, 0);
        for i in 0..data.len() {
            let lead = work[i];
            if lead != 0 {
                for j in 1..generator.len() {
                    work[i + j] ^= gf_mul(lead, generator[j]);
                }
            }
        }
        let mut codeword = data.to_vec();
        codeword.extend_from_slice(&work[data.len()..]);
        codeword
    }

    fn odd_parity(value: u8) -> u8 {
        let seven = value & 0x7f;
        if seven.count_ones() % 2 == 0 {
            seven | 0x80
        } else {
            seven
        }
    }

    fn acars_fixture() -> Vec<u8> {
        let mut body: Vec<u8> = b"2N123AB \x15Q0A\x02HELLO"
            .iter()
            .copied()
            .map(odd_parity)
            .collect();
        body.push(0x83); // ETX already has odd parity
        let crc = acars_crc16(&body);
        let mut wire = vec![0x16, 0x16, 0x01];
        wire.extend_from_slice(&body);
        wire.push(crc as u8);
        wire.push((crc >> 8) as u8);
        wire
    }

    fn hdlc_fixture(payload: &[u8]) -> Vec<bool> {
        let mut frame = payload.to_vec();
        let crc = !crc16_reflected(payload, 0xffff);
        frame.push(crc as u8);
        frame.push((crc >> 8) as u8);
        assert!(vdl2_fcs_valid(&frame));
        let raw = bytes_to_bits_lsb(&frame);
        let mut stuffed = Vec::new();
        let mut ones = 0;
        for bit in raw {
            stuffed.push(bit);
            if bit {
                ones += 1;
                if ones == 5 {
                    stuffed.push(false);
                    ones = 0;
                }
            } else {
                ones = 0;
            }
        }
        let flag = [false, true, true, true, true, true, true, false];
        let mut wire = flag.to_vec();
        wire.extend(stuffed);
        wire.extend(flag);
        wire
    }

    #[test]
    fn uat_short_frame_streams_across_chunks_and_validates_rs() {
        let mut data = [0u8; 18];
        data[0] = 0x03; // short type zero, qualifier three
        data[1..4].copy_from_slice(&[0xAB, 0xCD, 0xEF]);
        data[4..9].copy_from_slice(b"HELLO");
        let codeword = rs_encode(&data, 12);
        assert!(uat_rs_valid(&codeword, 12));

        let mut wire = bits_of_word(UAT_DOWNLINK_SYNC, UAT_SYNC_BITS);
        wire.extend(bytes_to_bits_msb(&codeword));
        let mut decoder = UatDecoder::new();
        decoder.feed_bits(&wire[..31]);
        decoder.feed_bits(&wire[31..173]);
        decoder.feed_bits(&wire[173..]);
        let messages = decoder.take_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].frame_kind, UatFrameKind::DownlinkShort);
        assert_eq!(messages[0].address_hex.as_deref(), Some("ABCDEF"));
        assert_eq!(messages[0].address_qualifier, Some(3));
    }

    #[test]
    fn uat_long_and_uplink_frames_validate() {
        let mut long_data = [0u8; 34];
        long_data[0] = 1 << 3;
        long_data[1..4].copy_from_slice(&[1, 2, 3]);
        let long_codeword = rs_encode(&long_data, 14);
        let mut down = bits_of_word(UAT_DOWNLINK_SYNC, UAT_SYNC_BITS);
        down.extend(bytes_to_bits_msb(&long_codeword));

        let mut uplink_codeword = Vec::new();
        for block_number in 0..UAT_UPLINK_BLOCKS {
            let mut block = [0u8; 72];
            block[0] = block_number as u8;
            uplink_codeword.extend(rs_encode(&block, 20));
        }
        let mut up = bits_of_word(UAT_UPLINK_SYNC, UAT_SYNC_BITS);
        up.extend(bytes_to_bits_msb(&uplink_codeword));

        let mut decoder = UatDecoder::new();
        decoder.feed_bits(&down);
        decoder.feed_bits(&[false; 19]);
        decoder.feed_bits(&up);
        let messages = decoder.take_messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].frame_kind, UatFrameKind::DownlinkLong);
        assert_eq!(messages[1].frame_kind, UatFrameKind::Uplink);
        assert_eq!(messages[1].payload.len(), 432);
    }

    #[test]
    fn uat_rejects_corrupt_rs_frame() {
        let mut data = [0u8; 18];
        data[1] = 0x44;
        let mut codeword = rs_encode(&data, 12);
        codeword[7] ^= 0x01;
        assert!(!uat_rs_valid(&codeword, 12));
        let mut wire = bits_of_word(UAT_DOWNLINK_SYNC, UAT_SYNC_BITS);
        wire.extend(bytes_to_bits_msb(&codeword));
        let mut decoder = UatDecoder::new();
        decoder.feed_bits(&wire);
        assert!(decoder.take_messages().is_empty());
    }

    #[test]
    fn acars_streaming_parser_extracts_header_text_and_crc() {
        let wire = acars_fixture();
        let bits = bytes_to_bits_lsb(&wire);
        let mut decoder = AcarsDecoder::default();
        for chunk in bits.chunks(13) {
            decoder.feed_bits(chunk);
        }
        let messages = decoder.take_messages();
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert!(message.crc_valid);
        assert_eq!(message.parity_errors, 0);
        assert_eq!(message.mode, Some('2'));
        assert_eq!(message.registration.as_deref(), Some("N123AB"));
        assert_eq!(message.label.as_deref(), Some("Q0"));
        assert_eq!(message.block_id, Some('A'));
        assert_eq!(message.text, "HELLO");
    }

    #[test]
    fn acars_reports_bad_bcs_without_fake_success() {
        let mut wire = acars_fixture();
        let last = wire.len() - 1;
        wire[last] ^= 0x40;
        let mut decoder = AcarsDecoder::default();
        decoder.feed_bits(&bytes_to_bits_lsb(&wire));
        let messages = decoder.take_messages();
        assert_eq!(messages.len(), 1);
        assert!(!messages[0].crc_valid);
    }

    #[test]
    fn vdl2_hdlc_destuffs_and_validates_fcs() {
        let payload = [0xFF, 0xF8, 0x7E, 0x01, 0x23, 0x45];
        let wire = hdlc_fixture(&payload);
        let mut decoder = Vdl2Decoder::new();
        for chunk in wire.chunks(7) {
            decoder.feed_bits(chunk);
        }
        let messages = decoder.take_messages();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].fcs_valid);
        assert_eq!(messages[0].payload, payload);
    }

    #[test]
    fn vdl2_nrzi_level_api_preserves_streaming_state() {
        let payload = b"AVLC";
        let bits = hdlc_fixture(payload);
        // Add one initial level because the first NRZI level establishes state.
        let mut level = false;
        let mut levels = vec![level];
        for bit in bits {
            if !bit {
                level = !level;
            }
            levels.push(level);
        }
        let mut decoder = Vdl2Decoder::new();
        decoder.feed_nrzi_levels(&levels[..11]);
        decoder.feed_nrzi_levels(&levels[11..]);
        let messages = decoder.take_messages();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].fcs_valid);
        assert_eq!(messages[0].payload, payload);
    }

    #[test]
    fn vdl2_bad_fcs_is_typed_as_invalid() {
        let payload = b"BAD";
        let mut wire = hdlc_fixture(payload);
        // Flip a non-flag data bit.
        wire[10] = !wire[10];
        let mut decoder = Vdl2Decoder::new();
        decoder.feed_bits(&wire);
        let messages = decoder.take_messages();
        assert_eq!(messages.len(), 1);
        assert!(!messages[0].fcs_valid);
    }
}
