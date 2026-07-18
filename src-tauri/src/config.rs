// config.rs — PulseScope runtime configuration, loaded from
// $HOME/pulsescope/config.toml. Mirrors the field shape used across the
// desktop SDR scanner category so power users have familiar knobs.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub device: DeviceConfig,
    pub scanner: ScannerConfig,
    pub demodulator: DemodulatorConfig,
    pub audio: AudioConfig,
    pub ui: UiConfig,
    pub recording: RecordingConfig,
    pub streaming: StreamingConfig,
    pub transcription: TranscriptionConfig,
    pub protection: ProtectionConfig,
    pub digital_decoder: DigitalDecoderConfig,
    pub rtl433: Rtl433Config,
    pub hd_radio: HdRadioConfig,
    pub stdc: StdcConfig,
    pub iridium: IridiumConfig,
    pub aero: AeroConfig,
    pub gps: GpsConfig,
    pub glonass: GlonassConfig,
    pub goes_lrit: GoesLritConfig,
    pub receiver_location: ReceiverLocationConfig,
    pub aprs: AprsConfig,
    pub acarsdec: AcarsdecConfig,
    pub dsd: DsdConfig,
    pub radiosonde: RadiosondeConfig,
    pub lora: LoraConfig,
    pub radio_reference: RadioReferenceConfig,
    pub aircraft_lookup: AircraftLookupConfig,
    pub trunking: TrunkingConfig,
    pub dump978: Dump978Config,
    pub vdl2: Vdl2Config,
    pub ble: BleConfig,
    pub scan_ranges: Vec<ScanRange>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            device: DeviceConfig::default(),
            scanner: ScannerConfig::default(),
            demodulator: DemodulatorConfig::default(),
            audio: AudioConfig::default(),
            ui: UiConfig::default(),
            recording: RecordingConfig::default(),
            streaming: StreamingConfig::default(),
            transcription: TranscriptionConfig::default(),
            protection: ProtectionConfig::default(),
            digital_decoder: DigitalDecoderConfig::default(),
            rtl433: Rtl433Config::default(),
            hd_radio: HdRadioConfig::default(),
            stdc: StdcConfig::default(),
            iridium: IridiumConfig::default(),
            aero: AeroConfig::default(),
            gps: GpsConfig::default(),
            glonass: GlonassConfig::default(),
            goes_lrit: GoesLritConfig::default(),
            receiver_location: ReceiverLocationConfig::default(),
            aprs: AprsConfig::default(),
            acarsdec: AcarsdecConfig::default(),
            dsd: DsdConfig::default(),
            radiosonde: RadiosondeConfig::default(),
            lora: LoraConfig::default(),
            radio_reference: RadioReferenceConfig::default(),
            aircraft_lookup: AircraftLookupConfig::default(),
            trunking: TrunkingConfig::default(),
            dump978: Dump978Config::default(),
            vdl2: Vdl2Config::default(),
            ble: BleConfig::default(),
            scan_ranges: crate::config::default_scan_ranges(),
        }
    }
}

impl Config {
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("failed to parse {path:?}: {e}; using defaults");
                Config::default()
            }),
            Err(_) => {
                tracing::info!("no config at {path:?}; using defaults");
                Config::default()
            }
        }
    }

    pub fn save(&self, data_dir: &Path) -> anyhow::Result<()> {
        let path = data_dir.join("config.toml");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(&path, text)?;
        Ok(())
    }
}

// ── section structs ───────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceConfig {
    pub sample_rate: u32,
    pub ppm_correction: f32,
    pub gain: String, // "auto" or numeric
    pub plutosdr_ip: String,
    pub saved_devices: Vec<String>,
    pub last_device_key: String,
    pub last_device_label: String,
    pub saturation_protection: bool,
}
impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            sample_rate: 10_000_000,
            ppm_correction: 0.0,
            gain: "auto".into(),
            plutosdr_ip: String::new(),
            saved_devices: Vec::new(),
            last_device_key: "driver=mock".into(),
            last_device_label: "Mock Source (Test Tones)".into(),
            saturation_protection: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ScannerConfig {
    pub fft_size: usize,
    pub fft_overlap: f32,
    pub update_rate_hz: f32,
    pub squelch_db: f32,
    pub hold_ms: u64,
    pub confirm_ms: u64,
    pub min_signal_width_bins: usize,
    pub max_vfos: usize,
    pub freq_step_hz: u32,
    pub dc_reject_hz: u32,
    pub hackrf_snr_boost_db: f32,
    pub auto_squelch_mode: AutoSquelchMode,
    pub manual_squelch_db: f32,
    pub noise_class_threshold: f32,
    pub scan_hold_on_audio: bool,
    pub scan_hold_max_ms: u64,
    pub per_freq_squelch: bool,
}
impl Default for ScannerConfig {
    fn default() -> Self {
        Self {
            fft_size: 4096,
            fft_overlap: 0.5,
            update_rate_hz: 20.0,
            squelch_db: 15.0,
            hold_ms: 3000,
            confirm_ms: 300,
            min_signal_width_bins: 5,
            max_vfos: 3,
            freq_step_hz: 5000,
            dc_reject_hz: 0,
            hackrf_snr_boost_db: 3.0,
            auto_squelch_mode: AutoSquelchMode::Adaptive,
            manual_squelch_db: 15.0,
            noise_class_threshold: 0.25,
            scan_hold_on_audio: true,
            scan_hold_max_ms: 10_000,
            per_freq_squelch: true,
        }
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AutoSquelchMode {
    Off,
    Adaptive,
    Manual,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DemodulatorConfig {
    pub default_mode: String,
    pub nfm_bandwidth_hz: u32,
    pub wfm_bandwidth_hz: u32,
    pub am_bandwidth_hz: u32,
    pub de_emphasis_us: u32,
}
impl Default for DemodulatorConfig {
    fn default() -> Self {
        Self {
            default_mode: "nfm".into(),
            nfm_bandwidth_hz: 12_500,
            wfm_bandwidth_hz: 200_000,
            am_bandwidth_hz: 10_000,
            de_emphasis_us: 75,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub buffer_ms: u32,
    pub master_volume: f32,
}
impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            buffer_ms: 50,
            master_volume: 0.7,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub theme: String,
    pub display_mode: String,
    pub waterfall_history_seconds: u32,
    pub spectrum_averaging: u32,
    pub show_frequency_labels: bool,
    pub show_vfo_markers: bool,
}
impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            display_mode: "waterfall".into(),
            waterfall_history_seconds: 30,
            spectrum_averaging: 4,
            show_frequency_labels: true,
            show_vfo_markers: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordingConfig {
    pub output_directory: String,
    pub audio_format: String,
    pub iq_format: String,
    pub trigger_mode: String,
    pub vox_threshold_db: f32,
    pub pre_record_seconds: f32,
    pub post_record_seconds: f32,
    pub max_duration_seconds: u32,
    pub iq_enabled: bool,
    pub audio_enabled: bool,
    pub skip_digital_signals: bool,
    pub min_duration_seconds: f32,
    pub skip_encrypted_calls: bool,
}
impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            output_directory: "recordings".into(),
            audio_format: "wav".into(),
            iq_format: "cf32".into(),
            trigger_mode: "manual".into(),
            vox_threshold_db: -30.0,
            pre_record_seconds: 2.0,
            post_record_seconds: 1.0,
            max_duration_seconds: 0,
            iq_enabled: false,
            audio_enabled: true,
            skip_digital_signals: true,
            min_duration_seconds: 1.5,
            skip_encrypted_calls: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamingConfig {
    pub audio_enabled: bool,
    pub audio_port: u16,
    pub audio_protocol: String,
    pub iq_enabled: bool,
    pub iq_port: u16,
    pub bind_address: String,
    pub max_clients: u32,
    pub sync_enabled: bool,
    pub sync_cluster: String,
}
impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            audio_enabled: false,
            audio_port: 7878,
            audio_protocol: "raw".into(),
            iq_enabled: false,
            iq_port: 1234,
            bind_address: "0.0.0.0".into(),
            max_clients: 10,
            sync_enabled: false,
            sync_cluster: "pulsescope".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TranscriptionConfig {
    pub enabled: bool,
    pub engine: String,
    pub model: String,
    pub language: String,
    pub min_segment_seconds: f32,
    pub max_segment_seconds: f32,
    pub save_transcripts: bool,
    pub transcript_directory: String,
}
impl Default for TranscriptionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            engine: "local".into(),
            model: "small".into(),
            language: "en".into(),
            min_segment_seconds: 1.0,
            max_segment_seconds: 30.0,
            save_transcripts: true,
            transcript_directory: "transcripts".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProtectionConfig {
    pub enabled: bool,
    pub rules: Vec<ProtectionRule>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtectionRule {
    pub name: String,
    pub start_hz: u64,
    pub end_hz: u64,
    pub action: String, // "Skip", "Warn"
}

// ── decoder sidecar configs ───────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DigitalDecoderConfig {
    pub enabled: bool,
    pub multimon_path: String,
    pub enabled_protocols: Vec<String>,
    pub auto_detect: bool,
    pub auto_detect_threshold: f32,
    pub max_history: usize,
    pub hold_on_digital: bool,
    pub digital_dwell_ms: u64,
    pub known_digital_ranges: Vec<String>,
    pub pocsag_force_mode: String,
    pub use_native_pocsag: bool,
    pub persist_log: bool,
}
impl Default for DigitalDecoderConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            multimon_path: "multimon-ng".into(),
            enabled_protocols: vec![
                "POCSAG512".into(),
                "POCSAG1200".into(),
                "POCSAG2400".into(),
                "FLEX".into(),
                "DTMF".into(),
                "EAS".into(),
                "AFSK1200".into(),
                "AFSK2400".into(),
                "FSK9600".into(),
                "MORSE_CW".into(),
            ],
            auto_detect: true,
            auto_detect_threshold: 0.5,
            max_history: 1000,
            hold_on_digital: true,
            digital_dwell_ms: 5000,
            known_digital_ranges: vec!["Pagers".into(), "UHF Pagers".into()],
            pocsag_force_mode: "auto".into(),
            use_native_pocsag: false,
            persist_log: false,
        }
    }
}

macro_rules! string_default {
    ($field:ident, $val:expr) => {
        pub $field: String
    };
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Rtl433Config {
    pub enabled: bool,
    pub path: String,
    pub protocols: Vec<String>,
    pub persist_log: bool,
    pub extended_decoders: bool,
    pub extra_args: String,
}
impl Default for Rtl433Config {
    fn default() -> Self {
        Self {
            enabled: false,
            path: "rtl_433".into(),
            protocols: Vec::new(),
            persist_log: false,
            extended_decoders: true,
            extra_args: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HdRadioConfig {
    pub enabled: bool,
    pub auto_on_fm_lock: bool,
    pub program: u32,
    pub stations: Vec<String>,
}
impl Default for HdRadioConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_on_fm_lock: false,
            program: 0,
            stations: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct StdcConfig {
    pub enabled: bool,
    pub path: String,
    pub uw_tolerance: u32,
}
impl Default for StdcConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: "sdr-stdc-helper".into(),
            uw_tolerance: 4,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct IridiumConfig {
    pub enabled: bool,
    pub surface_message_content: bool,
    pub message_content_acknowledged: bool,
    pub center_freq_hz: u64,
    pub sample_rate_hz: u32,
}
impl Default for IridiumConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            surface_message_content: false,
            message_content_acknowledged: false,
            center_freq_hz: 1_621_250_000,
            sample_rate_hz: 2_400_000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AeroConfig {
    pub enabled: bool,
    pub sniffer_path: String,
    pub satellite: String,
    pub center_freq_hz: u64,
    pub sample_rate_hz: u32,
}
impl Default for AeroConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sniffer_path: String::new(),
            satellite: "4F3".into(),
            center_freq_hz: 1_545_000_000,
            sample_rate_hz: 2_400_000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GpsConfig {
    pub enabled: bool,
    pub pass_interval_ms: u32,
    pub detection_threshold: f32,
    pub sample_rate_hz: u32,
    pub doppler_search_hz: u32,
}
impl Default for GpsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            pass_interval_ms: 1000,
            detection_threshold: 2.5,
            sample_rate_hz: 2_000_000,
            doppler_search_hz: 5000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GlonassConfig {
    pub enabled: bool,
    pub pass_interval_ms: u32,
    pub detection_threshold: f32,
    pub sample_rate_hz: u32,
    pub doppler_search_hz: u32,
}
impl Default for GlonassConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            pass_interval_ms: 1000,
            detection_threshold: 2.5,
            sample_rate_hz: 8_000_000,
            doppler_search_hz: 5000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GoesLritConfig {
    pub enabled: bool,
    pub satdump_path: String,
    pub satellite: String,
    pub sample_rate_hz: u32,
    pub output_image_dir: String,
}
impl Default for GoesLritConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            satdump_path: String::new(),
            satellite: "goes-19-east".into(),
            sample_rate_hz: 2_000_000,
            output_image_dir: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ReceiverLocationConfig {
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    pub altitude_m: f64,
}
impl Default for ReceiverLocationConfig {
    fn default() -> Self {
        Self {
            latitude_deg: 0.0,
            longitude_deg: 0.0,
            altitude_m: 0.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AprsConfig {
    pub enabled: bool,
    pub path: String,
    pub frequency_hz: u64,
}
impl Default for AprsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: "direwolf".into(),
            frequency_hz: 144_390_000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AcarsdecConfig {
    pub enabled: bool,
    pub frequencies: Vec<u64>,
    pub path: String,
}
impl Default for AcarsdecConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            frequencies: vec![131_550_000, 131_525_000, 131_725_000],
            path: "acarsdec".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DsdConfig {
    pub enabled: bool,
    pub mode: String,
    pub dsdneo_path: String,
}
impl Default for DsdConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "auto".into(),
            dsdneo_path: "dsd-neo".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RadiosondeConfig {
    pub enabled: bool,
    pub path: String,
    pub sonde_type: String,
    pub frequency_hz: u64,
}
impl Default for RadiosondeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: "rs41mod".into(),
            sonde_type: "rs41".into(),
            frequency_hz: 402_500_000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LoraConfig {
    pub enabled: bool,
    pub region: String,
    pub sync_word: u8,
    pub bandwidth_hz: u32,
    pub spreading_factors: Vec<u8>,
    pub spreading_factor: u8,
    pub app_s_key_hex: String,
    pub nwk_s_key_hex: String,
    pub helper_script: String,
    pub persist_log: bool,
    pub ldro_mode: String,
}
impl Default for LoraConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            region: "US915".into(),
            sync_word: 52,
            bandwidth_hz: 125_000,
            spreading_factors: vec![7, 8, 9, 10, 11, 12],
            spreading_factor: 7,
            app_s_key_hex: String::new(),
            nwk_s_key_hex: String::new(),
            helper_script: String::new(),
            persist_log: false,
            ldro_mode: "auto".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RadioReferenceConfig {
    pub username: String,
    pub password: String,
    pub api_key: String,
    pub zipcode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AircraftLookupConfig {
    pub enabled: bool,
    pub cache_ttl_days: u32,
}
impl Default for AircraftLookupConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cache_ttl_days: 7,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TrunkingConfig {
    pub enabled: bool,
    pub systems: Vec<String>,
    pub discovered_ccs: Vec<String>,
    pub cc_discovery_budget_secs: u32,
    pub log_verbose_dsd: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Dump978Config {
    pub enabled: bool,
    pub path: String,
    pub extra_args: Vec<String>,
}
impl Default for Dump978Config {
    fn default() -> Self {
        Self {
            enabled: false,
            path: "dump978".into(),
            extra_args: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Vdl2Config {
    pub enabled: bool,
    pub path: String,
    pub frequencies: Vec<u64>,
    pub persist_log: bool,
}
impl Default for Vdl2Config {
    fn default() -> Self {
        Self {
            enabled: false,
            path: "dumpvdl2".into(),
            frequencies: vec![136_975_000, 136_700_000, 136_725_000, 136_900_000],
            persist_log: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct BleConfig {
    pub enabled: bool,
    pub channels: Vec<u8>,
    pub channel_dwell_ms: u32,
    pub gain_db: f32,
    pub sample_rate_hz: u32,
    pub scan_decode: bool,
}
impl Default for BleConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            channels: vec![37, 38, 39],
            channel_dwell_ms: 1500,
            gain_db: 40.0,
            sample_rate_hz: 3_000_000,
            scan_decode: true,
        }
    }
}

// ── scan range ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanRange {
    pub name: String,
    pub start_hz: u64,
    pub end_hz: u64,
    pub mode: String, // am | nfm | wfm | lsb | usb
    pub channel_bw_hz: u32,
    pub max_vfos: u32,
    pub enabled: bool,
    pub dwell_ms: u32,
    pub squelch_db: f32,
    pub auto_squelch_mode: AutoSquelchMode,
    pub hold_ms: u32,
    pub sample_rate_hz: u32,
}
impl Default for ScanRange {
    fn default() -> Self {
        Self {
            name: String::new(),
            start_hz: 0,
            end_hz: 0,
            mode: "nfm".into(),
            channel_bw_hz: 12_500,
            max_vfos: 3,
            enabled: false,
            dwell_ms: 200,
            squelch_db: 15.0,
            auto_squelch_mode: AutoSquelchMode::Adaptive,
            hold_ms: 3000,
            sample_rate_hz: 2_000_000,
        }
    }
}

/// 75 default presets spanning AM broadcast through 5.8 GHz ISM.
/// Frequencies are public ITU / FCC band edges — no third-party data.
pub fn default_scan_ranges() -> Vec<ScanRange> {
    use AutoSquelchMode::*;
    vec![
        range(
            "AM Broadcast",
            540_000,
            1_700_000,
            "am",
            9_000,
            2,
            1_500_000,
            Adaptive,
        ),
        range(
            "160m Amateur",
            1_800_000,
            2_000_000,
            "lsb",
            2_700,
            3,
            250_000,
            Off,
        ),
        range(
            "80m Amateur",
            3_500_000,
            4_000_000,
            "lsb",
            2_700,
            3,
            768_000,
            Off,
        ),
        range(
            "60m Amateur",
            5_330_000,
            5_406_000,
            "usb",
            2_700,
            3,
            250_000,
            Off,
        ),
        range(
            "SW 49m", 5_900_000, 6_200_000, "am", 6_000, 2, 500_000, Adaptive,
        ),
        range(
            "40m Amateur",
            7_000_000,
            7_300_000,
            "lsb",
            2_700,
            3,
            500_000,
            Off,
        ),
        range(
            "SW 41m", 7_200_000, 7_450_000, "am", 6_000, 2, 500_000, Adaptive,
        ),
        range(
            "SW 31m", 9_400_000, 9_900_000, "am", 6_000, 2, 768_000, Adaptive,
        ),
        range(
            "30m Amateur",
            10_100_000,
            10_150_000,
            "usb",
            2_700,
            3,
            250_000,
            Off,
        ),
        range(
            "SW 25m", 11_600_000, 12_100_000, "am", 6_000, 2, 768_000, Adaptive,
        ),
        range(
            "SW 22m", 13_570_000, 13_870_000, "am", 6_000, 2, 500_000, Adaptive,
        ),
        range(
            "20m Amateur",
            14_000_000,
            14_350_000,
            "usb",
            2_700,
            3,
            500_000,
            Off,
        ),
        range(
            "SW 19m", 15_100_000, 15_800_000, "am", 6_000, 2, 960_000, Adaptive,
        ),
        range(
            "SW 16m", 17_480_000, 17_900_000, "am", 6_000, 2, 500_000, Adaptive,
        ),
        range(
            "17m Amateur",
            18_068_000,
            18_168_000,
            "usb",
            2_700,
            3,
            250_000,
            Off,
        ),
        range(
            "15m Amateur",
            21_000_000,
            21_450_000,
            "usb",
            2_700,
            3,
            500_000,
            Off,
        ),
        range(
            "SW 13m", 21_450_000, 21_850_000, "am", 6_000, 2, 500_000, Adaptive,
        ),
        range(
            "12m Amateur",
            24_890_000,
            24_990_000,
            "usb",
            2_700,
            3,
            250_000,
            Off,
        ),
        range(
            "SW 11m", 25_670_000, 26_100_000, "am", 6_000, 2, 500_000, Adaptive,
        ),
        range(
            "CB Radio", 26_965_000, 27_405_000, "am", 10_000, 3, 500_000, Adaptive,
        ),
        range(
            "10m Amateur",
            28_000_000,
            29_700_000,
            "am",
            6_000,
            3,
            2_000_000,
            Adaptive,
        ),
        range(
            "10m SSB", 28_000_000, 28_500_000, "usb", 2_700, 3, 768_000, Off,
        ),
        range(
            "10m FM", 29_510_000, 29_700_000, "nfm", 12_500, 3, 250_000, Adaptive,
        ),
        range(
            "Baby Monitors",
            49_830_000,
            49_890_000,
            "nfm",
            12_500,
            2,
            250_000,
            Adaptive,
        ),
        range(
            "6m Amateur",
            50_000_000,
            54_000_000,
            "nfm",
            12_500,
            3,
            5_000_000,
            Adaptive,
        ),
        range(
            "R/C Aircraft",
            72_000_000,
            73_000_000,
            "nfm",
            12_500,
            2,
            1_200_000,
            Adaptive,
        ),
        range(
            "R/C Surface",
            75_400_000,
            76_000_000,
            "nfm",
            12_500,
            2,
            768_000,
            Adaptive,
        ),
        range(
            "FM Broadcast",
            88_000_000,
            108_000_000,
            "wfm",
            200_000,
            2,
            2_000_000,
            Adaptive,
        ),
        range(
            "Aircraft AM",
            118_000_000,
            137_000_000,
            "am",
            8_500,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "ATC Ground",
            121_600_000,
            121_900_000,
            "am",
            8_500,
            3,
            500_000,
            Adaptive,
        ),
        range(
            "ACARS",
            129_000_000,
            132_000_000,
            "am",
            6_500,
            3,
            4_000_000,
            Adaptive,
        ),
        range(
            "NOAA APT",
            137_000_000,
            138_000_000,
            "nfm",
            40_000,
            1,
            1_200_000,
            Adaptive,
        ),
        range(
            "2m Amateur",
            144_000_000,
            148_000_000,
            "nfm",
            12_500,
            3,
            5_000_000,
            Adaptive,
        ),
        range(
            "MURS",
            151_820_000,
            154_600_000,
            "nfm",
            12_500,
            3,
            4_000_000,
            Adaptive,
        ),
        range(
            "VHF Business",
            151_000_000,
            154_000_000,
            "nfm",
            12_500,
            3,
            4_000_000,
            Adaptive,
        ),
        range(
            "Marine VHF",
            156_000_000,
            162_000_000,
            "nfm",
            25_000,
            3,
            7_000_000,
            Adaptive,
        ),
        range(
            "AIS",
            161_975_000,
            162_025_000,
            "nfm",
            25_000,
            1,
            250_000,
            Adaptive,
        ),
        range(
            "Railroad AAR",
            160_215_000,
            161_565_000,
            "nfm",
            12_500,
            3,
            1_500_000,
            Adaptive,
        ),
        range(
            "NOAA Weather",
            162_400_000,
            162_550_000,
            "nfm",
            25_000,
            1,
            250_000,
            Adaptive,
        ),
        range(
            "Federal Gov",
            162_000_000,
            174_000_000,
            "nfm",
            12_500,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "220 Amateur",
            222_000_000,
            225_000_000,
            "nfm",
            12_500,
            3,
            4_000_000,
            Adaptive,
        ),
        range(
            "Military Air",
            225_000_000,
            400_000_000,
            "am",
            8_500,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "Radiosonde",
            400_000_000,
            406_000_000,
            "nfm",
            10_000,
            3,
            7_000_000,
            Adaptive,
        ),
        range(
            "70cm Amateur",
            420_000_000,
            450_000_000,
            "nfm",
            12_500,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "ISM 433",
            433_050_000,
            434_790_000,
            "nfm",
            25_000,
            3,
            2_000_000,
            Adaptive,
        ),
        range(
            "UHF Business",
            450_000_000,
            470_000_000,
            "nfm",
            12_500,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "Public Safety UHF",
            453_000_000,
            458_000_000,
            "nfm",
            12_500,
            3,
            6_000_000,
            Adaptive,
        ),
        range(
            "UHF Pagers",
            454_000_000,
            461_000_000,
            "nfm",
            25_000,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "FRS/GMRS",
            462_550_000,
            467_725_000,
            "nfm",
            12_500,
            3,
            6_000_000,
            Adaptive,
        ),
        range(
            "UHF T-Band",
            470_000_000,
            512_000_000,
            "nfm",
            12_500,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "Wireless Mics (TV)",
            470_000_000,
            608_000_000,
            "nfm",
            200_000,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "Wireless Mics (Duplex Gap)",
            614_000_000,
            616_000_000,
            "nfm",
            200_000,
            3,
            2_400_000,
            Adaptive,
        ),
        range(
            "Wireless Mics (Guard Band)",
            657_000_000,
            663_000_000,
            "nfm",
            200_000,
            3,
            7_000_000,
            Adaptive,
        ),
        range(
            "700 PS Mobile",
            769_000_000,
            775_000_000,
            "nfm",
            12_500,
            3,
            7_000_000,
            Adaptive,
        ),
        range(
            "700 PS Base",
            799_000_000,
            805_000_000,
            "nfm",
            12_500,
            3,
            7_000_000,
            Adaptive,
        ),
        range(
            "800 Trunked",
            851_000_000,
            869_000_000,
            "nfm",
            12_500,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "33cm Amateur",
            902_000_000,
            928_000_000,
            "nfm",
            12_500,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "ISM 915",
            902_000_000,
            928_000_000,
            "nfm",
            25_000,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "Pagers",
            929_000_000,
            932_000_000,
            "nfm",
            25_000,
            3,
            4_000_000,
            Adaptive,
        ),
        range(
            "ADS-B UAT",
            978_000_000,
            978_200_000,
            "am",
            200_000,
            1,
            2_400_000,
            Adaptive,
        ),
        range(
            "ADS-B 1090",
            1_090_000_000,
            1_090_200_000,
            "am",
            1_000_000,
            1,
            2_400_000,
            Adaptive,
        ),
        range(
            "23cm Amateur",
            1_240_000_000,
            1_300_000_000,
            "nfm",
            12_500,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "Inmarsat STD-C / AERO (DL)",
            1_525_000_000,
            1_559_000_000,
            "nfm",
            12_000,
            1,
            2_400_000,
            Adaptive,
        ),
        range(
            "GPS L1 / Galileo E1",
            1_574_000_000,
            1_577_000_000,
            "nfm",
            2_046_000,
            1,
            2_400_000,
            Adaptive,
        ),
        range(
            "GLONASS L1",
            1_598_000_000,
            1_606_000_000,
            "nfm",
            8_000_000,
            1,
            2_400_000,
            Adaptive,
        ),
        range(
            "Iridium",
            1_616_000_000,
            1_626_500_000,
            "nfm",
            31_500,
            1,
            5_000_000,
            Adaptive,
        ),
        range(
            "Inmarsat AERO (UL)",
            1_626_500_000,
            1_660_500_000,
            "nfm",
            12_000,
            1,
            2_400_000,
            Adaptive,
        ),
        range(
            "GOES HRIT / LRIT",
            1_691_000_000,
            1_695_000_000,
            "nfm",
            1_500_000,
            1,
            5_000_000,
            Adaptive,
        ),
        range(
            "13cm Amateur (2300)",
            2_300_000_000,
            2_310_000_000,
            "nfm",
            12_500,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "13cm Amateur (2390)",
            2_390_000_000,
            2_450_000_000,
            "nfm",
            12_500,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "ISM 2.4 GHz",
            2_400_000_000,
            2_483_500_000,
            "nfm",
            25_000,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "CBRS 3.5 GHz",
            3_550_000_000,
            3_700_000_000,
            "nfm",
            12_500,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "4.9 GHz Public Safety",
            4_940_000_000,
            4_990_000_000,
            "nfm",
            12_500,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "5cm Amateur",
            5_650_000_000,
            5_925_000_000,
            "nfm",
            12_500,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "ISM 5.8 GHz",
            5_725_000_000,
            5_850_000_000,
            "nfm",
            25_000,
            3,
            8_000_000,
            Adaptive,
        ),
        range(
            "US Cellular 850 Uplink",
            824_000_000,
            849_000_000,
            "nfm",
            12_500,
            0,
            8_000_000,
            Off,
        ),
        range(
            "US Cellular 850 Downlink",
            869_000_000,
            894_000_000,
            "nfm",
            12_500,
            0,
            8_000_000,
            Off,
        ),
        range(
            "US PCS 1900 Uplink",
            1_850_000_000,
            1_910_000_000,
            "nfm",
            12_500,
            0,
            8_000_000,
            Off,
        ),
        range(
            "US PCS 1900 Downlink",
            1_930_000_000,
            1_990_000_000,
            "nfm",
            12_500,
            0,
            8_000_000,
            Off,
        ),
    ]
}

fn range(
    name: &str,
    start: u64,
    end: u64,
    mode: &str,
    bw: u32,
    vfos: u32,
    sr: u32,
    asq: AutoSquelchMode,
) -> ScanRange {
    ScanRange {
        name: name.into(),
        start_hz: start,
        end_hz: end,
        mode: mode.into(),
        channel_bw_hz: bw,
        max_vfos: vfos,
        enabled: false,
        dwell_ms: 200,
        squelch_db: 15.0,
        auto_squelch_mode: asq,
        hold_ms: 3000,
        sample_rate_hz: sr,
    }
}

#[allow(dead_code)]
pub fn poll_interval(cfg: &ScannerConfig) -> Duration {
    Duration::from_micros((1_000_000.0 / cfg.update_rate_hz.max(1.0)) as u64)
}
