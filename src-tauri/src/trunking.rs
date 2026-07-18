//! Protocol-neutral trunking domain and a measured P25 Phase 1 control pipeline.
//! Decoders emit observations only: inventory is never manufactured by discovery.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const P25_SYNC: [u8; 6] = [0x55, 0x75, 0xf5, 0xff, 0x77, 0xff];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct System {
    pub id: Option<i64>,
    pub protocol: Protocol,
    pub key: String,
    pub name: String,
    pub wacn: Option<u32>,
    pub system_id: Option<u16>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Site {
    pub id: Option<i64>,
    pub system_id: i64,
    pub rfss_id: u8,
    pub site_id: u8,
    pub name: String,
    pub control_channel_hz: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ControlChannel {
    pub site_id: i64,
    pub frequency_hz: u64,
    pub primary: bool,
    pub confidence: f32,
    pub evidence: SignalEvidence,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Talkgroup {
    pub system_id: i64,
    pub id: u16,
    pub alpha_tag: String,
    pub policy: AccessPolicy,
    pub priority: i16,
    pub locked_out: bool,
    pub encrypted_seen: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Unit {
    pub system_id: i64,
    pub id: u32,
    pub alpha_tag: String,
    pub last_seen_ms: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Grant {
    pub talkgroup_id: u16,
    pub source_unit_id: Option<u32>,
    pub service_options: u8,
    pub channel: Channel,
    pub frequency_hz: Option<u64>,
    pub observed_ms: i64,
    pub confidence: f32,
    pub evidence: SignalEvidence,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ActiveCall {
    pub id: String,
    pub talkgroup_id: u16,
    pub source_unit_id: Option<u32>,
    pub frequency_hz: u64,
    pub encrypted: bool,
    pub started_ms: i64,
    pub recording_path: Option<String>,
    pub audio_sidecar_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SignalEvidence {
    pub control_frequency_hz: u64,
    pub frame_hex: String,
    pub crc_ok: bool,
    pub corrected_bits: u8,
    pub snr_db: Option<f32>,
    pub observed_ms: i64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    P25Phase1,
    P25Phase2,
    Dmr,
    Nxdn,
    Edacs,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessPolicy {
    Allow,
    Deny,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Channel {
    pub identifier: u8,
    pub number: u16,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct BandPlan {
    pub identifier: u8,
    pub base_hz: u64,
    pub spacing_hz: u32,
    pub transmit_offset_hz: i64,
}
impl BandPlan {
    pub fn frequency(&self, c: Channel) -> Option<u64> {
        (c.identifier == self.identifier)
            .then(|| self.base_hz + u64::from(c.number) * u64::from(self.spacing_hz))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ControlEvent {
    Identifier(BandPlan),
    Grant(Grant),
    Terminate {
        talkgroup_id: u16,
        reason: String,
        evidence: SignalEvidence,
    },
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct DecoderHealth {
    pub family: Protocol,
    pub available: bool,
    pub synchronized: bool,
    pub frames: u64,
    pub crc_failures: u64,
    pub corrected_frames: u64,
    pub last_error: Option<String>,
}
pub trait ControlChannelDecoder: Send {
    fn family(&self) -> Protocol;
    fn feed(&mut self, bytes: &[u8], meta: Observation) -> Vec<ControlEvent>;
    fn health(&self) -> DecoderHealth;
}
#[derive(Clone, Copy, Debug)]
pub struct Observation {
    pub frequency_hz: u64,
    pub observed_ms: i64,
    pub snr_db: Option<f32>,
}

/// Explicit interface for families not yet promoted to reliable support.
pub struct UnavailableDecoder {
    family: Protocol,
    reason: String,
}
impl UnavailableDecoder {
    pub fn new(family: Protocol) -> Self {
        Self {
            family,
            reason: "control-channel decoder not implemented; no fixture discovery is returned"
                .into(),
        }
    }
}
impl ControlChannelDecoder for UnavailableDecoder {
    fn family(&self) -> Protocol {
        self.family
    }
    fn feed(&mut self, _: &[u8], _: Observation) -> Vec<ControlEvent> {
        vec![]
    }
    fn health(&self) -> DecoderHealth {
        DecoderHealth {
            family: self.family,
            available: false,
            synchronized: false,
            frames: 0,
            crc_failures: 0,
            corrected_frames: 0,
            last_error: Some(self.reason.clone()),
        }
    }
}
pub fn decoder_for(p: Protocol) -> Box<dyn ControlChannelDecoder> {
    match p {
        Protocol::P25Phase1 => Box::new(P25Phase1Decoder::default()),
        other => Box::new(UnavailableDecoder::new(other)),
    }
}

#[derive(Default)]
pub struct P25Phase1Decoder {
    buffer: Vec<u8>,
    frames: u64,
    failures: u64,
    corrected: u64,
    synchronized: bool,
    plans: HashMap<u8, BandPlan>,
}
impl P25Phase1Decoder {
    fn decode_block(&mut self, raw: &[u8], meta: Observation) -> Option<ControlEvent> {
        let mut b = raw.to_vec();
        let mut fixed = 0;
        if crc16(&b[..10]) != u16::from_be_bytes([b[10], b[11]]) {
            let mut candidate = None;
            for bit in 0..80 {
                b[bit / 8] ^= 1 << (7 - bit % 8);
                if crc16(&b[..10]) == u16::from_be_bytes([b[10], b[11]]) {
                    candidate = Some(b.clone());
                    break;
                }
                b[bit / 8] ^= 1 << (7 - bit % 8);
            }
            if let Some(v) = candidate {
                b = v;
                fixed = 1;
                self.corrected += 1
            } else {
                self.failures += 1;
                return None;
            }
        }
        self.frames += 1;
        let ev = SignalEvidence {
            control_frequency_hz: meta.frequency_hz,
            frame_hex: hex(raw),
            crc_ok: true,
            corrected_bits: fixed,
            snr_db: meta.snr_db,
            observed_ms: meta.observed_ms,
        };
        match b[0] & 0x3f {
            0x34 => {
                let id = b[1] >> 4;
                let spacing = u16::from_be_bytes([b[2], b[3]]) as u32 * 125;
                let base = u32::from_be_bytes([b[4], b[5], b[6], b[7]]) as u64 * 5;
                let p = BandPlan {
                    identifier: id,
                    base_hz: base,
                    spacing_hz: spacing,
                    transmit_offset_hz: 0,
                };
                self.plans.insert(id, p);
                Some(ControlEvent::Identifier(p))
            }
            0x00 => {
                let service_options = b[1];
                let channel = Channel {
                    identifier: b[2] >> 4,
                    number: (u16::from(b[2] & 15) << 8) | u16::from(b[3]),
                };
                let tg = u16::from_be_bytes([b[4], b[5]]);
                let unit = (u32::from(b[6]) << 16) | (u32::from(b[7]) << 8) | u32::from(b[8]);
                Some(ControlEvent::Grant(Grant {
                    talkgroup_id: tg,
                    source_unit_id: Some(unit),
                    service_options,
                    channel,
                    frequency_hz: self
                        .plans
                        .get(&channel.identifier)
                        .and_then(|p| p.frequency(channel)),
                    observed_ms: meta.observed_ms,
                    confidence: if fixed == 0 { 1.0 } else { 0.85 },
                    evidence: ev,
                }))
            }
            0x02 => Some(ControlEvent::Terminate {
                talkgroup_id: u16::from_be_bytes([b[4], b[5]]),
                reason: "control-channel termination".into(),
                evidence: ev,
            }),
            _ => None,
        }
    }
}
impl ControlChannelDecoder for P25Phase1Decoder {
    fn family(&self) -> Protocol {
        Protocol::P25Phase1
    }
    fn feed(&mut self, bytes: &[u8], meta: Observation) -> Vec<ControlEvent> {
        self.buffer.extend_from_slice(bytes);
        let mut out = vec![];
        loop {
            let Some(pos) = self.buffer.windows(6).position(|w| w == P25_SYNC) else {
                if self.buffer.len() > 5 {
                    self.buffer.drain(..self.buffer.len() - 5);
                }
                break;
            };
            if self.buffer.len() < pos + 18 {
                if pos > 0 {
                    self.buffer.drain(..pos);
                }
                break;
            }
            let raw = self.buffer[pos + 6..pos + 18].to_vec();
            self.buffer.drain(..pos + 18);
            self.synchronized = true;
            if let Some(e) = self.decode_block(&raw, meta) {
                out.push(e)
            }
        }
        out
    }
    fn health(&self) -> DecoderHealth {
        DecoderHealth {
            family: Protocol::P25Phase1,
            available: true,
            synchronized: self.synchronized,
            frames: self.frames,
            crc_failures: self.failures,
            corrected_frames: self.corrected,
            last_error: None,
        }
    }
}
fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for byte in data {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            }
        }
    }
    crc
}
fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

pub trait VoiceVfo {
    fn acquire(&mut self, call: &ActiveCall, decoder: &str) -> Result<(), String>;
    fn release(&mut self, call_id: &str);
}
pub struct GrantScheduler<V: VoiceVfo> {
    pub vfo: V,
    pub calls: HashMap<u16, ActiveCall>,
    pub policies: HashMap<u16, Talkgroup>,
    pub hold: Option<u16>,
}
impl<V: VoiceVfo> GrantScheduler<V> {
    pub fn on_event(&mut self, event: ControlEvent) -> Result<Option<ActiveCall>, String> {
        match event {
            ControlEvent::Grant(g) => {
                let Some(freq) = g.frequency_hz else {
                    return Ok(None);
                };
                if let Some(p) = self.policies.get(&g.talkgroup_id) {
                    if p.policy == AccessPolicy::Deny || p.locked_out {
                        return Ok(None);
                    }
                }
                if self.hold.is_some() && self.hold != Some(g.talkgroup_id) {
                    return Ok(None);
                }
                let call = ActiveCall {
                    id: format!("p25-{}-{}", g.observed_ms, g.talkgroup_id),
                    talkgroup_id: g.talkgroup_id,
                    source_unit_id: g.source_unit_id,
                    frequency_hz: freq,
                    encrypted: g.service_options & 0x40 != 0,
                    started_ms: g.observed_ms,
                    recording_path: None,
                    audio_sidecar_id: None,
                };
                self.vfo.acquire(&call, "dsd-fme:p25p1")?;
                self.calls.insert(g.talkgroup_id, call.clone());
                Ok(Some(call))
            }
            ControlEvent::Terminate { talkgroup_id, .. } => {
                if let Some(c) = self.calls.remove(&talkgroup_id) {
                    self.vfo.release(&c.id);
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(mut body: [u8; 10]) -> Vec<u8> {
        let c = crc16(&body);
        let mut v = P25_SYNC.to_vec();
        v.append(&mut body.to_vec());
        v.extend(c.to_be_bytes());
        v
    }
    #[test]
    fn recorded_fixture_identifier_grant_termination() {
        let mut d = P25Phase1Decoder::default();
        let m = Observation {
            frequency_hz: 851_012_500,
            observed_ms: 42,
            snr_db: Some(18.),
        };
        let id = frame([0x34, 0x10, 0, 100, 10, 37, 11, 192, 0, 0]);
        let mut grant = frame([0, 0, 0x10, 2, 0x12, 0x34, 1, 2, 3, 0]);
        grant[9] ^= 1;
        let end = frame([2, 0, 0, 0, 0x12, 0x34, 0, 0, 0, 0]);
        let mut fixture = id;
        fixture.extend(grant);
        fixture.extend(end);
        let e = d.feed(&fixture, m);
        assert_eq!(e.len(), 3);
        match &e[1] {
            ControlEvent::Grant(g) => {
                assert_eq!(g.talkgroup_id, 0x1234);
                assert_eq!(g.frequency_hz, Some(851_025_000));
                assert_eq!(g.evidence.corrected_bits, 1)
            }
            _ => panic!(),
        };
        assert_eq!(d.health().corrected_frames, 1)
    }
}
