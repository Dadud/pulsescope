//! Capability-aware one-tap listening presets for the shared receiver window.

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct ListeningMode {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub center_frequency_hz: u64,
    pub sample_rate_hz: u32,
    pub bandwidth_hz: u32,
    pub mode: &'static str,
    pub region: &'static str,
    pub deemphasis_us: Option<u32>,
    pub suggested_decoders: &'static [&'static str],
}

pub fn presets() -> Vec<ListeningMode> {
    vec![
        ListeningMode {
            id: "fm-broadcast",
            name: "FM broadcast",
            description: "Wideband FM scan with RDS-friendly de-emphasis defaults.",
            center_frequency_hz: 98_900_000,
            sample_rate_hz: 2_000_000,
            bandwidth_hz: 1_800_000,
            mode: "wfm",
            region: "broadcast-fm",
            deemphasis_us: Some(75),
            suggested_decoders: &["rds"],
        },
        ListeningMode {
            id: "aviation",
            name: "Aviation",
            description: "Airband voice monitoring around 121–136 MHz.",
            center_frequency_hz: 128_000_000,
            sample_rate_hz: 2_000_000,
            bandwidth_hz: 1_800_000,
            mode: "am",
            region: "aviation",
            deemphasis_us: None,
            suggested_decoders: &["adsb", "acars"],
        },
        ListeningMode {
            id: "marine-ais",
            name: "Marine / AIS",
            description: "Marine VHF voice plus AIS discriminator monitoring.",
            center_frequency_hz: 162_000_000,
            sample_rate_hz: 2_000_000,
            bandwidth_hz: 1_800_000,
            mode: "nfm",
            region: "marine",
            deemphasis_us: None,
            suggested_decoders: &["ais"],
        },
        ListeningMode {
            id: "pager",
            name: "Pager",
            description: "POCSAG/FLEX paging allocations are jurisdiction-specific; tune locally.",
            center_frequency_hz: 929_612_500,
            sample_rate_hz: 1_000_000,
            bandwidth_hz: 250_000,
            mode: "nfm",
            region: "pager",
            deemphasis_us: None,
            suggested_decoders: &["pocsag"],
        },
        ListeningMode {
            id: "mesh-915",
            name: "Mesh / ISM",
            description: "US915 mesh and ISM monitoring; select the legal plan for your region.",
            center_frequency_hz: 915_000_000,
            sample_rate_hz: 2_000_000,
            bandwidth_hz: 1_250_000,
            mode: "nfm",
            region: "us915",
            deemphasis_us: None,
            suggested_decoders: &["meshtastic", "meshcore", "rtl433"],
        },
        ListeningMode {
            id: "radiosonde",
            name: "Radiosonde",
            description: "400.15–406 MHz sonde telemetry. Checksum-valid frames only.",
            center_frequency_hz: 402_500_000,
            sample_rate_hz: 1_000_000,
            bandwidth_hz: 250_000,
            mode: "nfm",
            region: "radiosonde",
            deemphasis_us: None,
            suggested_decoders: &["radiosonde"],
        },
        ListeningMode {
            id: "goes-lrit",
            name: "GOES LRIT/HRIT",
            description: "GOES-East/West downlink product identification around 1.694 GHz.",
            center_frequency_hz: 1_694_100_000,
            sample_rate_hz: 2_400_000,
            bandwidth_hz: 1_500_000,
            mode: "nfm",
            region: "goes",
            deemphasis_us: None,
            suggested_decoders: &["goes"],
        },
        ListeningMode {
            id: "public-safety",
            name: "Public safety",
            description: "Trunked voice/control monitoring in the 700/800 MHz public-safety span.",
            center_frequency_hz: 851_000_000,
            sample_rate_hz: 2_000_000,
            bandwidth_hz: 1_250_000,
            mode: "nfm",
            region: "public-safety",
            deemphasis_us: None,
            suggested_decoders: &["p25", "dmr"],
        },
    ]
}

pub fn find(id: &str) -> Option<ListeningMode> {
    presets().into_iter().find(|mode| mode.id == id)
}
