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

pub fn decode_rds_audio(samples: &[f32], sample_rate_hz: f32) -> Vec<DecodedMessage> {
    crate::demod::decode_rds(samples, sample_rate_hz)
        .filter(|r| r.pi_code.is_some() && r.groups_found > 0)
        .into_iter()
        .map(|r| DecodedMessage {
            id: None,
            frequency_hz: 100_700_000,
            protocol: "rds".into(),
            message_type: "group".into(),
            address: r.pi_code.map(|pi| format!("{pi:04X}")).unwrap_or_default(),
            function_code: r.pty.map(|pty| format!("PTY{pty}")).unwrap_or_default(),
            content: r.program_service.clone().unwrap_or_default(),
            raw: r.program_service.unwrap_or_default(),
            encryption: "none".into(),
            timestamp_ms: crate::scanner::now_ms(),
        })
        .collect()
}

pub fn decode_cw_audio(samples: &[f32], sample_rate_hz: f32) -> Vec<DecodedMessage> {
    crate::demod::decode_cw(samples, sample_rate_hz, 700.0)
        .into_iter()
        .map(|text| DecodedMessage {
            id: None,
            frequency_hz: 14_020_000,
            protocol: "cw".into(),
            message_type: "morse".into(),
            address: String::new(),
            function_code: String::new(),
            content: text.clone(),
            raw: text,
            encryption: "none".into(),
            timestamp_ms: crate::scanner::now_ms(),
        })
        .collect()
}

pub fn decode_ble_iq(iq: &[Complex<f32>], sample_rate_hz: u32) -> Vec<DecodedMessage> {
    crate::ble::decode_iq(iq, sample_rate_hz)
        .into_iter()
        .filter(|adv| adv.crc_valid)
        .map(|adv| adv.to_decoded(2_402_000_000))
        .collect()
}

pub fn decode_lora_iq(iq: &[Complex<f32>], sample_rate_hz: u32) -> Vec<DecodedMessage> {
    crate::lora::decode_iq(iq, sample_rate_hz)
        .into_iter()
        .map(|packet| packet.to_decoded(915_000_000))
        .collect()
}

pub fn decode_tsbk_iq(iq: &[Complex<f32>], sample_rate_hz: u32) -> Vec<DecodedMessage> {
    crate::trunking::observe_control_channel(iq, sample_rate_hz)
        .grants
        .into_iter()
        .map(|grant| crate::trunking::grant_to_decoded(&grant, 851_012_500))
        .collect()
}

pub fn decode_uat_bits(bits: &[u8]) -> Vec<DecodedMessage> {
    let bools: Vec<bool> = bits.iter().map(|bit| *bit != 0).collect();
    let mut decoder = crate::aviation::UatDecoder::new();
    decoder.feed_bits(&bools);
    decoder
        .take_messages()
        .into_iter()
        .map(|m| DecodedMessage {
            id: None,
            frequency_hz: 978_000_000,
            protocol: "uat".into(),
            message_type: format!("{:?}", m.frame_kind).to_ascii_lowercase(),
            address: m.address_hex.clone().unwrap_or_default(),
            function_code: m
                .message_code
                .map(|code| format!("MC{code}"))
                .unwrap_or_default(),
            content: m
                .payload
                .iter()
                .filter(|b| (0x20..=0x7e).contains(*b))
                .map(|b| *b as char)
                .collect(),
            raw: hex::encode(&m.raw_codeword),
            encryption: "none".into(),
            timestamp_ms: crate::scanner::now_ms(),
        })
        .collect()
}

pub fn decode_acars_bits(bits: &[u8]) -> Vec<DecodedMessage> {
    let bools: Vec<bool> = bits.iter().map(|bit| *bit != 0).collect();
    let mut decoder = crate::aviation::AcarsDecoder::default();
    decoder.feed_bits(&bools);
    decoder
        .take_messages()
        .into_iter()
        .filter(|m| m.crc_valid)
        .map(|m| DecodedMessage {
            id: None,
            frequency_hz: 131_550_000,
            protocol: "acars".into(),
            message_type: m.label.clone().unwrap_or_else(|| "acars".into()),
            address: m.registration.clone().unwrap_or_default(),
            function_code: m.block_id.map(|c| c.to_string()).unwrap_or_default(),
            content: m.text.clone(),
            raw: hex::encode(&m.raw_bytes),
            encryption: "none".into(),
            timestamp_ms: crate::scanner::now_ms(),
        })
        .collect()
}

pub fn decode_vdl2_bits(bits: &[u8]) -> Vec<DecodedMessage> {
    let bools: Vec<bool> = bits.iter().map(|bit| *bit != 0).collect();
    let mut decoder = crate::aviation::Vdl2Decoder::new();
    decoder.feed_bits(&bools);
    decoder
        .take_messages()
        .into_iter()
        .filter(|m| m.fcs_valid)
        .map(|m| DecodedMessage {
            id: None,
            frequency_hz: 136_975_000,
            protocol: "vdl2".into(),
            message_type: "avlc".into(),
            address: String::new(),
            function_code: String::new(),
            content: m
                .payload
                .iter()
                .filter(|b| (0x20..=0x7e).contains(*b))
                .map(|b| *b as char)
                .collect(),
            raw: hex::encode(&m.raw_frame),
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
                "ble" => decode_ble_iq(&iq, artifact.sample_rate_hz),
                "meshtastic" | "meshcore" | "reticulum" | "modbus-lora" | "lorawan" | "lora" => {
                    decode_lora_iq(&iq, artifact.sample_rate_hz)
                }
                "p25-tsbk" => decode_tsbk_iq(&iq, artifact.sample_rate_hz),
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
                "rds" => decode_rds_audio(&artifact.samples, artifact.sample_rate_hz as f32),
                "cw" => decode_cw_audio(&artifact.samples, artifact.sample_rate_hz as f32),
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
        "bits" => {
            let artifact: RecordedNrzArtifact = serde_json::from_str(&text)?;
            Ok(match entry.protocol.as_str() {
                "uat" => decode_uat_bits(&artifact.bits),
                "acars" => decode_acars_bits(&artifact.bits),
                "vdl2" => decode_vdl2_bits(&artifact.bits),
                _ => anyhow::bail!("unsupported bits protocol {}", entry.protocol),
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

fn write_iq_artifact(
    root: &Path,
    file: &str,
    sample_rate_hz: u32,
    iq: &[Complex<f32>],
) -> anyhow::Result<String> {
    let artifact = RecordedIqArtifact {
        sample_rate_hz,
        iq: iq.iter().map(|c| [c.re, c.im]).collect(),
    };
    let path = root.join(file);
    std::fs::write(&path, serde_json::to_string_pretty(&artifact)?)?;
    Ok(hex::encode(Sha256::digest(std::fs::read(&path)?)))
}

fn iq_entry(
    id: &str,
    protocol: &str,
    file: &str,
    sha256: String,
    source: Option<&str>,
    info_contains: Option<&str>,
) -> RecordedFixtureEntry {
    RecordedFixtureEntry {
        id: id.into(),
        protocol: protocol.into(),
        file: file.into(),
        kind: "iq".into(),
        expected: FixtureExpectation {
            message_count_min: 1,
            protocol: protocol.into(),
            icao: None,
            mmsi: None,
            source: source.map(str::to_owned),
            info_contains: info_contains.map(str::to_owned),
        },
        sha256,
    }
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

    let uat_bits_artifact = RecordedNrzArtifact {
        baud: 1_041_667,
        bits: crate::aviation::synthesize_uat_abcdef_bits(),
    };
    let uat_path = root.join("uat-abcdef.bits.json");
    std::fs::write(&uat_path, serde_json::to_string_pretty(&uat_bits_artifact)?)?;
    let uat_sha = hex::encode(Sha256::digest(std::fs::read(&uat_path)?));

    let acars_bits_artifact = RecordedNrzArtifact {
        baud: 2400,
        bits: crate::aviation::synthesize_acars_hello_bits(),
    };
    let acars_path = root.join("acars-n123ab-hello.bits.json");
    std::fs::write(
        &acars_path,
        serde_json::to_string_pretty(&acars_bits_artifact)?,
    )?;
    let acars_sha = hex::encode(Sha256::digest(std::fs::read(&acars_path)?));

    let vdl2_bits_artifact = RecordedNrzArtifact {
        baud: 31_500,
        bits: crate::aviation::synthesize_vdl2_hello_bits(),
    };
    let vdl2_path = root.join("vdl2-hello.bits.json");
    std::fs::write(
        &vdl2_path,
        serde_json::to_string_pretty(&vdl2_bits_artifact)?,
    )?;
    let vdl2_sha = hex::encode(Sha256::digest(std::fs::read(&vdl2_path)?));

    let rds_artifact = RecordedAudioArtifact {
        sample_rate_hz: 190_000,
        samples: crate::demod::synthesize_rds_hello_multiplex(190_000.0),
    };
    let rds_path = root.join("rds-beef-hello.audio.json");
    std::fs::write(&rds_path, serde_json::to_string_pretty(&rds_artifact)?)?;
    let rds_sha = hex::encode(Sha256::digest(std::fs::read(&rds_path)?));

    let cw_artifact = RecordedAudioArtifact {
        sample_rate_hz: 8_000,
        samples: crate::demod::synthesize_cw_sos(8_000.0, 700.0),
    };
    let cw_path = root.join("cw-sos.audio.json");
    std::fs::write(&cw_path, serde_json::to_string_pretty(&cw_artifact)?)?;
    let cw_sha = hex::encode(Sha256::digest(std::fs::read(&cw_path)?));

    let ble_rate = crate::ble::BLE_FIXTURE_SAMPLE_RATE_HZ;
    let ble_sha = write_iq_artifact(
        root,
        "ble-pulse-advert.iq.json",
        ble_rate,
        &crate::ble::synthesize_pulse_advert_iq(ble_rate),
    )?;

    let lora_rate = crate::lora::LORA_FIXTURE_SAMPLE_RATE_HZ;
    let lora_sha = write_iq_artifact(
        root,
        "lora-meshtastic-hello.iq.json",
        lora_rate,
        &crate::lora::synthesize_meshtastic_hello_iq(lora_rate),
    )?;
    let meshcore_sha = write_iq_artifact(
        root,
        "lora-meshcore-hello.iq.json",
        lora_rate,
        &crate::lora::synthesize_meshcore_hello_iq(lora_rate),
    )?;
    let reticulum_sha = write_iq_artifact(
        root,
        "lora-reticulum-announce.iq.json",
        lora_rate,
        &crate::lora::synthesize_reticulum_announce_iq(lora_rate),
    )?;
    let modbus_sha = write_iq_artifact(
        root,
        "lora-modbus-read.iq.json",
        lora_rate,
        &crate::lora::synthesize_modbus_read_iq(lora_rate),
    )?;
    let lorawan_sha = write_iq_artifact(
        root,
        "lora-lorawan-identify.iq.json",
        lora_rate,
        &crate::lora::synthesize_lorawan_identify_iq(lora_rate),
    )?;
    let lora_enc_sha = write_iq_artifact(
        root,
        "lora-meshtastic-encrypted.iq.json",
        lora_rate,
        &crate::lora::synthesize_meshtastic_encrypted_iq(lora_rate),
    )?;

    let tsbk_rate = crate::trunking::P25_FIR_RATE_HZ;
    let tsbk_sha = write_iq_artifact(
        root,
        "p25-tsbk-group-grant.iq.json",
        tsbk_rate,
        &crate::trunking::synthesize_tsbk_control_iq(tsbk_rate),
    )?;

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
            RecordedFixtureEntry {
                id: "uat-abcdef".into(),
                protocol: "uat".into(),
                file: "uat-abcdef.bits.json".into(),
                kind: "bits".into(),
                expected: FixtureExpectation {
                    message_count_min: 1,
                    protocol: "uat".into(),
                    icao: Some("ABCDEF".into()),
                    mmsi: None,
                    source: None,
                    info_contains: Some("HELLO".into()),
                },
                sha256: uat_sha,
            },
            RecordedFixtureEntry {
                id: "acars-n123ab-hello".into(),
                protocol: "acars".into(),
                file: "acars-n123ab-hello.bits.json".into(),
                kind: "bits".into(),
                expected: FixtureExpectation {
                    message_count_min: 1,
                    protocol: "acars".into(),
                    icao: None,
                    mmsi: None,
                    source: Some("N123AB".into()),
                    info_contains: Some("HELLO".into()),
                },
                sha256: acars_sha,
            },
            RecordedFixtureEntry {
                id: "vdl2-hello".into(),
                protocol: "vdl2".into(),
                file: "vdl2-hello.bits.json".into(),
                kind: "bits".into(),
                expected: FixtureExpectation {
                    message_count_min: 1,
                    protocol: "vdl2".into(),
                    icao: None,
                    mmsi: None,
                    source: None,
                    info_contains: Some("HELLO".into()),
                },
                sha256: vdl2_sha,
            },
            RecordedFixtureEntry {
                id: "rds-beef-hello".into(),
                protocol: "rds".into(),
                file: "rds-beef-hello.audio.json".into(),
                kind: "audio".into(),
                expected: FixtureExpectation {
                    message_count_min: 1,
                    protocol: "rds".into(),
                    icao: None,
                    mmsi: None,
                    source: Some("BEEF".into()),
                    info_contains: Some("HELLO".into()),
                },
                sha256: rds_sha,
            },
            RecordedFixtureEntry {
                id: "cw-sos".into(),
                protocol: "cw".into(),
                file: "cw-sos.audio.json".into(),
                kind: "audio".into(),
                expected: FixtureExpectation {
                    message_count_min: 1,
                    protocol: "cw".into(),
                    icao: None,
                    mmsi: None,
                    source: None,
                    info_contains: Some("S".into()),
                },
                sha256: cw_sha,
            },
            RecordedFixtureEntry {
                id: "ble-pulse-advert".into(),
                protocol: "ble".into(),
                file: "ble-pulse-advert.iq.json".into(),
                kind: "iq".into(),
                expected: FixtureExpectation {
                    message_count_min: 1,
                    protocol: "ble".into(),
                    icao: None,
                    mmsi: None,
                    source: Some("AA:BB:CC:DD:EE:FF".into()),
                    info_contains: Some("PULSE".into()),
                },
                sha256: ble_sha,
            },
            iq_entry(
                "lora-meshtastic-hello",
                "meshtastic",
                "lora-meshtastic-hello.iq.json",
                lora_sha,
                Some("00abcdef"),
                Some("HELLO"),
            ),
            iq_entry(
                "lora-meshcore-hello",
                "meshcore",
                "lora-meshcore-hello.iq.json",
                meshcore_sha,
                None,
                Some("HELLO"),
            ),
            iq_entry(
                "lora-reticulum-announce",
                "reticulum",
                "lora-reticulum-announce.iq.json",
                reticulum_sha,
                None,
                Some("PULSE"),
            ),
            iq_entry(
                "lora-modbus-read",
                "modbus-lora",
                "lora-modbus-read.iq.json",
                modbus_sha,
                Some("1"),
                Some("function 3"),
            ),
            iq_entry(
                "lora-lorawan-identify",
                "lorawan",
                "lora-lorawan-identify.iq.json",
                lorawan_sha,
                Some("44332211"),
                Some("not decrypted"),
            ),
            iq_entry(
                "lora-meshtastic-encrypted",
                "meshtastic",
                "lora-meshtastic-encrypted.iq.json",
                lora_enc_sha,
                Some("00abcdef"),
                Some("encrypted mesh packet"),
            ),
            iq_entry(
                "p25-tsbk-group-grant",
                "p25-tsbk",
                "p25-tsbk-group-grant.iq.json",
                tsbk_sha,
                Some("1234"),
                Some("56789"),
            ),
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

    #[test]
    fn uat_acars_vdl2_bit_fixtures_decode_to_normalized_events() {
        let uat = decode_uat_bits(&crate::aviation::synthesize_uat_abcdef_bits());
        assert!(uat.iter().any(|m| m.address == "ABCDEF"));
        assert!(uat.iter().any(|m| m.content.contains("HELLO")));

        let acars = decode_acars_bits(&crate::aviation::synthesize_acars_hello_bits());
        assert!(acars.iter().any(|m| m.address == "N123AB"));
        assert!(acars.iter().any(|m| m.content.contains("HELLO")));

        let vdl2 = decode_vdl2_bits(&crate::aviation::synthesize_vdl2_hello_bits());
        assert!(vdl2.iter().any(|m| m.content.contains("HELLO")));
    }

    #[test]
    fn rds_and_cw_audio_fixtures_decode_to_normalized_events() {
        let rds = decode_rds_audio(
            &crate::demod::synthesize_rds_hello_multiplex(190_000.0),
            190_000.0,
        );
        assert!(rds.iter().any(|m| m.address == "BEEF"));
        assert!(rds.iter().any(|m| m.content.contains("HELLO")));

        let cw = decode_cw_audio(&crate::demod::synthesize_cw_sos(8_000.0, 700.0), 8_000.0);
        assert!(cw.iter().any(|m| m.content.contains('S')));
        assert!(cw.iter().any(|m| m.content.contains('O')));
    }

    #[test]
    fn ble_and_lora_iq_fixtures_decode_to_normalized_events() {
        let ble = decode_ble_iq(
            &crate::ble::synthesize_pulse_advert_iq(crate::ble::BLE_FIXTURE_SAMPLE_RATE_HZ),
            crate::ble::BLE_FIXTURE_SAMPLE_RATE_HZ,
        );
        assert!(ble.iter().any(|m| m.address == "AA:BB:CC:DD:EE:FF"));
        assert!(ble.iter().any(|m| m.content.contains("PULSE")));

        let lora = decode_lora_iq(
            &crate::lora::synthesize_meshtastic_hello_iq(crate::lora::LORA_FIXTURE_SAMPLE_RATE_HZ),
            crate::lora::LORA_FIXTURE_SAMPLE_RATE_HZ,
        );
        assert!(lora.iter().any(|m| m.protocol == "meshtastic"));
        assert!(lora.iter().any(|m| m.content.contains("HELLO")));
    }
}
