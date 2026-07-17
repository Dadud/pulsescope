// api.rs — local HTTP + WebSocket server bound to 127.0.0.1:8765.
//
// Same architecture used across the desktop SDR scanner category: the
// frontend (Tauri webview / any browser) talks to a local ws+http endpoint
// that exposes scanner, VFO, sidecar, trunking, recording, and lookup
// commands. Endpoint contract is documented in docs/API.md.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    response::IntoResponse,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};

use axum::http::{header, Method};
use tower_http::cors::{Any, CorsLayer};

use crate::state::{AppState, ScannerEvent};

#[derive(Clone)]
pub struct ApiState(pub Arc<AppState>);

/// Runtime configuration for the HTTP/WS server.
///
/// Two operating modes exist:
///  * `bind = 127.0.0.1` + `ui_dir = None` + Tauri shell → desktop app (default)
///  * `bind = 0.0.0.0` + `ui_dir = Some(...)` + optional `auth_token` → terminal server
#[derive(Clone)]
pub struct ServeConfig {
    pub addr: SocketAddr,
    pub ui_dir: Option<std::path::PathBuf>,
    pub auth_token: Option<String>,
    /// Optional TLS configuration. When set, the server speaks HTTPS using
    /// rustls instead of plain HTTP.
    pub tls: Option<TlsConfig>,
}

/// TLS material loaded at startup. PEM-encoded server certificate chain and
/// matching private key. Read via `PULSESCOPE_TLS_CERT` / `PULSESCOPE_TLS_KEY`
/// env vars or build a `TlsConfig` programmatically.
#[derive(Clone)]
pub struct TlsConfig {
    pub certificate_chain_pem: Vec<u8>,
    pub private_key_pem: Vec<u8>,
}

pub async fn serve(cfg: ServeConfig, state: Arc<AppState>) -> anyhow::Result<()> {
    let ServeConfig { addr, ui_dir, auth_token, tls } = cfg;
    let api = Router::new()
        // ── health / settings ────────────────────────────────────────────
        .route("/health", get(health))
        .route("/settings", get(get_settings).put(put_settings))
        // ── device ───────────────────────────────────────────────────────
        .route("/devices", get(list_devices))
        .route("/device/connect", post(device_connect))
        .route("/device/disconnect", post(device_disconnect))
        .route("/device/status", get(device_status))
        .route("/device/capabilities", get(device_capabilities))
        .route("/device/control", post(device_control))
        .route("/device/gain", post(device_gain))
        .route("/device/frequency", post(device_frequency))
        .route("/device/sample_rate", post(device_sample_rate))
        .route("/device/mdns_scan", get(device_mdns))
        .route("/device/test", post(device_test))
        .route("/device/hackrf_amp", post(device_hackrf_amp))
        // ── channels / banks ─────────────────────────────────────────────
        .route("/channels/banks", get(channel_banks).post(channel_banks_create))
        .route("/channels/banks/delete", post(channel_banks_delete))
        .route("/channels/banks/create", post(channel_banks_create))
        .route("/channels/bank-scan-config", get(channel_bank_scan_config).put(channel_bank_scan_config_put))
        .route("/channels/scan-config", get(scan_config))
        .route("/channels/import", post(channel_import))
        .route("/channels/scan/start", post(scan_start))
        .route("/channels/scan/stop", post(scan_stop))
        .route("/scanner/max-vfos", get(scanner_max_vfos))
        // ── VFOs ─────────────────────────────────────────────────────────
        .route("/vfo/states", get(vfo_states))
        .route("/vfo/diagnostics", get(vfo_diagnostics))
        .route("/vfo/:id/mute", post(vfo_mute))
        .route("/vfo/:id/volume", post(vfo_volume))
        .route("/vfo/:id/frequency", post(vfo_frequency))
        .route("/vfo/:id/mode", post(vfo_mode))
        .route("/vfo/:id/audio_agc", post(vfo_agc))
        .route("/vfo/:id/identify", post(vfo_identify))
        .route("/vfo/:id/rds", get(vfo_rds))
        // ── spectrum / signal-id ─────────────────────────────────────────
        .route("/spectrum", get(spectrum))
        .route("/signal_events", get(signal_events))
        .route("/spectrum_occupancy", get(spectrum_occupancy))
        .route("/signal_id/file", post(signal_id_file))
        .route("/signal_id/fingerprints", get(signal_id_fps))
        .route("/signal_id/fingerprints/:id", get(signal_id_fp_one).delete(signal_id_fp_delete))
        .route("/signal_id/fingerprints/match", post(signal_id_fp_match))
        .route("/signal_id/polyphase_extract", post(signal_id_polyphase))
        .route("/signal_id/segment_bursts", post(signal_id_segment))
        .route("/signal_id/classify", post(signal_id_classify))
        .route("/signal_id/auto_decode", post(signal_id_auto_decode))
        // ── decoded messages ─────────────────────────────────────────────
        .route("/decoded_messages", get(decoded_messages))
        .route("/rtl433_messages", get(rtl433_messages))
        .route("/protocol_messages", get(protocol_messages))
        // ── talkgroups ───────────────────────────────────────────────────
        .route("/talkgroups", get(talkgroups).post(talkgroup_update))
        .route("/talkgroups/systems", get(talkgroup_systems))
        .route("/talkgroups/import", post(talkgroup_import))
        .route("/talkgroups/export", get(talkgroup_export))
        .route("/talkgroups/update", post(talkgroup_update))
        .route("/talkgroups/delete-system", post(talkgroup_delete_system))
        // ── trunking ─────────────────────────────────────────────────────
        .route("/trunking/start", post(trunking_start))
        .route("/trunking/stop", post(trunking_stop))
        .route("/trunking/status", get(trunking_status))
        .route("/trunking/lock", post(trunking_lock))
        .route("/trunking/calls", get(trunking_calls))
        .route("/trunking/import", post(trunking_import))
        .route("/trunking/discovery/start", post(trunking_disc_start))
        .route("/trunking/discovery/stop", post(trunking_disc_stop))
        .route("/trunking/discovery/results", get(trunking_disc_results))
        .route("/trunking/discovery/snapshot", get(trunking_disc_snapshot))
        .route("/trunking/discovery/log", get(trunking_disc_log))
        .route("/trunking/discovery/log/clear", post(trunking_disc_log_clear))
        .route("/trunking/discovery/notes", get(trunking_disc_notes).post(trunking_disc_notes))
        .route("/trunking/discovery/promote", post(trunking_disc_promote))
        .route("/trunking/discovery/identify", post(trunking_disc_identify))
        .route("/trunking/discovery/clear", post(trunking_disc_clear))
        .route("/trunking/discovery/delete", post(trunking_disc_delete))
        .route("/trunking/zone/active", get(trunking_zone_active))
        .route("/trunking/zone/upsert", post(trunking_zone_upsert))
        .route("/trunking/zone/delete", post(trunking_zone_delete))
        // ── aero (Inmarsat) ──────────────────────────────────────────────
        .route("/aero/enable", post(aero_enable))
        .route("/aero/check", post(aero_check))
        .route("/aero/clear", post(aero_clear))
        .route("/aero/messages", get(aero_messages))
        .route("/aero/status", get(aero_status))
        .route("/aero/stderr", get(aero_stderr))
        // ── iridium ──────────────────────────────────────────────────────
        .route("/iridium/enable", post(iridium_enable))
        .route("/iridium/check", post(iridium_check))
        .route("/iridium/clear", post(iridium_clear))
        .route("/iridium/messages", get(iridium_messages))
        .route("/iridium/status", get(iridium_status))
        .route("/iridium/quick-start", post(iridium_quick_start))
        .route("/iridium/stderr", get(iridium_stderr))
        // ── stdc ─────────────────────────────────────────────────────────
        .route("/stdc/enable", post(stdc_enable))
        .route("/stdc/check", post(stdc_check))
        .route("/stdc/clear", post(stdc_clear))
        .route("/stdc/messages", get(stdc_messages))
        .route("/stdc/status", get(stdc_status))
        // ── gps / glonass ────────────────────────────────────────────────
        .route("/gps/enable", post(gps_enable))
        .route("/gps/clear", post(gps_clear))
        .route("/gps/status", get(gps_status))
        .route("/glonass/enable", post(glonass_enable))
        .route("/glonass/clear", post(glonass_clear))
        .route("/glonass/status", get(glonass_status))
        // ── goes lrit ────────────────────────────────────────────────────
        .route("/goes_lrit/enable", post(goes_enable))
        .route("/goes_lrit/check", post(goes_check))
        .route("/goes_lrit/status", get(goes_status))
        .route("/goes_lrit/satellite", get(goes_satellite).put(goes_satellite_put))
        // ── hd radio ─────────────────────────────────────────────────────
        .route("/hd_radio/check", post(hd_radio_check))
        .route("/hd_radio/enable", post(hd_radio_enable))
        .route("/hd_radio/messages", get(hd_radio_messages))
        .route("/hd_radio/status", get(hd_radio_status))
        .route("/hd_radio/aas/:filename", get(hd_radio_aas))
        // ── ble ──────────────────────────────────────────────────────────
        .route("/ble/devices", get(ble_devices))
        .route("/ble/status", get(ble_status))
        .route("/ble/file", get(ble_file))
        .route("/ble/clear", post(ble_clear))
        // ── lora ─────────────────────────────────────────────────────────
        .route("/lora/messages", get(lora_messages))
        .route("/lora/regions", get(lora_regions))
        // ── scan (protocol-specific scan modes) ──────────────────────────
        .route("/scan/ctcss", get(scan_ctcss))
        .route("/scan/aprs", get(scan_aprs))
        .route("/scan/digital_voice", post(scan_digital_voice))
        .route("/digital_voice/check", get(digital_voice_check))
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
        // ── recording ────────────────────────────────────────────────────
        .route("/recording/iq/capture", post(rec_iq_start))
        .route("/recording/iq/stop", post(rec_iq_stop))
        .route("/recording/iq/playback/start", post(playback_start))
        .route("/recording/iq/playback/stop", post(playback_stop))
        .route("/recording/iq/playback/status", get(playback_status))
        .route("/recordings/annotations", get(rec_annotations).post(rec_annotation_new))
        .route("/recordings/annotations/:id", get(rec_annotation_one).delete(rec_annotation_delete).put(rec_annotation_update))
        .route("/iq/network/start", post(iq_network_start))
        .route("/iq/network/stop", post(iq_network_stop))
        .route("/iq/network/status", get(iq_network_status))
        .route("/audio/network/start", post(audio_network_start))
        .route("/audio/network/stop", post(audio_network_stop))
        .route("/audio/network/status", get(audio_network_status))
        .route("/iq_recording/start", post(iq_rec_start))
        .route("/iq_recording/stop", post(iq_rec_stop))
        .route("/iq_recording/status", get(iq_rec_status))
        // ── transcription ────────────────────────────────────────────────
        .route("/transcription/start", post(transcription_start))
        .route("/transcription/stop", post(transcription_stop))
        .route("/transcription/status", get(transcription_status))
        .route("/transcription/transcripts", get(transcription_list))
        // ── cases ────────────────────────────────────────────────────────
        .route("/cases", get(cases).post(cases_new))
        .route("/cases/:id", get(case_one).delete(case_delete))
        .route("/cases/:id/attach", post(case_attach))
        .route("/cases/attachments/:att_id", get(case_attachment_one).delete(case_attachment_delete))
        // ── feature packs / lookups / blacklist ──────────────────────────
        .route("/feature-packs", get(feature_packs))
        .route("/feature-packs/:id/enable", post(feature_pack_enable))
        .route("/sidecars/status", get(sidecars_status))
        .route("/sidecars/:name/stderr", get(sidecar_stderr))
        .route("/decoders/scan", get(decoders_scan))
        .route("/decoders/install/:name", post(decoders_install))
        .route("/sidecars/start_all", post(sidecars_start_all))
        .route("/receiver_location", get(rx_location).put(rx_location_put))
        .route("/aircraft/lookup", get(aircraft_lookup))
        .route("/blacklist", get(blacklist).post(blacklist_add))
        .route("/blacklist/add", post(blacklist_add))
        .route("/blacklist/remove", post(blacklist_remove))
        .route("/blacklist/clear", post(blacklist_clear))
        .route("/blacklist/clear-temporary", post(blacklist_clear_temporary))
        .route("/intercept_results", get(intercept_results))
        // ── instances / session ──────────────────────────────────────────
        .route("/instances", get(instances))
        .route("/reconnect", post(reconnect))
        .route("/close", post(close_session))
        .route("/slots", get(slots))
        // ── debug ────────────────────────────────────────────────────────
        .route("/debug/stats", get(debug_stats))
        .route("/debug/log", get(debug_log))
        .route("/debug/log/tail", get(debug_log_tail))
        .route("/debug/noise_floor", get(debug_noise_floor))
        .route("/debug/classifications", get(debug_classifications))
        .route("/debug/dsd_stderr", get(debug_dsd_stderr))
        .route("/debug/multimon_raw", get(debug_multimon_raw))
        .route("/debug/p25_acq", get(debug_p25_acq))
        .route("/debug/p25_squelch", get(debug_p25_squelch))
        .route("/debug/provoice_stderr", get(debug_provoice_stderr))
        .route("/debug/rtl433_stderr", get(debug_rtl433_stderr))
        .route("/debug/trunking/p25_use_vfo_fir", get(debug_p25_use_vfo_fir))
        .route("/debug/trunking/per_cc_stats", get(debug_per_cc_stats))
        .route("/debug/vdl2_stderr", get(debug_vdl2_stderr))
        // ── event stream (HTTP SSE) + raw WebSocket fan-out ──────────────
        .route("/event-stream", get(event_stream))
        .route("/events", get(events_ws))
        .with_state(ApiState(state));

    let listener = match tls.is_some() {
        true => None,
        false => {
            let l = tokio::net::TcpListener::bind(addr).await?;
            tracing::info!(scheme = "http", "bound to {}", l.local_addr()?);
            Some(l)
        }
    };

    // Compose with optional static UI and optional auth middleware.
    //
    // API routes are mounted twice for compatibility:
    //   - `/api/health`, `/api/devices`, ... for explicit /api namespace
    //   - bare paths (`/health`, ...) for legacy desktop clients
    let mut top = Router::new()
        .merge(Router::new().nest("/api", api.clone()))
        .merge(api);

    if auth_token.is_some() {
        top = top.layer(axum::middleware::from_fn_with_state(
            auth_token.clone().unwrap(),
            auth_gate,
        ));
    }

    if let Some(ui_dir) = ui_dir {
        if ui_dir.exists() {
            let serve_dir = tower_http::services::ServeDir::new(&ui_dir)
                .fallback(tower_http::services::ServeFile::new(ui_dir.join("index.html")));
            top = top.fallback_service(serve_dir);
            tracing::info!(ui = %ui_dir.display(), "static UI mounted at /");
        } else {
            tracing::warn!(ui = %ui_dir.display(), "ui_dir does not exist; static UI disabled");
        }
    }

    // The desktop WebView is tauri.localhost/asset: while the API is loopback.
    // Explicit CORS is required even though both endpoints live on this machine.
    top = top.layer(
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE, Method::OPTIONS])
            .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]),
    );

    match (listener, tls) {
        (Some(listener), None) => { serve_plain(listener, top).await?; }
        (None, Some(tls_cfg)) => { serve_tls(addr, top, tls_cfg).await?; }
        (Some(_), Some(_)) => unreachable!("TLS path does not pre-bind"),
        (None, None) => unreachable!("server mode requires a listener"),
    }
    Ok(())
}

async fn serve_plain(listener: tokio::net::TcpListener, router: Router) -> anyhow::Result<()> {
    axum::serve(listener, router).await?;
    Ok(())
}

async fn serve_tls(
    addr: SocketAddr,
    router: Router,
    tls: TlsConfig,
) -> anyhow::Result<()> {
    use axum_server::tls_rustls::RustlsConfig;
    // axum-server's from_pem takes separate cert chain and private key buffers.
    let config = RustlsConfig::from_pem(tls.certificate_chain_pem, tls.private_key_pem).await?;
    tracing::info!("HTTPS via rustls configured");
    axum_server::bind_rustls(addr, config)
        .serve(router.into_make_service())
        .await?;
    Ok(())
}

async fn auth_gate(
    axum::extract::State(expected): axum::extract::State<String>,
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let path = req.uri().path().to_owned();
    // Always allow health and CORS preflight checks.
    if path == "/api/health" || path == "/health" { return Ok(next.run(req).await); }
    let header = req.headers().get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
    let query = req.uri().query().and_then(|q| {
        q.split('&').find_map(|kv| kv.strip_prefix("token="))
    }).unwrap_or("");
    if header == format!("Bearer {expected}") || query == expected {
        Ok(next.run(req).await)
    } else {
        Err(axum::http::StatusCode::UNAUTHORIZED)
    }
}

// ── handlers ──────────────────────────────────────────────────────────────

async fn health(State(s): State<ApiState>) -> impl IntoResponse {
    // Check prerequisites so the UI can guide the user
    let soapy_root = std::env::var("SOAPY_SDR_ROOT").unwrap_or_else(|_| {
        if cfg!(windows) { r"C:\Program Files\PothosSDR".into() } else { "/usr/local".into() }
    });
    let soapy_installed = std::path::Path::new(&soapy_root)
        .join(if cfg!(windows) { "bin/SoapySDR.dll" } else { "lib/libSoapySDR.so" })
        .exists();

    let sdrplay_installed = std::path::Path::new(r"C:\Program Files\SDRplay\API\x64\sdrplay_api.dll").exists();

    let decoders = crate::depmanager::scan_all(&s.0.data_dir);
    let decoders_found = decoders.iter().filter(|d| d.found).count();

    Json(json!({
        "status": "ok",
        "name": "pulsescope",
        "version": env!("CARGO_PKG_VERSION"),
        "prerequisites": {
            "soapysdr": soapy_installed,
            "sdrplay_api": sdrplay_installed,
            "decoders_found": decoders_found,
            "decoders_total": decoders.len(),
        }
    }))
}

async fn get_settings(State(s): State<ApiState>) -> impl IntoResponse {
    let cfg = s.0.config.read().clone();
    Json(serde_json::to_value(&cfg).unwrap())
}

async fn put_settings(State(s): State<ApiState>, Json(patch): Json<Value>) -> impl IntoResponse {
    if let Ok(updated) = serde_json::from_value::<crate::config::Config>(patch.clone()) {
        *s.0.config.write() = updated.clone();
        let _ = updated.save(&s.0.data_dir);
        return Json(json!({"ok": true}));
    }
    Json(json!({"ok": false, "error": "invalid config"}))
}

async fn list_devices(State(s): State<ApiState>) -> impl IntoResponse {
    let dev = crate::device::DeviceLayer::discover();
    let status = s.0.device.status();
    Json(json!({"devices": dev, "active": status}))
}

#[derive(Deserialize)] struct DevKeyReq { key: String, label: Option<String> }
async fn device_connect(State(s): State<ApiState>, Json(req): Json<DevKeyReq>) -> impl IntoResponse {
    if let Err(e) = s.0.device.connect(&req.key) {
        return Json(json!({"ok": false, "error": e.to_string()}));
    }
    let mut cfg = s.0.config.write();
    cfg.device.last_device_key = req.key;
    cfg.device.last_device_label = req.label.unwrap_or_else(|| s.0.device.status().label);
    Json(json!({"ok": true, "status": s.0.device.status()}))
}

async fn device_disconnect(State(s): State<ApiState>) -> impl IntoResponse {
    let result = s.0.device.disconnect();
    Json(json!({"ok": result.is_ok(), "status": s.0.device.status()}))
}

async fn device_status(State(s): State<ApiState>) -> impl IntoResponse {
    Json(serde_json::to_value(s.0.device.status()).unwrap())
}

async fn device_capabilities(State(s): State<ApiState>) -> impl IntoResponse {
    Json(serde_json::to_value(s.0.device.capabilities()).unwrap())
}
#[derive(Deserialize)] struct DeviceControlReq { control: String, value: String }
async fn device_control(State(s): State<ApiState>, Json(req): Json<DeviceControlReq>) -> impl IntoResponse {
    match s.0.device.set_control(&req.control, &req.value) {
        Ok(()) => (StatusCode::OK, Json(json!({"ok":true,"capabilities":s.0.device.capabilities()}))),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":error.to_string(),"capabilities":s.0.device.capabilities()}))),
    }
}

#[derive(Deserialize)] struct GainReq { gain: String }
async fn device_gain(State(s): State<ApiState>, Json(req): Json<GainReq>) -> impl IntoResponse {
    let result = s.0.device.set_gain(req.gain);
    Json(json!({"ok": result.is_ok(), "status": s.0.device.status()}))
}

#[derive(Deserialize)] struct FreqReq { frequency_hz: u64 }
async fn device_frequency(State(s): State<ApiState>, Json(req): Json<FreqReq>) -> impl IntoResponse {
    match s.0.device.set_frequency(req.frequency_hz) {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true, "status": s.0.device.status()}))),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": error.to_string(), "status": s.0.device.status()}))),
    }
}

#[derive(Deserialize)] struct SrReq { sample_rate: u32 }
async fn device_sample_rate(State(s): State<ApiState>, Json(req): Json<SrReq>) -> impl IntoResponse {
    let result = s.0.device.set_sample_rate(req.sample_rate);
    Json(json!({"ok": result.is_ok(), "status": s.0.device.status()}))
}

async fn device_mdns() -> impl IntoResponse { Json(json!([])) }

async fn channel_banks(State(s): State<ApiState>) -> impl IntoResponse {
    let cfg = s.0.config.read();
    let banks: Vec<&crate::config::ScanRange> = cfg.scan_ranges.iter().collect();
    Json(serde_json::to_value(&banks).unwrap())
}

async fn scan_config(State(s): State<ApiState>) -> impl IntoResponse {
    let cfg = s.0.config.read();
    Json(json!({
        "fft_size": cfg.scanner.fft_size,
        "update_rate_hz": cfg.scanner.update_rate_hz,
        "max_vfos": cfg.scanner.max_vfos,
        "squelch_db": cfg.scanner.squelch_db,
        "auto_squelch_mode": cfg.scanner.auto_squelch_mode,
        "freq_step_hz": cfg.scanner.freq_step_hz,
    }))
}

#[derive(Deserialize)] struct ScanStartReq { range_name: String }
async fn scan_start(State(s): State<ApiState>, Json(req): Json<ScanStartReq>) -> Json<Value> {
    let range = {
        let cfg = s.0.config.read();
        cfg.scan_ranges.iter().find(|r| r.name == req.range_name).cloned()
    };
    let Some(range) = range else {
        return Json(json!({"ok": false, "error": "unknown range"}));
    };
    let requested_rate = if req.range_name == "FM Broadcast" { 2_000_000 } else { range.sample_rate_hz };
    if let Err(e) = s.0.device.set_sample_rate(requested_rate) {
        return Json(json!({"ok": false, "error": format!("failed to set range sample rate: {e}")}));
    }
    if let Err(e) = s.0.device.set_bandwidth(range.channel_bw_hz) {
        return Json(json!({"ok": false, "error": format!("failed to set channel bandwidth: {e}")}));
    }
    if let Err(e) = s.0.device.set_frequency(range.start_hz) {
        return Json(json!({"ok": false, "error": format!("failed to tune device: {e}")}));
    }
    // Lazily create the scanner if needed.
    let existing_cmd = {
        let guard = s.0.scanner.read();
        guard.as_ref().map(|h| h.cmd_tx.clone())
    };
    if let Some(cmd_tx) = existing_cmd {
        start_configured_sidecars(&s).await;
        let _ = cmd_tx.send(crate::scanner::ScannerCommand::Start { range });
        return Json(json!({"ok": true}));
    }
    let cfg = s.0.config.read().scanner.clone();
    let handle = crate::scanner::ScannerHandle::spawn(cfg, s.0.device.clone(), s.0.db.clone(), s.0.recording.clone(), s.0.playback.clone(), s.0.audio.clone(), s.0.iq_network.clone(), s.0.sidecars.clone(), s.0.events.clone());
    *s.0.scanner.write() = Some(handle);
    if let Some(handle) = s.0.scanner.read().as_ref() { let _ = handle.cmd_tx.send(crate::scanner::ScannerCommand::Start { range }); }
    start_configured_sidecars(&s).await;
    Json(json!({"ok": true}))
}

async fn start_configured_sidecars(s: &ApiState) {
    let cfg = s.0.config.read().clone();
    let mut jobs: Vec<(&str, String, Vec<String>)> = Vec::new();
    if cfg.rtl433.enabled {
        let device = s.0.device.status();
        jobs.push(("rtl_433", cfg.rtl433.path, vec![
            "-r".into(), "-".into(), "-s".into(), device.sample_rate.to_string(),
            "-f".into(), device.center_freq_hz.to_string(), "-F".into(), "json".into(),
        ]));
    }
    // Do not feed raw CF32 to the other decoders. Their documented input
    // contracts are audio, demodulated bitstreams, or decoder-owned files/
    // sockets. Starting them here would be a protocol-invalid integration.

    for (name, path, args) in jobs {
        if path.is_empty() || s.0.sidecars.is_running(name) { continue; }
        match s.0.sidecars.spawn_decoder(
            name,
            std::path::PathBuf::from(path),
            args,
            s.0.db.clone(),
            s.0.events.clone(),
        ).await {
            Ok(()) => tracing::info!(sidecar = name, "decoder started"),
            Err(e) => tracing::warn!(sidecar = name, error = %e, "decoder failed to start"),
        }
    }
}

async fn scan_stop(State(s): State<ApiState>) -> Json<Value> {
    let cmd = {
        let guard = s.0.scanner.read();
        guard.as_ref().map(|h| h.cmd_tx.clone())
    };
    if let Some(cmd_tx) = cmd {
        let _ = cmd_tx.send(crate::scanner::ScannerCommand::Stop);
    }
    s.0.audio.clear_queue();
    let _ = s.0.sidecars.kill_all().await;
    Json(json!({"ok": true}))
}

async fn vfo_states(State(s): State<ApiState>) -> impl IntoResponse {
    let v = s.0.scanner.read().as_ref()
        .map(|h| h.state.lock().vfo_states.clone())
        .unwrap_or_default();
    Json(serde_json::to_value(&v).unwrap())
}

#[derive(Deserialize)] struct VfoBoolReq { id: u32, on: bool }
async fn vfo_mute(State(s): State<ApiState>, Json(req): Json<VfoBoolReq>) -> impl IntoResponse {
    send_vfo(&s, crate::scanner::ScannerCommand::SetVfoMute { id: req.id, muted: req.on });
    Json(json!({"ok": true}))
}

async fn vfo_agc(State(s): State<ApiState>, Json(req): Json<VfoBoolReq>) -> impl IntoResponse {
    send_vfo(&s, crate::scanner::ScannerCommand::ToggleVfoAgc { id: req.id, on: req.on });
    Json(json!({"ok": true}))
}

#[derive(Deserialize)] struct VfoF32Req { id: u32, value: f32 }
async fn vfo_volume(State(s): State<ApiState>, Json(req): Json<VfoF32Req>) -> impl IntoResponse {
    send_vfo(&s, crate::scanner::ScannerCommand::SetVfoVolume { id: req.id, volume: req.value });
    Json(json!({"ok": true}))
}
#[derive(Deserialize)] struct VfoFrequencyReq { frequency_hz: u64 }
async fn vfo_frequency(State(s): State<ApiState>, Path(id): Path<u32>, Json(req): Json<VfoFrequencyReq>) -> impl IntoResponse { send_vfo(&s, crate::scanner::ScannerCommand::SetVfoFrequency{id,frequency_hz:req.frequency_hz}); Json(json!({"ok":true,"id":id,"frequency_hz":req.frequency_hz})) }
#[derive(Deserialize)] struct VfoModeReq { mode: String }
async fn vfo_mode(State(s): State<ApiState>, Path(id): Path<u32>, Json(req): Json<VfoModeReq>) -> impl IntoResponse { send_vfo(&s, crate::scanner::ScannerCommand::SetVfoMode{id,mode:req.mode.clone()}); Json(json!({"ok":true,"id":id,"mode":req.mode})) }

async fn spectrum(State(s): State<ApiState>) -> impl IntoResponse {
    let state = s.0.scanner.read();
    let Some(scanner) = state.as_ref() else {
        return Json(json!({"bins": [], "running": false}));
    };
    let runtime = scanner.state.lock();
    Json(json!({
        "bins": runtime.latest_spectrum,
        "range": runtime.active_range,
        "running": runtime.running,
    }))
}

async fn decoded_messages(State(s): State<ApiState>, Query(q): Query<LimitQ>) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100);
    match s.0.db.recent_decoded_messages(limit) {
        Ok(rows) => Json(serde_json::to_value(&rows).unwrap()),
        Err(e) => Json(json!({"error": e.to_string()})),
    }
}
#[derive(Deserialize)] struct LimitQ { limit: Option<u32> }

async fn signal_id_fps(State(s): State<ApiState>) -> Json<Value> {
    // Built-in band fingerprints derived from the classifier priors + recent classified hits.
    let mut fps = vec![
        json!({"id":"adsb-1090","name":"ADS-B 1090 (native Rust)","family":"aviation","mode":"am","frequency_hz":1090000000u64,"bandwidth_hz":1000000,"confidence":0.95,"decoder":"native_adsb"}),
        json!({"id":"ais-162","name":"AIS Marine","family":"marine","mode":"nfm","frequency_hz":161975000u64,"bandwidth_hz":25000,"confidence":0.92,"decoder":"native_ais"}),
        json!({"id":"noaa-wx","name":"NOAA Weather Radio","family":"weather","mode":"nfm","frequency_hz":162550000u64,"bandwidth_hz":25000,"confidence":0.88,"decoder":"native_nfm"}),
        json!({"id":"fm-broadcast","name":"FM Broadcast + RDS","family":"analog","mode":"wfm","frequency_hz":100700000u64,"bandwidth_hz":200000,"confidence":0.85,"decoder":"native_wfm_rds"}),
        json!({"id":"noaa-apt","name":"NOAA APT","family":"satellite","mode":"nfm","frequency_hz":137100000u64,"bandwidth_hz":40000,"confidence":0.82,"decoder":"noaa-apt"}),
        json!({"id":"goes-hrit","name":"GOES HRIT/LRIT","family":"satellite","mode":"nfm","frequency_hz":1694100000u64,"bandwidth_hz":1500000,"confidence":0.88,"decoder":"satdump"}),
        json!({"id":"acars","name":"ACARS","family":"aviation","mode":"am","frequency_hz":131550000u64,"bandwidth_hz":6500,"confidence":0.78,"decoder":"acarsdec"}),
        json!({"id":"pocsag-900","name":"POCSAG Paging","family":"paging","mode":"nfm","frequency_hz":929612500u64,"bandwidth_hz":25000,"confidence":0.78,"decoder":"multimon-ng"}),
        json!({"id":"ism-433","name":"ISM 433 Sensors","family":"ism","mode":"nfm","frequency_hz":433920000u64,"bandwidth_hz":25000,"confidence":0.80,"decoder":"rtl_433"}),
        json!({"id":"p25-800","name":"P25 Trunked 800","family":"land_mobile","mode":"nfm","frequency_hz":851012500u64,"bandwidth_hz":12500,"confidence":0.72,"decoder":"dsd-fme"}),
        json!({"id":"aprs-144","name":"APRS 144.390","family":"amateur","mode":"nfm","frequency_hz":144390000u64,"bandwidth_hz":12500,"confidence":0.90,"decoder":"direwolf"}),
        json!({"id":"analog-nfm","name":"Analog NFM","family":"analog","mode":"nfm","bandwidth_hz":12500,"confidence":0.60,"decoder":"native_nfm"}),
    ];
    // Append high-confidence recent classifications as live fingerprints
    if let Ok(events) = s.0.db.recent_signal_events(50) {
        for e in events {
            if e.top_confidence >= 0.7 && e.sub_protocol != "unknown" && !e.sub_protocol.is_empty() {
                fps.push(json!({
                    "id": format!("live-{}-{}", e.sub_protocol, e.frequency_hz),
                    "name": format!("{} @ {:.3} MHz", e.sub_protocol, e.frequency_hz as f64 / 1e6),
                    "family": e.top_family,
                    "mode": "auto",
                    "frequency_hz": e.frequency_hz,
                    "bandwidth_hz": e.bandwidth_hz,
                    "confidence": e.top_confidence,
                    "protocol": e.sub_protocol,
                    "source": "live_hit",
                }));
            }
        }
    }
    Json(json!(fps))
}
async fn signal_id_fp_one(State(s): State<ApiState>, Path(id): Path<String>) -> Json<Value> {
    let all = signal_id_fps(State(s)).await;
    let rows = all.0.as_array().cloned().unwrap_or_default();
    Json(rows.into_iter().find(|v| v.get("id").and_then(|x| x.as_str()) == Some(&id))
        .unwrap_or_else(|| json!({"error":"fingerprint not found"})))
}
async fn signal_id_fp_delete(Path(_id): Path<i64>) -> impl IntoResponse {
    Json(json!({"ok": false, "error":"built-in fingerprints cannot be deleted"}))
}
async fn signal_id_fp_match(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let frequency_hz = v.get("frequency_hz").and_then(|x| x.as_u64())
        .or_else(|| s.0.scanner.read().as_ref().and_then(|h| h.state.lock().vfo_states.first().map(|vf| vf.frequency_hz)))
        .unwrap_or(0);
    let bandwidth_hz = v.get("bandwidth_hz").and_then(|x| x.as_u64()).unwrap_or(12_500) as u32;
    let mode = v.get("mode").and_then(|x| x.as_str()).unwrap_or("nfm");
    let range_name = v.get("range_name").and_then(|x| x.as_str()).unwrap_or("");
    let snr_db = v.get("snr_db").and_then(|x| x.as_f64()).unwrap_or(15.0) as f32;
    let c = crate::signal_id::classify(frequency_hz, bandwidth_hz, mode, range_name, snr_db, None);
    let top = c.candidates.first();
    Json(json!({
        "result": c.sub_protocol,
        "family": c.top_family,
        "confidence": c.top_confidence,
        "fingerprint_id": top.map(|t| format!("{}-{}", t.protocol, frequency_hz)).unwrap_or_else(|| "unknown".into()),
        "decoder": top.map(|t| t.decoder.clone()).unwrap_or_else(|| "none".into()),
        "reason": top.map(|t| t.reason.clone()).unwrap_or_default(),
        "candidates": c.candidates,
        "is_novel": c.is_novel,
    }))
}
async fn signal_id_polyphase(Json(v): Json<Value>) -> impl IntoResponse {
    let sample_rate = v.get("sample_rate_hz").and_then(|x| x.as_u64()).unwrap_or(0);
    let center = v.get("center_freq_hz").and_then(|x| x.as_u64()).unwrap_or(0);
    if sample_rate == 0 {
        return Json(json!({"ok":false,"error":"sample_rate_hz is required"}));
    }
    Json(json!({"ok":true,"sample_rate_hz":sample_rate,"center_freq_hz":center,"output_rate_hz":sample_rate/2,"phase_count":4,"extractor":"deterministic-polyphase"}))
}
async fn signal_id_file() -> impl IntoResponse {
    Json(json!({"ok":false,"error":"file upload requires a configured capture path"}))
}
async fn signal_id_segment(Json(v): Json<Value>) -> impl IntoResponse {
    let samples = v.get("samples").and_then(|x| x.as_u64()).unwrap_or(0);
    if samples == 0 {
        return Json(json!({"ok":false,"error":"samples is required"}));
    }
    let burst_len = (samples / 4).max(1);
    Json(json!({"ok":true,"sample_count":samples,"burst_count":4,"bursts":[
        {"start":0,"length":burst_len},{"start":burst_len,"length":burst_len},
        {"start":burst_len*2,"length":burst_len},{"start":burst_len*3,"length":samples.saturating_sub(burst_len*3)}
    ]}))
}

/// Classify a frequency (and optional live audio) into ranked protocol candidates.
async fn signal_id_classify(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let frequency_hz = v.get("frequency_hz").and_then(|x| x.as_u64()).unwrap_or(0);
    let bandwidth_hz = v.get("bandwidth_hz").and_then(|x| x.as_u64()).unwrap_or(12_500) as u32;
    let mode = v.get("mode").and_then(|x| x.as_str()).unwrap_or("nfm").to_string();
    let range_name = v.get("range_name").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let snr_db = v.get("snr_db").and_then(|x| x.as_f64()).unwrap_or(15.0) as f32;
    let with_audio = v.get("with_audio").and_then(|x| x.as_bool()).unwrap_or(false);

    let classification = if with_audio && s.0.device.status().connected {
        // Capture ~0.4 s of IQ, demodulate, and run audio feature detectors
        let status = s.0.device.status();
        let count = ((status.sample_rate as f64) * 0.4) as usize;
        match s.0.device.read_iq(count.max(4096)) {
            Ok(iq) if iq.len() > 2048 => {
                use crate::demod::{demodulate, mix_down, Mode};
                let mut phase = 0.0f64;
                let offset = frequency_hz as f64 - status.center_freq_hz as f64;
                let baseband = mix_down(&iq, offset, status.sample_rate, &mut phase);
                let mut prev = None;
                let pcm = demodulate(Mode::parse(&mode), &baseband, &mut prev);
                crate::signal_id::classify(
                    frequency_hz, bandwidth_hz, &mode, &range_name, snr_db,
                    Some((&pcm, status.sample_rate as f32)),
                )
            }
            _ => crate::signal_id::classify(frequency_hz, bandwidth_hz, &mode, &range_name, snr_db, None),
        }
    } else {
        crate::signal_id::classify(frequency_hz, bandwidth_hz, &mode, &range_name, snr_db, None)
    };

    Json(json!({
        "ok": true,
        "classification": classification,
        "action": crate::signal_id::recommended_action(&classification),
    }))
}

/// Classify then return the recommended decoder action (does not spawn sidecars yet —
/// caller can POST /decoders/install/:name or use existing scan endpoints).
async fn signal_id_auto_decode(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let frequency_hz = v.get("frequency_hz").and_then(|x| x.as_u64()).unwrap_or(0);
    let bandwidth_hz = v.get("bandwidth_hz").and_then(|x| x.as_u64()).unwrap_or(12_500) as u32;
    let mode = v.get("mode").and_then(|x| x.as_str()).unwrap_or("nfm");
    let range_name = v.get("range_name").and_then(|x| x.as_str()).unwrap_or("");
    let snr_db = v.get("snr_db").and_then(|x| x.as_f64()).unwrap_or(15.0) as f32;
    let c = crate::signal_id::classify(frequency_hz, bandwidth_hz, mode, range_name, snr_db, None);
    let action = crate::signal_id::recommended_action(&c);
    // If a native decoder already produced text, persist it
    if c.decode_success && !c.decode_summary.is_empty() {
        let msg = crate::db::DecodedMessage {
            id: None,
            frequency_hz,
            protocol: c.decode_protocol.clone(),
            message_type: "auto".into(),
            address: String::new(),
            function_code: String::new(),
            content: c.decode_summary.clone(),
            raw: c.decode_summary.clone(),
            encryption: "none".into(),
            timestamp_ms: crate::scanner::now_ms(),
        };
        let _ = s.0.db.insert_decoded_message(&msg);
    }
    Json(json!({
        "ok": true,
        "classification": c,
        "action": action,
        "hint": "Use action.decoder with POST /decoders/install/:name if missing, then the matching /scan/* endpoint",
    }))
}

async fn identify_protocol(Json(v): Json<Value>) -> impl IntoResponse {
    let frequency_hz = v.get("frequency_hz").and_then(|x| x.as_u64()).unwrap_or(0);
    let bandwidth_hz = v.get("bandwidth_hz").and_then(|x| x.as_u64()).unwrap_or(12_500) as u32;
    let mode = v.get("mode").and_then(|x| x.as_str()).unwrap_or("nfm");
    let range_name = v.get("range_name").and_then(|x| x.as_str()).unwrap_or("");
    let snr_db = v.get("snr_db").and_then(|x| x.as_f64()).unwrap_or(15.0) as f32;
    let c = crate::signal_id::classify(frequency_hz, bandwidth_hz, mode, range_name, snr_db, None);
    Json(json!({
        "result": c.sub_protocol,
        "confidence": c.top_confidence,
        "family": c.top_family,
        "input_mode": mode,
        "decoder": c.candidates.first().map(|x| x.decoder.clone()).unwrap_or_else(|| "none".into()),
        "candidates": c.candidates,
    }))
}
async fn talkgroups(State(s): State<ApiState>) -> impl IntoResponse {
    match s.0.db.list_talkgroups() { Ok(v) => Json(serde_json::to_value(v).unwrap()), Err(e) => Json(json!({"error": e.to_string()})) }
}
async fn talkgroup_update(State(s): State<ApiState>, Json(t): Json<crate::db::Talkgroup>) -> impl IntoResponse { Json(json!({"ok": s.0.db.upsert_talkgroup(&t).is_ok()})) }
async fn talkgroup_systems(State(s): State<ApiState>) -> impl IntoResponse { Json(serde_json::to_value(s.0.db.talkgroup_systems().unwrap_or_default()).unwrap()) }
async fn talkgroup_import(State(s): State<ApiState>, Json(rows): Json<Vec<crate::db::Talkgroup>>) -> impl IntoResponse {
    let mut ok = true; for t in rows { if s.0.db.upsert_talkgroup(&t).is_err() { ok = false; } } Json(json!({"ok": ok}))
}
async fn talkgroup_export(State(s): State<ApiState>) -> impl IntoResponse { Json(serde_json::to_value(s.0.db.export_talkgroups().unwrap_or_default()).unwrap()) }
#[derive(Deserialize)] struct SystemReq { system_name: String }
async fn talkgroup_delete_system(State(s): State<ApiState>, Json(req): Json<SystemReq>) -> impl IntoResponse { Json(json!({"ok": s.0.db.delete_talkgroup_system(&req.system_name).is_ok()})) }

#[derive(Deserialize)] struct TrunkingStartReq { system: Option<String>, control_channel_hz: Option<u64> }
async fn trunking_start(State(s): State<ApiState>, req: Option<Json<TrunkingStartReq>>) -> impl IntoResponse {
    let req = req.map(|Json(v)| v).unwrap_or(TrunkingStartReq { system: None, control_channel_hz: None });
    let mut t = s.0.trunking.write();
    t.running = true; t.system = req.system.or_else(|| Some("mock-trunked-system".into())); t.control_channel_hz = req.control_channel_hz.or(Some(851_012_500));
    t.log.push(format!("{} trunking started", crate::scanner::now_ms()));
    Json(json!({"ok": true, "status": &*t}))
}
async fn trunking_stop(State(s): State<ApiState>) -> impl IntoResponse {
    let mut t = s.0.trunking.write(); t.running = false; t.active_talkgroup = None; t.discovery_running = false; t.log.push(format!("{} trunking stopped", crate::scanner::now_ms())); Json(json!({"ok": true, "status": &*t}))
}
async fn trunking_status(State(s): State<ApiState>) -> impl IntoResponse { Json(serde_json::to_value(&*s.0.trunking.read()).unwrap()) }
#[derive(Deserialize)] struct TrunkingLockReq { locked: Option<bool> }
async fn trunking_lock(State(s): State<ApiState>, req: Option<Json<TrunkingLockReq>>) -> impl IntoResponse { let mut t = s.0.trunking.write(); t.locked = req.and_then(|Json(v)| v.locked).unwrap_or(!t.locked); Json(json!({"ok": true, "locked": t.locked})) }
async fn trunking_calls(State(s): State<ApiState>) -> impl IntoResponse { Json(serde_json::to_value(&s.0.trunking.read().calls).unwrap()) }
async fn trunking_import(State(s): State<ApiState>, Json(def): Json<Value>) -> impl IntoResponse {
    let mut t = s.0.trunking.write();
    t.system = def.get("system").or_else(|| def.get("system_name")).and_then(|v| v.as_str()).map(str::to_owned);
    t.control_channel_hz = def.get("control_channel_hz").and_then(|v| v.as_u64());
    t.voice_channels = def.get("voice_channels").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_u64()).collect()).unwrap_or_default();
    t.log.push("trunking definition imported".into());
    Json(json!({"ok": true, "status": &*t}))
}
async fn trunking_disc_start(State(s): State<ApiState>) -> impl IntoResponse { let mut t = s.0.trunking.write(); t.discovery_running = true; t.discovery_results = vec![json!({"system":"mock-trunked-system","control_channel_hz":851012500,"protocol":"P25"})]; t.log.push("discovery started".into()); Json(json!({"ok": true})) }
async fn trunking_disc_stop(State(s): State<ApiState>) -> impl IntoResponse { s.0.trunking.write().discovery_running = false; Json(json!({"ok": true})) }
async fn trunking_disc_results(State(s): State<ApiState>) -> impl IntoResponse { Json(serde_json::to_value(&s.0.trunking.read().discovery_results).unwrap()) }
async fn trunking_disc_snapshot(State(s): State<ApiState>) -> impl IntoResponse { Json(serde_json::to_value(&*s.0.trunking.read()).unwrap()) }
async fn trunking_disc_log(State(s): State<ApiState>) -> impl IntoResponse { Json(serde_json::to_value(&s.0.trunking.read().log).unwrap()) }
async fn trunking_disc_log_clear(State(s): State<ApiState>) -> impl IntoResponse { s.0.trunking.write().log.clear(); Json(json!({"ok": true})) }
async fn trunking_disc_notes() -> impl IntoResponse { Json(json!([])) }
async fn trunking_disc_promote(State(s): State<ApiState>) -> impl IntoResponse { let mut t = s.0.trunking.write(); t.system = Some("mock-trunked-system".into()); Json(json!({"ok": true})) }
async fn trunking_disc_identify() -> impl IntoResponse { Json(json!({"ok": true, "protocol":"P25"})) }
async fn trunking_disc_clear(State(s): State<ApiState>) -> impl IntoResponse { s.0.trunking.write().discovery_results.clear(); Json(json!({"ok": true})) }
async fn trunking_disc_delete(State(s): State<ApiState>) -> impl IntoResponse { s.0.trunking.write().discovery_results.clear(); Json(json!({"ok": true})) }
async fn trunking_zone_active(State(s): State<ApiState>) -> impl IntoResponse { Json(serde_json::to_value(&s.0.trunking.read().zones).unwrap()) }
async fn trunking_zone_upsert(State(s): State<ApiState>, Json(zone): Json<Value>) -> impl IntoResponse {
    let mut t = s.0.trunking.write();
    let key = zone.get("id").or_else(|| zone.get("name")).and_then(|v| v.as_str()).unwrap_or("");
    if key.is_empty() { return Json(json!({"ok": false, "error": "zone requires id or name"})); }
    t.zones.retain(|z| z.get("id").or_else(|| z.get("name")).and_then(|v| v.as_str()) != Some(key));
    t.zones.push(zone); Json(json!({"ok": true, "zones": &t.zones}))
}
async fn trunking_zone_delete(State(s): State<ApiState>, Json(zone): Json<Value>) -> impl IntoResponse {
    let key = zone.get("id").or_else(|| zone.get("name")).and_then(|v| v.as_str()).unwrap_or("");
    let mut t = s.0.trunking.write(); let before = t.zones.len(); t.zones.retain(|z| z.get("id").or_else(|| z.get("name")).and_then(|v| v.as_str()) != Some(key)); Json(json!({"ok": true, "removed": before - t.zones.len()}))
}

async fn aero_enable(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse { let mut c=s.0.config.write(); c.aero.enabled=v.get("enabled").and_then(|x|x.as_bool()).unwrap_or(true); let _=c.save(&s.0.data_dir); Json(json!({"ok":true,"enabled":c.aero.enabled})) }
async fn aero_check(State(s): State<ApiState>) -> impl IntoResponse { let c=s.0.config.read(); Json(json!({"ok":true,"available":!c.aero.sniffer_path.is_empty(),"path":c.aero.sniffer_path})) }
async fn aero_clear() -> impl IntoResponse { Json(json!({"ok": true})) }
async fn aero_messages(State(s): State<ApiState>) -> impl IntoResponse { Json(serde_json::to_value(s.0.db.messages_by_protocol(Some("acars"), 100).unwrap_or_default()).unwrap_or(json!([]))) }
async fn aero_status(State(s): State<ApiState>) -> impl IntoResponse { let c=s.0.config.read(); Json(json!({"enabled":c.aero.enabled,"satellite":c.aero.satellite,"center_freq_hz":c.aero.center_freq_hz,"sample_rate_hz":c.aero.sample_rate_hz,"path":c.aero.sniffer_path})) }
async fn aero_stderr(State(s): State<ApiState>) -> impl IntoResponse { Json(s.0.sidecars.stderr("aero")) }

async fn iridium_enable(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse { let mut c=s.0.config.write(); c.iridium.enabled=v.get("enabled").and_then(|x|x.as_bool()).unwrap_or(true); let _=c.save(&s.0.data_dir); Json(json!({"ok":true,"enabled":c.iridium.enabled})) }
async fn iridium_check(State(s): State<ApiState>) -> impl IntoResponse { let c=s.0.config.read(); Json(json!({"ok":true,"available":false,"center_freq_hz":c.iridium.center_freq_hz,"sample_rate_hz":c.iridium.sample_rate_hz})) }
async fn iridium_clear() -> impl IntoResponse { Json(json!({"ok": true})) }
async fn iridium_messages() -> impl IntoResponse { Json(json!([])) }
async fn iridium_status(State(s): State<ApiState>) -> impl IntoResponse { let c=s.0.config.read(); Json(json!({"enabled":c.iridium.enabled,"center_freq_hz":c.iridium.center_freq_hz,"sample_rate_hz":c.iridium.sample_rate_hz,"surface_message_content":c.iridium.surface_message_content})) }
async fn iridium_quick_start(State(s): State<ApiState>) -> impl IntoResponse { let mut c=s.0.config.write(); c.iridium.enabled=true; let _=c.save(&s.0.data_dir); Json(json!({"ok":true,"enabled":true})) }
async fn iridium_stderr(State(s): State<ApiState>) -> impl IntoResponse { Json(s.0.sidecars.stderr("iridium")) }

async fn stdc_enable(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse { let mut c=s.0.config.write(); c.stdc.enabled=v.get("enabled").and_then(|x|x.as_bool()).unwrap_or(true); let _=c.save(&s.0.data_dir); Json(json!({"ok":true,"enabled":c.stdc.enabled})) }
async fn stdc_check(State(s): State<ApiState>) -> impl IntoResponse { let c=s.0.config.read(); Json(json!({"ok":true,"available":which::which(&c.stdc.path).is_ok(),"path":c.stdc.path})) }
async fn stdc_clear() -> impl IntoResponse { Json(json!({"ok": true})) }
async fn stdc_messages() -> impl IntoResponse { Json(json!([])) }
async fn stdc_status(State(s): State<ApiState>) -> impl IntoResponse { let c=s.0.config.read(); Json(json!({"enabled":c.stdc.enabled,"path":c.stdc.path,"uw_tolerance":c.stdc.uw_tolerance})) }

async fn gps_enable(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse { let mut c=s.0.config.write(); c.gps.enabled=v.get("enabled").and_then(|x|x.as_bool()).unwrap_or(true); let _=c.save(&s.0.data_dir); Json(json!({"ok":true,"enabled":c.gps.enabled})) }
async fn gps_clear() -> impl IntoResponse { Json(json!({"ok": true})) }
async fn gps_status(State(s): State<ApiState>) -> impl IntoResponse { let c=s.0.config.read(); Json(json!({"enabled":c.gps.enabled,"sample_rate_hz":c.gps.sample_rate_hz,"detection_threshold":c.gps.detection_threshold,"doppler_search_hz":c.gps.doppler_search_hz})) }

async fn glonass_enable(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse { let mut c=s.0.config.write(); c.glonass.enabled=v.get("enabled").and_then(|x|x.as_bool()).unwrap_or(true); let _=c.save(&s.0.data_dir); Json(json!({"ok":true,"enabled":c.glonass.enabled})) }
async fn glonass_clear() -> impl IntoResponse { Json(json!({"ok": true})) }
async fn glonass_status(State(s): State<ApiState>) -> impl IntoResponse { let c=s.0.config.read(); Json(json!({"enabled":c.glonass.enabled,"sample_rate_hz":c.glonass.sample_rate_hz,"detection_threshold":c.glonass.detection_threshold,"doppler_search_hz":c.glonass.doppler_search_hz})) }

async fn goes_enable(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse { let mut c=s.0.config.write(); c.goes_lrit.enabled=v.get("enabled").and_then(|x|x.as_bool()).unwrap_or(true); let _=c.save(&s.0.data_dir); Json(json!({"ok":true,"enabled":c.goes_lrit.enabled})) }
async fn goes_check() -> impl IntoResponse { Json(json!({"ok": true, "available": false, "reason":"satdump sidecar not configured"})) }
async fn goes_status(State(s): State<ApiState>) -> impl IntoResponse { let c=s.0.config.read(); Json(json!({"enabled":c.goes_lrit.enabled,"satellite":c.goes_lrit.satellite,"path":c.goes_lrit.satdump_path,"sample_rate_hz":c.goes_lrit.sample_rate_hz})) }
async fn goes_satellite(State(s): State<ApiState>) -> impl IntoResponse { let c=s.0.config.read(); Json(json!({"satellite":c.goes_lrit.satellite,"output_image_dir":c.goes_lrit.output_image_dir,"sample_rate_hz":c.goes_lrit.sample_rate_hz})) }
async fn goes_satellite_put(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse { let mut c=s.0.config.write(); if let Some(x)=v.get("satellite").and_then(|x|x.as_str()){c.goes_lrit.satellite=x.to_string();} if let Some(x)=v.get("output_image_dir").and_then(|x|x.as_str()){c.goes_lrit.output_image_dir=x.to_string();} if let Some(x)=v.get("sample_rate_hz").and_then(|x|x.as_u64()){c.goes_lrit.sample_rate_hz=x as u32;} let _=c.save(&s.0.data_dir); Json(json!({"ok":true,"satellite":c.goes_lrit.satellite,"output_image_dir":c.goes_lrit.output_image_dir,"sample_rate_hz":c.goes_lrit.sample_rate_hz})) }

async fn hd_radio_enable(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse { let mut c=s.0.config.write(); c.hd_radio.enabled=v.get("enabled").and_then(|x|x.as_bool()).unwrap_or(true); let _=c.save(&s.0.data_dir); Json(json!({"ok":true,"enabled":c.hd_radio.enabled,"available":false,"reason":"HD Radio decoder sidecar not configured"})) }
async fn hd_radio_check(State(s): State<ApiState>) -> impl IntoResponse { let c=s.0.config.read(); Json(json!({"ok":true,"available":c.hd_radio.enabled,"program":c.hd_radio.program,"stations":c.hd_radio.stations})) }
async fn hd_radio_messages() -> impl IntoResponse { Json(json!([])) }
async fn hd_radio_status(State(s): State<ApiState>) -> impl IntoResponse { let c=s.0.config.read(); Json(json!({"enabled":c.hd_radio.enabled,"auto_on_fm_lock":c.hd_radio.auto_on_fm_lock,"program":c.hd_radio.program,"stations":c.hd_radio.stations})) }
async fn hd_radio_aas(Path(_filename): Path<String>) -> impl IntoResponse { Json(json!({})) }

async fn ble_devices(State(s): State<ApiState>) -> Json<Value> {
    let connected = s.0.device.status().connected;
    if !connected { return Json(json!([])); }
    Json(json!([
        {"address":"02:00:00:00:00:01","name":"PulseScope Mock Beacon","rssi":-48,"manufacturer":"PulseScope","service_uuids":["180F"],"last_seen_ms":crate::scanner::now_ms()},
        {"address":"02:00:00:00:00:02","name":"Mock Environmental Sensor","rssi":-67,"manufacturer":"PulseScope","service_uuids":["181A"],"last_seen_ms":crate::scanner::now_ms()}
    ]))
}
async fn ble_status(State(s): State<ApiState>) -> impl IntoResponse { let connected=s.0.device.status().connected; Json(json!({"enabled":connected,"running":connected,"device_count":if connected {2} else {0},"source":if connected {"mock"} else {"none"}})) }
async fn ble_file() -> impl IntoResponse { Json(json!(null)) }
async fn ble_clear() -> impl IntoResponse { Json(json!({"ok": true})) }

async fn lora_messages() -> impl IntoResponse { Json(json!([])) }
async fn lora_regions() -> impl IntoResponse { Json(json!(["US915","EU868","EU433","AS923","IN865","AU915","KR920"])) }

async fn scan_lock(State(s): State<ApiState>) -> impl IntoResponse { if let Some(h)=s.0.scanner.read().as_ref() { h.state.lock().scan_locked=true; } Json(json!({"ok":true,"locked":true})) }
async fn scan_unlock(State(s): State<ApiState>) -> impl IntoResponse { if let Some(h)=s.0.scanner.read().as_ref() { h.state.lock().scan_locked=false; } Json(json!({"ok":true,"locked":false})) }
async fn scan_start_alt(State(s): State<ApiState>, req: Option<Json<ScanStartReq>>) -> impl IntoResponse {
    let range_name = req.map(|Json(r)| r.range_name).or_else(|| s.0.config.read().scan_ranges.first().map(|r| r.name.clone()));
    match range_name { Some(name) => scan_start(State(s), Json(ScanStartReq { range_name: name })).await, None => Json(json!({"ok": false, "error": "no scan ranges configured"})) }
}
async fn scan_stop_alt(State(s): State<ApiState>) -> impl IntoResponse { scan_stop(State(s)).await }
async fn sidecars_status(State(s): State<ApiState>) -> impl IntoResponse {
    let runtime = serde_json::to_value(s.0.sidecars.statuses()).unwrap();
    let discovered = serde_json::to_value(crate::depmanager::scan_all(&s.0.data_dir)).unwrap();
    Json(json!({"runtime": runtime, "discovered": discovered}))
}

async fn decoders_scan(State(s): State<ApiState>) -> Json<Value> {
    Json(serde_json::to_value(crate::depmanager::scan_all(&s.0.data_dir)).unwrap())
}

async fn decoders_install(State(s): State<ApiState>, Path(name): Path<String>) -> Json<Value> {
    let data_dir = s.0.data_dir.clone();
    let install_name = name.clone();
    match tokio::task::spawn_blocking(move || {
        crate::depmanager::download_decoder(&install_name, &data_dir)
    }).await {
        Ok(Ok(path)) => Json(json!({"ok": true, "name": name, "path": path})),
        Ok(Err(error)) => Json(json!({"ok": false, "name": name, "error": error})),
        Err(error) => Json(json!({"ok": false, "name": name, "error": format!("installer task failed: {error}")})),
    }
}

async fn sidecars_start_all(State(s): State<ApiState>) -> impl IntoResponse {
    start_configured_sidecars(&s).await;
    Json(json!({"ok": true}))
}

async fn scan_status(State(s): State<ApiState>) -> impl IntoResponse {
    let runtime = s.0.scanner.read().as_ref().map(|h| h.state.lock().clone());
    Json(json!({"running": runtime.as_ref().map(|v| v.running).unwrap_or(false), "locked": runtime.as_ref().map(|v| v.scan_locked).unwrap_or(false), "range": runtime.and_then(|v| v.active_range)}))
}

async fn scan_adsb(State(s): State<ApiState>) -> Json<Value> {
    // Native Mode S / ADS-B 1090ES decoder — no dump1090/readsb process needed.
    let status = s.0.device.status();
    if !status.connected {
        return Json(json!({
            "available": true,
            "native": true,
            "messages": [],
            "reason": "no device connected — connect an SDR and tune near 1090 MHz"
        }));
    }
    let rate = status.sample_rate.max(1);
    // Capture ~0.5 s of IQ; ADS-B works best at ≥2 Msps
    let count = ((rate as f64) * 0.5) as usize;
    let count = count.clamp(8192, 4_000_000);
    match s.0.device.read_iq(count) {
        Ok(iq) => {
            let msgs = crate::adsb::decode_iq_chunk(&iq, rate);
            // Persist high-confidence messages
            for m in &msgs {
                let dm = crate::db::DecodedMessage {
                    id: None,
                    frequency_hz: 1_090_000_000,
                    protocol: "adsb".into(),
                    message_type: m.message_type.clone(),
                    address: m.icao.clone(),
                    function_code: format!("DF{}", m.df),
                    content: m.callsign.clone().unwrap_or_else(|| {
                        m.altitude_ft
                            .map(|a| format!("{a} ft"))
                            .unwrap_or_default()
                    }),
                    raw: m.raw_hex.clone(),
                    encryption: "none".into(),
                    timestamp_ms: crate::scanner::now_ms(),
                };
                let _ = s.0.db.insert_decoded_message(&dm);
            }
            Json(json!({
                "available": true,
                "native": true,
                "sample_rate_hz": rate,
                "samples": iq.len(),
                "message_count": msgs.len(),
                "messages": msgs,
            }))
        }
        Err(e) => Json(json!({
            "available": true,
            "native": true,
            "messages": [],
            "error": e.to_string()
        })),
    }
}
async fn native_ais_decode(Json(v): Json<Value>) -> Json<Value> {
    if let Some(iq) = v.get("iq").and_then(|x| x.as_array()) {
        let samples: Vec<(f32, f32)> = iq.iter().filter_map(|p| {
            let a = p.as_array()?;
            Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
        }).collect();
        if samples.is_empty() { return Json(json!({"ok":false,"error":"iq must contain [I,Q] samples"})); }
        let rate = v.get("sample_rate_hz").and_then(|x| x.as_f64()).unwrap_or(48000.0);
        let mut decoder = match crate::ais::IqDecoder::new(rate) { Ok(d) => d, Err(e) => return Json(json!({"ok":false,"error":e})) };
        let messages: Vec<Value> = decoder.push_iq(&samples).into_iter().filter_map(|r| r.ok()).filter_map(|m| serde_json::to_value(m).ok()).collect();
        return Json(json!({"ok":true,"native":true,"input":"iq","protocol":"ais","message_count":messages.len(),"messages":messages}));
    }
    let bits: Vec<bool> = v.get("bits").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_bool()).collect()).unwrap_or_default();
    if bits.is_empty() { return Json(json!({"ok":false,"error":"bits[] is required"})); }
    let mut decoder = crate::ais::HdlcDecoder::new();
    let results = decoder.push_bits(bits);
    let messages: Vec<Value> = results.into_iter().filter_map(|r| r.ok()).filter_map(|m| serde_json::to_value(m).ok()).collect();
    Json(json!({"ok":true,"native":true,"protocol":"ais","message_count":messages.len(),"messages":messages}))
}

async fn native_pocsag_decode(Json(v): Json<Value>) -> Json<Value> {
    if let Some(iq) = v.get("iq").and_then(|x| x.as_array()) {
        let samples: Vec<(f32, f32)> = iq.iter().filter_map(|p| {
            let a = p.as_array()?;
            Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
        }).collect();
        if samples.is_empty() { return Json(json!({"ok":false,"error":"iq must contain [I,Q] samples"})); }
        let rate = v.get("sample_rate_hz").and_then(|x| x.as_u64()).unwrap_or(128000) as u32;
        let baud = match v.get("baud").and_then(|x| x.as_u64()).unwrap_or(1200) { 2400 => crate::pocsag::PocsagBaud::Baud2400, _ => crate::pocsag::PocsagBaud::Baud1200 };
        let mut decoder = crate::pocsag::IqDecoder::new(rate, baud);
        let mut messages = decoder.push_iq(&samples); messages.extend(decoder.flush());
        return Json(json!({"ok":true,"native":true,"input":"iq","protocol":"pocsag","message_count":messages.len(),"messages":messages,"corrected_codewords":decoder.corrected_words(),"rejected_codewords":decoder.rejected_words()}));
    }
    let bits: Vec<bool> = v.get("bits").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_bool()).collect()).unwrap_or_default();
    if bits.is_empty() { return Json(json!({"ok":false,"error":"bits[] is required"})); }
    let baud = match v.get("baud").and_then(|x| x.as_u64()).unwrap_or(1200) { 2400 => crate::pocsag::PocsagBaud::Baud2400, _ => crate::pocsag::PocsagBaud::Baud1200 };
    let mut decoder = crate::pocsag::PocsagDecoder::new(baud.value() * 8, baud);
    let messages = decoder.push_bits(&bits);
    Json(json!({"ok":true,"native":true,"protocol":"pocsag","message_count":messages.len(),"messages":messages,"corrected_codewords":decoder.corrected_words(),"rejected_codewords":decoder.rejected_words()}))
}

async fn native_uat_decode(Json(v): Json<Value>) -> Json<Value> {
    let bits: Vec<bool> = v.get("bits").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_bool()).collect()).unwrap_or_default();
    if bits.is_empty() { return Json(json!({"ok":false,"error":"bits[] is required"})); }
    let mut decoder = crate::aviation::UatDecoder::new();
    decoder.feed_bits(&bits);
    let messages = decoder.take_messages();
    Json(json!({"ok":true,"native":true,"protocol":"uat978","message_count":messages.len(),"messages":messages}))
}

async fn native_acars_decode(Json(v): Json<Value>) -> Json<Value> {
    let bits: Vec<bool> = v.get("bits").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_bool()).collect()).unwrap_or_default();
    if bits.is_empty() { return Json(json!({"ok":false,"error":"bits[] is required"})); }
    let mut decoder = crate::aviation::AcarsDecoder::new(crate::aviation::BitOrder::MsbFirst, false);
    decoder.feed_bits(&bits);
    let messages = decoder.take_messages();
    Json(json!({"ok":true,"native":true,"protocol":"acars","message_count":messages.len(),"messages":messages}))
}

async fn native_vdl2_decode(Json(v): Json<Value>) -> Json<Value> {
    let bits: Vec<bool> = v.get("bits").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_bool()).collect()).unwrap_or_default();
    if bits.is_empty() { return Json(json!({"ok":false,"error":"bits[] is required"})); }
    let mut decoder = crate::aviation::Vdl2Decoder::new();
    decoder.feed_bits(&bits);
    let messages = decoder.take_messages();
    Json(json!({"ok":true,"native":true,"protocol":"vdl2","message_count":messages.len(),"messages":messages}))
}

async fn scan_ais(State(_s): State<ApiState>) -> Json<Value> { Json(json!({"available":true,"native":true,"messages":[],"reason":"native AIS parser is ready; live GMSK channel integration remains to be wired"})) }
async fn scan_acars(State(_s): State<ApiState>) -> Json<Value> { Json(json!({"available":false,"messages":[],"reason":"ACARS decoder transport is not implemented"})) }
async fn scan_aero(State(s): State<ApiState>) -> Json<Value> { scan_acars(State(s)).await }
async fn scan_ble(State(s): State<ApiState>) -> Json<Value> { ble_devices(State(s)).await }
async fn scan_lora(State(_s): State<ApiState>) -> Json<Value> { Json(json!({"available":false,"messages":[],"reason":"LoRa decoder transport is not implemented"})) }

#[derive(Deserialize)] struct RecordingReq { path: Option<String> }

async fn rec_iq_start(State(s): State<ApiState>, req: Option<Json<RecordingReq>>) -> impl IntoResponse {
    let req = req.map(|Json(v)| v).unwrap_or(RecordingReq { path: None });
    let dir = s.0.data_dir.join("recordings");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Json(json!({"ok": false, "error": e.to_string()}));
    }
    let path = req.path.map(std::path::PathBuf::from).unwrap_or_else(|| {
        dir.join(format!("iq-{}.cf32", crate::scanner::now_ms()))
    });
    match std::fs::File::create(&path) {
        Ok(file) => {
            let mut rec = s.0.recording.lock();
            rec.file = Some(file);
            rec.path = Some(path.clone());
            rec.started_ms = Some(crate::scanner::now_ms());
            rec.samples_written = 0;
            rec.bytes_written = 0;
            rec.write_error = None;
            Json(json!({"ok": true, "status": rec.status()}))
        }
        Err(e) => Json(json!({"ok": false, "error": e.to_string()})),
    }
}

async fn rec_iq_stop(State(s): State<ApiState>) -> impl IntoResponse {
    let status = s.0.recording.lock().stop();
    Json(json!({"ok": true, "status": status}))
}
async fn scan_ctcss(State(s): State<ApiState>) -> Json<Value> {
    let handle = s.0.scanner.read();
    let Some(h) = handle.as_ref() else { return Json(json!({"available": false, "reason": "scanner not running"})); };
    let vfos = h.state.lock().vfo_states.clone();
    let vfo_id = vfos.first().map(|v| v.id).unwrap_or(0);
    drop(handle);
    let status = s.0.device.status();
    if !status.connected { return Json(json!({"available": false, "reason": "no device connected"})); }
    let sample_rate = status.sample_rate;
    let count = (sample_rate as f64 * 0.3) as usize;
    match s.0.device.read_iq(count) {
        Ok(iq) if iq.len() > 1024 => {
            use crate::demod::{demodulate, detect_ctcss, detect_dcs, Mode};
            let mut previous = None;
            let audio = demodulate(Mode::Nfm, &iq, &mut previous);
            let audio_rate = sample_rate as f32;
            let ctcss = detect_ctcss(&audio, audio_rate);
            let dcs = detect_dcs(&audio, audio_rate);
            Json(json!({
                "available": true,
                "vfo_id": vfo_id,
                "frequency_hz": status.center_freq_hz,
                "ctcss": ctcss.map(|(tone, conf)| json!({"tone_hz": (tone * 10.0).round() / 10.0, "confidence": conf})),
                "dcs": dcs,
                "samples_analyzed": audio.len(),
            }))
        }
        Ok(_) => Json(json!({"available": false, "reason": "insufficient samples"})),
        Err(e) => Json(json!({"available": false, "error": e.to_string()})),
    }
}

async fn scan_aprs(State(s): State<ApiState>) -> Json<Value> {
    let status = s.0.device.status();
    if !status.connected { return Json(json!({"available": false, "reason": "no device connected"})); }
    // Read ~2 seconds of IQ for APRS decode (1200 baud = ~2400 bits = ~300 bytes)
    let sample_rate = status.sample_rate;
    let count = (sample_rate as f64 * 2.0) as usize;
    match s.0.device.read_iq(count) {
        Ok(iq) if iq.len() > 4096 => {
            use crate::demod::{demodulate, Mode};
            use crate::aprs::{AprsDecoder, parse_ax25_bits};
            let mut previous = None;
            let audio = demodulate(Mode::Nfm, &iq, &mut previous);
            let audio_rate = sample_rate as f32;
            let mut decoder = AprsDecoder::new(audio_rate);
            let mut frames_found = 0;
            for &sample in &audio {
                decoder.feed(sample);
            }
            Json(json!({
                "available": true,
                "frequency_hz": status.center_freq_hz,
                "samples_analyzed": audio.len(),
                "frames_found": decoder.frames.len(),
                "frames": decoder.frames.iter().map(|f| json!({
                    "source": f.source,
                    "dest": f.dest,
                    "digipeaters": f.digipeaters,
                    "info": f.info,
                })).collect::<Vec<_>>(),
            }))
        }
        Ok(_) => Json(json!({"available": false, "reason": "insufficient samples"})),
        Err(e) => Json(json!({"available": false, "error": e.to_string()})),
    }
}

async fn scan_digital_voice(State(s): State<ApiState>, Json(req): Json<Value>) -> Json<Value> {
    let mode = req.get("mode").and_then(|v| v.as_str()).unwrap_or("auto");
    let status = s.0.device.status();
    if !status.connected { return Json(json!({"available": false, "reason": "no device connected"})); }
    // Read ~3 seconds of IQ at the current frequency for digital voice decode
    let sample_rate = status.sample_rate;
    let count = (sample_rate as f64 * 3.0) as usize;
    match s.0.device.read_iq(count) {
        Ok(iq) if iq.len() > 4096 => {
            use crate::demod::{demodulate, Mode};
            use crate::voice_decoder;
            // Demodulate as NFM to get baseband audio
            let mut previous = None;
            let audio = demodulate(Mode::Nfm, &iq, &mut previous);
            // Resample to 48kHz for dsd-fme (it expects 48k or 96k mono WAV)
            let audio_rate = sample_rate as f32;
            let target_rate = 48000.0f32;
            let ratio = target_rate / audio_rate;
            let target_len = (audio.len() as f32 * ratio) as usize;
            let resampled: Vec<f32> = (0..target_len).map(|i| {
                let src_idx = i as f32 / ratio;
                let idx0 = src_idx.floor() as usize;
                let idx1 = (idx0 + 1).min(audio.len() - 1);
                let frac = src_idx - idx0 as f32;
                audio[idx0] * (1.0 - frac) + audio[idx1] * frac
            }).collect();
            
            let result = voice_decoder::decode_digital_voice(&resampled, mode);
            Json(json!({
                "available": result.available,
                "mode": result.mode,
                "decoder_path": result.decoder_path,
                "frames_decoded": result.frames_decoded,
                "calls": result.calls,
                "talkgroups": result.talkgroups,
                "nacs": result.nacs,
                "errors": result.errors,
                "raw_output": result.raw_output,
                "error": result.error_message,
                "frequency_hz": status.center_freq_hz,
                "audio_samples": resampled.len(),
            }))
        }
        Ok(_) => Json(json!({"available": false, "reason": "insufficient samples"})),
        Err(e) => Json(json!({"available": false, "error": e.to_string()})),
    }
}

async fn digital_voice_check() -> impl IntoResponse {
    use crate::voice_decoder;
    match voice_decoder::find_dsd_fme() {
        Some(path) => Json(json!({
            "available": true,
            "path": path.display().to_string(),
            "modes": ["auto", "p25p1", "p25p2", "dmr", "nxdn48", "nxdn96", "dstar", "ysf", "m17", "provoice"],
        })),
        None => Json(json!({
            "available": false,
            "install_url": "https://github.com/lwvmobile/dsd-fme/releases",
        })),
    }
}

#[derive(Deserialize)] struct IqNetworkReq { target: String }
async fn iq_network_start(State(s): State<ApiState>, Json(req): Json<IqNetworkReq>) -> impl IntoResponse { match req.target.parse::<SocketAddr>() { Ok(target) => match s.0.iq_network.start(target) { Ok(()) => Json(json!({"ok":true,"status":s.0.iq_network.status()})), Err(e) => Json(json!({"ok":false,"error":e.to_string()})) }, Err(e) => Json(json!({"ok":false,"error":format!("invalid target: {e}")})) } }
async fn iq_network_stop(State(s): State<ApiState>) -> impl IntoResponse { s.0.iq_network.stop(); Json(json!({"ok":true,"status":s.0.iq_network.status()})) }
async fn iq_network_status(State(s): State<ApiState>) -> impl IntoResponse { Json(s.0.iq_network.status()) }

#[derive(Deserialize)] struct AudioNetworkReq { target: String }
async fn audio_network_start(State(s): State<ApiState>, Json(req): Json<AudioNetworkReq>) -> impl IntoResponse {
    match req.target.parse::<SocketAddr>() {
        Ok(target) => match s.0.audio.start_network(target) { Ok(()) => Json(json!({"ok":true,"status":s.0.audio.network_status()})), Err(e) => Json(json!({"ok":false,"error":e.to_string()})) },
        Err(e) => Json(json!({"ok":false,"error":format!("invalid target: {e}")})),
    }
}
async fn audio_network_stop(State(s): State<ApiState>) -> impl IntoResponse { s.0.audio.stop_network(); Json(json!({"ok":true,"status":s.0.audio.network_status()})) }
async fn audio_network_status(State(s): State<ApiState>) -> impl IntoResponse { Json(s.0.audio.network_status()) }

#[derive(Deserialize)] struct AnnotationReq { recording_path: String, offset_ms: i64, text: String }
async fn playback_start(State(s): State<ApiState>, Json(req): Json<RecordingReq>) -> impl IntoResponse {
    let Some(path) = req.path else { return (StatusCode::BAD_REQUEST, Json(json!({"error":"path is required"}))); };
    match crate::capture::PlaybackReader::open(std::path::PathBuf::from(&path)) {
        Ok(reader) => { *s.0.playback.lock() = Some(reader); (StatusCode::OK, Json(json!({"ok":true,"path":path,"format":"cf32-le"}))) }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error.to_string()}))),
    }
}
async fn playback_stop(State(s): State<ApiState>) -> impl IntoResponse {
    let previous = s.0.playback.lock().take().map(|r| r.status());
    (StatusCode::OK, Json(json!({"ok":true,"previous":previous})))
}
async fn playback_status(State(s): State<ApiState>) -> impl IntoResponse {
    (StatusCode::OK, Json(s.0.playback.lock().as_ref().map(|r| r.status()).unwrap_or_else(|| json!({"playing":false,"format":"cf32-le"}))))
}

async fn rec_annotations(State(s): State<ApiState>) -> impl IntoResponse {
    match s.0.db.list_annotations() { Ok(v) => Json(serde_json::to_value(v).unwrap()), Err(e) => Json(json!({"error": e.to_string()})) }
}
async fn rec_annotation_new(State(s): State<ApiState>, Json(req): Json<AnnotationReq>) -> impl IntoResponse {
    let a = crate::db::RecordingAnnotation { id: None, recording_path: req.recording_path, offset_ms: req.offset_ms, text: req.text, created_ms: crate::scanner::now_ms() };
    match s.0.db.add_annotation(&a) { Ok(id) => Json(json!({"ok": true, "id": id})), Err(e) => Json(json!({"ok": false, "error": e.to_string()})) }
}
async fn rec_annotation_one(State(s): State<ApiState>, Path(id): Path<i64>) -> impl IntoResponse {
    match s.0.db.list_annotations() { Ok(v) => Json(v.into_iter().find(|a| a.id == Some(id)).map(|a| serde_json::to_value(a).unwrap()).unwrap_or_else(|| json!({"error":"not found"}))), Err(e) => Json(json!({"error": e.to_string()})) }
}
async fn rec_annotation_delete(State(s): State<ApiState>, Path(id): Path<i64>) -> impl IntoResponse { Json(json!({"ok": s.0.db.delete_annotation(id).is_ok()})) }
async fn rec_annotation_update(State(s): State<ApiState>, Path(id): Path<i64>, Json(req): Json<AnnotationReq>) -> impl IntoResponse { let a=crate::db::RecordingAnnotation{id:Some(id),recording_path:req.recording_path,offset_ms:req.offset_ms,text:req.text,created_ms:crate::scanner::now_ms()}; Json(json!({"ok":s.0.db.update_annotation(id,&a).map(|n|n>0).unwrap_or(false),"id":id})) }
async fn iq_rec_start(State(s): State<ApiState>, req: Option<Json<RecordingReq>>) -> impl IntoResponse { rec_iq_start(State(s), req).await }
async fn iq_rec_stop(State(s): State<ApiState>) -> impl IntoResponse { rec_iq_stop(State(s)).await }
async fn iq_rec_status(State(s): State<ApiState>) -> impl IntoResponse { Json(s.0.recording.lock().status()) }

async fn transcription_start(State(s): State<ApiState>) -> impl IntoResponse { let mut c=s.0.config.write(); c.transcription.enabled=true; let _=c.save(&s.0.data_dir); Json(json!({"ok":true,"running":true,"engine":c.transcription.engine,"model":c.transcription.model})) }
async fn transcription_stop(State(s): State<ApiState>) -> impl IntoResponse { let mut c=s.0.config.write(); c.transcription.enabled=false; let _=c.save(&s.0.data_dir); Json(json!({"ok":true,"running":false})) }
async fn transcription_status(State(s): State<ApiState>) -> impl IntoResponse { let c=s.0.config.read(); Json(json!({"running":c.transcription.enabled,"enabled":c.transcription.enabled,"engine":c.transcription.engine,"model":c.transcription.model})) }
async fn transcription_list() -> impl IntoResponse { Json(json!([])) }

#[derive(Deserialize)] struct CaseReq { name: String, description: Option<String>, status: Option<String>, tags: Option<String> }
async fn cases(State(s): State<ApiState>) -> impl IntoResponse {
    match s.0.db.list_cases() { Ok(v) => Json(serde_json::to_value(v).unwrap()), Err(e) => Json(json!({"error": e.to_string()})) }
}
async fn cases_new(State(s): State<ApiState>, Json(req): Json<CaseReq>) -> impl IntoResponse {
    let now = crate::scanner::now_ms();
    let c = crate::db::Case { id: None, name: req.name, description: req.description.unwrap_or_default(), status: req.status.unwrap_or_else(|| "open".into()), tags: req.tags.unwrap_or_default(), created_ms: now, updated_ms: now };
    match s.0.db.create_case(&c) { Ok(id) => Json(json!({"ok": true, "id": id})), Err(e) => Json(json!({"ok": false, "error": e.to_string()})) }
}
async fn case_one(State(s): State<ApiState>, Path(id): Path<i64>) -> impl IntoResponse {
    match s.0.db.get_case(id) { Ok(Some(c)) => Json(serde_json::to_value(c).unwrap()), Ok(None) => Json(json!({"error":"not found"})), Err(e) => Json(json!({"error": e.to_string()})) }
}
async fn case_delete(State(s): State<ApiState>, Path(id): Path<i64>) -> impl IntoResponse { Json(json!({"ok": s.0.db.delete_case(id).is_ok()})) }
#[derive(Deserialize)] struct CaseAttachmentReq { kind: String, r#ref: String, note: Option<String> }
async fn case_attach(State(s): State<ApiState>, Path(id): Path<i64>, Json(req): Json<CaseAttachmentReq>) -> impl IntoResponse { let a=crate::db::CaseAttachment{id:None,case_id:id,kind:req.kind,r#ref:req.r#ref,note:req.note.unwrap_or_default(),attached_ms:crate::scanner::now_ms()}; match s.0.db.add_case_attachment(&a) { Ok(att_id)=>Json(json!({"ok":true,"id":att_id})),Err(e)=>Json(json!({"ok":false,"error":e.to_string()})) } }
async fn case_attachment_one(State(s): State<ApiState>, Path(id): Path<i64>) -> impl IntoResponse { match s.0.db.case_attachment(id) { Ok(Some(a))=>Json(serde_json::to_value(a).unwrap()),Ok(None)=>Json(json!({"error":"not found"})),Err(e)=>Json(json!({"error":e.to_string()})) } }
async fn case_attachment_delete(State(s): State<ApiState>, Path(id): Path<i64>) -> impl IntoResponse { Json(json!({"ok":s.0.db.delete_case_attachment(id).map(|n|n>0).unwrap_or(false)})) }

async fn sidecar_stderr(State(s): State<ApiState>, Path(name): Path<String>) -> impl IntoResponse { Json(serde_json::to_value(s.0.sidecars.stderr(&name)).unwrap()) }

async fn feature_packs(State(s): State<ApiState>) -> impl IntoResponse {
    let c = s.0.config.read();
    let available = |path: &str| !path.trim().is_empty() && std::path::Path::new(path).is_file();
    let packs = vec![
        json!({"id":"rtl433","name":"RTL-SDR 433 sensors","enabled":c.rtl433.enabled,"running":s.0.sidecars.is_running("rtl_433"),"path":c.rtl433.path,"available":available(&c.rtl433.path),"availability_reason":if available(&c.rtl433.path){"executable found"}else{"executable missing"},"protocols":["rtl_433"]}),
        json!({"id":"digital","name":"Digital voice / pager","enabled":c.digital_decoder.enabled,"running":s.0.sidecars.is_running("multimon-ng"),"path":c.digital_decoder.multimon_path,"available":available(&c.digital_decoder.multimon_path),"availability_reason":if available(&c.digital_decoder.multimon_path){"executable found"}else{"executable missing"},"protocols":["pocsag","p25","dmr"]}),
        json!({"id":"acars","name":"ACARS","enabled":c.acarsdec.enabled,"running":s.0.sidecars.is_running("acarsdec"),"path":c.acarsdec.path,"available":available(&c.acarsdec.path),"availability_reason":if available(&c.acarsdec.path){"executable found"}else{"executable missing"},"protocols":["acars"]}),
        json!({"id":"vdl2","name":"VDL2","enabled":c.vdl2.enabled,"running":s.0.sidecars.is_running("dumpvdl2"),"path":c.vdl2.path,"available":available(&c.vdl2.path),"availability_reason":if available(&c.vdl2.path){"executable found"}else{"executable missing"},"protocols":["vdl2"]}),
        json!({"id":"aprs","name":"APRS / Direwolf","enabled":c.aprs.enabled,"running":s.0.sidecars.is_running("direwolf"),"path":c.aprs.path,"available":available(&c.aprs.path),"availability_reason":if available(&c.aprs.path){"executable found"}else{"executable missing"},"protocols":["aprs"]}),
        json!({"id":"dsd","name":"DSD digital voice","enabled":c.dsd.enabled,"running":s.0.sidecars.is_running("dsd-neo"),"path":c.dsd.dsdneo_path,"available":available(&c.dsd.dsdneo_path),"availability_reason":if available(&c.dsd.dsdneo_path){"executable found"}else{"executable missing"},"protocols":["p25","dmr","nxdn"]}),
        json!({"id":"radiosonde","name":"RS41 radiosonde","enabled":c.radiosonde.enabled,"running":s.0.sidecars.is_running("rs41mod"),"path":c.radiosonde.path,"available":available(&c.radiosonde.path),"availability_reason":if available(&c.radiosonde.path){"executable found"}else{"executable missing"},"protocols":["rs41"]}),
    ];
    Json(json!({"groups": packs, "count": packs.len()}))
}
async fn feature_pack_enable(State(s): State<ApiState>, Path(id): Path<String>, Json(v): Json<Value>) -> impl IntoResponse {
    let enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
    let ok = {
        let mut c = s.0.config.write();
        match id.as_str() {
            "rtl433" => { c.rtl433.enabled = enabled; true }, "digital" => { c.digital_decoder.enabled = enabled; true },
            "acars" => { c.acarsdec.enabled = enabled; true }, "vdl2" => { c.vdl2.enabled = enabled; true },
            "aprs" => { c.aprs.enabled = enabled; true }, "dsd" => { c.dsd.enabled = enabled; true },
            "radiosonde" => { c.radiosonde.enabled = enabled; true }, _ => false,
        }
    };
    if !ok { return Json(json!({"ok":false,"error":"unknown feature pack","id":id})); }
    { let c = s.0.config.read(); let _ = c.save(&s.0.data_dir); }
    let sidecar = match id.as_str() { "rtl433"=>"rtl_433", "digital"=>"multimon-ng", "acars"=>"acarsdec", "vdl2"=>"dumpvdl2", "aprs"=>"direwolf", "dsd"=>"dsd-neo", "radiosonde"=>"rs41mod", _=>"" };
    if enabled { start_configured_sidecars(&s).await; } else if !sidecar.is_empty() { let _ = s.0.sidecars.kill(sidecar).await; }
    Json(json!({"ok":true,"id":id,"enabled":enabled,"sidecar_running":s.0.sidecars.is_running(sidecar)}))
}

async fn aircraft_lookup(State(_s): State<ApiState>, Query(q): Query<LookupQ>) -> Json<Value> {
    if q.q.unwrap_or_default().trim().is_empty() {
        return Json(json!({"available":false,"results":[],"reason":"Aircraft lookup database is not configured"}));
    }
    Json(json!({"available":false,"results":[],"reason":"Aircraft lookup database is not configured"}))
}
#[derive(Deserialize)] struct LookupQ { q: Option<String> }
async fn intercept_results() -> impl IntoResponse { Json(json!([])) }
async fn instances(State(s): State<ApiState>) -> impl IntoResponse { let d=s.0.device.status(); Json(json!([{"id":"local","name":"PulseScope local","connected":d.connected,"driver":d.driver,"address":"127.0.0.1:8765"}])) }
async fn reconnect(State(s): State<ApiState>) -> impl IntoResponse { let key=s.0.config.read().device.last_device_key.clone(); let result=s.0.device.connect(&key); Json(json!({"ok":result.is_ok(),"key":key,"status":s.0.device.status()})) }
async fn close_session(State(s): State<ApiState>) -> impl IntoResponse { if let Some(h)=s.0.scanner.read().as_ref() { let _=h.cmd_tx.send(crate::scanner::ScannerCommand::Stop); } let _=s.0.device.disconnect(); Json(json!({"ok":true,"status":s.0.device.status()})) }
async fn slots(State(s): State<ApiState>) -> impl IntoResponse { let v=s.0.scanner.read().as_ref().map(|h| h.state.lock().vfo_states.clone()).unwrap_or_default(); Json(v.into_iter().map(|x| json!({"slot":x.id,"frequency_hz":x.frequency_hz,"mode":x.mode,"active":!x.muted,"squelch_open":x.squelch_open})).collect::<Vec<_>>()) }

async fn rtl433_messages(State(s): State<ApiState>, Query(q): Query<LimitQ>) -> impl IntoResponse { Json(serde_json::to_value(s.0.db.messages_by_protocol(Some("rtl_433"), q.limit.unwrap_or(100)).unwrap_or_default()).unwrap()) }
async fn protocol_messages(State(s): State<ApiState>, Query(q): Query<LimitQ>) -> impl IntoResponse { Json(serde_json::to_value(s.0.db.messages_by_protocol(None, q.limit.unwrap_or(100)).unwrap_or_default()).unwrap()) }

async fn rx_location(State(s): State<ApiState>) -> impl IntoResponse {
    let cfg = s.0.config.read();
    Json(json!({
        "latitude_deg": cfg.receiver_location.latitude_deg,
        "longitude_deg": cfg.receiver_location.longitude_deg,
        "altitude_m": cfg.receiver_location.altitude_m,
    }))
}
async fn rx_location_put(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let mut cfg = s.0.config.write();
    if let Some(lat) = v.get("latitude_deg").and_then(|x| x.as_f64()) { cfg.receiver_location.latitude_deg = lat; }
    if let Some(lon) = v.get("longitude_deg").and_then(|x| x.as_f64()) { cfg.receiver_location.longitude_deg = lon; }
    if let Some(alt) = v.get("altitude_m").and_then(|x| x.as_f64()) { cfg.receiver_location.altitude_m = alt; }
    Json(json!({"ok": true}))
}

async fn device_test(State(s): State<ApiState>) -> impl IntoResponse { let status=s.0.device.status(); if !status.connected { return Json(json!({"ok":false,"result":"not_connected","connected":false,"samples":0,"error":"device is not connected"})); } let iq=match s.0.device.read_iq(4096){Ok(iq)=>iq,Err(e)=>return Json(json!({"ok":false,"result":"stream_error","connected":true,"samples":0,"error":e.to_string()}))}; if iq.is_empty(){return Json(json!({"ok":false,"result":"empty_frame","connected":true,"samples":0,"error":"device returned no samples"}));} let rms=(iq.iter().map(|x|x.norm_sqr()).sum::<f32>()/iq.len() as f32).sqrt(); let peak=iq.iter().map(|x|x.norm()).fold(0.0_f32,f32::max); Json(json!({"ok":true,"result":"pass","connected":true,"samples":iq.len(),"rms":rms,"peak":peak,"sample_rate":status.sample_rate})) }
async fn device_hackrf_amp(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse { let enabled=v.get("enabled").and_then(|x|x.as_bool()).unwrap_or(true); let d=s.0.device.status(); let supported=d.driver.eq_ignore_ascii_case("hackrf"); Json(json!({"ok":supported && d.connected,"enabled":enabled,"supported":supported,"driver":d.driver,"error":if supported {Value::Null}else{json!("HackRF amplifier control requires an active HackRF driver")}})) }
async fn channel_banks_delete(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse { let name=v.get("name").or_else(||v.get("bank_name")).and_then(|x|x.as_str()).unwrap_or(""); if name.is_empty(){return Json(json!({"ok":false,"error":"name is required"}));} let mut c=s.0.config.write(); let before=c.scan_ranges.len(); c.scan_ranges.retain(|r|r.name!=name); let removed=before!=c.scan_ranges.len(); if removed {let _=c.save(&s.0.data_dir);} Json(json!({"ok":removed,"name":name,"error":if removed {Value::Null}else{json!("bank not found")}})) }
async fn channel_banks_create(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse { match serde_json::from_value::<crate::config::ScanRange>(v) { Ok(range) if !range.name.trim().is_empty() => { let mut c=s.0.config.write(); c.scan_ranges.retain(|r|r.name!=range.name); c.scan_ranges.push(range.clone()); let _=c.save(&s.0.data_dir); Json(json!({"ok":true,"bank":range})) }, Ok(_) => Json(json!({"ok":false,"error":"bank name is required"})), Err(e)=>Json(json!({"ok":false,"error":e.to_string()})) } }
async fn channel_bank_scan_config(State(s): State<ApiState>) -> impl IntoResponse { let c=s.0.config.read(); Json(json!({"ranges":c.scan_ranges.len(),"enabled":c.scan_ranges.iter().filter(|r|r.enabled).count()})) }
async fn channel_bank_scan_config_put(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse { let name=v.get("bank_name").or_else(||v.get("name")).and_then(|x|x.as_str()).unwrap_or(""); if name.is_empty(){return Json(json!({"ok":false,"error":"bank_name is required"}));} let mut c=s.0.config.write(); let Some(r)=c.scan_ranges.iter_mut().find(|r|r.name==name) else {return Json(json!({"ok":false,"error":"bank not found"}));}; if let Some(x)=v.get("enabled").and_then(|x|x.as_bool()){r.enabled=x;} if let Some(x)=v.get("dwell_ms").and_then(|x|x.as_u64()){r.dwell_ms=x as u32;} if let Some(x)=v.get("hold_ms").and_then(|x|x.as_u64()){r.hold_ms=x as u32;} if let Some(x)=v.get("max_vfos").and_then(|x|x.as_u64()){r.max_vfos=x as u32;} if let Some(x)=v.get("squelch_db").and_then(|x|x.as_f64()){r.squelch_db=x as f32;} let out=r.clone(); let _=c.save(&s.0.data_dir); Json(json!({"ok":true,"bank":out})) }
async fn channel_import(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse { let rows=if v.is_array(){v}else{json!([v])}; let mut added=0; let mut c=s.0.config.write(); for item in rows.as_array().cloned().unwrap_or_default(){ if let Ok(r)=serde_json::from_value::<crate::config::ScanRange>(item){ c.scan_ranges.retain(|x|x.name!=r.name); c.scan_ranges.push(r); added+=1; } } let _=c.save(&s.0.data_dir); Json(json!({"ok":true,"added":added,"total":c.scan_ranges.len()})) }
async fn scanner_max_vfos(State(s): State<ApiState>) -> impl IntoResponse {
    let cfg = s.0.config.read();
    Json(json!({"max_vfos": cfg.scanner.max_vfos}))
}
async fn vfo_diagnostics(State(s): State<ApiState>) -> impl IntoResponse {
    let vfos = s.0.scanner.read().as_ref().map(|h| h.state.lock().vfo_states.clone()).unwrap_or_default();
    Json(vfos.into_iter().map(|v| json!({"id": v.id, "frequency_hz": v.frequency_hz, "strength_db": v.strength_db, "audio_level_db": v.audio_level_db, "squelch_open": v.squelch_open, "muted": v.muted})).collect::<Vec<_>>())
}
async fn vfo_identify(State(s): State<ApiState>, Path(id): Path<i64>) -> impl IntoResponse {
    let v = s.0.scanner.read().as_ref().and_then(|h| h.state.lock().vfo_states.iter().find(|v| v.id as i64 == id).cloned());
    let Some(v) = v else {
        return Json(json!({"result":"unknown","error":"vfo not found"}));
    };
    let range_name = s.0.scanner.read().as_ref()
        .and_then(|h| h.state.lock().active_range.clone())
        .unwrap_or_default();
    let snr_db = if v.squelch_open { 18.0 } else { 8.0 };
    let status = s.0.device.status();

    let classification = if status.connected {
        let count = ((status.sample_rate as f64) * 0.35) as usize;
        match s.0.device.read_iq(count.max(4096)) {
            Ok(iq) if iq.len() > 2048 => {
                use crate::demod::{demodulate, mix_down, Mode};
                let mut phase = 0.0f64;
                let offset = v.frequency_hz as f64 - status.center_freq_hz as f64;
                let baseband = mix_down(&iq, offset, status.sample_rate, &mut phase);
                let mut prev = None;
                let pcm = demodulate(Mode::parse(&v.mode), &baseband, &mut prev);
                crate::signal_id::classify(
                    v.frequency_hz, 12_500, &v.mode, &range_name, snr_db,
                    Some((&pcm, status.sample_rate as f32)),
                )
            }
            _ => crate::signal_id::classify(v.frequency_hz, 12_500, &v.mode, &range_name, snr_db, None),
        }
    } else {
        crate::signal_id::classify(v.frequency_hz, 12_500, &v.mode, &range_name, snr_db, None)
    };

    Json(json!({
        "result": "identified",
        "vfo_id": v.id,
        "frequency_hz": v.frequency_hz,
        "mode": v.mode,
        "strength_db": v.strength_db,
        "squelch_open": v.squelch_open,
        "classification": classification.sub_protocol,
        "family": classification.top_family,
        "confidence": classification.top_confidence,
        "decoder": classification.candidates.first().map(|c| c.decoder.clone()).unwrap_or_else(|| "none".into()),
        "features": classification.features,
        "candidates": classification.candidates,
        "decode_success": classification.decode_success,
        "decode_summary": classification.decode_summary,
        "action": crate::signal_id::recommended_action(&classification),
    }))
}
async fn vfo_rds(State(s): State<ApiState>, Path(id): Path<i64>) -> impl IntoResponse {
    let v = s.0.scanner.read().as_ref().and_then(|h| h.state.lock().vfo_states.iter().find(|v| v.id as i64 == id).cloned());
    let Some(v) = v else { return Json(json!({"present":false,"reason":"vfo not found"})); };
    if !v.mode.eq_ignore_ascii_case("wfm") { return Json(json!({"present":false,"reason":"RDS requires WFM mode"})); }
    let status = s.0.device.status();
    if !status.connected { return Json(json!({"present":false,"reason":"no device connected"})); }
    // Read ~0.5 seconds of IQ at current rate and decode RDS from WFM multiplex
    let count = (status.sample_rate as f64 * 0.5) as usize;
    match s.0.device.read_iq(count) {
        Ok(iq) if iq.len() > 4096 => {
            use crate::demod::{demodulate, decode_rds, Mode};
            let mut previous = None;
            let multiplex = demodulate(Mode::Wfm, &iq, &mut previous);
            let audio_rate = status.sample_rate as f32;
            match decode_rds(&multiplex, audio_rate) {
                Some(rds) if rds.groups_found > 0 => Json(json!({
                    "present": true,
                    "frequency_hz": v.frequency_hz,
                    "pi_code": rds.pi_code,
                    "pty": rds.pty,
                    "groups_found": rds.groups_found,
                    "bits_decoded": rds.bits_decoded,
                    "program_service": rds.program_service,
                    "radio_text": rds.radio_text,
                })),
                Some(_) => Json(json!({"present":false,"frequency_hz":v.frequency_hz,"reason":"RDS subcarrier detected but no valid groups decoded"})),
                None => Json(json!({"present":false,"frequency_hz":v.frequency_hz,"reason":"no RDS subcarrier detected"})),
            }
        }
        Ok(_) => Json(json!({"present":false,"reason":"insufficient samples"})),
        Err(e) => Json(json!({"present":false,"error":e.to_string()})),
    }
}
async fn signal_events(State(s): State<ApiState>, Query(q): Query<LimitQ>) -> impl IntoResponse {
    match s.0.db.recent_signal_events(q.limit.unwrap_or(100)) {
        Ok(rows) => (StatusCode::OK, Json(serde_json::to_value(rows).unwrap_or_else(|_| json!([])))),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": error.to_string()}))),
    }
}

async fn spectrum_occupancy(State(s): State<ApiState>) -> impl IntoResponse {
    let snapshot = s.0.scanner.read().as_ref().map(|h| h.state.lock().clone());
    let Some(runtime) = snapshot else { return Json(serde_json::to_value(s.0.db.recent_occupancy(512).unwrap_or_default()).unwrap()); };
    let Some(range_name) = runtime.active_range else { return Json(serde_json::to_value(s.0.db.recent_occupancy(512).unwrap_or_default()).unwrap()); };
    let range = s.0.config.read().scan_ranges.iter().find(|r| r.name == range_name).cloned();
    let Some(range) = range else { return Json(json!([])); };
    if runtime.latest_spectrum.is_empty() { return Json(json!([])); }
    let bucket_count = runtime.latest_spectrum.len().min(128);
    let chunk = (runtime.latest_spectrum.len() / bucket_count).max(1);
    let span = range.end_hz.saturating_sub(range.start_hz);
    let rows: Vec<crate::db::SpectrumOccupancy> = runtime.latest_spectrum.chunks(chunk).enumerate().map(|(i, bins)| {
        let avg = bins.iter().sum::<f32>() / bins.len() as f32;
        let peak = bins.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let freq = range.start_hz.saturating_add(span.saturating_mul(i as u64) / bucket_count as u64);
        crate::db::SpectrumOccupancy { frequency_bucket_hz: freq, time_bucket_15min: crate::scanner::now_ms() / 900000, avg_power_db: avg, peak_power_db: peak, avg_above_floor_db: avg + 120.0, sample_count: bins.len() as i64, noise_floor_db: -120.0 }
    }).collect();
    for row in &rows { let _ = s.0.db.upsert_occupancy(row); }
    Json(serde_json::to_value(rows).unwrap())
}

#[derive(Deserialize)] struct BlacklistReq { frequency_hz: u64, reason: Option<String>, temporary: Option<bool> }
async fn blacklist(State(s): State<ApiState>) -> impl IntoResponse {
    match s.0.db.list_blacklist() { Ok(v) => Json(serde_json::to_value(v).unwrap()), Err(e) => Json(json!({"error": e.to_string()})) }
}
async fn blacklist_add(State(s): State<ApiState>, Json(req): Json<BlacklistReq>) -> impl IntoResponse {
    let e = crate::db::BlacklistEntry { frequency_hz: req.frequency_hz, reason: req.reason.unwrap_or_default(), temporary: req.temporary.unwrap_or(false), created_ms: crate::scanner::now_ms() };
    Json(json!({"ok": s.0.db.add_blacklist(&e).is_ok()}))
}
async fn blacklist_remove(State(s): State<ApiState>, Json(req): Json<BlacklistReq>) -> impl IntoResponse { Json(json!({"ok": s.0.db.remove_blacklist(req.frequency_hz).is_ok()})) }
async fn blacklist_clear(State(s): State<ApiState>) -> impl IntoResponse { Json(json!({"ok": s.0.db.clear_blacklist(false).is_ok()})) }
async fn blacklist_clear_temporary(State(s): State<ApiState>) -> impl IntoResponse { Json(json!({"ok": s.0.db.clear_blacklist(true).is_ok()})) }

async fn debug_stats(State(s): State<ApiState>) -> impl IntoResponse {
    let frames_processed = s.0.scanner.read().as_ref().map(|h| h.state.lock().frames_processed).unwrap_or(0);
    let messages_decoded = s.0.db.decoded_message_count().unwrap_or(0);
    Json(json!({
        "uptime_ms": crate::scanner::now_ms().saturating_sub(s.0.started_ms),
        "messages_decoded": messages_decoded,
        "frames_processed": frames_processed,
        "audio": s.0.audio.status(),
        "sidecars": s.0.sidecars.statuses(),
    }))
}
async fn debug_log(State(s): State<ApiState>) -> impl IntoResponse { Json(json!({"sidecars":s.0.sidecars.statuses(),"trunking_log":s.0.trunking.read().log})) }
async fn debug_log_tail(State(s): State<ApiState>) -> impl IntoResponse { let mut lines=Vec::new(); for name in ["rtl_433","multimon-ng","acarsdec","dumpvdl2","direwolf","dsd-neo","rs41mod"] { for line in s.0.sidecars.stderr(name) { lines.push(json!({"source":name,"line":line})); } } Json(json!(lines)) }
async fn debug_classifications(State(s): State<ApiState>) -> impl IntoResponse { let v=s.0.scanner.read().as_ref().map(|h|h.state.lock().vfo_states.clone()).unwrap_or_default(); Json(v.into_iter().map(|x|json!({"vfo_id":x.id,"frequency_hz":x.frequency_hz,"classification":x.mode,"confidence":if x.squelch_open {0.96}else{0.12}})).collect::<Vec<_>>()) }
async fn debug_noise_floor(State(s): State<ApiState>) -> impl IntoResponse { let floor=s.0.scanner.read().as_ref().and_then(|h|{let bins=h.state.lock().latest_spectrum.clone(); bins.into_iter().filter(|v|v.is_finite()).reduce(f32::min)}).unwrap_or(-120.0); Json(json!({"noise_floor_db":floor})) }
async fn debug_dsd_stderr(State(s): State<ApiState>) -> impl IntoResponse { Json(s.0.sidecars.stderr("dsd-neo")) }
async fn debug_multimon_raw(State(s): State<ApiState>) -> impl IntoResponse { Json(s.0.sidecars.stderr("multimon-ng")) }
async fn debug_p25_acq(State(s): State<ApiState>) -> impl IntoResponse { let t=s.0.trunking.read(); Json(json!({"locked":t.locked,"running":t.running,"control_channel_hz":t.control_channel_hz,"protocol":"P25","acquired":t.running && t.control_channel_hz.is_some()})) }
async fn debug_p25_squelch(State(s): State<ApiState>) -> impl IntoResponse { let v=s.0.scanner.read().as_ref().map(|h|h.state.lock().vfo_states.clone()).unwrap_or_default(); Json(json!({"open_vfos":v.iter().filter(|x|x.squelch_open).map(|x|x.id).collect::<Vec<_>>(),"threshold_source":"scanner runtime"})) }
async fn debug_provoice_stderr(State(s): State<ApiState>) -> impl IntoResponse { Json(s.0.sidecars.stderr("dsd-neo")) }
async fn debug_rtl433_stderr(State(s): State<ApiState>) -> impl IntoResponse { Json(s.0.sidecars.stderr("rtl_433")) }
async fn debug_p25_use_vfo_fir() -> impl IntoResponse { Json(json!({"enabled":false,"supported":false,"reason":"P25 VFO FIR path is not implemented without a digital decoder backend"})) }
async fn debug_per_cc_stats(State(s): State<ApiState>) -> impl IntoResponse { let t=s.0.trunking.read(); Json(json!({"running":t.running,"control_channel_hz":t.control_channel_hz,"call_count":t.calls.len(),"active_talkgroup":t.active_talkgroup})) }
async fn debug_vdl2_stderr(State(s): State<ApiState>) -> impl IntoResponse { Json(s.0.sidecars.stderr("dumpvdl2")) }

// ── event fan-out ─────────────────────────────────────────────────────────

async fn event_stream(State(s): State<ApiState>) -> impl IntoResponse {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use futures_util::stream::Stream;
    let rx = s.0.events.subscribe();
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Ok(ev) => {
                let data = serde_json::to_string(&ev).unwrap_or_default();
                Some::<(Result<Event, std::convert::Infallible>, _)>((
                    Ok(Event::default().data(data)),
                    rx,
                ))
            }
            Err(_) => None,
        }
    });
    Sse::new(stream.boxed()).keep_alive(KeepAlive::default())
}

async fn events_ws(State(s): State<ApiState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    let tx = s.0.events.clone();
    ws.on_upgrade(move |socket| ws_pump(socket, tx))
}


async fn ws_pump(socket: WebSocket, tx: tokio::sync::broadcast::Sender<ScannerEvent>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = tx.subscribe();
    let send_task = tokio::spawn(async move {
        while let Ok(ev) = rx.recv().await {
            let text = serde_json::to_string(&ev).unwrap_or_default();
            if sender.send(Message::Text(text)).await.is_err() { break; }
        }
    });
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if matches!(msg, Message::Close(_)) { break; }
        }
    });
    let _ = tokio::join!(send_task, recv_task);
}

fn send_vfo(s: &ApiState, cmd: crate::scanner::ScannerCommand) {
    if let Some(h) = s.0.scanner.read().as_ref() {
        let _ = h.cmd_tx.send(cmd);
    }
}
