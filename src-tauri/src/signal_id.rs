//! Auto signal identification for PulseScope.
//!
//! Combines three independent cues:
//!   1. Frequency-band priors (known allocations)
//!   2. Bandwidth / mode heuristics
//!   3. Optional audio feature detectors (CTCSS, DCS, DTMF, CW, RTTY, SSTV, RDS, APRS tones)
//!
//! Returns ranked candidates so the scanner can auto-route to the right decoder.

use serde::{Deserialize, Serialize};

use crate::demod::{
    decode_cw, decode_navtex, decode_rds, decode_rtty, detect_ctcss, detect_dcs, detect_dtmf,
    detect_sstv_mode, Mode,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtocolCandidate {
    pub protocol: String,
    pub family: String,
    pub confidence: f32,
    pub decoder: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Classification {
    pub frequency_hz: u64,
    pub bandwidth_hz: u32,
    pub mode: String,
    pub range_name: String,
    pub signal_class: String,
    pub top_family: String,
    pub top_confidence: f32,
    pub sub_protocol: String,
    pub symbol_rate: f32,
    pub decode_success: bool,
    pub decode_protocol: String,
    pub decode_summary: String,
    pub likely_proprietary: bool,
    pub is_novel: bool,
    pub candidates: Vec<ProtocolCandidate>,
    pub features: Vec<String>,
}

/// Band prior: (start_hz, end_hz, protocol, family, decoder, base_confidence, reason)
struct BandPrior {
    start: u64,
    end: u64,
    protocol: &'static str,
    family: &'static str,
    decoder: &'static str,
    confidence: f32,
    reason: &'static str,
    proprietary: bool,
}

const BAND_PRIORS: &[BandPrior] = &[
    BandPrior {
        start: 540_000,
        end: 1_700_000,
        protocol: "am_broadcast",
        family: "analog",
        decoder: "native_am",
        confidence: 0.75,
        reason: "AM broadcast band",
        proprietary: false,
    },
    BandPrior {
        start: 490_000,
        end: 518_000,
        protocol: "navtex",
        family: "marine",
        decoder: "native_navtex",
        confidence: 0.70,
        reason: "NAVTEX 490/518 kHz",
        proprietary: false,
    },
    BandPrior {
        start: 88_000_000,
        end: 108_000_000,
        protocol: "fm_broadcast",
        family: "analog",
        decoder: "native_wfm_rds",
        confidence: 0.85,
        reason: "FM broadcast band",
        proprietary: false,
    },
    BandPrior {
        start: 118_000_000,
        end: 137_000_000,
        protocol: "aircraft_am",
        family: "aviation",
        decoder: "native_am",
        confidence: 0.80,
        reason: "VHF airband",
        proprietary: false,
    },
    BandPrior {
        start: 129_000_000,
        end: 132_000_000,
        protocol: "acars",
        family: "aviation",
        decoder: "native_acars",
        confidence: 0.78,
        reason: "ACARS channel allocation",
        proprietary: false,
    },
    BandPrior {
        start: 136_000_000,
        end: 137_000_000,
        protocol: "vdl2",
        family: "aviation",
        decoder: "native_vdl2",
        confidence: 0.72,
        reason: "VDL Mode 2 band",
        proprietary: false,
    },
    BandPrior {
        start: 137_000_000,
        end: 138_000_000,
        protocol: "noaa_apt",
        family: "satellite",
        decoder: "noaa-apt",
        confidence: 0.82,
        reason: "NOAA APT band",
        proprietary: false,
    },
    BandPrior {
        start: 144_000_000,
        end: 148_000_000,
        protocol: "amateur_vhf",
        family: "amateur",
        decoder: "direwolf",
        confidence: 0.55,
        reason: "2m amateur (voice/APRS)",
        proprietary: false,
    },
    BandPrior {
        start: 144_390_000,
        end: 144_400_000,
        protocol: "aprs",
        family: "amateur",
        decoder: "direwolf",
        confidence: 0.90,
        reason: "APRS 144.390 MHz",
        proprietary: false,
    },
    BandPrior {
        start: 156_000_000,
        end: 162_000_000,
        protocol: "marine_vhf",
        family: "marine",
        decoder: "native_nfm",
        confidence: 0.70,
        reason: "Marine VHF",
        proprietary: false,
    },
    BandPrior {
        start: 161_975_000,
        end: 162_025_000,
        protocol: "ais",
        family: "marine",
        decoder: "native_ais",
        confidence: 0.92,
        reason: "AIS channels",
        proprietary: false,
    },
    BandPrior {
        start: 162_400_000,
        end: 162_550_000,
        protocol: "noaa_weather",
        family: "weather",
        decoder: "native_nfm",
        confidence: 0.88,
        reason: "NOAA Weather Radio",
        proprietary: false,
    },
    BandPrior {
        start: 400_000_000,
        end: 406_000_000,
        protocol: "radiosonde",
        family: "weather",
        decoder: "rtl_433",
        confidence: 0.75,
        reason: "Radiosonde band",
        proprietary: false,
    },
    BandPrior {
        start: 433_050_000,
        end: 434_790_000,
        protocol: "ism_433",
        family: "ism",
        decoder: "rtl_433",
        confidence: 0.80,
        reason: "ISM 433 MHz",
        proprietary: false,
    },
    BandPrior {
        start: 454_000_000,
        end: 461_000_000,
        protocol: "pocsag",
        family: "paging",
        decoder: "native_pocsag",
        confidence: 0.65,
        reason: "UHF pager allocation",
        proprietary: false,
    },
    BandPrior {
        start: 462_550_000,
        end: 467_725_000,
        protocol: "frs_gmrs",
        family: "land_mobile",
        decoder: "native_nfm",
        confidence: 0.70,
        reason: "FRS/GMRS",
        proprietary: false,
    },
    BandPrior {
        start: 851_000_000,
        end: 869_000_000,
        protocol: "p25_trunked",
        family: "land_mobile",
        decoder: "dsd-fme",
        confidence: 0.72,
        reason: "800 MHz trunked public safety",
        proprietary: false,
    },
    BandPrior {
        start: 902_000_000,
        end: 928_000_000,
        protocol: "ism_915",
        family: "ism",
        decoder: "rtl_433",
        confidence: 0.75,
        reason: "ISM 915 MHz",
        proprietary: false,
    },
    BandPrior {
        start: 929_000_000,
        end: 932_000_000,
        protocol: "pocsag",
        family: "paging",
        decoder: "multimon-ng",
        confidence: 0.78,
        reason: "900 MHz paging",
        proprietary: false,
    },
    BandPrior {
        start: 978_000_000,
        end: 978_200_000,
        protocol: "uat978",
        family: "aviation",
        decoder: "native_uat978",
        confidence: 0.90,
        reason: "ADS-B UAT 978",
        proprietary: false,
    },
    BandPrior {
        start: 1_090_000_000,
        end: 1_090_200_000,
        protocol: "adsb",
        family: "aviation",
        decoder: "native_adsb",
        confidence: 0.95,
        reason: "ADS-B 1090 MHz",
        proprietary: false,
    },
    BandPrior {
        start: 1_525_000_000,
        end: 1_559_000_000,
        protocol: "inmarsat",
        family: "satellite",
        decoder: "satdump",
        confidence: 0.70,
        reason: "Inmarsat L-band DL",
        proprietary: false,
    },
    BandPrior {
        start: 1_574_000_000,
        end: 1_577_000_000,
        protocol: "gps_l1",
        family: "satellite",
        decoder: "satdump",
        confidence: 0.80,
        reason: "GPS L1 / Galileo E1",
        proprietary: false,
    },
    BandPrior {
        start: 1_616_000_000,
        end: 1_626_500_000,
        protocol: "iridium",
        family: "satellite",
        decoder: "iridiumlive",
        confidence: 0.82,
        reason: "Iridium band",
        proprietary: false,
    },
    BandPrior {
        start: 1_691_000_000,
        end: 1_695_000_000,
        protocol: "goes_hrit",
        family: "satellite",
        decoder: "satdump",
        confidence: 0.88,
        reason: "GOES HRIT/LRIT",
        proprietary: false,
    },
    // HD Radio overlays the FM band; lower confidence, only if features suggest digital
    BandPrior {
        start: 88_000_000,
        end: 108_000_000,
        protocol: "hd_radio",
        family: "digital_broadcast",
        decoder: "nrsc5",
        confidence: 0.35,
        reason: "FM band may carry HD Radio",
        proprietary: false,
    },
    // European TETRA
    BandPrior {
        start: 380_000_000,
        end: 400_000_000,
        protocol: "tetra",
        family: "land_mobile",
        decoder: "tetraear",
        confidence: 0.65,
        reason: "TETRA allocation",
        proprietary: true,
    },
    BandPrior {
        start: 410_000_000,
        end: 430_000_000,
        protocol: "tetra",
        family: "land_mobile",
        decoder: "tetraear",
        confidence: 0.55,
        reason: "TETRA secondary",
        proprietary: true,
    },
];

/// Range-name keyword boosts.
fn range_boost(range_name: &str) -> Vec<(&'static str, f32)> {
    let r = range_name.to_ascii_lowercase();
    let mut out = Vec::new();
    let pairs: &[(&str, &str, f32)] = &[
        ("acars", "acars", 0.20),
        ("ais", "ais", 0.25),
        ("ads-b", "adsb", 0.25),
        ("adsb", "adsb", 0.25),
        ("uat", "uat978", 0.25),
        ("noaa apt", "noaa_apt", 0.25),
        ("noaa weather", "noaa_weather", 0.20),
        ("goes", "goes_hrit", 0.25),
        ("iridium", "iridium", 0.25),
        ("inmarsat", "inmarsat", 0.20),
        ("gps", "gps_l1", 0.20),
        ("radiosonde", "radiosonde", 0.20),
        ("pager", "pocsag", 0.20),
        ("marine", "marine_vhf", 0.10),
        ("aircraft", "aircraft_am", 0.15),
        ("fm broadcast", "fm_broadcast", 0.20),
        ("trunk", "p25_trunked", 0.15),
        ("ism", "ism_433", 0.10),
        ("frs", "frs_gmrs", 0.15),
        ("gmrs", "frs_gmrs", 0.15),
        ("aprs", "aprs", 0.25),
        ("amateur", "amateur_vhf", 0.08),
    ];
    for (kw, proto, boost) in pairs {
        if r.contains(kw) {
            out.push((*proto, *boost));
        }
    }
    out
}

fn bandwidth_hints(bw_hz: u32, mode: &str) -> Vec<(&'static str, f32, &'static str)> {
    let mut h = Vec::new();
    let m = mode.to_ascii_lowercase();
    if (150_000..=250_000).contains(&bw_hz) {
        h.push(("fm_broadcast", 0.15, "≈200 kHz FM channel"));
    }
    if bw_hz >= 800_000 {
        h.push(("adsb", 0.10, "very wide channel (pulse radar/ADS-B style)"));
    }
    if (10_000..=30_000).contains(&bw_hz) && (m == "nfm" || m == "fm" || m.is_empty()) {
        h.push(("analog_nfm", 0.10, "12.5–25 kHz NFM channel"));
        h.push(("ais", 0.05, "AIS uses 25 kHz NFM"));
        h.push(("p25_trunked", 0.05, "P25 uses ~12.5 kHz"));
    }
    if bw_hz <= 10_000 && (m == "usb" || m == "lsb") {
        h.push(("rtty", 0.10, "narrow SSB channel"));
        h.push(("cw", 0.08, "narrow CW channel"));
    }
    if (30_000..=50_000).contains(&bw_hz) {
        h.push(("noaa_apt", 0.12, "≈40 kHz APT channel"));
    }
    if bw_hz >= 1_000_000 {
        h.push(("goes_hrit", 0.10, "MHz-class satellite downlink"));
        h.push(("hd_radio", 0.05, "wide digital overlay"));
    }
    h
}

/// Analyse demodulated audio and return feature tags + protocol boosts.
pub fn analyse_audio(
    samples: &[f32],
    sample_rate: f32,
    mode: Mode,
) -> (Vec<String>, Vec<(&'static str, f32, String)>) {
    let mut features = Vec::new();
    let mut boosts = Vec::new();
    if samples.len() < (sample_rate as usize / 4).max(1024) {
        return (features, boosts);
    }

    // Energy / activity
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    if rms < 1e-4 {
        features.push("silence".into());
        return (features, boosts);
    }
    features.push(format!("rms={rms:.4}"));

    // CTCSS / DCS on NFM
    if matches!(mode, Mode::Nfm | Mode::Wfm) {
        if let Some((tone, snr)) = detect_ctcss(samples, sample_rate) {
            features.push(format!("ctcss={tone:.1}Hz@{snr:.1}dB"));
            boosts.push(("analog_nfm", 0.25, format!("CTCSS {tone:.1} Hz")));
        }
        if let Some(code) = detect_dcs(samples, sample_rate) {
            features.push(format!("dcs={code}"));
            boosts.push(("analog_nfm", 0.25, format!("DCS {code}")));
        }
    }

    // DTMF
    if let Some(digits) = detect_dtmf(samples, sample_rate) {
        if !digits.is_empty() {
            features.push(format!("dtmf={digits}"));
            boosts.push(("dtmf", 0.30, format!("DTMF {digits}")));
            boosts.push(("analog_nfm", 0.10, "DTMF on channel".into()));
        }
    }

    // CW (USB/LSB/NFM)
    if let Some(text) = decode_cw(samples, sample_rate, 700.0) {
        if text.chars().filter(|c| c.is_alphanumeric()).count() >= 2 {
            features.push(format!("cw={text}"));
            boosts.push(("cw", 0.35, format!("CW text: {text}")));
        }
    }

    // RTTY
    if let Some(text) = decode_rtty(samples, sample_rate, 2125.0, 2295.0, 45.45) {
        if text.chars().filter(|c| c.is_alphanumeric()).count() >= 3 {
            features.push(format!("rtty={text}"));
            boosts.push(("rtty", 0.40, format!("RTTY: {text}")));
        }
    }

    // NAVTEX
    if let Some(text) = decode_navtex(samples, sample_rate) {
        if text.chars().filter(|c| c.is_alphanumeric()).count() >= 3 {
            features.push(format!("navtex={text}"));
            boosts.push(("navtex", 0.45, format!("NAVTEX: {text}")));
        }
    }

    // SSTV mode detect
    if let Some(mode_name) = detect_sstv_mode(samples, sample_rate) {
        features.push(format!("sstv={mode_name:?}"));
        boosts.push(("sstv", 0.40, format!("SSTV mode {:?}", mode_name)));
    }

    // RDS on WFM multiplex
    if matches!(mode, Mode::Wfm) {
        if let Some(rds) = decode_rds(samples, sample_rate) {
            if rds.groups_found > 0 {
                features.push(format!("rds_groups={}", rds.groups_found));
                boosts.push((
                    "fm_broadcast",
                    0.30,
                    format!("RDS groups={} PI={:?}", rds.groups_found, rds.pi_code),
                ));
            }
        }
    }

    // APRS tone energy (1200/2200 Hz) — lightweight Goertzel-style proxy via energy bands
    let e1200 = tone_energy(samples, sample_rate, 1200.0);
    let e2200 = tone_energy(samples, sample_rate, 2200.0);
    let e_avg = tone_energy(samples, sample_rate, 1600.0).max(1e-9);
    if e1200 / e_avg > 3.0 && e2200 / e_avg > 2.0 {
        features.push("afsk1200_tones".into());
        boosts.push(("aprs", 0.25, "AFSK 1200/2200 Hz tone pair".into()));
        boosts.push(("pocsag", 0.05, "FSK-like audio".into()));
    }

    // Digital-voice-ish: high zero-crossing rate with no CTCSS → try dsd-fme
    let zcr = zero_crossing_rate(samples);
    if zcr > 0.15 && matches!(mode, Mode::Nfm) {
        features.push(format!("zcr={zcr:.3}"));
        boosts.push(("p25", 0.12, "high ZCR suggests digital voice".into()));
        boosts.push(("dmr", 0.10, "high ZCR suggests digital voice".into()));
    }

    (features, boosts)
}

fn tone_energy(samples: &[f32], sample_rate: f32, freq: f32) -> f32 {
    // Single-bin Goertzel
    let k = (0.5 + (samples.len() as f32 * freq / sample_rate)) as usize;
    let w = std::f32::consts::TAU * k as f32 / samples.len() as f32;
    let coeff = 2.0 * w.cos();
    let mut s1 = 0.0f32;
    let mut s2 = 0.0f32;
    for &x in samples {
        let s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - coeff * s1 * s2
}

fn zero_crossing_rate(samples: &[f32]) -> f32 {
    if samples.len() < 2 {
        return 0.0;
    }
    let mut crosses = 0usize;
    for w in samples.windows(2) {
        if w[0] == 0.0 {
            continue;
        }
        if (w[0] > 0.0) != (w[1] > 0.0) {
            crosses += 1;
        }
    }
    crosses as f32 / (samples.len() - 1) as f32
}

/// Classify a signal from frequency + optional context. Pure function — no I/O.
pub fn classify(
    frequency_hz: u64,
    bandwidth_hz: u32,
    mode: &str,
    range_name: &str,
    snr_db: f32,
    audio: Option<(&[f32], f32)>,
) -> Classification {
    use std::collections::HashMap;

    let mut scores: HashMap<String, (f32, String, String, String, bool)> = HashMap::new();
    // key -> (score, family, decoder, reason, proprietary)

    // 1. Band priors
    for b in BAND_PRIORS {
        if frequency_hz >= b.start && frequency_hz <= b.end {
            let entry = scores.entry(b.protocol.to_string()).or_insert((
                0.0,
                b.family.to_string(),
                b.decoder.to_string(),
                b.reason.to_string(),
                b.proprietary,
            ));
            if b.confidence > entry.0 {
                *entry = (
                    b.confidence,
                    b.family.to_string(),
                    b.decoder.to_string(),
                    b.reason.to_string(),
                    b.proprietary,
                );
            } else {
                entry.0 = (entry.0 + b.confidence * 0.3).min(0.98);
            }
        }
    }

    // 1b. ARRL US amateur band-plan segments (public allocation facts)
    crate::arrl_bandplan::apply_arrl_scores(frequency_hz, &mut scores);

    // 2. Range name boosts
    for (proto, boost) in range_boost(range_name) {
        let entry = scores.entry(proto.to_string()).or_insert((
            0.2,
            "inferred".into(),
            decoder_for(proto).into(),
            format!("range name '{range_name}'"),
            false,
        ));
        entry.0 = (entry.0 + boost).min(0.98);
        if entry.1 == "inferred" {
            entry.1 = family_for(proto).into();
        }
    }

    // 3. Bandwidth / mode hints
    for (proto, boost, reason) in bandwidth_hints(bandwidth_hz, mode) {
        let entry = scores.entry(proto.to_string()).or_insert((
            0.15,
            family_for(proto).into(),
            decoder_for(proto).into(),
            reason.to_string(),
            false,
        ));
        entry.0 = (entry.0 + boost).min(0.98);
        if entry.3.is_empty() {
            entry.3 = reason.to_string();
        }
    }

    // Mode string itself
    match mode.to_ascii_lowercase().as_str() {
        "wfm" => {
            let e = scores.entry("fm_broadcast".into()).or_insert((
                0.4,
                "analog".into(),
                "native_wfm_rds".into(),
                "WFM mode selected".into(),
                false,
            ));
            e.0 = (e.0 + 0.15).min(0.98);
        }
        "am" => {
            let e = scores.entry("aircraft_am".into()).or_insert((
                0.3,
                "aviation".into(),
                "native_am".into(),
                "AM mode selected".into(),
                false,
            ));
            e.0 = (e.0 + 0.10).min(0.98);
        }
        "nfm" | "fm" => {
            let e = scores.entry("analog_nfm".into()).or_insert((
                0.35,
                "analog".into(),
                "native_nfm".into(),
                "NFM mode selected".into(),
                false,
            ));
            e.0 = (e.0 + 0.10).min(0.98);
        }
        _ => {}
    }

    // SNR soft influence
    let snr_scale = if snr_db >= 20.0 {
        1.05
    } else if snr_db >= 12.0 {
        1.0
    } else {
        0.85
    };
    for v in scores.values_mut() {
        v.0 = (v.0 * snr_scale).min(0.98);
    }

    // 4. Audio features
    let mut features = Vec::new();
    let mut decode_success = false;
    let mut decode_protocol = String::new();
    let mut decode_summary = String::new();

    if let Some((samples, rate)) = audio {
        let mode_enum = Mode::parse(mode);
        let (feat, boosts) = analyse_audio(samples, rate, mode_enum);
        features = feat;
        for (proto, boost, reason) in boosts {
            let entry = scores.entry(proto.to_string()).or_insert((
                0.2,
                family_for(proto).into(),
                decoder_for(proto).into(),
                reason.clone(),
                false,
            ));
            entry.0 = (entry.0 + boost).min(0.99);
            entry.3 = reason.clone();
            // Treat a successful native decode as real evidence
            if boost >= 0.30 {
                decode_success = true;
                decode_protocol = proto.to_string();
                decode_summary = reason;
            }
        }
    }

    // Sort candidates
    let mut candidates: Vec<ProtocolCandidate> = scores
        .into_iter()
        .map(
            |(protocol, (confidence, family, decoder, reason, _prop))| ProtocolCandidate {
                protocol,
                family,
                confidence,
                decoder,
                reason,
            },
        )
        .collect();
    candidates.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(8);

    let top = candidates.first().cloned().unwrap_or(ProtocolCandidate {
        protocol: "unknown".into(),
        family: "unknown".into(),
        confidence: 0.0,
        decoder: "none".into(),
        reason: "no matching prior".into(),
    });

    let likely_proprietary = top.protocol == "tetra"
        || top.protocol == "p25_trunked" && top.confidence < 0.5
        || top.family == "land_mobile" && top.confidence < 0.4;

    let is_novel = top.confidence < 0.35;

    Classification {
        frequency_hz,
        bandwidth_hz,
        mode: mode.to_string(),
        range_name: range_name.to_string(),
        signal_class: if is_novel {
            "unclassified".into()
        } else {
            top.family.clone()
        },
        top_family: top.family.clone(),
        top_confidence: top.confidence,
        sub_protocol: top.protocol.clone(),
        symbol_rate: 0.0,
        decode_success,
        decode_protocol,
        decode_summary,
        likely_proprietary,
        is_novel,
        candidates,
        features,
    }
}

fn family_for(proto: &str) -> &'static str {
    match proto {
        "adsb" | "uat978" | "acars" | "vdl2" | "aircraft_am" => "aviation",
        "ais" | "marine_vhf" | "navtex" => "marine",
        "noaa_apt" | "goes_hrit" | "iridium" | "inmarsat" | "gps_l1" | "radiosonde"
        | "noaa_weather" => "satellite",
        "fm_broadcast" | "am_broadcast" | "hd_radio" | "analog_nfm" | "analog_wfm" => "analog",
        "p25" | "dmr" | "p25_trunked" | "tetra" | "frs_gmrs" => "land_mobile",
        "pocsag" | "flex" => "paging",
        "ism_433" | "ism_915" => "ism",
        "aprs" | "amateur_vhf" | "amateur_uhf" | "cw" | "rtty" | "sstv" | "ssb_voice" | "fm_voice" | "digital_weak" => "amateur",
        "dtmf" => "signaling",
        _ => "unknown",
    }
}

fn decoder_for(proto: &str) -> &'static str {
    match proto {
        "adsb" => "native_adsb",
        "uat978" => "native_uat978",
        "acars" => "native_acars",
        "vdl2" => "native_vdl2",
        "ais" => "native_ais",
        "noaa_apt" => "noaa-apt",
        "goes_hrit" | "inmarsat" | "gps_l1" | "satellite" => "satdump",
        "iridium" => "iridiumlive",
        "pocsag" | "flex" | "dtmf" => "native_pocsag",
        "aprs" => "native_aprs",
        "cw" => "native_cw",
        "rtty" | "digital_weak" => "native_rtty",
        "ssb_voice" | "fm_voice" | "amateur_uhf" => "none",
        "p25" | "dmr" | "p25_trunked" | "nxdn" | "dstar" | "ysf" | "m17" => "dsd-fme",
        "ism_433" | "ism_915" | "radiosonde" => "rtl_433",
        "hd_radio" => "nrsc5",
        "tetra" => "tetraear",
        "navtex" => "native_navtex",
        "sstv" => "native_sstv",
        "fm_broadcast" => "native_wfm_rds",
        "analog_nfm" | "frs_gmrs" | "marine_vhf" | "noaa_weather" => "native_nfm",
        "aircraft_am" | "am_broadcast" => "native_am",
        _ => "none",
    }
}

/// Recommended next decoder action from a classification.
pub fn recommended_action(c: &Classification) -> serde_json::Value {
    let top = c.candidates.first();
    serde_json::json!({
        "protocol": c.sub_protocol,
        "family": c.top_family,
        "confidence": c.top_confidence,
        "decoder": top.map(|t| t.decoder.clone()).unwrap_or_else(|| "none".into()),
        "auto_decode": c.top_confidence >= 0.55 && top.map(|t| t.decoder.as_str()) != Some("none"),
        "decode_success": c.decode_success,
        "decode_summary": c.decode_summary,
        "features": c.features,
        "candidates": c.candidates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_adsb_by_frequency() {
        let c = classify(1_090_000_000, 1_000_000, "am", "ADS-B 1090", 25.0, None);
        assert_eq!(c.sub_protocol, "adsb");
        assert!(c.top_confidence >= 0.9);
        assert_eq!(c.candidates[0].decoder, "native_adsb");
    }

    #[test]
    fn classifies_ais_channels() {
        let c = classify(161_975_000, 25_000, "nfm", "AIS", 18.0, None);
        assert_eq!(c.sub_protocol, "ais");
        assert!(c.top_confidence >= 0.9);
    }

    #[test]
    fn classifies_noaa_weather() {
        let c = classify(162_550_000, 25_000, "nfm", "NOAA Weather", 30.0, None);
        assert_eq!(c.sub_protocol, "noaa_weather");
        assert!(c.top_confidence >= 0.8);
    }

    #[test]
    fn range_name_boosts_aprs() {
        let c = classify(144_390_000, 12_500, "nfm", "2m APRS", 15.0, None);
        assert_eq!(c.sub_protocol, "aprs");
    }

    #[test]
    fn unknown_uhf_is_novel_or_low() {
        let c = classify(700_123_456, 12_500, "nfm", "Custom", 14.0, None);
        // May get analog_nfm from mode, but should not claim high-confidence satellite
        assert!(c.top_confidence < 0.7 || c.sub_protocol == "analog_nfm");
    }

    #[test]
    fn fm_broadcast_gets_rds_decoder() {
        let c = classify(100_700_000, 200_000, "wfm", "FM Broadcast", 40.0, None);
        assert!(c.sub_protocol == "fm_broadcast" || c.sub_protocol == "hd_radio");
        let has_wfm = c.candidates.iter().any(|x| x.decoder == "native_wfm_rds");
        assert!(has_wfm);
    }

    #[test]
    fn arrl_forty_meter_voice_segment() {
        let c = classify(7_150_000, 2_700, "lsb", "40m Amateur", 18.0, None);
        assert!(
            c.sub_protocol == "ssb_voice" || c.candidates.iter().any(|x| x.protocol == "ssb_voice"),
            "expected ssb_voice candidate, got {:?}",
            c.candidates.first().map(|x| x.protocol.clone())
        );
        assert_eq!(c.mode, "lsb");
    }
}
