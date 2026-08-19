//! Automatic decode routing from live IQ captures.
//!
//! When the scanner detects a signal (or a VFO opens squelch), classify with
//! demodulated audio and run native decoders without per-protocol UI toggles.

use rustfft::num_complex::Complex;

use crate::ais::IqDecoder as AisIqDecoder;
use crate::aviation::{AcarsIqDecoder, BitOrder, UatIqDecoder, Vdl2IqDecoder};
use crate::db::DecodedMessage;
use crate::aprs::AprsDecoder;
use crate::demod::{decode_cw, decode_navtex, decode_rds, decode_rtty, demodulate, low_pass_complex, mix_down, Mode};
use crate::pocsag::{IqDecoder as PocsagIqDecoder, PocsagBaud};
use crate::signal_id::{classify, Classification};

const TARGET_AGC_RMS: f32 = 0.18;

pub fn try_decode_signal(
    iq: &[Complex<f32>],
    device_center_hz: u64,
    sample_rate: u32,
    frequency_hz: u64,
    channel_bw_hz: u32,
    mode: &str,
    range_name: &str,
    snr_db: f32,
    threshold: f32,
    timestamp_ms: i64,
    use_arrl_bandplan: bool,
) -> Vec<DecodedMessage> {
    if iq.len() < 2048 || sample_rate == 0 {
        return Vec::new();
    }

    let demod_mode = if use_arrl_bandplan {
        crate::arrl_bandplan::recommended_mode(frequency_hz, mode)
    } else {
        mode
    };
    let demod_rate = sample_rate.min(500_000).max(8_000);
    let demod_audio = extract_demod_audio(
        iq,
        device_center_hz,
        frequency_hz,
        sample_rate,
        demod_rate,
        demod_mode,
    );
    let classification = classify(
        frequency_hz,
        channel_bw_hz,
        demod_mode,
        range_name,
        snr_db,
        Some((&demod_audio, demod_rate as f32)),
    );

    let mut out = Vec::new();
    if classification.decode_success {
        out.push(decoded_from_classification(&classification, frequency_hz, timestamp_ms));
    }

    let channel_iq = extract_channel_iq(iq, device_center_hz, frequency_hz, sample_rate, demod_mode);
    let mut tried = std::collections::HashSet::new();
    for candidate in classification.candidates.iter().filter(|c| c.confidence >= threshold) {
        if candidate.decoder == "none" || tried.contains(&candidate.decoder) {
            continue;
        }
        tried.insert(candidate.decoder.clone());
        out.extend(run_native_decoder(
            &candidate.decoder,
            &channel_iq,
            sample_rate,
            &demod_audio,
            demod_rate,
            frequency_hz,
            &classification,
            timestamp_ms,
        ));
    }

    // ARRL weak-signal slots: try CW + RTTY even if top candidate was voice.
    if use_arrl_bandplan {
        for decoder in ["native_cw", "native_rtty", "native_aprs"] {
            if tried.contains(decoder) {
                continue;
            }
            tried.insert(decoder.to_string());
            out.extend(run_native_decoder(
                decoder,
                &channel_iq,
                sample_rate,
                &demod_audio,
                demod_rate,
                frequency_hz,
                &classification,
                timestamp_ms,
            ));
        }
    }

    out
}

fn decoded_from_classification(
    c: &Classification,
    frequency_hz: u64,
    timestamp_ms: i64,
) -> DecodedMessage {
    DecodedMessage {
        id: None,
        frequency_hz,
        protocol: c.decode_protocol.clone(),
        message_type: c.signal_class.clone(),
        address: String::new(),
        function_code: c.sub_protocol.clone(),
        content: c.decode_summary.clone(),
        raw: c.features.join("; "),
        encryption: "none".into(),
        timestamp_ms,
    }
}

pub fn extract_channel_iq(
    iq: &[Complex<f32>],
    device_center_hz: u64,
    target_hz: u64,
    sample_rate: u32,
    mode: &str,
) -> Vec<(f32, f32)> {
    let mut phase = 0.0;
    let offset = target_hz as f64 - device_center_hz as f64;
    let mixed = mix_down(iq, offset, sample_rate, &mut phase);
    let parsed = Mode::parse(mode);
    let cutoff = match parsed {
        Mode::Wfm => 100_000.0,
        Mode::Nfm => 12_500.0,
        _ => 5_000.0,
    };
    let mut filter = Complex::new(0.0, 0.0);
    let filtered = low_pass_complex(&mixed, cutoff, sample_rate, &mut filter);
    filtered.iter().map(|c| (c.re, c.im)).collect()
}

pub fn extract_demod_audio(
    iq: &[Complex<f32>],
    device_center_hz: u64,
    target_hz: u64,
    sample_rate: u32,
    output_rate: u32,
    mode: &str,
) -> Vec<f32> {
    let mut phase = 0.0;
    let offset = target_hz as f64 - device_center_hz as f64;
    let mixed = mix_down(iq, offset, sample_rate, &mut phase);
    let parsed = Mode::parse(mode);
    let cutoff = match parsed {
        Mode::Wfm => 100_000.0,
        Mode::Nfm => 12_500.0,
        _ => 5_000.0,
    };
    let mut filter = Complex::new(0.0, 0.0);
    let filtered = low_pass_complex(&mixed, cutoff, sample_rate, &mut filter);
    let mut prev = None;
    let mut pcm = demodulate(parsed, &filtered, &mut prev);
    apply_agc(&mut pcm, TARGET_AGC_RMS);
  if output_rate != sample_rate && output_rate > 0 {
        let factor = (sample_rate / output_rate).max(1) as usize;
        pcm = crate::demod::decimate_average(&pcm, factor);
    }
    pcm
}

fn apply_agc(samples: &mut [f32], target_rms: f32) {
    if samples.is_empty() {
        return;
    }
    let rms = (samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32).sqrt();
    if rms > 1e-5 {
        let gain = (target_rms / rms).min(10.0);
        for sample in samples.iter_mut() {
            *sample *= gain;
        }
    }
}

pub fn run_native_decoder(
    decoder: &str,
    channel_iq: &[(f32, f32)],
    sample_rate: u32,
    demod_audio: &[f32],
    demod_rate: u32,
    frequency_hz: u64,
    classification: &Classification,
    timestamp_ms: i64,
) -> Vec<DecodedMessage> {
    let mut out = Vec::new();
    match decoder {
        "native_ais" => {
            if let Ok(mut dec) = AisIqDecoder::new(sample_rate as f64) {
                for result in dec.push_iq(channel_iq) {
                    if let Ok(msg) = result {
                        let content = serde_json::to_string(&msg).unwrap_or_else(|_| "ais".into());
                        out.push(DecodedMessage {
                            id: None,
                            frequency_hz,
                            protocol: "ais".into(),
                            message_type: "position".into(),
                            address: String::new(),
                            function_code: String::new(),
                            content,
                            raw: String::new(),
                            encryption: "none".into(),
                            timestamp_ms,
                        });
                    }
                }
            }
        }
        "native_pocsag" | "multimon-ng" => {
            for baud in [PocsagBaud::Baud1200, PocsagBaud::Baud2400] {
                let mut dec = PocsagIqDecoder::new(sample_rate, baud);
                let mut messages = dec.push_iq(channel_iq);
                messages.extend(dec.flush());
                for m in messages {
                    out.push(DecodedMessage {
                        id: None,
                        frequency_hz,
                        protocol: "pocsag".into(),
                        message_type: format!("{:?}", m.encoding).to_lowercase(),
                        address: m.ric.to_string(),
                        function_code: format!("fn{}", m.function),
                        content: m.text.clone(),
                        raw: String::new(),
                        encryption: "none".into(),
                        timestamp_ms,
                    });
                }
            }
        }
        "native_acars" => {
            let mut dec = AcarsIqDecoder::new(sample_rate, BitOrder::MsbFirst, false);
            dec.push_iq(channel_iq);
            for m in dec.take_messages() {
                let raw = m
                    .raw_bytes
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join("");
                out.push(DecodedMessage {
                    id: None,
                    frequency_hz,
                    protocol: "acars".into(),
                    message_type: m.label.clone().unwrap_or_else(|| "acars".into()),
                    address: m.registration.clone().unwrap_or_default(),
                    function_code: m.mode.map(|c| c.to_string()).unwrap_or_default(),
                    content: m.text.clone(),
                    raw,
                    encryption: "none".into(),
                    timestamp_ms,
                });
            }
        }
        "native_vdl2" => {
            let mut dec = Vdl2IqDecoder::new(sample_rate);
            dec.push_iq(channel_iq);
            for m in dec.take_messages() {
                let raw = m
                    .raw_frame
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join("");
                out.push(DecodedMessage {
                    id: None,
                    frequency_hz,
                    protocol: "vdl2".into(),
                    message_type: "vdl2".into(),
                    address: String::new(),
                    function_code: if m.fcs_valid { "fcs_ok" } else { "fcs_bad" }.into(),
                    content: format!("{} payload bytes", m.payload.len()),
                    raw,
                    encryption: "none".into(),
                    timestamp_ms,
                });
            }
        }
        "native_uat978" | "uat978" => {
            let mut dec = UatIqDecoder::new(sample_rate);
            dec.push_iq(channel_iq);
            for m in dec.take_messages() {
                let raw = m
                    .raw_codeword
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join("");
                out.push(DecodedMessage {
                    id: None,
                    frequency_hz,
                    protocol: "uat978".into(),
                    message_type: format!("{:?}", m.frame_kind).to_lowercase(),
                    address: m.address_hex.clone().unwrap_or_default(),
                    function_code: m
                        .message_code
                        .map(|c| format!("code_{c}"))
                        .unwrap_or_default(),
                    content: format!("{} payload bytes", m.payload.len()),
                    raw,
                    encryption: "none".into(),
                    timestamp_ms,
                });
            }
        }
        "native_wfm_rds" => {
            if let Some(rds) = decode_rds(demod_audio, demod_rate as f32) {
                let content = rds
                    .radio_text
                    .clone()
                    .or(rds.program_service.clone())
                    .unwrap_or_else(|| "RDS".into());
                out.push(DecodedMessage {
                    id: None,
                    frequency_hz,
                    protocol: "rds".into(),
                    message_type: "rds".into(),
                    address: rds.pi_code.map(|pi| format!("{pi:04X}")).unwrap_or_default(),
                    function_code: String::new(),
                    content,
                    raw: String::new(),
                    encryption: "none".into(),
                    timestamp_ms,
                });
            }
        }
        "native_navtex" => {
            if let Some(text) = decode_navtex(demod_audio, demod_rate as f32) {
                out.push(DecodedMessage {
                    id: None,
                    frequency_hz,
                    protocol: "navtex".into(),
                    message_type: "navtex".into(),
                    address: String::new(),
                    function_code: String::new(),
                    content: text,
                    raw: String::new(),
                    encryption: "none".into(),
                    timestamp_ms,
                });
            }
        }
        "rtl_433" => {
            if classification.top_confidence >= 0.55 {
                out.push(DecodedMessage {
                    id: None,
                    frequency_hz,
                    protocol: classification.sub_protocol.clone(),
                    message_type: "auto_detect".into(),
                    address: String::new(),
                    function_code: decoder.into(),
                    content: format!(
                        "Auto-detected {} — rtl_433 sidecar is decoding the live IQ feed",
                        classification.top_family
                    ),
                    raw: classification.features.join("; "),
                    encryption: "none".into(),
                    timestamp_ms,
                });
            }
        }
        "native_cw" | "cw" => {
            for tone in [700.0, 600.0, 800.0] {
                if let Some(text) = decode_cw(demod_audio, demod_rate as f32, tone) {
                    if text.chars().filter(|c| c.is_alphanumeric()).count() >= 2 {
                        out.push(DecodedMessage {
                            id: None,
                            frequency_hz,
                            protocol: "cw".into(),
                            message_type: "cw".into(),
                            address: String::new(),
                            function_code: format!("{tone:.0}hz"),
                            content: text,
                            raw: String::new(),
                            encryption: "none".into(),
                            timestamp_ms,
                        });
                        break;
                    }
                }
            }
        }
        "native_rtty" | "rtty" | "digital_weak" => {
            let profiles: [(f32, f32, f32); 3] = [
                (170.0, 2125.0, 45.0),
                (1275.0, 1445.0, 45.0),
                (1615.0, 1785.0, 50.0),
            ];
            for (mark, space, baud) in profiles {
                if let Some(text) = decode_rtty(demod_audio, demod_rate as f32, mark, space, baud) {
                    if text.trim().len() >= 3 {
                        out.push(DecodedMessage {
                            id: None,
                            frequency_hz,
                            protocol: "rtty".into(),
                            message_type: "rtty".into(),
                            address: String::new(),
                            function_code: format!("{mark:.0}/{space:.0}"),
                            content: text,
                            raw: String::new(),
                            encryption: "none".into(),
                            timestamp_ms,
                        });
                        break;
                    }
                }
            }
        }
        "native_aprs" | "direwolf" => {
            let mut dec = AprsDecoder::new(demod_rate as f32);
            for &sample in demod_audio {
                dec.feed(sample);
            }
            for frame in dec.frames {
                out.push(DecodedMessage {
                    id: None,
                    frequency_hz,
                    protocol: "aprs".into(),
                    message_type: "aprs".into(),
                    address: frame.source.clone(),
                    function_code: frame.dest.clone(),
                    content: frame.info.clone(),
                    raw: String::new(),
                    encryption: "none".into(),
                    timestamp_ms,
                });
            }
        }
        _ => {}
    }
    out
}
