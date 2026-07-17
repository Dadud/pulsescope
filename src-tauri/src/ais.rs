//! Native, clean-room AIS link-layer and message decoder.
//!
//! # Input contract
//! [`HdlcDecoder::push_bit`] accepts **NRZI-decoded data bits** in over-the-air
//! time order (`true` = one, `false` = zero), including HDLC flags and stuffed
//! zeroes. It does not accept packed bytes or raw audio. [`NrziHdlcDecoder`]
//! adds NRZI line-level decoding (`1` = no transition, `0` = transition).
//! [`DiscriminatorDecoder`] is deliberately only a fixed-clock helper for a
//! symbol-timed, zero-centred discriminator waveform. It is not a carrier,
//! matched-filter, or clock-recovery implementation; production GMSK audio must
//! be filtered and clock-recovered before this boundary.

use serde::{Deserialize, Serialize};
use std::fmt;

const FLAG: [bool; 8] = [false, true, true, true, true, true, true, false];
const MAX_FRAME_BITS: usize = 16_384;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AisDecodeError {
    FrameTooShort,
    FrameTooLong,
    InvalidBitStuffing,
    NonOctetFrame(usize),
    CrcMismatch {
        expected: u16,
        calculated: u16,
    },
    InvalidSixBitCharacter(char),
    InvalidFillBits(u8),
    PayloadTooShort {
        message_type: u8,
        needed: usize,
        actual: usize,
    },
    UnsupportedMessageType(u8),
}

impl fmt::Display for AisDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameTooShort => write!(f, "AIS HDLC frame is too short"),
            Self::FrameTooLong => write!(f, "AIS HDLC frame exceeded {MAX_FRAME_BITS} bits"),
            Self::InvalidBitStuffing => write!(f, "invalid HDLC bit stuffing or abort sequence"),
            Self::NonOctetFrame(n) => {
                write!(f, "unstuffed HDLC frame has {n} bits (not octet aligned)")
            }
            Self::CrcMismatch {
                expected,
                calculated,
            } => write!(
                f,
                "AIS CRC mismatch: received {expected:04x}, calculated {calculated:04x}"
            ),
            Self::InvalidSixBitCharacter(c) => {
                write!(f, "invalid AIS six-bit payload character {c:?}")
            }
            Self::InvalidFillBits(n) => write!(f, "invalid AIS six-bit fill-bit count {n}"),
            Self::PayloadTooShort {
                message_type,
                needed,
                actual,
            } => write!(
                f,
                "AIS type {message_type} needs {needed} bits, got {actual}"
            ),
            Self::UnsupportedMessageType(t) => write!(f, "unsupported AIS message type {t}"),
        }
    }
}

impl std::error::Error for AisDecodeError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum AisMessage {
    PositionClassA(PositionReportClassA),
    StaticVoyage(StaticVoyageData),
    PositionClassB(PositionReportClassB),
    StaticDataReport(StaticDataReport),
}

impl AisMessage {
    pub fn message_type(&self) -> u8 {
        match self {
            Self::PositionClassA(v) => v.message_type,
            Self::StaticVoyage(_) => 5,
            Self::PositionClassB(_) => 18,
            Self::StaticDataReport(_) => 24,
        }
    }

    pub fn mmsi(&self) -> u32 {
        match self {
            Self::PositionClassA(v) => v.mmsi,
            Self::StaticVoyage(v) => v.mmsi,
            Self::PositionClassB(v) => v.mmsi,
            Self::StaticDataReport(v) => v.mmsi(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionReportClassA {
    pub message_type: u8,
    pub repeat_indicator: u8,
    pub mmsi: u32,
    pub navigation_status: u8,
    pub rate_of_turn_raw: i8,
    pub rate_of_turn_deg_per_min: Option<f32>,
    pub speed_over_ground_knots: Option<f32>,
    pub position_accuracy: bool,
    pub longitude_deg: Option<f64>,
    pub latitude_deg: Option<f64>,
    pub course_over_ground_deg: Option<f32>,
    pub true_heading_deg: Option<u16>,
    pub timestamp_second: Option<u8>,
    pub maneuver_indicator: u8,
    pub raim: bool,
    pub radio_status: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Eta {
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dimensions {
    pub to_bow_m: u16,
    pub to_stern_m: u16,
    pub to_port_m: u8,
    pub to_starboard_m: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StaticVoyageData {
    pub repeat_indicator: u8,
    pub mmsi: u32,
    pub ais_version: u8,
    pub imo_number: Option<u32>,
    pub call_sign: String,
    pub vessel_name: String,
    pub ship_type: u8,
    pub dimensions: Dimensions,
    pub position_fix_type: u8,
    pub eta: Option<Eta>,
    pub draught_m: Option<f32>,
    pub destination: String,
    pub data_terminal_ready: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionReportClassB {
    pub repeat_indicator: u8,
    pub mmsi: u32,
    pub speed_over_ground_knots: Option<f32>,
    pub position_accuracy: bool,
    pub longitude_deg: Option<f64>,
    pub latitude_deg: Option<f64>,
    pub course_over_ground_deg: Option<f32>,
    pub true_heading_deg: Option<u16>,
    pub timestamp_second: Option<u8>,
    pub carrier_sense_unit: bool,
    pub display: bool,
    pub dsc: bool,
    pub band: bool,
    pub message_22: bool,
    pub assigned_mode: bool,
    pub raim: bool,
    pub radio_status: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "part")]
pub enum StaticDataReport {
    A {
        repeat_indicator: u8,
        mmsi: u32,
        vessel_name: String,
    },
    B {
        repeat_indicator: u8,
        mmsi: u32,
        ship_type: u8,
        vendor_id: String,
        unit_model_code: u8,
        serial_number: u32,
        call_sign: String,
        dimensions: Option<Dimensions>,
        mothership_mmsi: Option<u32>,
    },
}

impl StaticDataReport {
    pub fn mmsi(&self) -> u32 {
        match self {
            Self::A { mmsi, .. } | Self::B { mmsi, .. } => *mmsi,
        }
    }
}

/// Streaming HDLC decoder for already NRZI-decoded AIS data bits.
#[derive(Debug, Default)]
pub struct HdlcDecoder {
    in_frame: bool,
    bits: Vec<bool>,
}

impl HdlcDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push one chronological data bit. A closing flag can produce one result.
    /// Bad frames are returned as errors without losing synchronization.
    pub fn push_bit(&mut self, bit: bool) -> Option<Result<AisMessage, AisDecodeError>> {
        self.bits.push(bit);
        if self.bits.len() >= FLAG.len() && self.bits[self.bits.len() - FLAG.len()..] == FLAG {
            let result = if self.in_frame {
                let frame_len = self.bits.len() - FLAG.len();
                if frame_len == 0 {
                    None
                } else {
                    Some(decode_hdlc_frame(&self.bits[..frame_len]))
                }
            } else {
                None
            };
            self.in_frame = true;
            self.bits.clear();
            return result;
        }
        if self.bits.len() > MAX_FRAME_BITS {
            self.bits.clear();
            self.in_frame = false;
            return Some(Err(AisDecodeError::FrameTooLong));
        }
        None
    }

    pub fn push_bits<I: IntoIterator<Item = bool>>(
        &mut self,
        bits: I,
    ) -> Vec<Result<AisMessage, AisDecodeError>> {
        bits.into_iter()
            .filter_map(|bit| self.push_bit(bit))
            .collect()
    }

    pub fn reset(&mut self) {
        self.in_frame = false;
        self.bits.clear();
    }
}

/// Streaming NRZI line-level decoder followed by HDLC/AIS decoding.
#[derive(Debug, Default)]
pub struct NrziHdlcDecoder {
    previous_level: Option<bool>,
    hdlc: HdlcDecoder,
}

impl NrziHdlcDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a sliced NRZI line level. The first level establishes state and
    /// emits no data bit. Thereafter unchanged = 1 and transition = 0.
    pub fn push_level(&mut self, level: bool) -> Option<Result<AisMessage, AisDecodeError>> {
        let previous = self.previous_level.replace(level)?;
        self.hdlc.push_bit(level == previous)
    }

    pub fn push_levels<I: IntoIterator<Item = bool>>(
        &mut self,
        levels: I,
    ) -> Vec<Result<AisMessage, AisDecodeError>> {
        levels
            .into_iter()
            .filter_map(|level| self.push_level(level))
            .collect()
    }

    pub fn reset(&mut self) {
        self.previous_level = None;
        self.hdlc.reset();
    }
}

/// Minimal fixed-clock slicer for zero-centred discriminator samples.
///
/// `samples_per_symbol` may be fractional (for example 5.0 at 48 kHz/9600).
/// Samples are integrate-and-dump averaged per symbol and then interpreted as
/// NRZI line levels. This helper assumes symbol alignment and has no timing loop.
#[derive(Debug)]
pub struct DiscriminatorDecoder {
    samples_per_symbol: f64,
    phase: f64,
    sum: f64,
    count: usize,
    nrzi: NrziHdlcDecoder,
}

impl DiscriminatorDecoder {
    pub fn new(sample_rate_hz: f64) -> Result<Self, &'static str> {
        if !sample_rate_hz.is_finite() || sample_rate_hz < 9_600.0 {
            return Err("sample rate must be finite and at least 9600 Hz");
        }
        Ok(Self {
            samples_per_symbol: sample_rate_hz / 9_600.0,
            phase: 0.0,
            sum: 0.0,
            count: 0,
            nrzi: NrziHdlcDecoder::new(),
        })
    }

    pub fn push_sample(&mut self, sample: f32) -> Option<Result<AisMessage, AisDecodeError>> {
        self.sum += sample as f64;
        self.count += 1;
        self.phase += 1.0;
        if self.phase + 1e-9 < self.samples_per_symbol {
            return None;
        }
        self.phase -= self.samples_per_symbol;
        let level = self.sum >= 0.0;
        self.sum = 0.0;
        self.count = 0;
        self.nrzi.push_level(level)
    }

    pub fn push_samples(&mut self, samples: &[f32]) -> Vec<Result<AisMessage, AisDecodeError>> {
        samples
            .iter()
            .filter_map(|&sample| self.push_sample(sample))
            .collect()
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.sum = 0.0;
        self.count = 0;
        self.nrzi.reset();
    }
}

/// CRC-16/X-25 used for the AIS HDLC frame check sequence.
pub fn crc16_x25(bytes: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
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
    !crc
}

/// Decode one NMEA 0183 `!AIVDM`/`!AIVDO` six-bit payload (without sentence
/// framing). `fill_bits` is the final sentence field and must be 0..=5.
pub fn decode_sixbit_payload(payload: &str, fill_bits: u8) -> Result<AisMessage, AisDecodeError> {
    if fill_bits > 5 {
        return Err(AisDecodeError::InvalidFillBits(fill_bits));
    }
    let mut bits = Vec::with_capacity(payload.len() * 6);
    for ch in payload.chars() {
        let code = ch as u32;
        if !((48..=87).contains(&code) || (96..=119).contains(&code)) {
            return Err(AisDecodeError::InvalidSixBitCharacter(ch));
        }
        let mut value = (code - 48) as u8;
        if value > 40 {
            value = value.saturating_sub(8);
        }
        if value > 63 {
            return Err(AisDecodeError::InvalidSixBitCharacter(ch));
        }
        for shift in (0..6).rev() {
            bits.push((value >> shift) & 1 != 0);
        }
    }
    if fill_bits as usize > bits.len() {
        return Err(AisDecodeError::InvalidFillBits(fill_bits));
    }
    bits.truncate(bits.len() - fill_bits as usize);
    decode_payload_bits(&bits)
}

/// Decode AIS application bits in specification order (MSB first).
pub fn decode_payload_bits(bits: &[bool]) -> Result<AisMessage, AisDecodeError> {
    if bits.len() < 6 {
        return Err(AisDecodeError::PayloadTooShort {
            message_type: 0,
            needed: 6,
            actual: bits.len(),
        });
    }
    let message_type = get_u(bits, 0, 6) as u8;
    match message_type {
        1..=3 => parse_class_a(bits, message_type).map(AisMessage::PositionClassA),
        5 => parse_type_5(bits).map(AisMessage::StaticVoyage),
        18 => parse_type_18(bits).map(AisMessage::PositionClassB),
        24 => parse_type_24(bits).map(AisMessage::StaticDataReport),
        other => Err(AisDecodeError::UnsupportedMessageType(other)),
    }
}

fn decode_hdlc_frame(stuffed: &[bool]) -> Result<AisMessage, AisDecodeError> {
    let bits = unstuff(stuffed)?;
    if bits.len() < 24 {
        return Err(AisDecodeError::FrameTooShort);
    }
    if bits.len() % 8 != 0 {
        return Err(AisDecodeError::NonOctetFrame(bits.len()));
    }
    let mut frame = Vec::with_capacity(bits.len() / 8);
    for chunk in bits.chunks_exact(8) {
        let mut byte = 0u8;
        for (i, &bit) in chunk.iter().enumerate() {
            if bit {
                byte |= 1 << i;
            }
        }
        frame.push(byte);
    }
    let data_len = frame.len() - 2;
    let expected = u16::from_le_bytes([frame[data_len], frame[data_len + 1]]);
    let calculated = crc16_x25(&frame[..data_len]);
    if expected != calculated {
        return Err(AisDecodeError::CrcMismatch {
            expected,
            calculated,
        });
    }

    // HDLC transmits each octet least-significant bit first, while AIS field
    // diagrams and NMEA six-bit payloads are most-significant bit first.
    let mut app_bits = Vec::with_capacity(data_len * 8);
    for &byte in &frame[..data_len] {
        for shift in 0..8 {
            app_bits.push((byte >> shift) & 1 != 0);
        }
    }
    decode_payload_bits(&app_bits)
}

fn unstuff(bits: &[bool]) -> Result<Vec<bool>, AisDecodeError> {
    let mut out = Vec::with_capacity(bits.len());
    let mut ones = 0u8;
    let mut i = 0usize;
    while i < bits.len() {
        let bit = bits[i];
        out.push(bit);
        i += 1;
        if bit {
            ones += 1;
            if ones == 5 {
                if i >= bits.len() || bits[i] {
                    return Err(AisDecodeError::InvalidBitStuffing);
                }
                i += 1; // discard stuffed zero
                ones = 0;
            }
        } else {
            ones = 0;
        }
    }
    Ok(out)
}

fn require(bits: &[bool], message_type: u8, needed: usize) -> Result<(), AisDecodeError> {
    if bits.len() < needed {
        Err(AisDecodeError::PayloadTooShort {
            message_type,
            needed,
            actual: bits.len(),
        })
    } else {
        Ok(())
    }
}

fn get_u(bits: &[bool], start: usize, width: usize) -> u64 {
    bits[start..start + width]
        .iter()
        .fold(0, |v, &b| (v << 1) | b as u64)
}

fn get_i(bits: &[bool], start: usize, width: usize) -> i64 {
    let value = get_u(bits, start, width) as i64;
    let sign = 1i64 << (width - 1);
    if value & sign != 0 {
        value - (1i64 << width)
    } else {
        value
    }
}

fn text(bits: &[bool], start: usize, width: usize) -> String {
    const TABLE: &[u8; 64] = b"@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_ !\"#$%&'()*+,-./0123456789:;<=>?";
    let mut value = String::new();
    for offset in (0..width).step_by(6) {
        value.push(TABLE[get_u(bits, start + offset, 6) as usize] as char);
    }
    value.trim_end_matches(['@', ' ']).to_string()
}

fn longitude(raw: i64) -> Option<f64> {
    if raw.abs() >= 181 * 600_000 {
        None
    } else {
        Some(raw as f64 / 600_000.0)
    }
}
fn latitude(raw: i64) -> Option<f64> {
    if raw.abs() >= 91 * 600_000 {
        None
    } else {
        Some(raw as f64 / 600_000.0)
    }
}
fn sog(raw: u64) -> Option<f32> {
    if raw == 1023 {
        None
    } else {
        Some(raw as f32 / 10.0)
    }
}
fn cog(raw: u64) -> Option<f32> {
    if raw >= 3600 {
        None
    } else {
        Some(raw as f32 / 10.0)
    }
}
fn heading(raw: u64) -> Option<u16> {
    if raw == 511 {
        None
    } else {
        Some(raw as u16)
    }
}
fn timestamp(raw: u64) -> Option<u8> {
    if raw >= 60 {
        None
    } else {
        Some(raw as u8)
    }
}

fn parse_class_a(bits: &[bool], message_type: u8) -> Result<PositionReportClassA, AisDecodeError> {
    require(bits, message_type, 168)?;
    let rot = get_i(bits, 42, 8) as i8;
    let rot_value = if rot == -128 {
        None
    } else {
        let x = rot as f32 / 4.733;
        Some(x.signum() * x * x)
    };
    Ok(PositionReportClassA {
        message_type,
        repeat_indicator: get_u(bits, 6, 2) as u8,
        mmsi: get_u(bits, 8, 30) as u32,
        navigation_status: get_u(bits, 38, 4) as u8,
        rate_of_turn_raw: rot,
        rate_of_turn_deg_per_min: rot_value,
        speed_over_ground_knots: sog(get_u(bits, 50, 10)),
        position_accuracy: bits[60],
        longitude_deg: longitude(get_i(bits, 61, 28)),
        latitude_deg: latitude(get_i(bits, 89, 27)),
        course_over_ground_deg: cog(get_u(bits, 116, 12)),
        true_heading_deg: heading(get_u(bits, 128, 9)),
        timestamp_second: timestamp(get_u(bits, 137, 6)),
        maneuver_indicator: get_u(bits, 143, 2) as u8,
        raim: bits[148],
        radio_status: get_u(bits, 149, 19) as u32,
    })
}

fn parse_type_5(bits: &[bool]) -> Result<StaticVoyageData, AisDecodeError> {
    require(bits, 5, 424)?;
    let month = get_u(bits, 274, 4) as u8;
    let day = get_u(bits, 278, 5) as u8;
    let hour = get_u(bits, 283, 5) as u8;
    let minute = get_u(bits, 288, 6) as u8;
    let eta = if month == 0 || day == 0 || hour >= 24 || minute >= 60 {
        None
    } else {
        Some(Eta {
            month,
            day,
            hour,
            minute,
        })
    };
    let draught = get_u(bits, 294, 8) as u8;
    let imo = get_u(bits, 40, 30) as u32;
    Ok(StaticVoyageData {
        repeat_indicator: get_u(bits, 6, 2) as u8,
        mmsi: get_u(bits, 8, 30) as u32,
        ais_version: get_u(bits, 38, 2) as u8,
        imo_number: (imo != 0).then_some(imo),
        call_sign: text(bits, 70, 42),
        vessel_name: text(bits, 112, 120),
        ship_type: get_u(bits, 232, 8) as u8,
        dimensions: Dimensions {
            to_bow_m: get_u(bits, 240, 9) as u16,
            to_stern_m: get_u(bits, 249, 9) as u16,
            to_port_m: get_u(bits, 258, 6) as u8,
            to_starboard_m: get_u(bits, 264, 6) as u8,
        },
        position_fix_type: get_u(bits, 270, 4) as u8,
        eta,
        draught_m: (draught != 0).then_some(draught as f32 / 10.0),
        destination: text(bits, 302, 120),
        data_terminal_ready: !bits[422], // transmitted DTE bit is 0 when ready
    })
}

fn parse_type_18(bits: &[bool]) -> Result<PositionReportClassB, AisDecodeError> {
    require(bits, 18, 168)?;
    Ok(PositionReportClassB {
        repeat_indicator: get_u(bits, 6, 2) as u8,
        mmsi: get_u(bits, 8, 30) as u32,
        speed_over_ground_knots: sog(get_u(bits, 46, 10)),
        position_accuracy: bits[56],
        longitude_deg: longitude(get_i(bits, 57, 28)),
        latitude_deg: latitude(get_i(bits, 85, 27)),
        course_over_ground_deg: cog(get_u(bits, 112, 12)),
        true_heading_deg: heading(get_u(bits, 124, 9)),
        timestamp_second: timestamp(get_u(bits, 133, 6)),
        carrier_sense_unit: bits[141],
        display: bits[142],
        dsc: bits[143],
        band: bits[144],
        message_22: bits[145],
        assigned_mode: bits[146],
        raim: bits[147],
        radio_status: get_u(bits, 148, 20) as u32,
    })
}

fn parse_type_24(bits: &[bool]) -> Result<StaticDataReport, AisDecodeError> {
    require(bits, 24, 40)?;
    let repeat = get_u(bits, 6, 2) as u8;
    let mmsi = get_u(bits, 8, 30) as u32;
    match get_u(bits, 38, 2) {
        0 => {
            require(bits, 24, 160)?;
            Ok(StaticDataReport::A {
                repeat_indicator: repeat,
                mmsi,
                vessel_name: text(bits, 40, 120),
            })
        }
        1 => {
            require(bits, 24, 168)?;
            let auxiliary = mmsi / 10_000_000 == 98;
            let dimensions = (!auxiliary).then(|| Dimensions {
                to_bow_m: get_u(bits, 132, 9) as u16,
                to_stern_m: get_u(bits, 141, 9) as u16,
                to_port_m: get_u(bits, 150, 6) as u8,
                to_starboard_m: get_u(bits, 156, 6) as u8,
            });
            Ok(StaticDataReport::B {
                repeat_indicator: repeat,
                mmsi,
                ship_type: get_u(bits, 40, 8) as u8,
                vendor_id: text(bits, 48, 18),
                unit_model_code: get_u(bits, 66, 4) as u8,
                serial_number: get_u(bits, 70, 20) as u32,
                call_sign: text(bits, 90, 42),
                dimensions,
                mothership_mmsi: auxiliary.then(|| get_u(bits, 132, 30) as u32),
            })
        }
        _ => Err(AisDecodeError::UnsupportedMessageType(24)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_u(bits: &mut [bool], start: usize, width: usize, value: u64) {
        for i in 0..width {
            bits[start + i] = value & (1 << (width - 1 - i)) != 0;
        }
    }
    fn set_i(bits: &mut [bool], start: usize, width: usize, value: i64) {
        set_u(bits, start, width, (value as u64) & ((1u64 << width) - 1));
    }
    fn char_value(c: char) -> u8 {
        const TABLE: &str = "@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_ !\"#$%&'()*+,-./0123456789:;<=>?";
        TABLE.chars().position(|v| v == c).unwrap() as u8
    }
    fn set_text(bits: &mut [bool], start: usize, width: usize, value: &str) {
        let chars = value.chars().chain(std::iter::repeat('@'));
        for (i, c) in chars.take(width / 6).enumerate() {
            set_u(bits, start + i * 6, 6, char_value(c) as u64);
        }
    }
    fn stuff(bits: &[bool]) -> Vec<bool> {
        let mut out = Vec::new();
        let mut ones = 0;
        for &bit in bits {
            out.push(bit);
            if bit {
                ones += 1;
                if ones == 5 {
                    out.push(false);
                    ones = 0;
                }
            } else {
                ones = 0;
            }
        }
        out
    }
    fn framed(app_bits: &[bool]) -> Vec<bool> {
        assert_eq!(app_bits.len() % 8, 0);
        // Chronological AIS field bits become HDLC octet bits 0..7.
        let mut bytes = Vec::new();
        for chunk in app_bits.chunks_exact(8) {
            let mut b = 0;
            for (i, bit) in chunk.iter().enumerate() {
                if *bit {
                    b |= 1 << i;
                }
            }
            bytes.push(b);
        }
        let crc = crc16_x25(&bytes);
        bytes.extend_from_slice(&crc.to_le_bytes());
        let mut wire = Vec::new();
        wire.extend(FLAG);
        let raw: Vec<bool> = bytes
            .iter()
            .flat_map(|b| (0..8).map(move |i| b & (1 << i) != 0))
            .collect();
        wire.extend(stuff(&raw));
        wire.extend(FLAG);
        wire
    }
    fn framed_with_bad_crc(app_bits: &[bool]) -> Vec<bool> {
        assert_eq!(app_bits.len() % 8, 0);
        let mut bytes = Vec::new();
        for chunk in app_bits.chunks_exact(8) {
            let mut byte = 0;
            for (i, bit) in chunk.iter().enumerate() {
                if *bit {
                    byte |= 1 << i;
                }
            }
            bytes.push(byte);
        }
        let bad_crc = crc16_x25(&bytes) ^ 1;
        bytes.extend_from_slice(&bad_crc.to_le_bytes());
        let raw: Vec<bool> = bytes
            .iter()
            .flat_map(|b| (0..8).map(move |i| b & (1 << i) != 0))
            .collect();
        let mut wire = Vec::new();
        wire.extend(FLAG);
        wire.extend(stuff(&raw));
        wire.extend(FLAG);
        wire
    }
    fn nrzi_levels(data: &[bool], initial: bool) -> Vec<bool> {
        let mut level = initial;
        let mut levels = vec![level];
        for &bit in data {
            if !bit {
                level = !level;
            }
            levels.push(level);
        }
        levels
    }

    fn type1_fixture() -> Vec<bool> {
        let mut b = vec![false; 168];
        set_u(&mut b, 0, 6, 1);
        set_u(&mut b, 6, 2, 0);
        set_u(&mut b, 8, 30, 366_123_456);
        set_u(&mut b, 38, 4, 5);
        set_i(&mut b, 42, 8, 10);
        set_u(&mut b, 50, 10, 123);
        b[60] = true;
        set_i(&mut b, 61, 28, (-70.25f64 * 600_000.0) as i64);
        set_i(&mut b, 89, 27, (41.5f64 * 600_000.0) as i64);
        set_u(&mut b, 116, 12, 876);
        set_u(&mut b, 128, 9, 90);
        set_u(&mut b, 137, 6, 37);
        set_u(&mut b, 143, 2, 1);
        b[148] = true;
        set_u(&mut b, 149, 19, 0x12345);
        b
    }

    #[test]
    fn streaming_hdlc_decodes_type_1_with_crc_and_stuffing() {
        let wire = framed(&type1_fixture());
        let mut decoder = HdlcDecoder::new();
        let output = decoder.push_bits(wire);
        assert_eq!(output.len(), 1);
        let AisMessage::PositionClassA(msg) = output.into_iter().next().unwrap().unwrap() else {
            panic!()
        };
        assert_eq!(msg.message_type, 1);
        assert_eq!(msg.mmsi, 366_123_456);
        assert_eq!(msg.navigation_status, 5);
        assert_eq!(msg.speed_over_ground_knots, Some(12.3));
        assert!((msg.longitude_deg.unwrap() + 70.25).abs() < 1e-6);
        assert!((msg.latitude_deg.unwrap() - 41.5).abs() < 1e-6);
        assert_eq!(msg.course_over_ground_deg, Some(87.6));
        assert_eq!(msg.true_heading_deg, Some(90));
        assert!(msg.raim);
    }

    #[test]
    fn routes_all_class_a_position_message_types() {
        for message_type in 1..=3 {
            let mut bits = type1_fixture();
            set_u(&mut bits, 0, 6, message_type);
            let message = decode_payload_bits(&bits).unwrap();
            assert_eq!(message.message_type(), message_type as u8);
            assert_eq!(message.mmsi(), 366_123_456);
        }
    }

    #[test]
    fn nrzi_stream_handles_arbitrary_chunks() {
        let levels = nrzi_levels(&framed(&type1_fixture()), false);
        let mut decoder = NrziHdlcDecoder::new();
        let mut output = Vec::new();
        for chunk in levels.chunks(7) {
            output.extend(decoder.push_levels(chunk.iter().copied()));
        }
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].as_ref().unwrap().mmsi(), 366_123_456);
    }

    #[test]
    fn fixed_clock_discriminator_helper_decodes() {
        let levels = nrzi_levels(&framed(&type1_fixture()), true);
        let samples: Vec<f32> = levels
            .iter()
            .flat_map(|&v| std::iter::repeat_n(if v { 0.8 } else { -0.8 }, 5))
            .collect();
        let mut decoder = DiscriminatorDecoder::new(48_000.0).unwrap();
        let output = decoder.push_samples(&samples);
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].as_ref().unwrap().message_type(), 1);
    }

    #[test]
    fn crc_error_is_reported_and_next_frame_recovers() {
        let mut bad = framed_with_bad_crc(&type1_fixture());
        let good = framed(&type1_fixture());
        bad.extend(good.iter().skip(8)); // shared closing/opening flag
        let mut decoder = HdlcDecoder::new();
        let output = decoder.push_bits(bad);
        assert!(matches!(output[0], Err(AisDecodeError::CrcMismatch { .. })));
        assert!(output
            .iter()
            .any(|v| v.as_ref().is_ok_and(|m| m.mmsi() == 366_123_456)));
    }

    #[test]
    fn parses_type_5_static_voyage() {
        let mut b = vec![false; 424];
        set_u(&mut b, 0, 6, 5);
        set_u(&mut b, 8, 30, 211_234_567);
        set_u(&mut b, 38, 2, 1);
        set_u(&mut b, 40, 30, 9_123_456);
        set_text(&mut b, 70, 42, "CALL123");
        set_text(&mut b, 112, 120, "RUSTY MARINER");
        set_u(&mut b, 232, 8, 70);
        set_u(&mut b, 240, 9, 25);
        set_u(&mut b, 249, 9, 5);
        set_u(&mut b, 258, 6, 4);
        set_u(&mut b, 264, 6, 3);
        set_u(&mut b, 270, 4, 1);
        set_u(&mut b, 274, 4, 7);
        set_u(&mut b, 278, 5, 16);
        set_u(&mut b, 283, 5, 12);
        set_u(&mut b, 288, 6, 30);
        set_u(&mut b, 294, 8, 45);
        set_text(&mut b, 302, 120, "BOSTON");
        let AisMessage::StaticVoyage(v) = decode_payload_bits(&b).unwrap() else {
            panic!()
        };
        assert_eq!(v.vessel_name, "RUSTY MARINER");
        assert_eq!(v.call_sign, "CALL123");
        assert_eq!(v.destination, "BOSTON");
        assert_eq!(v.draught_m, Some(4.5));
        assert_eq!(
            v.eta,
            Some(Eta {
                month: 7,
                day: 16,
                hour: 12,
                minute: 30
            })
        );
        assert!(v.data_terminal_ready);
    }

    #[test]
    fn parses_type_18_and_type_24_parts() {
        let mut b18 = vec![false; 168];
        set_u(&mut b18, 0, 6, 18);
        set_u(&mut b18, 8, 30, 338_765_432);
        set_u(&mut b18, 46, 10, 77);
        set_i(&mut b18, 57, 28, (12.5 * 600_000.0) as i64);
        set_i(&mut b18, 85, 27, (-33.25 * 600_000.0) as i64);
        set_u(&mut b18, 112, 12, 1234);
        set_u(&mut b18, 124, 9, 124);
        b18[147] = true;
        let AisMessage::PositionClassB(v) = decode_payload_bits(&b18).unwrap() else {
            panic!()
        };
        assert_eq!(v.mmsi, 338_765_432);
        assert_eq!(v.speed_over_ground_knots, Some(7.7));
        assert!(v.raim);

        let mut a = vec![false; 160];
        set_u(&mut a, 0, 6, 24);
        set_u(&mut a, 8, 30, 244_123_456);
        set_u(&mut a, 38, 2, 0);
        set_text(&mut a, 40, 120, "FERRIS");
        assert!(
            matches!(decode_payload_bits(&a).unwrap(), AisMessage::StaticDataReport(StaticDataReport::A { vessel_name, .. }) if vessel_name == "FERRIS")
        );

        let mut b = vec![false; 168];
        set_u(&mut b, 0, 6, 24);
        set_u(&mut b, 8, 30, 244_123_456);
        set_u(&mut b, 38, 2, 1);
        set_u(&mut b, 40, 8, 36);
        set_text(&mut b, 48, 18, "RS1");
        set_u(&mut b, 66, 4, 3);
        set_u(&mut b, 70, 20, 42);
        set_text(&mut b, 90, 42, "FERRIS1");
        set_u(&mut b, 132, 9, 10);
        set_u(&mut b, 141, 9, 2);
        assert!(matches!(
            decode_payload_bits(&b).unwrap(),
            AisMessage::StaticDataReport(StaticDataReport::B {
                serial_number: 42,
                dimensions: Some(Dimensions { to_bow_m: 10, .. }),
                ..
            })
        ));
    }

    #[test]
    fn decodes_known_nmea_sixbit_position_payload() {
        // Publicly documented style of AIVDM payload; this assertion verifies
        // six-bit armoring and message routing, not reception from live RF.
        let msg = decode_sixbit_payload("15N:@`0P00PD;88MD5MTDww@2D0l", 0).unwrap();
        assert_eq!(msg.message_type(), 1);
    }
}
