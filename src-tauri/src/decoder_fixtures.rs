//! Recorded-IQ end-to-end decoder fixtures and conformance runners.
//!
//! Synthetic clean-room IQ/audio artifacts live under `fixtures/recorded-iq/`.
//! Tests load each fixture, run the production decode path, and assert normalized
//! event fields — the gate required before catalog `available` may become true.

use std::path::{Path, PathBuf};

use rustfft::num_complex::Complex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::adsb::AdsbDecoder;
use crate::ais::IqDecoder;
use crate::aprs::{AprsDecoder, AprsFrame};
use crate::db::DecodedMessage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedIqArtifact {
    pub sample_rate_hz: u32,
    pub iq: Vec<[f32; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedNrzArtifact {
    pub baud: u32,
    pub bits: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedAudioArtifact {
    pub sample_rate_hz: u32,
    pub samples: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureExpectation {
    pub message_count_min: usize,
    pub protocol: String,
    #[serde(default)]
    pub icao: Option<String>,
    #[serde(default)]
    pub mmsi: Option<u32>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub info_contains: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedFixtureEntry {
    pub id: String,
    pub protocol: String,
    pub file: String,
    pub kind: String,
    pub expected: FixtureExpectation,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedFixtureManifest {
    pub schema: String,
    pub version: u16,
    pub fixtures: Vec<RecordedFixtureEntry>,
}

pub fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/recorded-iq")
}

pub fn load_manifest(root: &Path) -> anyhow::Result<RecordedFixtureManifest> {
    let path = root.join("manifest.json");
    let text = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&text)?)
}

pub fn verify_sha256(path: &Path, expected: &str) -> anyhow::Result<()> {
    let digest = Sha256::digest(std::fs::read(path)?);
    let hex = hex::encode(digest);
    if hex != expected.to_ascii_lowercase() {
        anyhow::bail!("fixture checksum mismatch for {}", path.display());
    }
    Ok(())
}

pub fn decode_adsb_iq(iq: &[Complex<f32>], sample_rate_hz: u32) -> Vec<DecodedMessage> {
    let mut decoder = match AdsbDecoder::new(sample_rate_hz) {
        Some(d) => d,
        None => return Vec::new(),
    };
    decoder.feed_iq(iq);
    decoder
        .take_messages()
        .into_iter()
        .map(|m| DecodedMessage {
            id: None,
            frequency_hz: 1_090_000_000,
            protocol: "adsb".into(),
            message_type: m.message_type.clone(),
            address: m.icao.clone(),
            function_code: format!("DF{}", m.df),
            content: m
                .callsign
                .clone()
                .or_else(|| m.altitude_ft.map(|a| format!("{a} ft")))
                .unwrap_or_default(),
            raw: m.raw_hex.clone(),
            encryption: "none".into(),
            timestamp_ms: crate::scanner::now_ms(),
        })
        .collect()
}

pub fn decode_ais_iq(samples: &[(f32, f32)], sample_rate_hz: u32) -> Vec<DecodedMessage> {
    let mut decoder = match IqDecoder::new(sample_rate_hz as f64) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut batch = samples.to_vec();
    batch.extend_from_slice(samples);
    decoder
        .push_iq(&batch)
        .into_iter()
        .filter_map(|r| r.ok())
        .map(|m| DecodedMessage {
            id: None,
            frequency_hz: 162_000_000,
            protocol: "ais".into(),
            message_type: format!("type_{}", m.message_type()),
            address: m.mmsi().to_string(),
            function_code: String::new(),
            content: serde_json::to_string(&m).unwrap_or_default(),
            raw: String::new(),
            encryption: "none".into(),
            timestamp_ms: crate::scanner::now_ms(),
        })
        .collect()
}

pub fn decode_aprs_nrz_bits(bits: &[u8]) -> Vec<DecodedMessage> {
    crate::aprs::parse_ax25_bits(bits)
        .into_iter()
        .map(|f| DecodedMessage {
            id: None,
            frequency_hz: 144_390_000,
            protocol: "aprs".into(),
            message_type: "ax25".into(),
            address: f.source.clone(),
            function_code: f.dest.clone(),
            content: f.info.clone(),
            raw: f.info.clone(),
            encryption: "none".into(),
            timestamp_ms: crate::scanner::now_ms(),
        })
        .collect()
}

pub fn decode_aprs_audio(samples: &[f32], sample_rate_hz: f32) -> Vec<DecodedMessage> {
    let mut decoder = AprsDecoder::new(sample_rate_hz);
    for &sample in samples {
        decoder.feed(sample);
    }
    let mut frames: Vec<AprsFrame> = decoder.frames.clone();
    if frames.is_empty() {
        let bits = crate::aprs::recover_nrz_bits_chunked(samples, sample_rate_hz);
        frames = crate::aprs::parse_ax25_bits(&bits);
    }
    frames
        .iter()
        .map(|f| DecodedMessage {
            id: None,
            frequency_hz: 144_390_000,
            protocol: "aprs".into(),
            message_type: "ax25".into(),
            address: f.source.clone(),
            function_code: f.dest.clone(),
            content: f.info.clone(),
            raw: f.info.clone(),
            encryption: "none".into(),
            timestamp_ms: crate::scanner::now_ms(),
        })
        .collect()
}

pub fn decode_pocsag_audio(samples: &[f32], sample_rate_hz: u32) -> Vec<DecodedMessage> {
    let mut decoder =
        crate::pocsag::PocsagDecoder::new(sample_rate_hz, crate::pocsag::PocsagBaud::Baud1200);
    decoder
        .push_audio(samples)
        .into_iter()
        .map(|m| DecodedMessage {
            id: None,
            frequency_hz: 929_612_500,
            protocol: "pocsag".into(),
            message_type: "pager".into(),
            address: m.ric.to_string(),
            function_code: m.function.to_string(),
            content: m.text.clone(),
            raw: m.text,
            encryption: "none".into(),
            timestamp_ms: crate::scanner::now_ms(),
        })
        .collect()
}

pub fn decode_rtty_audio(samples: &[f32], sample_rate_hz: f32) -> Vec<DecodedMessage> {
    crate::demod::decode_rtty(samples, sample_rate_hz, 2125.0, 1955.0, 50.0)
        .into_iter()
        .map(|text| DecodedMessage {
            id: None,
            frequency_hz: 14_080_000,
            protocol: "rtty".into(),
            message_type: "text".into(),
            address: String::new(),
            function_code: String::new(),
            content: text.clone(),
            raw: text,
            encryption: "none".into(),
            timestamp_ms: crate::scanner::now_ms(),
        })
        .collect()
}

pub fn decode_navtex_audio(samples: &[f32], sample_rate_hz: f32) -> Vec<DecodedMessage> {
    crate::demod::decode_navtex(samples, sample_rate_hz)
        .into_iter()
        .map(|text| DecodedMessage {
            id: None,
            frequency_hz: 518_000,
            protocol: "navtex".into(),
            message_type: "text".into(),
            address: String::new(),
            function_code: String::new(),
            content: text.clone(),
            raw: text,
            encryption: "none".into(),
            timestamp_ms: crate::scanner::now_ms(),
        })
        .collect()
}

pub fn run_recorded_fixture(
    root: &Path,
    entry: &RecordedFixtureEntry,
) -> anyhow::Result<Vec<DecodedMessage>> {
    let path = root.join(&entry.file);
    verify_sha256(&path, &entry.sha256)?;
    let text = std::fs::read_to_string(&path)?;
    match entry.kind.as_str() {
        "iq" => {
            let artifact: RecordedIqArtifact = serde_json::from_str(&text)?;
            let iq: Vec<Complex<f32>> = artifact
                .iq
                .iter()
                .map(|pair| Complex::new(pair[0], pair[1]))
                .collect();
            Ok(match entry.protocol.as_str() {
                "adsb" => decode_adsb_iq(&iq, artifact.sample_rate_hz),
                "ais" => {
                    let pairs: Vec<(f32, f32)> = artifact.iq.iter().map(|p| (p[0], p[1])).collect();
                    decode_ais_iq(&pairs, artifact.sample_rate_hz)
                }
                _ => anyhow::bail!("unsupported IQ protocol {}", entry.protocol),
            })
        }
        "audio" => {
            let artifact: RecordedAudioArtifact = serde_json::from_str(&text)?;
            Ok(match entry.protocol.as_str() {
                "aprs" => decode_aprs_audio(&artifact.samples, artifact.sample_rate_hz as f32),
                "pocsag" => decode_pocsag_audio(&artifact.samples, artifact.sample_rate_hz),
                "rtty" => decode_rtty_audio(&artifact.samples, artifact.sample_rate_hz as f32),
                "navtex" => decode_navtex_audio(&artifact.samples, artifact.sample_rate_hz as f32),
                _ => anyhow::bail!("unsupported audio protocol {}", entry.protocol),
            })
        }
        "nrz_bits" => {
            let artifact: RecordedNrzArtifact = serde_json::from_str(&text)?;
            Ok(match entry.protocol.as_str() {
                "aprs" => decode_aprs_nrz_bits(&artifact.bits),
                _ => anyhow::bail!("unsupported nrz protocol {}", entry.protocol),
            })
        }
        other => anyhow::bail!("unsupported fixture kind {}", other),
    }
}

pub fn assert_expectation(
    messages: &[DecodedMessage],
    expected: &FixtureExpectation,
) -> anyhow::Result<()> {
    if messages.len() < expected.message_count_min {
        anyhow::bail!(
            "expected at least {} messages, got {}",
            expected.message_count_min,
            messages.len()
        );
    }
    if let Some(icao) = &expected.icao {
        if !messages
            .iter()
            .any(|m| m.address.eq_ignore_ascii_case(icao))
        {
            anyhow::bail!("expected ICAO {icao} in {:?}", messages);
        }
    }
    if let Some(mmsi) = expected.mmsi {
        if !messages.iter().any(|m| m.address == mmsi.to_string()) {
            anyhow::bail!("expected MMSI {mmsi} in {:?}", messages);
        }
    }
    if let Some(source) = &expected.source {
        if !messages.iter().any(|m| m.address == *source) {
            anyhow::bail!("expected source {source} in {:?}", messages);
        }
    }
    if let Some(info) = &expected.info_contains {
        if !messages
            .iter()
            .any(|m| m.content.contains(info) || m.raw.contains(info))
        {
            anyhow::bail!("expected info containing {info} in {:?}", messages);
        }
    }
    if !messages.is_empty() && messages[0].protocol != expected.protocol {
        anyhow::bail!("protocol mismatch");
    }
    Ok(())
}

pub fn write_canonical_fixtures(root: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(root)?;

    let adsb_rate = 2_000_000u32;
    let adsb_iq = crate::adsb::synthesize_df17_iq([0xAB, 0xCD, 0xEF], adsb_rate);
    let adsb_artifact = RecordedIqArtifact {
        sample_rate_hz: adsb_rate,
        iq: adsb_iq.iter().map(|c| [c.re, c.im]).collect(),
    };
    let adsb_path = root.join("adsb-df17-abcdef.iq.json");
    std::fs::write(&adsb_path, serde_json::to_string_pretty(&adsb_artifact)?)?;
    let adsb_sha = hex::encode(Sha256::digest(std::fs::read(&adsb_path)?));

    let ais_rate = 48_000.0;
    let ais_pairs = crate::ais::synthesize_type1_position_iq(ais_rate);
    let ais_artifact = RecordedIqArtifact {
        sample_rate_hz: ais_rate as u32,
        iq: ais_pairs.iter().map(|(i, q)| [*i, *q]).collect(),
    };
    let ais_path = root.join("ais-type1-position.iq.json");
    std::fs::write(&ais_path, serde_json::to_string_pretty(&ais_artifact)?)?;
    let ais_sha = hex::encode(Sha256::digest(std::fs::read(&ais_path)?));

    let aprs_bits: Vec<u8> = {
        let mut nrz_bits: Vec<u8> = vec![0, 1, 1, 1, 1, 1, 1, 0];
        let payload = crate::aprs::hello_world_ax25_payload();
        for &byte in &payload {
            for bit in (0..8).rev() {
                nrz_bits.push((byte >> bit) & 1);
            }
        }
        nrz_bits.extend([0, 1, 1, 1, 1, 1, 1, 0]);
        nrz_bits
    };
    let aprs_nrz_artifact = RecordedNrzArtifact {
        baud: 1200,
        bits: aprs_bits,
    };
    let aprs_path = root.join("aprs-w1aw-hello.nrz.json");
    std::fs::write(
        &aprs_path,
        serde_json::to_string_pretty(&aprs_nrz_artifact)?,
    )?;
    let aprs_sha = hex::encode(Sha256::digest(std::fs::read(&aprs_path)?));

    let pocsag_rate = 24_000u32;
    let pocsag_samples = crate::pocsag::synthesize_alphanumeric_audio(
        pocsag_rate,
        crate::pocsag::PocsagBaud::Baud1200,
        42_002,
        3,
        "HELLO",
    );
    let pocsag_artifact = RecordedAudioArtifact {
        sample_rate_hz: pocsag_rate,
        samples: pocsag_samples,
    };
    let pocsag_path = root.join("pocsag-hello.audio.json");
    std::fs::write(
        &pocsag_path,
        serde_json::to_string_pretty(&pocsag_artifact)?,
    )?;
    let pocsag_sha = hex::encode(Sha256::digest(std::fs::read(&pocsag_path)?));

    let rtty_rate = 8_000.0;
    let rtty_artifact = RecordedAudioArtifact {
        sample_rate_hz: rtty_rate as u32,
        samples: crate::demod::synthesize_rtty_hello(rtty_rate),
    };
    let rtty_path = root.join("rtty-hello.audio.json");
    std::fs::write(&rtty_path, serde_json::to_string_pretty(&rtty_artifact)?)?;
    let rtty_sha = hex::encode(Sha256::digest(std::fs::read(&rtty_path)?));

    let navtex_artifact = RecordedAudioArtifact {
        sample_rate_hz: rtty_rate as u32,
        samples: crate::demod::synthesize_navtex_hello(rtty_rate),
    };
    let navtex_path = root.join("navtex-hello.audio.json");
    std::fs::write(
        &navtex_path,
        serde_json::to_string_pretty(&navtex_artifact)?,
    )?;
    let navtex_sha = hex::encode(Sha256::digest(std::fs::read(&navtex_path)?));

    let manifest = RecordedFixtureManifest {
        schema: "pulsescope.recorded-iq-fixture".into(),
        version: 1,
        fixtures: vec![
            RecordedFixtureEntry {
                id: "adsb-df17-abcdef".into(),
                protocol: "adsb".into(),
                file: "adsb-df17-abcdef.iq.json".into(),
                kind: "iq".into(),
                expected: FixtureExpectation {
                    message_count_min: 1,
                    protocol: "adsb".into(),
                    icao: Some("ABCDEF".into()),
                    mmsi: None,
                    source: None,
                    info_contains: None,
                },
                sha256: adsb_sha,
            },
            RecordedFixtureEntry {
                id: "ais-type1-position".into(),
                protocol: "ais".into(),
                file: "ais-type1-position.iq.json".into(),
                kind: "iq".into(),
                expected: FixtureExpectation {
                    message_count_min: 1,
                    protocol: "ais".into(),
                    icao: None,
                    mmsi: Some(366_123_456),
                    source: None,
                    info_contains: None,
                },
                sha256: ais_sha,
            },
            RecordedFixtureEntry {
                id: "aprs-w1aw-hello".into(),
                protocol: "aprs".into(),
                file: "aprs-w1aw-hello.nrz.json".into(),
                kind: "nrz_bits".into(),
                expected: FixtureExpectation {
                    message_count_min: 1,
                    protocol: "aprs".into(),
                    icao: None,
                    mmsi: None,
                    source: Some("W1AW".into()),
                    info_contains: Some("Hello world".into()),
                },
                sha256: aprs_sha,
            },
            RecordedFixtureEntry {
                id: "pocsag-hello".into(),
                protocol: "pocsag".into(),
                file: "pocsag-hello.audio.json".into(),
                kind: "audio".into(),
                expected: FixtureExpectation {
                    message_count_min: 1,
                    protocol: "pocsag".into(),
                    icao: None,
                    mmsi: None,
                    source: Some("42002".into()),
                    info_contains: Some("HELLO".into()),
                },
                sha256: pocsag_sha,
            },
            RecordedFixtureEntry {
                id: "rtty-hello".into(),
                protocol: "rtty".into(),
                file: "rtty-hello.audio.json".into(),
                kind: "audio".into(),
                expected: FixtureExpectation {
                    message_count_min: 1,
                    protocol: "rtty".into(),
                    icao: None,
                    mmsi: None,
                    source: None,
                    info_contains: Some("HELLO".into()),
                },
                sha256: rtty_sha,
            },
            RecordedFixtureEntry {
                id: "navtex-hello".into(),
                protocol: "navtex".into(),
                file: "navtex-hello.audio.json".into(),
                kind: "audio".into(),
                expected: FixtureExpectation {
                    message_count_min: 1,
                    protocol: "navtex".into(),
                    icao: None,
                    mmsi: None,
                    source: None,
                    info_contains: Some("HELLO".into()),
                },
                sha256: navtex_sha,
            },
        ],
    };
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_synthesis_passes_e2e_expectations() {
        let root = fixture_root();
        write_canonical_fixtures(&root).expect("write fixtures");
        let manifest = load_manifest(&root).expect("manifest");
        assert_eq!(manifest.schema, "pulsescope.recorded-iq-fixture");
        for entry in &manifest.fixtures {
            let messages = run_recorded_fixture(&root, entry).expect("run fixture");
            assert_expectation(&messages, &entry.expected).expect("expectations");
        }
    }

    #[test]
    fn adsb_fixture_decodes_to_normalized_event() {
        let rate = 2_000_000;
        let iq = crate::adsb::synthesize_df17_iq([0xAB, 0xCD, 0xEF], rate);
        let messages = decode_adsb_iq(&iq, rate);
        assert!(messages.iter().any(|m| m.address == "ABCDEF"));
    }

    #[test]
    fn ais_fixture_decodes_to_normalized_event() {
        let pairs = crate::ais::synthesize_type1_position_iq(48_000.0);
        let messages = decode_ais_iq(&pairs, 48_000);
        assert!(messages.iter().any(|m| m.address == "366123456"));
    }

    #[test]
    fn aprs_fixture_decodes_to_normalized_event() {
        let mut nrz_bits: Vec<u8> = vec![0, 1, 1, 1, 1, 1, 1, 0];
        let payload = crate::aprs::hello_world_ax25_payload();
        for &byte in &payload {
            for bit in (0..8).rev() {
                nrz_bits.push((byte >> bit) & 1);
            }
        }
        nrz_bits.extend([0, 1, 1, 1, 1, 1, 1, 0]);
        let messages = decode_aprs_nrz_bits(&nrz_bits);
        assert!(messages.iter().any(|m| m.address == "W1AW"));
        assert!(messages.iter().any(|m| m.content.contains("Hello world")));
    }
}
