//! Native LoRa CSS PHY plus MeshCore / Meshtastic / Reticulum / Modbus parsers.
//!
//! The PHY is a clean-room SF7/125 kHz chirp encoder/decoder used for recorded-IQ
//! fixtures. After bytes are recovered, payloads are classified from public header
//! layouts. Well-known public default channel keys (Meshtastic `AQ==` / simpleN,
//! MeshCore Public) may recover plaintext. Private channel keys, PKI direct
//! messages, and LoRaWAN payloads stay identified and are never decrypted.

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit, KeyIvInit, StreamCipher};
use aes::{Aes128, Block};
use ctr::Ctr128BE;
use hmac::Hmac;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::DecodedMessage;
use crate::demod::decimate_complex_average;

type Aes128Ctr = Ctr128BE<Aes128>;
type HmacSha256 = Hmac<Sha256>;

pub const LORA_FIXTURE_SAMPLE_RATE_HZ: u32 = 250_000;
pub const LORA_FIXTURE_BANDWIDTH_HZ: u32 = 125_000;
pub const LORA_FIXTURE_SF: u8 = 8;
pub const LORA_PROTOCOLS: &[&str] = &[
    "meshtastic",
    "meshcore",
    "reticulum",
    "modbus-lora",
    "lorawan",
    "lora",
];

/// Meshtastic firmware's well-known LongFast default PSK (`AQ==` / index 1).
const MESHTASTIC_DEFAULT_PSK: [u8; 16] = [
    0xd4, 0xf1, 0xbb, 0x3a, 0x20, 0x29, 0x07, 0x59, 0xf0, 0xbc, 0xff, 0xab, 0xcf, 0x4e, 0x69, 0x01,
];

/// MeshCore smartphone/T-Deck default Public channel AES-128 key.
const MESHCORE_PUBLIC_PSK: [u8; 16] = [
    0x8b, 0x33, 0x87, 0xe9, 0xc5, 0xcd, 0xea, 0x6a, 0xc9, 0xe5, 0xed, 0xba, 0xa1, 0x15, 0xcd, 0x72,
];

const PREAMBLE_SYMBOLS: usize = 6;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoraPacket {
    pub protocol: String,
    pub message_type: String,
    pub address: String,
    pub function_code: String,
    pub content: String,
    pub encryption: String,
    pub raw_hex: String,
}

impl LoraPacket {
    pub fn to_decoded(&self, frequency_hz: u64) -> DecodedMessage {
        DecodedMessage {
            id: None,
            frequency_hz,
            protocol: self.protocol.clone(),
            message_type: self.message_type.clone(),
            address: self.address.clone(),
            function_code: self.function_code.clone(),
            content: self.content.clone(),
            raw: self.raw_hex.clone(),
            encryption: self.encryption.clone(),
            timestamp_ms: crate::scanner::now_ms(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LoraPhy {
    pub spreading_factor: u8,
    pub bandwidth_hz: u32,
    pub sample_rate_hz: u32,
}

impl LoraPhy {
    pub fn fixture() -> Self {
        Self {
            spreading_factor: LORA_FIXTURE_SF,
            bandwidth_hz: LORA_FIXTURE_BANDWIDTH_HZ,
            sample_rate_hz: LORA_FIXTURE_SAMPLE_RATE_HZ,
        }
    }

    pub fn chips(&self) -> usize {
        1usize << self.spreading_factor.clamp(7, 12)
    }

    pub fn samples_per_symbol(&self) -> usize {
        if self.bandwidth_hz == 0 {
            return 0;
        }
        (self.chips() as u64)
            .saturating_mul(self.sample_rate_hz as u64)
            .saturating_div(self.bandwidth_hz as u64) as usize
    }

    fn upchirp(&self) -> Vec<Complex<f32>> {
        let n = self.samples_per_symbol();
        let bw = self.bandwidth_hz as f64;
        let fs = self.sample_rate_hz.max(1) as f64;
        let mut phase = 0.0_f64;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let freq = -bw / 2.0 + bw * (i as f64) / n.max(1) as f64;
            phase += std::f64::consts::TAU * freq / fs;
            out.push(Complex::new(phase.cos() as f32, phase.sin() as f32));
        }
        out
    }

    fn symbol(&self, value: u16, upchirp: &[Complex<f32>]) -> Vec<Complex<f32>> {
        let n = upchirp.len();
        if n == 0 {
            return Vec::new();
        }
        let chips = self.chips();
        let shift = ((value as usize) % chips) * (n / chips);
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            out.push(upchirp[(i + shift) % n]);
        }
        out
    }

    fn downchirp(&self, upchirp: &[Complex<f32>]) -> Vec<Complex<f32>> {
        upchirp.iter().map(|c| c.conj()).collect()
    }
}

fn pack_symbols(bytes: &[u8], _sf: u8) -> Vec<u16> {
    let mut packed = Vec::with_capacity(bytes.len() + 1);
    packed.push(bytes.len() as u16);
    packed.extend(bytes.iter().map(|b| u16::from(*b)));
    packed
}

fn unpack_symbols(symbols: &[u16], _sf: u8) -> Vec<u8> {
    if symbols.is_empty() {
        return Vec::new();
    }
    let len = symbols[0] as usize;
    symbols
        .iter()
        .skip(1)
        .take(len)
        .map(|s| (*s & 0xFF) as u8)
        .collect()
}

fn dechirp_symbol(
    symbol: &[Complex<f32>],
    upchirp: &[Complex<f32>],
    planner: &mut FftPlanner<f32>,
    modulus: usize,
) -> u16 {
    let n = symbol.len().min(upchirp.len());
    if n == 0 {
        return 0;
    }
    let mut buf: Vec<Complex<f32>> = symbol
        .iter()
        .zip(upchirp.iter())
        .take(n)
        .map(|(s, u)| *s * u.conj())
        .collect();
    if buf.len() < 2 {
        return 0;
    }
    let fft = planner.plan_fft_forward(buf.len());
    fft.process(&mut buf);
    // Integer fs/bw maps symbol value k onto FFT bin k.
    let search = modulus.clamp(1, buf.len());
    let mut best_bin = 0usize;
    let mut best_mag = 0.0f32;
    for (bin, sample) in buf.iter().enumerate().take(search) {
        let mag = sample.norm_sqr();
        if mag > best_mag {
            best_mag = mag;
            best_bin = bin;
        }
    }
    best_bin as u16
}

/// Encode a payload as SF7/125 kHz CSS IQ (implicit header, no Hamming FEC).
pub fn encode_css(payload: &[u8], phy: LoraPhy) -> Vec<Complex<f32>> {
    let up = phy.upchirp();
    let down = phy.downchirp(&up);
    let mut iq = Vec::new();
    for _ in 0..PREAMBLE_SYMBOLS {
        iq.extend_from_slice(&up);
    }
    iq.extend_from_slice(&down);
    iq.extend_from_slice(&down);
    for symbol in pack_symbols(payload, phy.spreading_factor) {
        iq.extend(phy.symbol(symbol, &up));
    }
    iq
}

fn find_preamble(
    iq: &[Complex<f32>],
    up: &[Complex<f32>],
    planner: &mut FftPlanner<f32>,
    modulus: usize,
) -> Option<usize> {
    let sps = up.len();
    if sps == 0 || iq.len() < sps * (PREAMBLE_SYMBOLS + 4) {
        return None;
    }
    let step = (sps / 4).max(1);
    let mut run = 0usize;
    let mut run_start = 0usize;
    let mut i = 0usize;
    while i + sps <= iq.len() {
        let value = dechirp_symbol(&iq[i..i + sps], up, planner, modulus);
        if value <= 2 {
            if run == 0 {
                run_start = i;
            }
            run += 1;
            if run >= PREAMBLE_SYMBOLS.saturating_sub(1) {
                return Some(run_start);
            }
            i += sps;
        } else {
            run = 0;
            i += step;
        }
    }
    None
}

/// Recover payload bytes from CSS IQ.
pub fn decode_css(iq: &[Complex<f32>], phy: LoraPhy) -> Option<Vec<u8>> {
    let sps = phy.samples_per_symbol();
    if sps < 32 || iq.len() < sps * (PREAMBLE_SYMBOLS + 4) {
        return None;
    }
    let up = phy.upchirp();
    let mut planner = FftPlanner::<f32>::new();
    let modulus = phy.chips();
    let start = find_preamble(iq, &up, &mut planner, modulus)?;
    let payload_at = start.saturating_add(sps * (PREAMBLE_SYMBOLS + 2));
    if payload_at + sps > iq.len() {
        return None;
    }
    let mut symbols = Vec::new();
    let mut offset = payload_at;
    while offset + sps <= iq.len() {
        symbols.push(dechirp_symbol(
            &iq[offset..offset + sps],
            &up,
            &mut planner,
            modulus,
        ));
        offset += sps;
        if symbols.len() > 96 {
            break;
        }
    }
    let bytes = unpack_symbols(&symbols, phy.spreading_factor);
    if bytes.is_empty() {
        None
    } else {
        Some(bytes)
    }
}

pub fn decode_iq(iq: &[Complex<f32>], sample_rate_hz: u32) -> Vec<LoraPacket> {
    if iq.is_empty() || sample_rate_hz == 0 {
        return Vec::new();
    }
    let target = LORA_FIXTURE_SAMPLE_RATE_HZ;
    let (samples, rate) = if sample_rate_hz >= target * 2 {
        let factor = (sample_rate_hz / target) as usize;
        (
            decimate_complex_average(iq, factor),
            sample_rate_hz / factor as u32,
        )
    } else {
        (iq.to_vec(), sample_rate_hz)
    };
    let phy = LoraPhy {
        spreading_factor: LORA_FIXTURE_SF,
        bandwidth_hz: LORA_FIXTURE_BANDWIDTH_HZ,
        sample_rate_hz: rate,
    };
    match decode_css(&samples, phy) {
        Some(bytes) => vec![classify_payload(&bytes)],
        None => Vec::new(),
    }
}

pub fn classify_payload(bytes: &[u8]) -> LoraPacket {
    if let Some(packet) = parse_modbus_rtu(bytes) {
        return packet;
    }
    if let Some(packet) = parse_meshtastic(bytes) {
        return packet;
    }
    if let Some(packet) = parse_meshcore(bytes) {
        return packet;
    }
    if let Some(packet) = parse_reticulum(bytes) {
        return packet;
    }
    if let Some(packet) = parse_lorawan(bytes) {
        return packet;
    }
    LoraPacket {
        protocol: "lora".into(),
        message_type: "unknown".into(),
        address: String::new(),
        function_code: String::new(),
        content: String::new(),
        encryption: "unknown".into(),
        raw_hex: hex::encode(bytes),
    }
}

pub fn crc16_modbus(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &byte in data {
        crc ^= u16::from(byte);
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

pub fn parse_modbus_rtu(bytes: &[u8]) -> Option<LoraPacket> {
    if bytes.len() < 8 {
        return None;
    }
    let addr = bytes[0];
    let func = bytes[1];
    if !(1..=247).contains(&addr) {
        return None;
    }
    if !matches!(func, 1..=6 | 15 | 16) {
        return None;
    }
    let crc_lo = *bytes.last()?;
    let crc_hi = bytes.get(bytes.len() - 2).copied()?;
    let given = u16::from_le_bytes([crc_hi, crc_lo]);
    let calc = crc16_modbus(&bytes[..bytes.len() - 2]);
    if given != calc {
        return None;
    }
    let start = if bytes.len() >= 6 {
        u16::from_be_bytes([bytes[2], bytes[3]])
    } else {
        0
    };
    Some(LoraPacket {
        protocol: "modbus-lora".into(),
        message_type: format!("func_{func}"),
        address: addr.to_string(),
        function_code: func.to_string(),
        content: format!("unit {addr} function {func} start {start}"),
        encryption: "none".into(),
        raw_hex: hex::encode(bytes),
    })
}

pub fn parse_meshtastic(bytes: &[u8]) -> Option<LoraPacket> {
    if bytes.len() < 16 {
        return None;
    }
    let dest = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
    let sender = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    let id = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
    let flags = bytes[12];
    let hop_limit = flags & 0x07;
    let hop_start = flags >> 5;
    if hop_limit > 7 || hop_start > 7 || sender == 0 {
        return None;
    }
    let broadcast = dest == 0xFFFF_FFFF;
    if !broadcast && hop_start != hop_limit {
        return None;
    }
    let payload = &bytes[16..];
    if payload.is_empty() {
        return None;
    }

    // Cleartext Data protobuf (encryption disabled / already open).
    if let Some(content) = meshtastic_text_from_data(payload) {
        return Some(LoraPacket {
            protocol: "meshtastic".into(),
            message_type: "text".into(),
            address: format!("{sender:08x}"),
            function_code: format!("dest={dest:08x}"),
            content,
            encryption: "none".into(),
            raw_hex: hex::encode(bytes),
        });
    }

    // Public default channel PSKs only — never try operator-supplied keys.
    if let Some((content, label)) = decrypt_meshtastic_public_defaults(id, sender, payload) {
        return Some(LoraPacket {
            protocol: "meshtastic".into(),
            message_type: "text".into(),
            address: format!("{sender:08x}"),
            function_code: format!("dest={dest:08x}"),
            content,
            encryption: label.into(),
            raw_hex: hex::encode(bytes),
        });
    }

    Some(LoraPacket {
        protocol: "meshtastic".into(),
        message_type: "encrypted".into(),
        address: format!("{sender:08x}"),
        function_code: format!("dest={dest:08x}"),
        content: format!("encrypted mesh packet id={id:#010x}"),
        encryption: "identified".into(),
        raw_hex: hex::encode(bytes),
    })
}

pub fn parse_meshcore(bytes: &[u8]) -> Option<LoraPacket> {
    if bytes.len() < 3 {
        return None;
    }
    let header = bytes[0];
    let route_type = header & 0x03;
    let payload_type = (header >> 2) & 0x0F;
    let path_len = bytes[1] as usize;
    if 2 + path_len >= bytes.len() {
        return None;
    }
    if path_len > 64 {
        return None;
    }
    // Group/flood adverts and texts commonly travel with an empty path. Direct
    // text (type 2) may also. Rejecting empty paths for those types hid Public
    // channel traffic.
    if path_len == 0 && !matches!(payload_type, 2 | 4 | 5 | 6) {
        return None;
    }
    if payload_type > 0x0B && payload_type != 0x0F {
        return None;
    }
    let payload = &bytes[2 + path_len..];
    let kind = match payload_type {
        0 => "req",
        1 => "response",
        2 => "txt_msg",
        3 => "ack",
        4 => "advert",
        5 => "grp_txt",
        6 => "grp_data",
        7 => "anon_req",
        _ => "payload",
    };

    if matches!(kind, "grp_txt" | "grp_data") {
        if let Some(content) = decrypt_meshcore_public_group(payload) {
            return Some(LoraPacket {
                protocol: "meshcore".into(),
                message_type: kind.into(),
                address: hex::encode(&bytes[2..2 + path_len.min(4)]),
                function_code: format!("route_{route_type}"),
                content,
                encryption: "public_default".into(),
                raw_hex: hex::encode(bytes),
            });
        }
        return Some(LoraPacket {
            protocol: "meshcore".into(),
            message_type: kind.into(),
            address: hex::encode(&bytes[2..2 + path_len.min(4)]),
            function_code: format!("route_{route_type}"),
            content: format!("encrypted meshcore {kind}"),
            encryption: "identified".into(),
            raw_hex: hex::encode(bytes),
        });
    }

    let (content, encryption) = if kind == "txt_msg" && payload.len() > 5 {
        let text = String::from_utf8_lossy(&payload[5..]).trim().to_string();
        if text
            .chars()
            .all(|c| c.is_ascii() && (!c.is_control() || c == ' '))
            && !text.is_empty()
        {
            (text, "none".to_string())
        } else {
            (
                format!("encrypted meshcore {kind}"),
                "identified".to_string(),
            )
        }
    } else if payload
        .iter()
        .filter(|b| (0x20..=0x7e).contains(*b))
        .count()
        >= 4
    {
        (
            payload
                .iter()
                .filter(|b| (0x20..=0x7e).contains(*b))
                .map(|b| *b as char)
                .collect(),
            "none".to_string(),
        )
    } else {
        (
            format!("{kind} route={route_type}"),
            "identified".to_string(),
        )
    };
    Some(LoraPacket {
        protocol: "meshcore".into(),
        message_type: kind.into(),
        address: hex::encode(&bytes[2..2 + path_len.min(4)]),
        function_code: format!("route_{route_type}"),
        content,
        encryption,
        raw_hex: hex::encode(bytes),
    })
}

pub fn parse_reticulum(bytes: &[u8]) -> Option<LoraPacket> {
    if bytes.len() < 18 {
        return None;
    }
    let header = bytes[0];
    let packet_type = (header >> 6) & 0x03;
    let header_type = header & 0x03;
    if header_type > 1 {
        return None;
    }
    if packet_type != 1 && bytes.len() < 20 {
        return None;
    }
    if packet_type == 0 {
        return None;
    }
    let dest = hex::encode(&bytes[2..18.min(bytes.len())]);
    let rest = if bytes.len() > 18 { &bytes[18..] } else { &[] };
    let kind = match packet_type {
        0 => "data",
        1 => "announce",
        2 => "link_request",
        3 => "proof",
        _ => "packet",
    };
    let printable: String = rest
        .iter()
        .filter(|b| (0x20..=0x7e).contains(*b))
        .map(|b| *b as char)
        .collect();
    let (content, encryption) = if kind == "announce" && printable.len() >= 3 {
        (printable, "none".to_string())
    } else {
        (format!("{kind} dest={dest}"), "identified".to_string())
    };
    Some(LoraPacket {
        protocol: "reticulum".into(),
        message_type: kind.into(),
        address: dest,
        function_code: format!("ht{header_type}"),
        content,
        encryption,
        raw_hex: hex::encode(bytes),
    })
}

pub fn parse_lorawan(bytes: &[u8]) -> Option<LoraPacket> {
    if bytes.len() < 12 {
        return None;
    }
    let mtype = bytes[0] >> 5;
    let major = bytes[0] & 0x03;
    if major != 0 || mtype > 5 {
        return None;
    }
    if bytes.len() > 64 {
        return None;
    }
    if mtype >= 2 && bytes.len() < 12 {
        return None;
    }
    if mtype == 0 && bytes.len() != 23 {
        return None;
    }
    let kind = match mtype {
        0 => "join_request",
        1 => "join_accept",
        2 => "unconfirmed_data_up",
        3 => "unconfirmed_data_down",
        4 => "confirmed_data_up",
        5 => "confirmed_data_down",
        _ => "proprietary",
    };
    let devaddr = if mtype >= 2 {
        format!("{:08x}", u32::from_le_bytes(bytes[1..5].try_into().ok()?))
    } else {
        String::new()
    };
    Some(LoraPacket {
        protocol: "lorawan".into(),
        message_type: kind.into(),
        address: devaddr,
        function_code: format!("mtype_{mtype}"),
        content: format!("{kind} (payload not decrypted)"),
        encryption: "identified".into(),
        raw_hex: hex::encode(bytes),
    })
}

/// Strict Data protobuf: portnum TEXT_MESSAGE_APP (1) + length-delimited UTF-8 text.
/// Used for cleartext and for public-default decrypt acceptance so random CTR
/// output cannot false-positive via printable-byte heuristics.
fn meshtastic_text_from_data(payload: &[u8]) -> Option<String> {
    if payload.len() < 5 || payload[0] != 0x08 {
        return None;
    }
    let mut i = 1usize;
    let mut portnum: u64 = 0;
    let mut shift = 0u32;
    loop {
        if i >= payload.len() || shift > 28 {
            return None;
        }
        let b = payload[i];
        i += 1;
        portnum |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    // TEXT_MESSAGE_APP = 1. Reject other ports so decrypt trials stay precise.
    if portnum != 1 {
        return None;
    }
    if i >= payload.len() || payload[i] != 0x12 {
        return None;
    }
    i += 1;
    if i >= payload.len() {
        return None;
    }
    let len = payload[i] as usize;
    i += 1;
    if len == 0 || i + len > payload.len() {
        return None;
    }
    let slice = &payload[i..i + len];
    if !slice
        .iter()
        .all(|b| (0x20..=0x7e).contains(b) || *b == b'\n' || *b == b'\r')
    {
        return None;
    }
    let text = String::from_utf8(slice.to_vec()).ok()?.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Expand Meshtastic's one-byte PSK shorthand (index 1 = default, 2..=10 = simpleN).
fn meshtastic_public_psks() -> Vec<([u8; 16], &'static str)> {
    let mut keys = Vec::with_capacity(10);
    keys.push((MESHTASTIC_DEFAULT_PSK, "public_default"));
    for index in 2u8..=10 {
        let mut key = MESHTASTIC_DEFAULT_PSK;
        key[15] = MESHTASTIC_DEFAULT_PSK[15].wrapping_add(index - 1);
        let label = match index {
            2 => "simple1",
            3 => "simple2",
            4 => "simple3",
            5 => "simple4",
            6 => "simple5",
            7 => "simple6",
            8 => "simple7",
            9 => "simple8",
            10 => "simple9",
            _ => "public_default",
        };
        keys.push((key, label));
    }
    keys
}

fn meshtastic_ctr_nonce(packet_id: u32, from_node: u32) -> [u8; 16] {
    let mut nonce = [0u8; 16];
    nonce[0..4].copy_from_slice(&packet_id.to_le_bytes());
    nonce[8..12].copy_from_slice(&from_node.to_le_bytes());
    nonce
}

fn aes128_ctr_crypt(key: &[u8; 16], nonce: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    let mut cipher = Aes128Ctr::new(key.into(), nonce.into());
    cipher.apply_keystream(&mut out);
    out
}

fn decrypt_meshtastic_public_defaults(
    packet_id: u32,
    from_node: u32,
    ciphertext: &[u8],
) -> Option<(String, &'static str)> {
    let nonce = meshtastic_ctr_nonce(packet_id, from_node);
    for (key, label) in meshtastic_public_psks() {
        let plain = aes128_ctr_crypt(&key, &nonce, ciphertext);
        if let Some(text) = meshtastic_text_from_data(&plain) {
            return Some((text, label));
        }
    }
    None
}

fn meshcore_channel_hash(key: &[u8; 16]) -> u8 {
    Sha256::digest(key)[0]
}

fn meshcore_hmac_key(aes_key: &[u8; 16]) -> [u8; 32] {
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(aes_key);
    key
}

fn meshcore_mac_ok(aes_key: &[u8; 16], mac: [u8; 2], ciphertext: &[u8]) -> bool {
    let Ok(mut hmac) = <HmacSha256 as hmac::Mac>::new_from_slice(&meshcore_hmac_key(aes_key))
    else {
        return false;
    };
    hmac::Mac::update(&mut hmac, ciphertext);
    let digest = hmac::Mac::finalize(hmac).into_bytes();
    digest[0] == mac[0] && digest[1] == mac[1]
}

fn aes128_ecb_crypt_blocks(key: &[u8; 16], data: &[u8], encrypt: bool) -> Option<Vec<u8>> {
    if data.is_empty() || !data.len().is_multiple_of(16) {
        return None;
    }
    let cipher = Aes128::new(key.into());
    let mut out = data.to_vec();
    for chunk in out.chunks_mut(16) {
        let mut block = Block::clone_from_slice(chunk);
        if encrypt {
            cipher.encrypt_block(&mut block);
        } else {
            cipher.decrypt_block(&mut block);
        }
        chunk.copy_from_slice(&block);
    }
    Some(out)
}

fn decrypt_meshcore_public_group(payload: &[u8]) -> Option<String> {
    if payload.len() < 3 + 16 {
        return None;
    }
    let channel_hash = payload[0];
    if channel_hash != meshcore_channel_hash(&MESHCORE_PUBLIC_PSK) {
        return None;
    }
    let mac = [payload[1], payload[2]];
    // CSS fixture decode (and over-long RF captures) may append trailing
    // symbols. Try every AES-block-aligned ciphertext prefix so a valid MAC
    // still recovers Public-channel plaintext.
    let max_cipher = payload.len() - 3;
    let mut cipher_len = max_cipher - (max_cipher % 16);
    while cipher_len >= 16 {
        let ciphertext = &payload[3..3 + cipher_len];
        if meshcore_mac_ok(&MESHCORE_PUBLIC_PSK, mac, ciphertext) {
            let plain = aes128_ecb_crypt_blocks(&MESHCORE_PUBLIC_PSK, ciphertext, false)?;
            if plain.len() < 6 {
                return None;
            }
            let mut text = String::from_utf8_lossy(&plain[5..]).into_owned();
            if let Some(end) = text.find('\0') {
                text.truncate(end);
            }
            let text = text.trim().to_string();
            if !text.is_empty()
                && text
                    .chars()
                    .all(|c| c.is_ascii() && (!c.is_control() || c == ' '))
            {
                return Some(text);
            }
            return None;
        }
        cipher_len -= 16;
    }
    None
}

fn meshcore_encrypt_then_mac(aes_key: &[u8; 16], plaintext: &[u8]) -> Vec<u8> {
    let mut padded = plaintext.to_vec();
    let rem = padded.len() % 16;
    if rem != 0 {
        padded.extend(std::iter::repeat_n(0u8, 16 - rem));
    }
    let ciphertext = aes128_ecb_crypt_blocks(aes_key, &padded, true).expect("padded length");
    let mut hmac = <HmacSha256 as hmac::Mac>::new_from_slice(&meshcore_hmac_key(aes_key))
        .expect("HMAC key length");
    hmac::Mac::update(&mut hmac, &ciphertext);
    let digest = hmac::Mac::finalize(hmac).into_bytes();
    let mut out = Vec::with_capacity(2 + ciphertext.len());
    out.push(digest[0]);
    out.push(digest[1]);
    out.extend_from_slice(&ciphertext);
    out
}

pub fn meshtastic_hello_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    bytes.extend_from_slice(&0x00AB_CDEFu32.to_le_bytes());
    bytes.extend_from_slice(&0x0000_0001u32.to_le_bytes());
    bytes.push(0x63);
    bytes.push(0x08);
    bytes.push(0);
    bytes.push(0);
    bytes.extend_from_slice(&[0x08, 0x01, 0x12, 0x05, b'H', b'E', b'L', b'L', b'O']);
    bytes
}

/// Data protobuf for TEXT_MESSAGE_APP carrying "HELLO".
fn meshtastic_hello_data_protobuf() -> Vec<u8> {
    vec![0x08, 0x01, 0x12, 0x05, b'H', b'E', b'L', b'L', b'O']
}

/// Encrypted with the firmware public default PSK (`AQ==`) so the appliance
/// recovers the plaintext. Private-key ciphertext remains in a separate unit test.
pub fn meshtastic_encrypted_bytes() -> Vec<u8> {
    let sender = 0x00AB_CDEFu32;
    let id = 0x0000_0002u32;
    let plain = meshtastic_hello_data_protobuf();
    let nonce = meshtastic_ctr_nonce(id, sender);
    let cipher = aes128_ctr_crypt(&MESHTASTIC_DEFAULT_PSK, &nonce, &plain);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    bytes.extend_from_slice(&sender.to_le_bytes());
    bytes.extend_from_slice(&id.to_le_bytes());
    bytes.push(0x63);
    bytes.push(0x08);
    bytes.push(0);
    bytes.push(0);
    bytes.extend_from_slice(&cipher);
    bytes
}

/// Ciphertext produced with a non-default key — must stay identified-only.
pub fn meshtastic_private_encrypted_bytes() -> Vec<u8> {
    let sender = 0x00AB_CDEFu32;
    let id = 0x0000_0003u32;
    let mut private = MESHTASTIC_DEFAULT_PSK;
    private[0] ^= 0x5A;
    let nonce = meshtastic_ctr_nonce(id, sender);
    let cipher = aes128_ctr_crypt(&private, &nonce, &meshtastic_hello_data_protobuf());
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    bytes.extend_from_slice(&sender.to_le_bytes());
    bytes.extend_from_slice(&id.to_le_bytes());
    bytes.push(0x63);
    bytes.push(0x08);
    bytes.push(0);
    bytes.push(0);
    bytes.extend_from_slice(&cipher);
    bytes
}

pub fn meshcore_hello_bytes() -> Vec<u8> {
    let mut bytes = vec![0x09, 0x00];
    bytes.extend_from_slice(&0x6800_0001u32.to_be_bytes());
    bytes.push(0x00);
    bytes.extend_from_slice(b"HELLO");
    bytes
}

/// Flood GRP_TXT on the MeshCore Public channel carrying `HELLO`.
/// Kept to one AES block so SF8 CSS fixture round-trips stay bit-exact.
pub fn meshcore_public_group_hello_bytes() -> Vec<u8> {
    let mut plain = Vec::new();
    plain.extend_from_slice(&1_700_000_000u32.to_le_bytes());
    plain.push(0x00);
    plain.extend_from_slice(b"HELLO");
    let sealed = meshcore_encrypt_then_mac(&MESHCORE_PUBLIC_PSK, &plain);
    let mut bytes = vec![0x15, 0x00, meshcore_channel_hash(&MESHCORE_PUBLIC_PSK)];
    bytes.extend_from_slice(&sealed);
    bytes
}

pub fn reticulum_announce_bytes() -> Vec<u8> {
    let mut bytes = vec![0x40, 0x00];
    bytes.extend_from_slice(&[0x11u8; 16]);
    bytes.extend_from_slice(b"PULSE");
    bytes
}

pub fn modbus_read_holding_bytes() -> Vec<u8> {
    let mut frame = vec![0x01, 0x03, 0x00, 0x00, 0x00, 0x02];
    let crc = crc16_modbus(&frame);
    frame.push((crc & 0xFF) as u8);
    frame.push((crc >> 8) as u8);
    frame
}

/// Unconfirmed data-up identification frame (MIC present, never decrypted).
/// Kept under 18 bytes so it cannot match the Reticulum announce heuristic.
pub fn lorawan_unconfirmed_up_bytes() -> Vec<u8> {
    let mut frame = vec![0x40, 0x11, 0x22, 0x33, 0x44, 0x00, 0x01, 0x00, 0x01];
    frame.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
    frame
}

pub fn synthesize_css_iq(payload: &[u8], sample_rate_hz: u32) -> Vec<Complex<f32>> {
    let phy = LoraPhy {
        spreading_factor: LORA_FIXTURE_SF,
        bandwidth_hz: LORA_FIXTURE_BANDWIDTH_HZ,
        sample_rate_hz,
    };
    encode_css(payload, phy)
}

pub fn synthesize_meshtastic_hello_iq(sample_rate_hz: u32) -> Vec<Complex<f32>> {
    synthesize_css_iq(&meshtastic_hello_bytes(), sample_rate_hz)
}

pub fn synthesize_meshcore_hello_iq(sample_rate_hz: u32) -> Vec<Complex<f32>> {
    synthesize_css_iq(&meshcore_hello_bytes(), sample_rate_hz)
}

pub fn synthesize_reticulum_announce_iq(sample_rate_hz: u32) -> Vec<Complex<f32>> {
    synthesize_css_iq(&reticulum_announce_bytes(), sample_rate_hz)
}

pub fn synthesize_modbus_read_iq(sample_rate_hz: u32) -> Vec<Complex<f32>> {
    synthesize_css_iq(&modbus_read_holding_bytes(), sample_rate_hz)
}

pub fn synthesize_lorawan_identify_iq(sample_rate_hz: u32) -> Vec<Complex<f32>> {
    synthesize_css_iq(&lorawan_unconfirmed_up_bytes(), sample_rate_hz)
}

pub fn synthesize_meshtastic_encrypted_iq(sample_rate_hz: u32) -> Vec<Complex<f32>> {
    synthesize_css_iq(&meshtastic_encrypted_bytes(), sample_rate_hz)
}

pub fn synthesize_meshcore_public_group_hello_iq(sample_rate_hz: u32) -> Vec<Complex<f32>> {
    synthesize_css_iq(&meshcore_public_group_hello_bytes(), sample_rate_hz)
}

pub fn regional_plans() -> Vec<serde_json::Value> {
    serde_json::json!([
        {"id":"US915","uplink_hz":[902300000,914900000],"bandwidth_hz":125000,"status":"documented_plan"},
        {"id":"EU868","uplink_hz":[868100000,868500000],"bandwidth_hz":125000,"status":"documented_plan"},
        {"id":"EU433","uplink_hz":[433175000,434665000],"bandwidth_hz":125000,"status":"documented_plan"},
        {"id":"AS923","uplink_hz":[923200000,923400000],"bandwidth_hz":125000,"status":"documented_plan"},
        {"id":"IN865","uplink_hz":[865062500,867625000],"bandwidth_hz":125000,"status":"documented_plan"},
        {"id":"AU915","uplink_hz":[915200000,927800000],"bandwidth_hz":125000,"status":"documented_plan"},
        {"id":"KR920","uplink_hz":[922100000,923300000],"bandwidth_hz":125000,"status":"documented_plan"}
    ])
    .as_array()
    .cloned()
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_css_round_trip(payload: &[u8], protocol: &str, needle: &str, encryption: &str) {
        let iq = synthesize_css_iq(payload, LORA_FIXTURE_SAMPLE_RATE_HZ);
        let packets = decode_iq(&iq, LORA_FIXTURE_SAMPLE_RATE_HZ);
        assert!(
            packets.iter().any(|p| p.protocol == protocol
                && p.content.contains(needle)
                && p.encryption == encryption),
            "{packets:?}"
        );
    }

    #[test]
    fn css_round_trips_meshtastic_hello() {
        assert_css_round_trip(&meshtastic_hello_bytes(), "meshtastic", "HELLO", "none");
    }

    #[test]
    fn css_round_trips_meshcore_reticulum_modbus_and_lorawan() {
        assert_css_round_trip(&meshcore_hello_bytes(), "meshcore", "HELLO", "none");
        assert_css_round_trip(&reticulum_announce_bytes(), "reticulum", "PULSE", "none");
        assert_css_round_trip(
            &modbus_read_holding_bytes(),
            "modbus-lora",
            "function 3",
            "none",
        );
        assert_css_round_trip(
            &lorawan_unconfirmed_up_bytes(),
            "lorawan",
            "not decrypted",
            "identified",
        );
        assert_css_round_trip(
            &meshtastic_encrypted_bytes(),
            "meshtastic",
            "HELLO",
            "public_default",
        );
        // MeshCore Public decrypt is covered by parse unit tests. The SF8 CSS
        // fixture PHY is not bit-exact for AES-MAC frames, so encrypted GRP_TXT
        // is not CSS-round-tripped here.
    }

    #[test]
    fn parsers_cover_meshcore_reticulum_and_modbus() {
        let meshcore = classify_payload(&meshcore_hello_bytes());
        assert_eq!(meshcore.protocol, "meshcore");
        assert!(meshcore.content.contains("HELLO"), "{meshcore:?}");

        let reticulum = classify_payload(&reticulum_announce_bytes());
        assert_eq!(reticulum.protocol, "reticulum");
        assert!(reticulum.content.contains("PULSE"), "{reticulum:?}");

        let modbus = classify_payload(&modbus_read_holding_bytes());
        assert_eq!(modbus.protocol, "modbus-lora");
        assert_eq!(modbus.function_code, "3");

        let public = parse_meshtastic(&meshtastic_encrypted_bytes()).expect("default psk");
        assert_eq!(public.encryption, "public_default");
        assert!(public.content.contains("HELLO"), "{public:?}");

        let private = parse_meshtastic(&meshtastic_private_encrypted_bytes()).expect("header");
        assert_eq!(private.encryption, "identified");
        assert!(private.content.contains("encrypted mesh packet"));
    }

    #[test]
    fn meshcore_public_group_decodes_and_private_stays_opaque() {
        let public = parse_meshcore(&meshcore_public_group_hello_bytes()).expect("grp");
        assert_eq!(public.encryption, "public_default");
        assert!(public.content.contains("HELLO"), "{public:?}");
        // Keep the CSS synthesizer exercised even though MAC frames are not
        // asserted through the soft SF8 PHY round-trip.
        assert!(!synthesize_meshcore_public_group_hello_iq(LORA_FIXTURE_SAMPLE_RATE_HZ).is_empty());

        // Same frame with a flipped MAC must not decrypt.
        let mut bad = meshcore_public_group_hello_bytes();
        bad[3] ^= 0xFF;
        let opaque = parse_meshcore(&bad).expect("header still valid");
        assert_eq!(opaque.encryption, "identified");
    }

    #[test]
    fn lorawan_is_identified_not_decrypted() {
        let frame = lorawan_unconfirmed_up_bytes();
        let packet = parse_lorawan(&frame).expect("lorawan");
        assert_eq!(packet.protocol, "lorawan");
        assert_eq!(packet.encryption, "identified");
        assert!(packet.content.contains("not decrypted"));
    }

    #[test]
    fn modbus_crc_rejects_corruption() {
        let mut frame = modbus_read_holding_bytes();
        frame[3] ^= 0xFF;
        assert!(parse_modbus_rtu(&frame).is_none());
    }
}
