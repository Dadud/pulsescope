//! Scanner API route composition. Handler contracts live in the parent module.

use super::*;

pub(super) fn router() -> Router<ApiState> {
    Router::new()
        .route(
            "/channels/banks",
            get(channel_banks).post(channel_banks_create),
        )
        .route("/channels/banks/delete", post(channel_banks_delete))
        .route("/channels/banks/create", post(channel_banks_create))
        .route(
            "/channels/bank-scan-config",
            get(channel_bank_scan_config).put(channel_bank_scan_config_put),
        )
        .route("/channels/scan-config", get(scan_config))
        .route("/channels/import", post(channel_import))
        .route("/channels/scan/start", post(scan_start))
        .route("/channels/scan/stop", post(scan_stop))
        .route("/scanner/max-vfos", get(scanner_max_vfos))
        .route("/vfo/states", get(vfo_states))
        .route("/vfo/diagnostics", get(vfo_diagnostics))
        .route("/vfo/:id/mute", post(vfo_mute))
        .route("/vfo/:id/volume", post(vfo_volume))
        .route("/vfo/:id/frequency", post(vfo_frequency))
        .route("/vfo/:id/mode", post(vfo_mode))
        .route("/vfo/:id/audio_agc", post(vfo_agc))
        .route("/vfo/:id/identify", post(vfo_identify))
        .route("/vfo/:id/rds", get(vfo_rds))
        .route("/spectrum", get(spectrum))
        .route("/signal_events", get(signal_events))
        .route("/spectrum_occupancy", get(spectrum_occupancy))
        .route("/signal_id/file", post(signal_id_file))
        .route("/signal_id/fingerprints", get(signal_id_fps))
        .route(
            "/signal_id/fingerprints/:id",
            get(signal_id_fp_one).delete(signal_id_fp_delete),
        )
        .route("/signal_id/fingerprints/match", post(signal_id_fp_match))
        .route("/signal_id/polyphase_extract", post(signal_id_polyphase))
        .route("/signal_id/segment_bursts", post(signal_id_segment))
        .route("/signal_id/classify", post(signal_id_classify))
        .route("/signal_id/auto_decode", post(signal_id_auto_decode))
        .route("/scan/ctcss", get(scan_ctcss))
        .route("/scan/aprs", get(scan_aprs))
        .route("/scan/digital_voice", post(scan_digital_voice))
        .route("/scan/identify_protocol", post(identify_protocol))
        .route("/scan/lock", post(scan_lock))
        .route("/scan/unlock", post(scan_unlock))
        .route("/scan/start", post(scan_start_alt))
        .route("/scan/stop", post(scan_stop_alt))
        .route("/scan/status", get(scan_status))
        .route("/scan/adsb", get(scan_adsb))
        .route("/scan/ais", get(scan_ais).post(native_ais_decode))
        .route("/scan/acars", get(scan_acars).post(native_acars_decode))
        .route("/scan/pocsag", post(native_pocsag_decode))
        .route("/scan/uat", post(native_uat_decode))
        .route("/scan/vdl2", post(native_vdl2_decode))
        .route("/scan/aero", get(scan_aero))
        .route("/scan/ble", get(scan_ble))
        .route("/scan/lora", get(scan_lora))
        .route("/jobs", get(jobs_list).post(jobs_create))
        .route("/jobs/:id", axum::routing::delete(jobs_delete))
}
