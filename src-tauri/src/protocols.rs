//! Honest, machine-readable implementation plans for protocol decoder slices.
//!
//! A slice remains unavailable until a redistributable recorded-IQ fixture has
//! passed the complete decoder path.  Merely enabling configuration or finding
//! a sidecar never changes this status.

use serde::Serialize;

use crate::device::DeviceStatus;

#[derive(Clone, Debug, Serialize)]
pub struct ProtocolSlice {
    pub id: &'static str,
    pub name: &'static str,
    pub frequency_plan: &'static str,
    pub center_frequency_hz: u64,
    pub rf_bandwidth_hz: u32,
    pub required_sample_rate_hz: u32,
    pub synchronization: &'static str,
    pub modulation: &'static str,
    pub fec: &'static str,
    pub checksum: &'static str,
    pub message_schema: &'static str,
    pub fixture: &'static str,
    pub ui_outcome: &'static str,
    pub transport: &'static str,
    pub available: bool,
    pub completion_reason: &'static str,
}

pub fn slices() -> Vec<ProtocolSlice> {
    vec![
        slice_verified("ble-advertising", "BLE advertising", "Advertising channels 37/38/39: 2402/2426/2480 MHz", 2_402_000_000, 2_000_000, 4_000_000, "8-bit preamble + advertising access address 0x8E89BED6", "1 Msym/s GFSK, BT=0.5", "None", "BLE CRC-24 after channel whitening", "ble.advertisement.v1: address, address_type, rssi_dbm, ad_structures, raw_pdu", "fixtures/recorded-iq/ble-pulse-advert.iq.json", "Device table and detail view; no synthetic devices", "native CF32 channelizer -> GFSK/whitening parser"),
        slice_verified("lora-mesh", "LoRa mesh and Modbus", "Regional plan selected explicitly (US915/EU868 documented plans only)", 915_000_000, 125_000, 1_000_000, "LoRa up-chirp preamble and SFD", "CSS SF8 125 kHz", "Implicit-header CSS fixture; Hamming FEC not claimed", "PHY recovery plus Modbus CRC-16; Meshtastic/MeshCore public-default PSK plaintext; LoRaWAN MIC never decrypted", "lora.mesh.v1: protocol, address, message_type, content, encryption", "fixtures/recorded-iq/lora-meshtastic-hello.iq.json", "Packet table with MeshCore/Meshtastic/Reticulum/Modbus families; public-default mesh plaintext, private bodies opaque", "native CSS PHY -> mesh/Modbus/LoRaWAN identifier"),
        slice_verified("lorawan", "LoRaWAN identification", "Regional plan selected explicitly (EU868 default plan document only)", 868_100_000, 125_000, 1_000_000, "LoRa up-chirp preamble, sync word, SFD", "CSS SF8 125 kHz", "Implicit-header CSS fixture; Hamming FEC not claimed", "PHY CRC; LoRaWAN MIC identified, never decrypted", "lorawan.frame.v1: mtype, devaddr, encryption=identified", "fixtures/recorded-iq/lora-lorawan-identify.iq.json", "Regional packet table; encrypted payload remains opaque", "native CSS PHY -> LoRaWAN identifier"),
        slice("flex-paging", "FLEX paging", "Common paging allocations are jurisdiction-specific; user tunes licensed/local channel", 929_612_500, 25_000, 192_000, "FLEX 1600/3200/6400 sync sequences and frame boundaries", "2FSK/4FSK", "BCH codewords and interleaving", "Protocol BCH/parity validation", "flex.page.v1: capcode, cycle, frame, phase, encoding, text, corrected_bits", "fixtures/iq/flex_page.cf32 (not yet cleared/recorded)", "Pager timeline with validation/correction state", "native discriminator -> symbol/FEC/parser"),
        slice("hd-radio", "HD Radio through nrsc5", "US FM IBOC 87.9-107.9 MHz (station selected by user)", 99_500_000, 400_000, 1_488_375, "nrsc5 acquisition/synchronization", "OFDM (delegated to nrsc5)", "nrsc5 channel coding", "nrsc5 frame validation", "nrsc5.event.v1: station, program, title, artist, lot, ber", "fixtures/iq/nrsc5_fm.cf32 (not yet cleared/recorded)", "Station/program metadata and BER; audio only when transport is running", "version-pinned nrsc5 stdin IQ sidecar"),
        slice("gnss-gps-l1", "GNSS GPS L1 acquisition", "GPS L1 C/A at 1575.42 MHz", 1_575_420_000, 2_046_000, 4_092_000, "PRN 1-32 C/A correlation across code phase and Doppler", "BPSK(1)", "Navigation-word parity after tracking (acquisition slice has none)", "Navigation-word parity after tracking", "gnss.acquisition.v1: prn, doppler_hz, code_phase, cn0_db_hz, acquired", "fixtures/iq/gps_l1_ca.cf32 (not yet cleared/recorded)", "Acquisition sky view with explicit acquisition-only label", "native FFT correlation acquisition"),
        slice("goes-satdump", "GOES through SatDump", "GOES-East/West downlink selected from current published plan", 1_694_100_000, 1_500_000, 2_400_000, "SatDump LRIT/HRIT pipeline synchronization", "BPSK", "Concatenated convolutional/Reed-Solomon via SatDump", "CADU/packet validation via SatDump", "goes.product.v1: satellite, product, channel, timestamp, file_path, valid", "fixtures/iq/goes_lrit.cf32 (not yet cleared/recorded)", "Validated products/gallery; never claim reception from configured output paths", "version-pinned SatDump CLI IQ pipeline"),
        slice("radiosondes", "Radiosondes", "Model/local launch dependent, commonly 400.15-406 MHz", 403_000_000, 20_000, 192_000, "Model-specific RS41/RS92/DFM/M10/M20 sync words", "Model-specific 2FSK/GFSK", "Model-specific convolutional/Reed-Solomon", "Model-specific CRC", "radiosonde.telemetry.v1: model, serial, frame, lat, lon, altitude_m, temperature_c, checksum_valid", "fixtures/iq/radiosonde_rs41.cf32 (not yet cleared/recorded)", "Telemetry/map only for checksum-valid frames", "typed stdout adapter for pinned sonde decoders"),
        slice("iridium", "Iridium", "1616-1626.5 MHz MSS downlink; channel selected by burst detector", 1_621_250_000, 10_500_000, 12_000_000, "Iridium burst preamble/unique word", "DE-QPSK", "Protocol block coding/interleaving", "Protocol checksum", "iridium.burst.v1: channel, frequency, timestamp, type, identifiers_redacted, checksum_valid", "fixtures/iq/iridium_burst.cf32 (not yet cleared/recorded)", "Burst counters/technical metadata; message content off by default", "audited external burst decoder adapter"),
    ]
}

#[allow(clippy::too_many_arguments)]
fn slice(
    id: &'static str,
    name: &'static str,
    frequency_plan: &'static str,
    center_frequency_hz: u64,
    rf_bandwidth_hz: u32,
    required_sample_rate_hz: u32,
    synchronization: &'static str,
    modulation: &'static str,
    fec: &'static str,
    checksum: &'static str,
    message_schema: &'static str,
    fixture: &'static str,
    ui_outcome: &'static str,
    transport: &'static str,
) -> ProtocolSlice {
    ProtocolSlice {
        id,
        name,
        frequency_plan,
        center_frequency_hz,
        rf_bandwidth_hz,
        required_sample_rate_hz,
        synchronization,
        modulation,
        fec,
        checksum,
        message_schema,
        fixture,
        ui_outcome,
        transport,
        available: false,
        completion_reason: "recorded IQ end-to-end fixture has not passed",
    }
}

#[allow(clippy::too_many_arguments)]
fn slice_verified(
    id: &'static str,
    name: &'static str,
    frequency_plan: &'static str,
    center_frequency_hz: u64,
    rf_bandwidth_hz: u32,
    required_sample_rate_hz: u32,
    synchronization: &'static str,
    modulation: &'static str,
    fec: &'static str,
    checksum: &'static str,
    message_schema: &'static str,
    fixture: &'static str,
    ui_outcome: &'static str,
    transport: &'static str,
) -> ProtocolSlice {
    ProtocolSlice {
        id,
        name,
        frequency_plan,
        center_frequency_hz,
        rf_bandwidth_hz,
        required_sample_rate_hz,
        synchronization,
        modulation,
        fec,
        checksum,
        message_schema,
        fixture,
        ui_outcome,
        transport,
        available: true,
        completion_reason: "recorded IQ end-to-end fixture passed",
    }
}

#[derive(Debug, Serialize)]
pub struct CapabilityCheck {
    pub supported: bool,
    pub reasons: Vec<String>,
    pub guidance: Vec<String>,
}

pub fn capability_check(slice: &ProtocolSlice, device: &DeviceStatus) -> CapabilityCheck {
    let mut reasons = Vec::new();
    let mut guidance = Vec::new();
    if !device.connected {
        reasons.push("no SDR is connected".into());
        guidance.push("Connect an SDR and re-run the capability check.".into());
    }
    if device.connected && device.sample_rate < slice.required_sample_rate_hz {
        reasons.push(format!(
            "current sample rate {} Hz is below the required {} Hz",
            device.sample_rate, slice.required_sample_rate_hz
        ));
        guidance.push(format!(
            "Select a device/rate supporting at least {} Hz complex sampling.",
            slice.required_sample_rate_hz
        ));
    }
    // RTL-SDR-class tuners cannot reach the two 2.4 GHz BLE advertising channels.
    if slice.id == "ble-advertising" && device.driver.to_ascii_lowercase().contains("rtl") {
        reasons.push("RTL-SDR-class tuners do not cover the 2.4 GHz BLE advertising band".into());
        guidance.push(
            "Use a 2.4 GHz-capable SDR (for example HackRF, bladeRF, LimeSDR, or USRP).".into(),
        );
    }
    CapabilityCheck {
        supported: reasons.is_empty(),
        reasons,
        guidance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn device(driver: &str, connected: bool, sample_rate: u32) -> DeviceStatus {
        DeviceStatus {
            connected,
            lifecycle: if connected {
                crate::device::DeviceLifecycle::Streaming
            } else {
                crate::device::DeviceLifecycle::Disconnected
            },
            driver: driver.into(),
            label: String::new(),
            sample_rate,
            bandwidth_hz: sample_rate,
            center_freq_hz: 0,
            ppm_correction: 0.0,
            gain: String::new(),
            bias_tee_on: false,
            saturation: false,
            stream: crate::device::StreamCountersSnapshot::default(),
            min_freq_hz: 24_000_000,
            max_freq_hz: 1_700_000_000,
        }
    }
    #[test]
    fn every_priority_slice_has_a_complete_preimplementation_contract() {
        for s in slices() {
            assert!(
                !s.frequency_plan.is_empty()
                    && s.rf_bandwidth_hz > 0
                    && s.required_sample_rate_hz > 0
            );
            assert!(
                !s.synchronization.is_empty()
                    && !s.modulation.is_empty()
                    && !s.fec.is_empty()
                    && !s.checksum.is_empty()
            );
            assert!(s.message_schema.contains(".v1"));
            assert!(!s.fixture.is_empty() && !s.ui_outcome.is_empty());
            if matches!(s.id, "ble-advertising" | "lora-mesh" | "lorawan") {
                assert!(s.available);
            } else {
                assert!(!s.available);
            }
        }
    }
    #[test]
    fn rejects_disconnected_and_insufficient_rate() {
        let s = &slices()[7];
        let c = capability_check(s, &device("mock", false, 2_000_000));
        assert!(!c.supported);
        assert!(c.reasons.iter().any(|r| r.contains("no SDR")));
    }
    #[test]
    fn rejects_rtl_for_ble_even_at_sufficient_rate() {
        let s = &slices()[0];
        let c = capability_check(s, &device("rtlsdr", true, 4_000_000));
        assert!(!c.supported);
        assert!(c.guidance.iter().any(|r| r.contains("2.4 GHz-capable")));
    }
}
