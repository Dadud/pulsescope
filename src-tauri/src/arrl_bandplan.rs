//! ARRL US amateur band-plan segments (public regulatory facts).
//!
//! Used to choose demod mode, rank protocol candidates, and route auto-decode
//! without copying proprietary band charts — edges follow published ARRL/US
//! amateur allocations and typical emission types.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct ArrlSegment {
    pub start_hz: u64,
    pub end_hz: u64,
    pub label: &'static str,
    pub protocol: &'static str,
    pub family: &'static str,
    pub mode: &'static str,
    pub decoder: &'static str,
    pub confidence: f32,
}

/// Published ARRL band-plan style segments for US amateur allocations.
const ARRL_SEGMENTS: &[ArrlSegment] = &[
    // 160m
    ArrlSegment {
        start_hz: 1_800_000,
        end_hz: 1_840_000,
        label: "160m CW",
        protocol: "cw",
        family: "amateur",
        mode: "usb",
        decoder: "native_cw",
        confidence: 0.88,
    },
    ArrlSegment {
        start_hz: 1_840_000,
        end_hz: 1_850_000,
        label: "160m RTTY/data",
        protocol: "rtty",
        family: "amateur",
        mode: "usb",
        decoder: "native_rtty",
        confidence: 0.82,
    },
    ArrlSegment {
        start_hz: 1_850_000,
        end_hz: 1_890_000,
        label: "160m weak-signal digi",
        protocol: "digital_weak",
        family: "amateur",
        mode: "usb",
        decoder: "native_rtty",
        confidence: 0.70,
    },
    ArrlSegment {
        start_hz: 1_890_000,
        end_hz: 2_000_000,
        label: "160m SSB voice",
        protocol: "ssb_voice",
        family: "amateur",
        mode: "lsb",
        decoder: "none",
        confidence: 0.85,
    },
    // 80m
    ArrlSegment {
        start_hz: 3_500_000,
        end_hz: 3_600_000,
        label: "80m CW",
        protocol: "cw",
        family: "amateur",
        mode: "usb",
        decoder: "native_cw",
        confidence: 0.88,
    },
    ArrlSegment {
        start_hz: 3_600_000,
        end_hz: 3_700_000,
        label: "80m digi/RTTY",
        protocol: "rtty",
        family: "amateur",
        mode: "usb",
        decoder: "native_rtty",
        confidence: 0.80,
    },
    ArrlSegment {
        start_hz: 3_700_000,
        end_hz: 3_800_000,
        label: "80m weak-signal digi",
        protocol: "digital_weak",
        family: "amateur",
        mode: "usb",
        decoder: "native_rtty",
        confidence: 0.72,
    },
    ArrlSegment {
        start_hz: 3_800_000,
        end_hz: 4_000_000,
        label: "80m SSB voice",
        protocol: "ssb_voice",
        family: "amateur",
        mode: "lsb",
        decoder: "none",
        confidence: 0.86,
    },
    // 40m
    ArrlSegment {
        start_hz: 7_000_000,
        end_hz: 7_035_000,
        label: "40m CW",
        protocol: "cw",
        family: "amateur",
        mode: "usb",
        decoder: "native_cw",
        confidence: 0.90,
    },
    ArrlSegment {
        start_hz: 7_035_000,
        end_hz: 7_050_000,
        label: "40m weak-signal digi",
        protocol: "digital_weak",
        family: "amateur",
        mode: "usb",
        decoder: "native_rtty",
        confidence: 0.78,
    },
    ArrlSegment {
        start_hz: 7_050_000,
        end_hz: 7_075_000,
        label: "40m RTTY/data",
        protocol: "rtty",
        family: "amateur",
        mode: "usb",
        decoder: "native_rtty",
        confidence: 0.84,
    },
    ArrlSegment {
        start_hz: 7_075_000,
        end_hz: 7_100_000,
        label: "40m weak-signal digi",
        protocol: "digital_weak",
        family: "amateur",
        mode: "usb",
        decoder: "native_rtty",
        confidence: 0.76,
    },
    ArrlSegment {
        start_hz: 7_100_000,
        end_hz: 7_125_000,
        label: "40m SSB voice",
        protocol: "ssb_voice",
        family: "amateur",
        mode: "lsb",
        decoder: "none",
        confidence: 0.88,
    },
    ArrlSegment {
        start_hz: 7_125_000,
        end_hz: 7_175_000,
        label: "40m mixed weak-signal",
        protocol: "digital_weak",
        family: "amateur",
        mode: "usb",
        decoder: "native_rtty",
        confidence: 0.68,
    },
    ArrlSegment {
        start_hz: 7_175_000,
        end_hz: 7_200_000,
        label: "40m SSB voice",
        protocol: "ssb_voice",
        family: "amateur",
        mode: "lsb",
        decoder: "none",
        confidence: 0.88,
    },
    ArrlSegment {
        start_hz: 7_200_000,
        end_hz: 7_300_000,
        label: "40m SSB voice (extended)",
        protocol: "ssb_voice",
        family: "amateur",
        mode: "lsb",
        decoder: "none",
        confidence: 0.86,
    },
    // 30m (CW/data only)
    ArrlSegment {
        start_hz: 10_100_000,
        end_hz: 10_140_000,
        label: "30m CW",
        protocol: "cw",
        family: "amateur",
        mode: "usb",
        decoder: "native_cw",
        confidence: 0.88,
    },
    ArrlSegment {
        start_hz: 10_140_000,
        end_hz: 10_150_000,
        label: "30m weak-signal digi",
        protocol: "digital_weak",
        family: "amateur",
        mode: "usb",
        decoder: "native_rtty",
        confidence: 0.78,
    },
    // 20m
    ArrlSegment {
        start_hz: 14_000_000,
        end_hz: 14_070_000,
        label: "20m CW",
        protocol: "cw",
        family: "amateur",
        mode: "usb",
        decoder: "native_cw",
        confidence: 0.90,
    },
    ArrlSegment {
        start_hz: 14_070_000,
        end_hz: 14_095_000,
        label: "20m weak-signal digi",
        protocol: "digital_weak",
        family: "amateur",
        mode: "usb",
        decoder: "native_rtty",
        confidence: 0.78,
    },
    ArrlSegment {
        start_hz: 14_095_000,
        end_hz: 14_112_000,
        label: "20m RTTY/data",
        protocol: "rtty",
        family: "amateur",
        mode: "usb",
        decoder: "native_rtty",
        confidence: 0.84,
    },
    ArrlSegment {
        start_hz: 14_150_000,
        end_hz: 14_350_000,
        label: "20m SSB voice",
        protocol: "ssb_voice",
        family: "amateur",
        mode: "usb",
        decoder: "none",
        confidence: 0.88,
    },
    // 15m
    ArrlSegment {
        start_hz: 21_000_000,
        end_hz: 21_070_000,
        label: "15m CW",
        protocol: "cw",
        family: "amateur",
        mode: "usb",
        decoder: "native_cw",
        confidence: 0.88,
    },
    ArrlSegment {
        start_hz: 21_070_000,
        end_hz: 21_200_000,
        label: "15m weak-signal digi",
        protocol: "digital_weak",
        family: "amateur",
        mode: "usb",
        decoder: "native_rtty",
        confidence: 0.76,
    },
    ArrlSegment {
        start_hz: 21_200_000,
        end_hz: 21_450_000,
        label: "15m SSB voice",
        protocol: "ssb_voice",
        family: "amateur",
        mode: "usb",
        decoder: "none",
        confidence: 0.86,
    },
    // 10m
    ArrlSegment {
        start_hz: 28_000_000,
        end_hz: 28_070_000,
        label: "10m CW",
        protocol: "cw",
        family: "amateur",
        mode: "usb",
        decoder: "native_cw",
        confidence: 0.88,
    },
    ArrlSegment {
        start_hz: 28_070_000,
        end_hz: 28_190_000,
        label: "10m weak-signal digi",
        protocol: "digital_weak",
        family: "amateur",
        mode: "usb",
        decoder: "native_rtty",
        confidence: 0.76,
    },
    ArrlSegment {
        start_hz: 28_300_000,
        end_hz: 29_700_000,
        label: "10m SSB/FM voice",
        protocol: "ssb_voice",
        family: "amateur",
        mode: "usb",
        decoder: "none",
        confidence: 0.82,
    },
    // 6m
    ArrlSegment {
        start_hz: 50_000_000,
        end_hz: 54_000_000,
        label: "6m weak-signal/voice",
        protocol: "amateur_vhf",
        family: "amateur",
        mode: "usb",
        decoder: "none",
        confidence: 0.65,
    },
    // 2m
    ArrlSegment {
        start_hz: 144_000_000,
        end_hz: 144_100_000,
        label: "2m weak-signal",
        protocol: "digital_weak",
        family: "amateur",
        mode: "usb",
        decoder: "native_rtty",
        confidence: 0.62,
    },
    ArrlSegment {
        start_hz: 144_390_000,
        end_hz: 144_400_000,
        label: "2m APRS",
        protocol: "aprs",
        family: "amateur",
        mode: "nfm",
        decoder: "native_aprs",
        confidence: 0.92,
    },
    ArrlSegment {
        start_hz: 144_500_000,
        end_hz: 148_000_000,
        label: "2m FM voice",
        protocol: "fm_voice",
        family: "amateur",
        mode: "nfm",
        decoder: "none",
        confidence: 0.80,
    },
    // 70cm
    ArrlSegment {
        start_hz: 420_000_000,
        end_hz: 450_000_000,
        label: "70cm mixed",
        protocol: "amateur_uhf",
        family: "amateur",
        mode: "nfm",
        decoder: "none",
        confidence: 0.60,
    },
    ArrlSegment {
        start_hz: 431_000_000,
        end_hz: 433_000_000,
        label: "70cm weak-signal",
        protocol: "digital_weak",
        family: "amateur",
        mode: "nfm",
        decoder: "native_rtty",
        confidence: 0.58,
    },
];

/// Best ARRL segment for a frequency (prefers narrower, higher-confidence matches).
pub fn segment_at(frequency_hz: u64) -> Option<&'static ArrlSegment> {
    ARRL_SEGMENTS
        .iter()
        .filter(|s| frequency_hz >= s.start_hz && frequency_hz <= s.end_hz)
        .max_by(|a, b| {
            let score_a = (a.confidence * 1000.0) as i32 - (a.end_hz - a.start_hz) as i32 / 1000;
            let score_b = (b.confidence * 1000.0) as i32 - (b.end_hz - b.start_hz) as i32 / 1000;
            score_a.cmp(&score_b)
        })
}

/// Demod mode suggested by ARRL segment, else scan-range mode.
pub fn recommended_mode(frequency_hz: u64, fallback_mode: &str) -> &str {
    if let Some(seg) = segment_at(frequency_hz) {
        seg.mode
    } else {
        fallback_mode
    }
}

/// Apply ARRL segment priors into the classifier score map.
pub fn apply_arrl_scores(
    frequency_hz: u64,
    scores: &mut HashMap<String, (f32, String, String, String, bool)>,
) {
    if let Some(seg) = segment_at(frequency_hz) {
        let entry = scores.entry(seg.protocol.to_string()).or_insert((
            0.0,
            seg.family.to_string(),
            seg.decoder.to_string(),
            seg.label.to_string(),
            false,
        ));
        if seg.confidence > entry.0 {
            *entry = (
                seg.confidence,
                seg.family.to_string(),
                seg.decoder.to_string(),
                seg.label.to_string(),
                false,
            );
        } else {
            entry.0 = (entry.0 + seg.confidence * 0.25).min(0.98);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forty_meter_voice_is_lsb() {
        let seg = segment_at(7_110_000).expect("40m segment");
        assert_eq!(seg.protocol, "ssb_voice");
        assert_eq!(seg.mode, "lsb");
    }

    #[test]
    fn forty_meter_cw_segment() {
        let seg = segment_at(7_020_000).expect("40m cw");
        assert_eq!(seg.protocol, "cw");
        assert_eq!(seg.decoder, "native_cw");
    }

    #[test]
    fn aprs_segment() {
        let seg = segment_at(144_390_000).expect("aprs");
        assert_eq!(seg.protocol, "aprs");
        assert_eq!(seg.decoder, "native_aprs");
    }
}
