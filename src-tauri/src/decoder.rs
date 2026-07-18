//! Common contract and routing for in-process protocol decoders.
//!
//! Decoders deliberately exchange a protocol-neutral envelope.  This keeps the
//! scanner, persistence, event and HTTP layers independent of decoder structs,
//! while `payload` remains a stable, versioned protocol-specific schema and
//! `raw_frame` preserves diagnostic evidence.

use std::collections::BTreeMap;

use rustfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NativeInputFormat {
    ComplexF32,
    DiscriminatorF32,
    AudioF32,
    HardBits,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecoderMetrics {
    pub samples_received: u64,
    pub frames_attempted: u64,
    pub valid_frames: u64,
    pub checksum_failures: u64,
    pub corrected_frames: u64,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecoderDescriptor {
    pub protocol: String,
    pub available: bool,
    pub supported_sample_rates_hz: Vec<u32>,
    pub bandwidth_hz: (u32, u32),
    pub input_format: NativeInputFormat,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeMessage {
    pub schema: String,
    pub schema_version: u16,
    pub protocol: String,
    pub frequency_hz: u64,
    pub message_type: String,
    pub address: String,
    pub payload: serde_json::Value,
    pub raw_frame: String,
    pub received_at_ms: i64,
}

pub trait NativeDecoder: Send {
    fn descriptor(&self) -> DecoderDescriptor;
    fn reset(&mut self);
    fn feed_iq(&mut self, samples: &[Complex<f32>], metadata: &ChannelMetadata);
    fn take_messages(&mut self) -> Vec<NativeMessage>;
    fn metrics(&self) -> DecoderMetrics;
    fn report_failure(&mut self, error: String);
}

#[derive(Clone, Debug, Default)]
pub struct ChannelMetadata {
    pub bank_name: String,
    pub frequency_hz: u64,
    pub bandwidth_hz: u32,
    pub protocol_hint: Option<String>,
    pub decoder_enabled: bool,
}

/// Pure routing decision used by the live scanner and fixture tests.
pub fn route_protocol(
    meta: &ChannelMetadata,
    classified_protocol: Option<&str>,
) -> Option<&'static str> {
    if !meta.decoder_enabled {
        return None;
    }
    let hint = meta
        .protocol_hint
        .as_deref()
        .or(classified_protocol)
        .unwrap_or("")
        .to_ascii_lowercase();
    let bank = meta.bank_name.to_ascii_lowercase();
    let matches = |names: &[&str]| names.iter().any(|n| hint == *n || bank.contains(n));
    if matches(&["adsb", "ads-b", "mode_s"])
        || (1_089_000_000..=1_091_000_000).contains(&meta.frequency_hz)
    {
        Some("adsb")
    } else if matches(&["ais", "marine_ais"])
        || (161_950_000..=162_050_000).contains(&meta.frequency_hz)
    {
        Some("ais")
    } else if matches(&["aprs", "ax25"]) {
        Some("aprs")
    } else if matches(&["pocsag", "pager"]) {
        Some("pocsag")
    } else if matches(&["uat978", "uat"]) {
        Some("uat978")
    } else if matches(&["acars"]) {
        Some("acars")
    } else if matches(&["vdl2"]) {
        Some("vdl2")
    } else {
        None
    }
}

pub fn descriptors() -> Vec<DecoderDescriptor> {
    vec![
        descriptor(
            "adsb",
            vec![2_000_000, 2_400_000],
            (1_000_000, 3_000_000),
            NativeInputFormat::ComplexF32,
        ),
        descriptor(
            "ais",
            vec![48_000, 96_000, 192_000],
            (20_000, 30_000),
            NativeInputFormat::ComplexF32,
        ),
        descriptor(
            "aprs",
            vec![48_000, 96_000],
            (10_000, 25_000),
            NativeInputFormat::AudioF32,
        ),
        descriptor(
            "pocsag",
            vec![48_000, 96_000],
            (10_000, 25_000),
            NativeInputFormat::DiscriminatorF32,
        ),
        descriptor(
            "uat978",
            vec![2_083_334],
            (1_000_000, 2_000_000),
            NativeInputFormat::ComplexF32,
        ),
        descriptor(
            "acars",
            vec![48_000, 96_000],
            (10_000, 25_000),
            NativeInputFormat::ComplexF32,
        ),
        descriptor(
            "vdl2",
            vec![105_000, 210_000],
            (20_000, 50_000),
            NativeInputFormat::ComplexF32,
        ),
    ]
}

fn descriptor(
    protocol: &str,
    rates: Vec<u32>,
    bandwidth_hz: (u32, u32),
    input_format: NativeInputFormat,
) -> DecoderDescriptor {
    DecoderDescriptor {
        protocol: protocol.into(),
        available: true,
        supported_sample_rates_hz: rates,
        bandwidth_hz,
        input_format,
    }
}

pub fn metrics_snapshot(metrics: &BTreeMap<String, DecoderMetrics>) -> serde_json::Value {
    serde_json::to_value(metrics).unwrap_or_else(|_| serde_json::json!({}))
}

pub struct AdsbNativeDecoder {
    inner: crate::adsb::AdsbDecoder,
    out: Vec<NativeMessage>,
    metrics: DecoderMetrics,
}

impl AdsbNativeDecoder {
    pub fn new(sample_rate: u32) -> Option<Self> {
        Some(Self {
            inner: crate::adsb::AdsbDecoder::new(sample_rate)?,
            out: vec![],
            metrics: DecoderMetrics::default(),
        })
    }
}

impl NativeDecoder for AdsbNativeDecoder {
    fn descriptor(&self) -> DecoderDescriptor {
        descriptors().remove(0)
    }
    fn reset(&mut self) {
        self.inner = crate::adsb::AdsbDecoder::new(2_000_000).expect("valid ADS-B rate");
        self.out.clear();
        self.metrics.last_error = None;
    }
    fn feed_iq(&mut self, samples: &[Complex<f32>], meta: &ChannelMetadata) {
        self.metrics.samples_received += samples.len() as u64;
        self.inner.feed_iq(samples);
        for msg in self.inner.take_messages() {
            self.metrics.frames_attempted += 1;
            self.metrics.valid_frames += 1;
            self.out.push(NativeMessage {
                schema: "pulsescope.adsb.message".into(),
                schema_version: 1,
                protocol: "adsb".into(),
                frequency_hz: meta.frequency_hz,
                message_type: msg.message_type.clone(),
                address: msg.icao.clone(),
                raw_frame: msg.raw_hex.clone(),
                payload: serde_json::to_value(msg).unwrap_or_default(),
                received_at_ms: crate::scanner::now_ms(),
            });
        }
    }
    fn take_messages(&mut self) -> Vec<NativeMessage> {
        std::mem::take(&mut self.out)
    }
    fn metrics(&self) -> DecoderMetrics {
        self.metrics.clone()
    }
    fn report_failure(&mut self, error: String) {
        self.metrics.last_error = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routing_obeys_configuration_and_metadata() {
        let mut m = ChannelMetadata {
            bank_name: "Harbor".into(),
            frequency_hz: 162_025_000,
            bandwidth_hz: 25_000,
            protocol_hint: None,
            decoder_enabled: true,
        };
        assert_eq!(route_protocol(&m, Some("ais")), Some("ais"));
        m.decoder_enabled = false;
        assert_eq!(route_protocol(&m, Some("ais")), None);
    }
    #[test]
    fn descriptors_define_full_contract() {
        assert!(descriptors().iter().all(|d| d.available
            && !d.supported_sample_rates_hz.is_empty()
            && d.bandwidth_hz.0 <= d.bandwidth_hz.1));
    }
    #[test]
    fn fixture_to_persistence_and_ui_json_contract() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../fixtures/native-decoders/manifest.json"))
                .unwrap();
        assert_eq!(manifest["fixtures"].as_array().unwrap().len(), 5);
        let db = crate::db::Db::open(std::path::Path::new(":memory:")).unwrap();
        let envelope = serde_json::json!({"schema":"pulsescope.adsb.message","schema_version":1,"payload":{"icao":"ABCDEF"}});
        let row = crate::db::DecodedMessage {
            id: None,
            frequency_hz: 1_090_000_000,
            protocol: "adsb".into(),
            message_type: "identification".into(),
            address: "ABCDEF".into(),
            function_code: "pulsescope.adsb.message".into(),
            content: envelope.to_string(),
            raw: "8DABCDEF00".into(),
            encryption: "none".into(),
            timestamp_ms: 1,
        };
        db.insert_decoded_message(&row).unwrap();
        let api_rows = db.recent_decoded_messages(1).unwrap();
        let ui_json = serde_json::to_value(&api_rows).unwrap();
        assert_eq!(ui_json[0]["raw"], "8DABCDEF00");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(ui_json[0]["content"].as_str().unwrap())
                .unwrap()["schema_version"],
            1
        );
    }
}
