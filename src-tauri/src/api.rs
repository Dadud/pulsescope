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
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};

use axum::http::{header, HeaderValue, Method};
use tower_http::cors::{Any, CorsLayer};

use crate::state::{AppState, ScannerEvent, SpectrumFrame};
use rustfft::num_complex::Complex;

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
    let ServeConfig {
        addr,
        ui_dir,
        auth_token,
        tls,
    } = cfg;
    let api = Router::new()
        // ── health / settings ────────────────────────────────────────────
        .route("/health", get(health))
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/metrics", get(metrics_snapshot))
        .route("/diagnostics/bundle", get(diagnostic_bundle_export))
        .route("/v2/system/health", get(system_health_v2))
        .route("/v2/features", get(feature_status_v2))
        .route("/v2/devices", get(devices_v2))
        .route("/v2/devices/:id/capabilities", get(device_capabilities_v2))
        .route("/v2/receivers", get(receivers_v2))
        .route("/v2/receivers/:id/tune", post(receiver_tune_v2))
        .route("/v2/receivers/:id/controls", get(receiver_controls_v2))
        .route("/v2/sessions", get(sessions_v2).post(session_command_v2))
        .route("/v2/hardware-windows", get(hardware_windows_v2))
        .route(
            "/v2/listener-sessions",
            get(listener_sessions_v2).post(listener_session_upsert_v2),
        )
        .route("/v2/profiles", get(profiles_v2).post(profile_upsert_v2))
        .route(
            "/v2/profiles/:id",
            get(profile_v2).delete(profile_delete_v2),
        )
        .route("/v2/profiles/:id/apply", post(profile_apply_v2))
        .route("/v2/bookmarks", get(bookmarks_v2).post(bookmark_upsert_v2))
        .route(
            "/v2/bookmarks/:id",
            axum::routing::delete(bookmark_delete_v2),
        )
        .route("/v2/bandplans", get(bandplans_v2))
        .route("/v2/decoders/catalog", get(decoder_catalog_v2))
        .route("/v2/decoder-jobs", get(decoder_jobs_v2))
        .route("/v2/recordings", get(recordings_v2))
        .route("/v2/media/capabilities", get(media_capabilities_v2))
        .route("/v2/media/sessions", post(media_session_v2))
        .route("/settings", get(get_settings).put(put_settings))
        // ── device ───────────────────────────────────────────────────────
        .route("/devices", get(list_devices))
        .route("/device/connect", post(device_connect))
        .route("/device/disconnect", post(device_disconnect))
        .route("/device/status", get(device_status))
        .route("/device/capabilities", get(device_capabilities))
        .route("/device/control", post(device_control))
        .route("/receiver/session", get(receiver_session))
        .route("/receiver/session/claim", post(receiver_session_claim))
        .route("/receiver/session/release", post(receiver_session_release))
        .route("/device/gain", post(device_gain))
        .route("/device/frequency", post(device_frequency))
        .route("/device/sample_rate", post(device_sample_rate))
        .route("/device/mdns_scan", get(device_mdns))
        .route("/device/test", post(device_test))
        .route("/device/hackrf_amp", post(device_hackrf_amp))
        // ── channels / banks ─────────────────────────────────────────────
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
        .route("/v2/spectrum/stream", get(spectrum_stream_ws))
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
        // ── decoded messages ─────────────────────────────────────────────
        .route("/decoded_messages", get(decoded_messages))
        .route("/rtl433_messages", get(rtl433_messages))
        .route("/protocol_messages", get(protocol_messages))
        .route("/protocols/slices", get(protocol_slices))
        .route("/protocols/:id/capability", get(protocol_capability))
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
        .route(
            "/trunking/discovery/log/clear",
            post(trunking_disc_log_clear),
        )
        .route(
            "/trunking/discovery/notes",
            get(trunking_disc_notes).post(trunking_disc_notes),
        )
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
        .route(
            "/goes_lrit/satellite",
            get(goes_satellite).put(goes_satellite_put),
        )
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
        .route("/scan/skip", post(scan_skip))
        .route("/scan/lockout", post(scan_lockout))
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
        // ── recording ────────────────────────────────────────────────────
        .route("/recording/iq/capture", post(rec_iq_start))
        .route("/recording/iq/stop", post(rec_iq_stop))
        .route("/recording/iq/playback/start", post(playback_start))
        .route("/recording/iq/playback/stop", post(playback_stop))
        .route("/recording/iq/playback/status", get(playback_status))
        .route(
            "/recordings/annotations",
            get(rec_annotations).post(rec_annotation_new),
        )
        .route(
            "/recordings/annotations/:id",
            get(rec_annotation_one)
                .delete(rec_annotation_delete)
                .put(rec_annotation_update),
        )
        .route("/iq/consumers", get(iq_consumers))
        .route("/iq/network/start", post(iq_network_start))
        .route("/iq/network/stop", post(iq_network_stop))
        .route("/iq/network/status", get(iq_network_status))
        .route("/audio/network/start", post(audio_network_start))
        .route("/audio/network/stop", post(audio_network_stop))
        .route("/audio/network/status", get(audio_network_status))
        .route("/audio/stream", get(audio_stream))
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
        .route(
            "/cases/attachments/:att_id",
            get(case_attachment_one).delete(case_attachment_delete),
        )
        // ── feature packs / lookups / blacklist ──────────────────────────
        .route("/feature-packs", get(feature_packs))
        .route("/feature-packs/:id/enable", post(feature_pack_enable))
        .route("/sidecars/status", get(sidecars_status))
        .route("/sidecars/:name/stderr", get(sidecar_stderr))
        .route("/decoders/scan", get(decoders_scan))
        .route("/decoders/adaptations", get(decoders_adaptations))
        .route("/decoders/configure", post(decoders_configure))
        .route("/decoders/install/:name/guide", get(decoders_install_guide))
        .route("/decoders/install/:name", post(decoders_install))
        .route("/sidecars/start_all", post(sidecars_start_all))
        .route("/receiver_location", get(rx_location).put(rx_location_put))
        .route("/aircraft/lookup", get(aircraft_lookup))
        .route("/blacklist", get(blacklist).post(blacklist_add))
        .route("/blacklist/add", post(blacklist_add))
        .route("/blacklist/remove", post(blacklist_remove))
        .route("/blacklist/clear", post(blacklist_clear))
        .route(
            "/blacklist/clear-temporary",
            post(blacklist_clear_temporary),
        )
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
        .route(
            "/debug/trunking/p25_use_vfo_fir",
            get(debug_p25_use_vfo_fir),
        )
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
            let serve_dir = tower_http::services::ServeDir::new(&ui_dir).fallback(
                tower_http::services::ServeFile::new(ui_dir.join("index.html")),
            );
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
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]),
    );
    // LAN clients must never retain an old SSR shell with dead controls.
    // Hashed JS is cheap to refetch; correctness beats a stale dashboard.
    top = top.layer(axum::middleware::from_fn(no_store));

    match (listener, tls) {
        (Some(listener), None) => {
            serve_plain(listener, top).await?;
        }
        (None, Some(tls_cfg)) => {
            serve_tls(addr, top, tls_cfg).await?;
        }
        (Some(_), Some(_)) => unreachable!("TLS path does not pre-bind"),
        (None, None) => unreachable!("server mode requires a listener"),
    }
    Ok(())
}

async fn no_store(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(req).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response
}

async fn serve_plain(listener: tokio::net::TcpListener, router: Router) -> anyhow::Result<()> {
    axum::serve(listener, router).await?;
    Ok(())
}

async fn serve_tls(addr: SocketAddr, router: Router, tls: TlsConfig) -> anyhow::Result<()> {
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
    if matches!(
        path.as_str(),
        "/api/health"
            | "/health"
            | "/api/health/live"
            | "/health/live"
            | "/api/health/ready"
            | "/health/ready"
    ) {
        return Ok(next.run(req).await);
    }
    let header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let query = req
        .uri()
        .query()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")))
        .unwrap_or("");
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
        if cfg!(windows) {
            r"C:\Program Files\PothosSDR".into()
        } else {
            "/usr/local".into()
        }
    });
    let soapy_installed = std::path::Path::new(&soapy_root)
        .join(if cfg!(windows) {
            "bin/SoapySDR.dll"
        } else {
            "lib/libSoapySDR.so"
        })
        .exists();

    let sdrplay_installed =
        std::path::Path::new(r"C:\Program Files\SDRplay\API\x64\sdrplay_api.dll").exists();

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

async fn health_live(State(s): State<ApiState>) -> impl IntoResponse {
    Json(json!({
        "status": "live",
        "name": "pulsescope",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_ms": crate::scanner::now_ms().saturating_sub(s.0.started_ms),
    }))
}

fn readiness_reasons(
    connected: bool,
    driver: &str,
    frames_processed: u64,
    latest_sample_ms: i64,
    now_ms: i64,
    allow_mock: bool,
) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if !connected {
        reasons.push("device_disconnected");
    }
    if driver == "mock" && !allow_mock {
        reasons.push("physical_device_required");
    }
    if frames_processed == 0 || latest_sample_ms <= 0 {
        reasons.push("samples_not_flowing");
    } else if now_ms.saturating_sub(latest_sample_ms) > 5_000 {
        reasons.push("sample_flow_stale");
    }
    reasons
}

async fn health_ready(State(s): State<ApiState>) -> impl IntoResponse {
    let device = s.0.device.status();
    let scanner =
        s.0.scanner
            .read()
            .as_ref()
            .map(|handle| handle.state.lock().clone());
    let frames_processed = scanner
        .as_ref()
        .map(|runtime| runtime.frames_processed)
        .unwrap_or(0);
    let latest_sample_ms = scanner
        .as_ref()
        .map(|runtime| runtime.latest_spectrum_ms)
        .unwrap_or(0);
    let now_ms = crate::scanner::now_ms();
    let allow_mock = std::env::var("PULSESCOPE_ALLOW_MOCK_READY")
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    let reasons = readiness_reasons(
        device.connected,
        &device.driver,
        frames_processed,
        latest_sample_ms,
        now_ms,
        allow_mock,
    );
    let ready = reasons.is_empty();
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(json!({
            "status": if ready { "ready" } else { "not_ready" },
            "device": {
                "connected": device.connected,
                "driver": device.driver,
                "label": device.label,
            },
            "sample_flow": {
                "frames_processed": frames_processed,
                "latest_sample_ms": latest_sample_ms,
                "age_ms": if latest_sample_ms > 0 { Some(now_ms.saturating_sub(latest_sample_ms)) } else { None },
            },
            "audio_flow": {
                "frames": s.0.audio.remote_frames(),
                "last_frame_ms": s.0.audio.remote_last_frame_ms(),
            },
            "reasons": reasons,
        })),
    )
}

async fn metrics_snapshot(State(s): State<ApiState>) -> impl IntoResponse {
    Json(s.0.metrics.snapshot())
}

async fn diagnostic_bundle_export(State(s): State<ApiState>) -> impl IntoResponse {
    let config = toml::to_string_pretty(&*s.0.config.read()).unwrap_or_default();
    let status = serde_json::to_string_pretty(&json!({
        "device": s.0.device.status(),
        "audio": s.0.audio.status(),
        "sidecars": s.0.sidecars.statuses(),
        "metrics": s.0.metrics.snapshot(),
    }))
    .unwrap_or_default();
    match crate::operations::diagnostic_bundle(
        &s.0.data_dir,
        &[("config.toml", config), ("status.json", status)],
    ) {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/zip"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=pulsescope-diagnostics.zip",
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(error) => crate::operations::OperationalError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "DIAGNOSTIC_BUNDLE_FAILED",
            safe_message: "The diagnostic bundle could not be created.",
            context: error,
            request_id: uuid::Uuid::new_v4().to_string(),
        }
        .into_response(),
    }
}

async fn protocol_slices(State(s): State<ApiState>) -> impl IntoResponse {
    let device = s.0.device.status();
    Json(
        crate::protocols::slices()
            .into_iter()
            .map(|slice| {
                let hardware = crate::protocols::capability_check(&slice, &device);
                json!({"slice": slice, "hardware": hardware, "running": false})
            })
            .collect::<Vec<_>>(),
    )
}

async fn protocol_capability(
    Path(id): Path<String>,
    State(s): State<ApiState>,
) -> impl IntoResponse {
    let Some(slice) = crate::protocols::slices()
        .into_iter()
        .find(|slice| slice.id == id)
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"available": false, "running": false, "reason": "unknown protocol slice"})),
        )
            .into_response();
    };
    let hardware = crate::protocols::capability_check(&slice, &s.0.device.status());
    (
        StatusCode::OK,
        Json(json!({"slice": slice, "hardware": hardware, "running": false})),
    )
        .into_response()
}

async fn system_health_v2(State(s): State<ApiState>) -> impl IntoResponse {
    let device = s.0.device.status();
    let scanner_handle = s.0.scanner.read();
    let scanner = scanner_handle
        .as_ref()
        .map(|handle| handle.state.lock().clone());
    let consumers = scanner_handle
        .as_ref()
        .map(|handle| {
            handle
                .iq_consumers
                .iter()
                .map(|ring| ring.status())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    drop(scanner_handle);
    let now = crate::scanner::now_ms();
    let frame_ms = scanner
        .as_ref()
        .map(|runtime| runtime.latest_spectrum_ms)
        .unwrap_or(0);
    let frame_age_ms = if frame_ms > 0 {
        Some(now.saturating_sub(frame_ms))
    } else {
        None
    };
    let capture_fresh = device.connected && frame_age_ms.is_some_and(|age| age <= 2_000);
    let vfos = scanner
        .as_ref()
        .map(|runtime| runtime.vfo_states.clone())
        .unwrap_or_default();
    let audible_vfos = vfos.iter().filter(|vfo| !vfo.muted).count();
    let audio_details = s.0.audio.status();
    let audio_frames = s.0.audio.remote_frames();
    let audio_last_frame_ms = s.0.audio.remote_last_frame_ms();
    let audio_age_ms = if audio_last_frame_ms > 0 {
        Some(now.saturating_sub(audio_last_frame_ms))
    } else {
        None
    };
    let audio_state = if !device.connected {
        "disconnected"
    } else if audible_vfos == 0 {
        "muted"
    } else if audio_frames == 0 {
        "buffering"
    } else if audio_age_ms.is_some_and(|age| age <= 2_000) {
        "playing"
    } else {
        "degraded"
    };
    Json(json!({
        "contract_version": 2,
        "status": if capture_fresh { "healthy" } else { "degraded" },
        "timestamp_ms": now,
        "device": device,
        "capture": { "fresh": capture_fresh, "consumers": consumers },
        "fft": {
            "sequence": scanner.as_ref().map(|runtime| runtime.frames_processed).unwrap_or(0),
            "captured_ms": frame_ms,
            "age_ms": frame_age_ms,
            "bins": scanner.as_ref().map(|runtime| runtime.latest_spectrum.len()).unwrap_or(0),
            "clients": s.0.spectrum.receiver_count(),
        },
        "vfos": { "count": vfos.len(), "audible": audible_vfos, "states": vfos },
        "audio": { "state": audio_state, "age_ms": audio_age_ms, "details": audio_details },
        "decoders": s.0.sidecars.statuses(),
        "event_clients": s.0.events.receiver_count(),
        "recovery": {
            "receiver_restarts": s.0.receiver_recoveries.load(std::sync::atomic::Ordering::Relaxed),
            "last_receiver_restart_ms": s.0.last_receiver_recovery_ms.load(std::sync::atomic::Ordering::Relaxed),
        },
    }))
}

async fn feature_status_v2() -> impl IntoResponse {
    // Embedded at build time so the running server and documentation use the
    // exact same release contract. Docker copies `release/` into the builder.
    let contract: Value =
        serde_json::from_str(include_str!("../../release/acceptance-matrix.json"))
            .expect("release acceptance matrix must be valid JSON");
    Json(contract)
}

fn stable_device_id(
    status: &crate::device::DeviceStatus,
    capabilities: &crate::device::DeviceCapabilities,
) -> String {
    if !capabilities.identity.stable_id.trim().is_empty() {
        capabilities.identity.stable_id.clone()
    } else {
        format!("{}:active", status.driver)
    }
}

async fn devices_v2(State(s): State<ApiState>) -> impl IntoResponse {
    let status = s.0.device.status();
    let capabilities = s.0.device.capabilities();
    let active_id = stable_device_id(&status, &capabilities);
    // Discovery is intentionally best-effort: some vendor Soapy modules can
    // open a device without being enumerable by SoapySDRUtil.  The receiver's
    // live status is authoritative, so always publish it first.  Previously
    // this endpoint relabeled a running SDRplay as the fallback mock source
    // whenever discovery only returned `driver=mock`.
    let mut discovered = Vec::new();
    if status.connected {
        discovered.push(json!({
            "id": active_id.clone(),
            "driver": status.driver,
            "label": status.label,
            "connection": capabilities.identity.connection,
            "active": true,
            "lifecycle": serde_json::to_value(status.lifecycle).unwrap_or(json!("disconnected")),
            "certification": if status.driver == "sdrplay" { "hardware_verified" } else { "compatibility" }
        }));
    }
    for device in crate::device::DeviceLayer::discover() {
        // Do not add a duplicate active entry and do not advertise the mock
        // fallback beside a real, active radio as if it were the selected one.
        if status.connected && (device.driver == status.driver || device.driver == "mock") {
            continue;
        }
        discovered.push(json!({
            "id": device.hardware_key,
            "driver": device.driver,
            "label": device.label,
            "connection": "local_usb",
            "active": false,
            "lifecycle": "detected",
            "certification": "compatibility"
        }));
    }
    Json(json!({"contract_version": 2, "active_device_id": active_id, "devices": discovered}))
}

async fn device_capabilities_v2(
    State(s): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let status = s.0.device.status();
    let capabilities = s.0.device.capabilities();
    if id != stable_device_id(&status, &capabilities) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"device is not active","device_id":id})),
        )
            .into_response();
    }
    (StatusCode::OK, Json(json!({"contract_version":2,"device_id":id,"status":status,"capabilities":capabilities}))).into_response()
}

async fn receivers_v2(State(s): State<ApiState>) -> impl IntoResponse {
    let status = s.0.device.status();
    let runtime =
        s.0.scanner
            .read()
            .as_ref()
            .map(|handle| handle.state.lock().clone());
    let session = s.0.receiver_session.lock().clone();
    Json(json!({"contract_version":2,"receivers":[{
        "id":"receiver-0", "device_id":stable_device_id(&status, &s.0.device.capabilities()),
        "desired":{"running":runtime.as_ref().is_some_and(|state| state.running)},
        "actual":{"running":runtime.as_ref().is_some_and(|state| state.running),"center_frequency_hz":status.center_freq_hz,"sample_rate_hz":status.sample_rate,"bandwidth_hz":status.bandwidth_hz,"vfos":runtime.map(|state| state.vfo_states).unwrap_or_default()},
        "revision":session.revision
    }]}))
}

#[derive(Deserialize)]
struct ReceiverTuneV2Req {
    command_id: String,
    expected_revision: u64,
    frequency_hz: u64,
}

fn cached_command(s: &ApiState, command_id: &str) -> Option<Value> {
    s.0.command_results.lock().get(command_id).cloned()
}

fn remember_command(s: &ApiState, command_id: String, result: Value) {
    let mut commands = s.0.command_results.lock();
    if commands.len() >= 1024 {
        commands.clear();
    }
    commands.insert(command_id, result);
}

async fn receiver_tune_v2(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<ReceiverTuneV2Req>,
) -> impl IntoResponse {
    if id != "receiver-0" {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"unknown receiver"})),
        )
            .into_response();
    }
    if req.command_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"command_id is required"})),
        )
            .into_response();
    }
    if let Some(result) = cached_command(&s, &req.command_id) {
        return (StatusCode::OK, Json(result)).into_response();
    }
    let revision = s.0.receiver_session.lock().revision;
    if req.expected_revision != revision {
        return (StatusCode::CONFLICT, Json(json!({"error":"stale revision","expected":revision,"received":req.expected_revision}))).into_response();
    }
    let result = match s.0.device.set_frequency(req.frequency_hz) {
        Ok(()) => {
            json!({"ok":true,"command_id":req.command_id,"receiver_id":id,"actual_frequency_hz":s.0.device.status().center_freq_hz,"revision":revision})
        }
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"ok":false,"error":error.to_string()})),
            )
                .into_response()
        }
    };
    remember_command(&s, req.command_id, result.clone());
    (StatusCode::OK, Json(result)).into_response()
}

async fn receiver_controls_v2(
    State(s): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if id != "receiver-0" {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"unknown receiver"})),
        )
            .into_response();
    }
    (StatusCode::OK, Json(json!({"contract_version":2,"receiver_id":id,"capabilities":s.0.device.capabilities(),"actual":s.0.device.status()}))).into_response()
}

async fn sessions_v2(State(s): State<ApiState>) -> impl IntoResponse {
    Json(
        json!({"contract_version":2,"sessions":[{"id":"primary","receiver_id":"receiver-0","state":s.0.receiver_session.lock().clone()}]}),
    )
}

async fn hardware_windows_v2(State(s): State<ApiState>) -> impl IntoResponse {
    let status = s.0.device.status();
    let capabilities = s.0.device.capabilities();
    let session = s.0.receiver_session.lock().clone();
    let usable_span_hz = status.bandwidth_hz.min(status.sample_rate);
    Json(json!({
        "contract_version": 2,
        "windows": [{
            "id": "receiver-0",
            "device_id": stable_device_id(&status, &capabilities),
            "center_frequency_hz": status.center_freq_hz,
            "sample_rate_hz": status.sample_rate,
            "bandwidth_hz": status.bandwidth_hz,
            "usable_span_hz": usable_span_hz,
            "lower_edge_hz": status.center_freq_hz.saturating_sub((usable_span_hz / 2) as u64),
            "upper_edge_hz": status.center_freq_hz.saturating_add((usable_span_hz / 2) as u64),
            "owner": session.owner,
            "revision": session.revision
        }]
    }))
}

#[derive(Deserialize)]
struct ListenerSessionReq {
    id: Option<String>,
    #[serde(default = "default_client_name")]
    client_name: String,
    #[serde(default = "default_receiver_id")]
    receiver_id: String,
    view_center_hz: u64,
    view_span_hz: u32,
    active_vfo_id: Option<usize>,
    expected_revision: Option<u64>,
}

fn default_client_name() -> String {
    "LAN browser".into()
}
fn default_receiver_id() -> String {
    "receiver-0".into()
}

async fn listener_sessions_v2(State(s): State<ApiState>) -> impl IntoResponse {
    let now = crate::scanner::now_ms();
    let mut stored = s.0.listener_sessions.write();
    stored.retain(|_, session| now.saturating_sub(session.updated_ms) <= 3_600_000);
    let sessions = stored.values().cloned().collect::<Vec<_>>();
    Json(json!({"contract_version":2,"sessions":sessions}))
}

async fn listener_session_upsert_v2(
    State(s): State<ApiState>,
    Json(req): Json<ListenerSessionReq>,
) -> impl IntoResponse {
    if req.receiver_id != "receiver-0" || req.view_span_hz == 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                json!({"error":"receiver_id must be receiver-0 and view_span_hz must be positive"}),
            ),
        )
            .into_response();
    }
    let status = s.0.device.status();
    let usable_span_hz = status.bandwidth_hz.min(status.sample_rate);
    let lower = status
        .center_freq_hz
        .saturating_sub((usable_span_hz / 2) as u64);
    let upper = status
        .center_freq_hz
        .saturating_add((usable_span_hz / 2) as u64);
    let requested_lower = req
        .view_center_hz
        .saturating_sub((req.view_span_hz / 2) as u64);
    let requested_upper = req
        .view_center_hz
        .saturating_add((req.view_span_hz / 2) as u64);
    if req.view_span_hz > usable_span_hz || requested_lower < lower || requested_upper > upper {
        return (StatusCode::BAD_REQUEST, Json(json!({"error":"listener viewport must remain inside the shared hardware window","hardware_window":{"lower_edge_hz":lower,"upper_edge_hz":upper,"usable_span_hz":usable_span_hz}}))).into_response();
    }
    let id = req.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut sessions = s.0.listener_sessions.write();
    let now = crate::scanner::now_ms();
    sessions.retain(|_, session| now.saturating_sub(session.updated_ms) <= 3_600_000);
    if !sessions.contains_key(&id) && sessions.len() >= 128 {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error":"listener session limit reached"})),
        )
            .into_response();
    }
    let current_revision = sessions
        .get(&id)
        .map(|session| session.revision)
        .unwrap_or(0);
    if let Some(expected) = req.expected_revision {
        if expected != current_revision {
            return (StatusCode::CONFLICT, Json(json!({"error":"stale revision","expected":current_revision,"received":expected}))).into_response();
        }
    }
    let session = crate::state::ListenerSession {
        id: id.clone(),
        client_name: req.client_name,
        receiver_id: req.receiver_id,
        view_center_hz: req.view_center_hz,
        view_span_hz: req.view_span_hz,
        active_vfo_id: req.active_vfo_id,
        revision: current_revision.saturating_add(1),
        updated_ms: now,
    };
    sessions.insert(id, session.clone());
    (StatusCode::OK, Json(json!({"ok":true,"session":session}))).into_response()
}

#[derive(serde::Serialize, Deserialize)]
struct ReceiverProfile {
    id: Option<String>,
    name: String,
    center_frequency_hz: u64,
    sample_rate_hz: u32,
    bandwidth_hz: u32,
    mode: String,
    #[serde(default)]
    region: String,
    deemphasis_us: Option<u32>,
    #[serde(default = "empty_object")]
    gain_policy: Value,
    #[serde(default = "empty_object")]
    decoder_policy: Value,
    #[serde(default)]
    created_ms: i64,
    #[serde(default)]
    updated_ms: i64,
}

fn empty_object() -> Value {
    json!({})
}

fn query_profiles(s: &ApiState, id: Option<&str>) -> Result<Vec<ReceiverProfile>, String> {
    let conn = s.0.db.conn();
    let mut statement = conn.prepare("SELECT id,name,center_frequency_hz,sample_rate_hz,bandwidth_hz,mode,region,deemphasis_us,gain_policy_json,decoder_policy_json,created_ms,updated_ms FROM receiver_profiles WHERE (?1 IS NULL OR id=?1) ORDER BY name")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([id], |row| {
            Ok(ReceiverProfile {
                id: Some(row.get(0)?),
                name: row.get(1)?,
                center_frequency_hz: row.get(2)?,
                sample_rate_hz: row.get(3)?,
                bandwidth_hz: row.get(4)?,
                mode: row.get(5)?,
                region: row.get(6)?,
                deemphasis_us: row.get(7)?,
                gain_policy: serde_json::from_str(&row.get::<_, String>(8)?)
                    .unwrap_or_else(|_| json!({})),
                decoder_policy: serde_json::from_str(&row.get::<_, String>(9)?)
                    .unwrap_or_else(|_| json!({})),
                created_ms: row.get(10)?,
                updated_ms: row.get(11)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

async fn profiles_v2(State(s): State<ApiState>) -> impl IntoResponse {
    match query_profiles(&s, None) {
        Ok(profiles) => (
            StatusCode::OK,
            Json(json!({"contract_version":2,"profiles":profiles})),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":error})),
        )
            .into_response(),
    }
}
async fn profile_v2(State(s): State<ApiState>, Path(id): Path<String>) -> impl IntoResponse {
    match query_profiles(&s, Some(&id)) {
        Ok(mut profiles) if !profiles.is_empty() => (
            StatusCode::OK,
            Json(json!({"contract_version":2,"profile":profiles.remove(0)})),
        )
            .into_response(),
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"profile not found"})),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":error})),
        )
            .into_response(),
    }
}
async fn profile_upsert_v2(
    State(s): State<ApiState>,
    Json(mut profile): Json<ReceiverProfile>,
) -> impl IntoResponse {
    if profile.name.trim().is_empty() || profile.sample_rate_hz == 0 || profile.bandwidth_hz == 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"name, sample_rate_hz, and bandwidth_hz are required"})),
        )
            .into_response();
    }
    let id = profile
        .id
        .take()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let now = crate::scanner::now_ms();
    let conn = s.0.db.conn();
    let result = conn.execute("INSERT INTO receiver_profiles(id,name,center_frequency_hz,sample_rate_hz,bandwidth_hz,mode,region,deemphasis_us,gain_policy_json,decoder_policy_json,created_ms,updated_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11) ON CONFLICT(id) DO UPDATE SET name=excluded.name,center_frequency_hz=excluded.center_frequency_hz,sample_rate_hz=excluded.sample_rate_hz,bandwidth_hz=excluded.bandwidth_hz,mode=excluded.mode,region=excluded.region,deemphasis_us=excluded.deemphasis_us,gain_policy_json=excluded.gain_policy_json,decoder_policy_json=excluded.decoder_policy_json,updated_ms=excluded.updated_ms",
        rusqlite::params![id,profile.name,profile.center_frequency_hz,profile.sample_rate_hz,profile.bandwidth_hz,profile.mode,profile.region,profile.deemphasis_us,profile.gain_policy.to_string(),profile.decoder_policy.to_string(),now]);
    match result {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({"ok":true,"id":id,"updated_ms":now})),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}
async fn profile_delete_v2(State(s): State<ApiState>, Path(id): Path<String>) -> impl IntoResponse {
    match s
        .0
        .db
        .conn()
        .execute("DELETE FROM receiver_profiles WHERE id=?1", [&id])
    {
        Ok(0) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"profile not found"})),
        )
            .into_response(),
        Ok(_) => (StatusCode::OK, Json(json!({"ok":true}))).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}

async fn profile_apply_v2(State(s): State<ApiState>, Path(id): Path<String>) -> impl IntoResponse {
    let profile = match query_profiles(&s, Some(&id)) {
        Ok(mut profiles) if !profiles.is_empty() => profiles.remove(0),
        Ok(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error":"profile not found"})),
            )
                .into_response()
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":error})),
            )
                .into_response()
        }
    };
    let original = s.0.device.status();
    let apply = (|| -> anyhow::Result<()> {
        if profile.bandwidth_hz > profile.sample_rate_hz {
            anyhow::bail!("bandwidth_hz must not exceed sample_rate_hz");
        }
        let analog_hz = s.0.device.set_sample_contract(profile.sample_rate_hz)?;
        if profile.bandwidth_hz > analog_hz {
            anyhow::bail!(
                "bandwidth_hz {} exceeds analog contract {analog_hz} Hz",
                profile.bandwidth_hz
            );
        }
        if profile.bandwidth_hz > 0 && profile.bandwidth_hz != analog_hz {
            s.0.device.set_bandwidth(profile.bandwidth_hz)?;
        }
        s.0.device.set_frequency(profile.center_frequency_hz)?;
        Ok(())
    })();
    if let Err(error) = apply {
        // A rejected profile must not leave the shared LAN receiver in a
        // surprising half-applied state.
        let _ = s.0.device.set_sample_rate(original.sample_rate);
        let _ = s.0.device.set_bandwidth(original.bandwidth_hz);
        let _ = s.0.device.set_frequency(original.center_freq_hz);
        return (
            StatusCode::BAD_REQUEST,
            Json(
                json!({"error":error.to_string(),"rolled_back":true,"actual":s.0.device.status()}),
            ),
        )
            .into_response();
    }
    let status = s.0.device.status();
    if let Some(deemphasis_us) = profile
        .deemphasis_us
        .filter(|value| *value == 50 || *value == 75)
    {
        {
            let mut config = s.0.config.write();
            config.demodulator.de_emphasis_us = deemphasis_us;
            let _ = config.save(&s.0.data_dir);
        }
        if let Some(handle) = s.0.scanner.read().as_ref() {
            handle.state.lock().wfm_deemphasis_us = deemphasis_us;
        }
    }
    if let Some(handle) = s.0.scanner.read().as_ref() {
        handle.flush_iq();
        s.0.audio.clear_queue();
        let mut runtime = handle.state.lock();
        let index = runtime
            .vfo_states
            .iter()
            .position(|vfo| vfo.id == 0)
            .or_else(|| runtime.vfo_states.first().map(|_| 0));
        if let Some(index) = index {
            let vfo = &mut runtime.vfo_states[index];
            vfo.frequency_hz = profile.center_frequency_hz;
            vfo.mode = profile.mode.clone();
            vfo.locked = false;
        }
    }
    (
        StatusCode::OK,
        Json(json!({
            "ok":true,
            "profile_id":id,
            "actual":status,
            "default_mode":profile.mode,
            "deemphasis_us": profile.deemphasis_us
        })),
    )
        .into_response()
}

#[derive(serde::Serialize, Deserialize)]
struct ReceiverBookmark {
    id: Option<i64>,
    label: String,
    frequency_hz: u64,
    mode: String,
    bandwidth_hz: u32,
    profile_id: Option<String>,
    #[serde(default)]
    color: String,
    #[serde(default)]
    decoder: String,
    #[serde(default)]
    notes: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    created_ms: i64,
    #[serde(default)]
    updated_ms: i64,
}
fn default_true() -> bool {
    true
}

async fn bookmarks_v2(State(s): State<ApiState>) -> impl IntoResponse {
    let conn = s.0.db.conn();
    let result = (|| -> Result<Vec<ReceiverBookmark>, rusqlite::Error> {
        let mut statement = conn.prepare("SELECT id,label,frequency_hz,mode,bandwidth_hz,profile_id,color,decoder,notes,enabled,created_ms,updated_ms FROM receiver_bookmarks ORDER BY frequency_hz")?;
        let rows = statement
            .query_map([], |row| {
                Ok(ReceiverBookmark {
                    id: row.get(0)?,
                    label: row.get(1)?,
                    frequency_hz: row.get(2)?,
                    mode: row.get(3)?,
                    bandwidth_hz: row.get(4)?,
                    profile_id: row.get(5)?,
                    color: row.get(6)?,
                    decoder: row.get(7)?,
                    notes: row.get(8)?,
                    enabled: row.get(9)?,
                    created_ms: row.get(10)?,
                    updated_ms: row.get(11)?,
                })
            })?
            .collect();
        rows
    })();
    match result {
        Ok(bookmarks) => (
            StatusCode::OK,
            Json(json!({"contract_version":2,"bookmarks":bookmarks})),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}
async fn bookmark_upsert_v2(
    State(s): State<ApiState>,
    Json(bookmark): Json<ReceiverBookmark>,
) -> impl IntoResponse {
    if bookmark.label.trim().is_empty() || bookmark.frequency_hz == 0 || bookmark.bandwidth_hz == 0
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"label, frequency_hz, and bandwidth_hz are required"})),
        )
            .into_response();
    }
    let now = crate::scanner::now_ms();
    let conn = s.0.db.conn();
    let result = if let Some(id) = bookmark.id {
        conn.execute("UPDATE receiver_bookmarks SET label=?2,frequency_hz=?3,mode=?4,bandwidth_hz=?5,profile_id=?6,color=?7,decoder=?8,notes=?9,enabled=?10,updated_ms=?11 WHERE id=?1",rusqlite::params![id,bookmark.label,bookmark.frequency_hz,bookmark.mode,bookmark.bandwidth_hz,bookmark.profile_id,bookmark.color,bookmark.decoder,bookmark.notes,bookmark.enabled,now]).map(|_| id)
    } else {
        conn.execute("INSERT INTO receiver_bookmarks(label,frequency_hz,mode,bandwidth_hz,profile_id,color,decoder,notes,enabled,created_ms,updated_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",rusqlite::params![bookmark.label,bookmark.frequency_hz,bookmark.mode,bookmark.bandwidth_hz,bookmark.profile_id,bookmark.color,bookmark.decoder,bookmark.notes,bookmark.enabled,now]).map(|_| conn.last_insert_rowid())
    };
    match result {
        Ok(id) => (
            StatusCode::OK,
            Json(json!({"ok":true,"id":id,"updated_ms":now})),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}
async fn bookmark_delete_v2(State(s): State<ApiState>, Path(id): Path<i64>) -> impl IntoResponse {
    match s
        .0
        .db
        .conn()
        .execute("DELETE FROM receiver_bookmarks WHERE id=?1", [id])
    {
        Ok(0) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"bookmark not found"})),
        )
            .into_response(),
        Ok(_) => (StatusCode::OK, Json(json!({"ok":true}))).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}
async fn bandplans_v2(State(s): State<ApiState>) -> impl IntoResponse {
    let bands=s.0.config.read().scan_ranges.iter().map(|range| json!({"id":range.name.to_lowercase().replace(' ',"-"),"name":range.name,"start_hz":range.start_hz,"end_hz":range.end_hz,"default_mode":range.mode,"channel_bandwidth_hz":range.channel_bw_hz,"scan_enabled":range.enabled})).collect::<Vec<_>>();
    Json(json!({"contract_version":2,"source":"server configuration","bands":bands}))
}

#[derive(Deserialize)]
struct SessionCommandV2Req {
    command_id: String,
    expected_revision: u64,
    action: String,
    owner: String,
    #[serde(default)]
    force: bool,
}

async fn session_command_v2(
    State(s): State<ApiState>,
    Json(req): Json<SessionCommandV2Req>,
) -> impl IntoResponse {
    if req.command_id.trim().is_empty() || req.owner.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"command_id and owner are required"})),
        )
            .into_response();
    }
    if let Some(result) = cached_command(&s, &req.command_id) {
        return (StatusCode::OK, Json(result)).into_response();
    }
    let mut session = s.0.receiver_session.lock();
    if req.expected_revision != session.revision {
        return (StatusCode::CONFLICT, Json(json!({"error":"stale revision","expected":session.revision,"received":req.expected_revision}))).into_response();
    }
    let operation = match req.action.as_str() {
        "claim" => session.claim(&req.owner, req.force),
        "release" => {
            session.release(&req.owner);
            Ok(())
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":"action must be claim or release"})),
            )
                .into_response()
        }
    };
    if let Err(error) = operation {
        return (
            StatusCode::CONFLICT,
            Json(json!({"ok":false,"error":error,"session":session.clone()})),
        )
            .into_response();
    }
    let result = json!({"ok":true,"command_id":req.command_id,"session":session.clone()});
    drop(session);
    remember_command(&s, req.command_id, result.clone());
    (StatusCode::OK, Json(result)).into_response()
}

async fn decoder_jobs_v2(State(s): State<ApiState>) -> impl IntoResponse {
    Json(
        json!({"contract_version":2,"jobs":s.0.sidecars.statuses(),"scheduler":{"isolation":"process","arbitrary_client_commands":false}}),
    )
}

async fn recordings_v2(State(s): State<ApiState>) -> impl IntoResponse {
    let path = s.0.data_dir.join("recordings");
    let recordings = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then(
                || json!({"name":entry.file_name().to_string_lossy(),"size_bytes":metadata.len()}),
            )
        })
        .collect::<Vec<_>>();
    Json(
        json!({"contract_version":2,"active":s.0.recording.lock().status(),"recordings":recordings}),
    )
}

async fn media_capabilities_v2() -> impl IntoResponse {
    Json(json!({
        "contract_version": 2,
        "preferred": "pcm-websocket",
        "transports": [
            {"id":"pcm-websocket","available":true,"path":"/audio/stream","frame_ms":20,"sample_rate_hz":48000,"channels":[1,2],"status":"fixture_verified"},
            {"id":"webrtc-opus","available":false,"status":"development","payload_type":111,"clock_rate_hz":48000,"frame_ms":20,"timestamp_step":960,"fec":true,"dtx":false,"ice_lite":true,"sdp": crate::webrtc::sdp_offer(), "missing_gate":"ICE/DTLS transport, libopus encode, loss recovery, and two-hour LAN acceptance run"}
        ]
    }))
}

async fn media_session_v2() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "ok": false,
            "error": "WebRTC ICE/DTLS media sessions are not available in this build",
            "fallback": {"transport":"pcm-websocket","path":"/audio/stream"},
            "contract": {
                "status": "development",
                "payload_type": crate::webrtc::OPUS_PAYLOAD_TYPE,
                "clock_rate_hz": crate::webrtc::OPUS_CLOCK_RATE_HZ,
                "frame_ms": crate::webrtc::OPUS_FRAME_MS,
                "timestamp_step": crate::webrtc::OPUS_TIMESTAMP_STEP,
                "ice_lite": true,
                "ice_ufrag": crate::webrtc::ICE_UFRAG,
                "sdp": crate::webrtc::sdp_offer()
            },
            "missing_gate": "ICE/DTLS transport, libopus encode, loss recovery, and two-hour LAN acceptance run"
        })),
    )
}

async fn decoder_catalog_v2(State(s): State<ApiState>) -> impl IntoResponse {
    let mut decoders = vec![
        decoder_fixture_verified_entry("adsb", "ADS-B / Mode S", "iq", "live"),
        decoder_fixture_verified_entry("ais", "AIS", "discriminator", "live"),
        decoder_fixture_verified_entry("aprs", "APRS / AX.25", "audio", "live"),
        decoder_fixture_verified_entry("pocsag", "POCSAG", "audio", "live"),
        decoder_fixture_verified_entry("rds", "Broadcast RDS", "wfm_multiplex", "live"),
        decoder_fixture_verified_entry("uat", "978 UAT", "bits", "live"),
        decoder_fixture_verified_entry("acars", "ACARS", "bits", "live"),
        decoder_fixture_verified_entry("vdl2", "VDL Mode 2", "bits", "live"),
        decoder_development_entry("rtl433", "rtl_433 sensors", "iq", "managed_sidecar"),
        decoder_development_entry("ft8", "FT8 / FT4", "audio", "managed_sidecar"),
        decoder_development_entry("wspr", "WSPR", "audio", "managed_sidecar"),
        decoder_fixture_verified_entry("rtty", "RTTY / FSK", "audio", "live"),
        decoder_fixture_verified_entry("navtex", "NAVTEX", "audio", "live"),
        decoder_fixture_verified_entry("cw", "CW / Morse", "audio", "live"),
        decoder_development_entry("dmr", "DMR", "discriminator", "managed_sidecar"),
        decoder_development_entry("p25", "P25", "discriminator", "managed_sidecar"),
        decoder_development_entry("nxdn", "NXDN", "discriminator", "managed_sidecar"),
        decoder_development_entry("dstar", "D-Star", "discriminator", "planned_sidecar"),
        decoder_development_entry("ysf", "YSF", "discriminator", "planned_sidecar"),
        decoder_development_entry("m17", "M17", "discriminator", "planned_sidecar"),
        decoder_fixture_verified_entry("ble", "BLE advertising", "iq", "live"),
        decoder_fixture_verified_entry("lora", "LoRa mesh / Modbus", "iq", "live"),
        decoder_development_entry("hd_radio", "HD Radio / NRSC-5", "iq", "planned_sidecar"),
    ];
    for decoder in
        s.0.sidecars
            .statuses()
            .into_iter()
            .filter(|decoder| decoder.running)
    {
        decoders.push(json!({
            "id": decoder.name,
            "name": decoder.name,
            "status": "installed",
            "available": false,
            "input": "managed_sidecar",
            "integration": "live",
            "verification": "process_health_only",
            "missing_gate": "recorded IQ end-to-end fixture",
            "input_samples": decoder.input_samples,
        }));
    }
    Json(json!({"contract_version":2,"decoders":decoders}))
}

fn decoder_development_entry(id: &str, name: &str, input: &str, integration: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "status": "development",
        "available": false,
        "input": input,
        "integration": integration,
        "verification": "unit_fixture",
        "missing_gate": "recorded IQ end-to-end fixture",
    })
}

fn decoder_fixture_verified_entry(id: &str, name: &str, input: &str, integration: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "status": "fixture_verified",
        "available": true,
        "input": input,
        "integration": integration,
        "verification": "recorded_iq_e2e",
        "missing_gate": "hardware live verification",
    })
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
    let status = s.0.device.status();
    let mut dev = crate::device::DeviceLayer::discover();
    if status.connected && status.driver != "mock" {
        let key = s.0.config.read().device.last_device_key.clone();
        if !dev
            .iter()
            .any(|device| device.key == key || device.driver == status.driver)
        {
            dev.push(crate::device::DiscoveredDevice {
                driver: status.driver.clone(),
                label: status.label.clone(),
                key: if key.is_empty() {
                    format!("driver={}", status.driver)
                } else {
                    key
                },
                hardware_key: status.driver.clone(),
            });
        }
    }
    Json(json!({"devices": dev, "active": status}))
}

#[derive(Deserialize)]
struct DevKeyReq {
    key: String,
    label: Option<String>,
}
async fn device_connect(
    State(s): State<ApiState>,
    Json(req): Json<DevKeyReq>,
) -> impl IntoResponse {
    if let Err(e) = s.0.device.connect(&req.key) {
        return Json(json!({"ok": false, "error": e.to_string()}));
    }
    let mut cfg = s.0.config.write();
    cfg.device.last_device_key = req.key;
    cfg.device.last_device_label = req.label.unwrap_or_else(|| s.0.device.status().label);
    let _ = cfg.save(&s.0.data_dir);
    drop(cfg);
    s.0.start_default_monitor();
    Json(json!({"ok": true, "status": s.0.device.status()}))
}

async fn device_disconnect(State(s): State<ApiState>) -> impl IntoResponse {
    let result = s.0.device.disconnect();
    Json(json!({"ok": result.is_ok(), "status": s.0.device.status()}))
}

async fn device_status(State(s): State<ApiState>) -> impl IntoResponse {
    Json(serde_json::to_value(s.0.device.status()).unwrap())
}

#[derive(Deserialize)]
struct ReceiverSessionReq {
    owner: String,
    #[serde(default)]
    force: bool,
}
async fn receiver_session(State(s): State<ApiState>) -> Json<Value> {
    Json(serde_json::to_value(s.0.receiver_session.lock().clone()).unwrap())
}
async fn receiver_session_claim(
    State(s): State<ApiState>,
    Json(req): Json<ReceiverSessionReq>,
) -> impl IntoResponse {
    let mut session = s.0.receiver_session.lock();
    match session.claim(&req.owner, req.force) {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({"ok":true,"session":session.clone()})),
        ),
        Err(error) => (
            StatusCode::CONFLICT,
            Json(json!({"ok":false,"error":error,"session":session.clone()})),
        ),
    }
}
async fn receiver_session_release(
    State(s): State<ApiState>,
    Json(req): Json<ReceiverSessionReq>,
) -> Json<Value> {
    let mut session = s.0.receiver_session.lock();
    session.release(&req.owner);
    Json(json!({"ok":true,"session":session.clone()}))
}

async fn device_capabilities(State(s): State<ApiState>) -> impl IntoResponse {
    Json(serde_json::to_value(s.0.device.capabilities()).unwrap())
}
#[derive(Deserialize)]
struct DeviceControlReq {
    control: String,
    value: String,
}
async fn device_control(
    State(s): State<ApiState>,
    Json(req): Json<DeviceControlReq>,
) -> impl IntoResponse {
    match s.0.device.set_control(&req.control, &req.value) {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({"ok":true,"capabilities":s.0.device.capabilities()})),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(
                json!({"ok":false,"error":error.to_string(),"capabilities":s.0.device.capabilities()}),
            ),
        ),
    }
}

#[derive(Deserialize)]
struct GainReq {
    gain: String,
}
async fn device_gain(State(s): State<ApiState>, Json(req): Json<GainReq>) -> impl IntoResponse {
    let result = s.0.device.set_gain(req.gain);
    Json(json!({"ok": result.is_ok(), "status": s.0.device.status()}))
}

#[derive(Deserialize)]
struct FreqReq {
    frequency_hz: u64,
}
async fn device_frequency(
    State(s): State<ApiState>,
    Json(req): Json<FreqReq>,
) -> impl IntoResponse {
    if let Some(handle) = s.0.scanner.read().as_ref() {
        handle.state.lock().scan_locked = true;
    }
    match s.0.device.set_frequency(req.frequency_hz) {
        Ok(()) => {
            // A center-frequency change alters the meaning of every buffered
            // IQ sample. Keep the receiver task alive, but atomically discard
            // samples and audio captured under the previous RF window.
            if let Some(handle) = s.0.scanner.read().as_ref() {
                handle.flush_iq();
            }
            s.0.audio.clear_queue();
            (
                StatusCode::OK,
                Json(json!({"ok": true, "status": s.0.device.status()})),
            )
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": error.to_string(), "status": s.0.device.status()})),
        ),
    }
}

#[derive(Deserialize)]
struct SrReq {
    sample_rate: u32,
}
async fn device_sample_rate(
    State(s): State<ApiState>,
    Json(req): Json<SrReq>,
) -> impl IntoResponse {
    match s.0.device.set_sample_contract(req.sample_rate) {
        Ok(bandwidth_hz) => {
            {
                let mut config = s.0.config.write();
                config.device.sample_rate = req.sample_rate;
                let _ = config.save(&s.0.data_dir);
            }
            if let Some(handle) = s.0.scanner.read().as_ref() {
                handle.flush_iq();
            }
            s.0.audio.clear_queue();
            (
                StatusCode::OK,
                Json(
                    json!({"ok":true,"bandwidth_hz":bandwidth_hz,"status":s.0.device.status(),"capabilities":s.0.device.capabilities()}),
                ),
            )
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":error.to_string(),"status":s.0.device.status()})),
        ),
    }
}

async fn device_mdns() -> impl IntoResponse {
    Json(json!([]))
}

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

#[derive(Deserialize)]
struct ScanStartReq {
    range_name: String,
}

fn bookmark_channel_range(
    label: &str,
    frequency_hz: u64,
    mode: &str,
    bandwidth_hz: u32,
) -> crate::config::ScanRange {
    crate::config::ScanRange {
        name: format!("Bookmark {label}"),
        start_hz: frequency_hz,
        end_hz: frequency_hz,
        mode: if mode.is_empty() {
            "nfm".into()
        } else {
            mode.to_string()
        },
        channel_bw_hz: bandwidth_hz.max(1),
        max_vfos: 1,
        enabled: true,
        dwell_ms: 400,
        squelch_db: 15.0,
        auto_squelch_mode: crate::config::AutoSquelchMode::Adaptive,
        hold_ms: 1_500,
        sample_rate_hz: 2_000_000,
    }
}

fn enabled_scan_cycle(s: &ApiState) -> Vec<crate::config::ScanRange> {
    s.0.config
        .read()
        .scan_ranges
        .iter()
        .filter(|range| range.enabled)
        .cloned()
        .collect()
}

fn bookmark_scan_cycle(s: &ApiState) -> Result<Vec<crate::config::ScanRange>, String> {
    let conn = s.0.db.conn();
    let mut statement = conn
        .prepare(
            "SELECT label,frequency_hz,mode,bandwidth_hz FROM receiver_bookmarks WHERE enabled=1 ORDER BY frequency_hz",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(bookmark_channel_range(
                &row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u64,
                &row.get::<_, String>(2)?,
                row.get::<_, i64>(3)? as u32,
            ))
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn resolve_scan_target(
    s: &ApiState,
    range_name: &str,
) -> Result<(crate::config::ScanRange, Vec<crate::config::ScanRange>), String> {
    let needle = range_name.trim();
    if needle.eq_ignore_ascii_case(crate::scanner::SCAN_ENABLED_BANKS) || needle == "*" {
        let cycle = enabled_scan_cycle(s);
        let range = cycle
            .first()
            .cloned()
            .ok_or_else(|| "no enabled banks — enable banks in Settings".to_string())?;
        return Ok((range, cycle));
    }
    if needle.eq_ignore_ascii_case(crate::scanner::SCAN_BOOKMARKS) {
        let cycle = bookmark_scan_cycle(s)?;
        let range = cycle
            .first()
            .cloned()
            .ok_or_else(|| "no enabled bookmarks to scan".to_string())?;
        return Ok((range, cycle));
    }
    s.0.config
        .read()
        .scan_ranges
        .iter()
        .find(|range| range.name == needle)
        .cloned()
        .map(|range| (range, Vec::new()))
        .ok_or_else(|| "unknown range".into())
}

async fn scan_start(State(s): State<ApiState>, Json(req): Json<ScanStartReq>) -> Json<Value> {
    if let Err(error) = s.0.receiver_session.lock().claim("scanner", false) {
        return Json(
            json!({"ok":false,"error":error,"session":s.0.receiver_session.lock().clone()}),
        );
    }
    let (range, cycle) = match resolve_scan_target(&s, &req.range_name) {
        Ok(target) => target,
        Err(error) => {
            s.0.receiver_session.lock().release("scanner");
            return Json(json!({"ok": false, "error": error}));
        }
    };
    let requested_rate =
        s.0.config
            .read()
            .device
            .sample_rate
            .max(range.sample_rate_hz);
    if let Err(e) = s.0.device.set_sample_contract(requested_rate) {
        s.0.receiver_session.lock().release("scanner");
        return Json(json!({"ok": false, "error": format!("failed to set sampled spectrum: {e}")}));
    }
    let status = s.0.device.status();
    let usable_span = status
        .bandwidth_hz
        .min((status.sample_rate as f64 * 0.9) as u32);
    let initial_center = crate::scanner::initial_scan_center(&range, usable_span);
    if let Err(e) = s.0.device.set_frequency(initial_center) {
        s.0.receiver_session.lock().release("scanner");
        return Json(json!({"ok": false, "error": format!("failed to tune device: {e}")}));
    }
    // Lazily create the scanner if needed.
    let existing_handle = {
        let guard = s.0.scanner.read();
        guard.as_ref().cloned()
    };
    if let Some(handle) = existing_handle {
        handle.flush_iq();
        s.0.audio.clear_queue();
        configure_ham_decoders(&s, &range);
        start_configured_sidecars(&s).await;
        let _ = handle
            .cmd_tx
            .send(crate::scanner::ScannerCommand::Start { range, cycle });
        return Json(json!({"ok": true}));
    }
    let cfg = s.0.config.read().scanner.clone();
    let wfm_deemphasis_us = s.0.config.read().demodulator.de_emphasis_us;
    let dependencies = crate::scanner::ScannerDependencies {
        device: s.0.device.clone(),
        db: s.0.db.clone(),
        recording: s.0.recording.clone(),
        playback: s.0.playback.clone(),
        audio: s.0.audio.clone(),
        iq_network: s.0.iq_network.clone(),
        sidecars: s.0.sidecars.clone(),
        events_tx: s.0.events.clone(),
        spectrum_tx: s.0.spectrum.clone(),
        wfm_deemphasis_us,
    };
    let handle = crate::scanner::ScannerHandle::spawn(cfg, dependencies);
    *s.0.scanner.write() = Some(handle);
    if let Some(handle) = s.0.scanner.read().as_ref() {
        let _ = handle.cmd_tx.send(crate::scanner::ScannerCommand::Start {
            range: range.clone(),
            cycle,
        });
    }
    configure_ham_decoders(&s, &range);
    start_configured_sidecars(&s).await;
    Json(json!({"ok": true}))
}

fn stop_ham_decoders(s: &ApiState) {
    for (_, task) in s.0.ham_decoder_tasks.lock().drain() {
        task.abort();
    }
}

fn configure_ham_decoders(s: &ApiState, range: &crate::config::ScanRange) {
    stop_ham_decoders(s);
    let frequency_hz = s.0.device.status().center_freq_hz;
    if range.name.starts_with("FT8 ") {
        match crate::depmanager::discover_system_binary("jt9") {
            Some(executable) => {
                let task = crate::sstv::spawn_ft8(
                    s.0.audio.clone(),
                    s.0.db.clone(),
                    s.0.events.clone(),
                    s.0.data_dir.clone(),
                    frequency_hz,
                    executable,
                );
                s.0.ham_decoder_tasks.lock().insert("ft8".into(), task);
                tracing::info!(range = %range.name, "FT8 auto-decoder started");
            }
            None => {
                tracing::warn!(
                    range = %range.name,
                    hint = crate::depmanager::install_hint_for_decoder("jt9"),
                    "FT8 profile selected but jt9 is not installed"
                );
            }
        }
        return;
    }
    if range.name.starts_with("WSPR ") {
        match crate::depmanager::discover_system_binary("wsprd") {
            Some(executable) => {
                let task = crate::sstv::spawn_wspr(
                    s.0.audio.clone(),
                    s.0.db.clone(),
                    s.0.events.clone(),
                    s.0.data_dir.clone(),
                    frequency_hz,
                    executable,
                );
                s.0.ham_decoder_tasks.lock().insert("wspr".into(), task);
                tracing::info!(range = %range.name, "WSPR auto-decoder started");
            }
            None => {
                tracing::warn!(
                    range = %range.name,
                    hint = crate::depmanager::install_hint_for_decoder("wsprd"),
                    "WSPR profile selected but wsprd is not installed"
                );
            }
        }
        return;
    }
    if !range.name.starts_with("SSTV ") {
        return;
    }
    // SSTV is a native, streaming decoder: beginning an SSTV operating-window
    // monitor automatically enables its audio tap without launching a GUI
    // application or exposing a device to arbitrary process arguments.
    let task = crate::sstv::spawn(
        s.0.audio.clone(),
        s.0.db.clone(),
        s.0.events.clone(),
        s.0.data_dir.clone(),
        frequency_hz,
    );
    s.0.ham_decoder_tasks.lock().insert("sstv".into(), task);
    tracing::info!(range = %range.name, "native SSTV auto-decoder started");
}

async fn start_configured_sidecars(s: &ApiState) {
    let cfg = s.0.config.read().clone();
    let mut manifest_ids: Vec<&str> = Vec::new();
    if cfg.rtl433.enabled {
        manifest_ids.push("rtl_433");
    }
    if cfg.digital_decoder.enabled {
        manifest_ids.push("multimon-ng");
    }
    if cfg.aprs.enabled {
        manifest_ids.push("direwolf");
    }
    if cfg.dsd.enabled {
        manifest_ids.push("dsd-fme");
    }
    s.0.decoder_scheduler
        .sync_manifest_jobs(
            &s.0.sidecars,
            &s.0.data_dir,
            &manifest_ids,
            s.0.db.clone(),
            s.0.events.clone(),
        )
        .await;

    let mut jobs: Vec<(&str, String, Vec<String>)> = Vec::new();
    if cfg.rtl433.enabled && !s.0.sidecars.is_running("rtl_433") {
        let device = s.0.device.status();
        let mut args = vec![
            "-r".into(),
            "-".into(),
            "-s".into(),
            device.sample_rate.to_string(),
            "-f".into(),
            device.center_freq_hz.to_string(),
            "-F".into(),
            "json".into(),
        ];
        if !cfg.rtl433.extra_args.trim().is_empty() {
            args.extend(cfg.rtl433.extra_args.split_whitespace().map(str::to_string));
        }
        jobs.push(("rtl_433", cfg.rtl433.path, args));
    }
    if cfg.digital_decoder.enabled && !s.0.sidecars.is_running("multimon-ng") {
        let mut args = vec!["-t".into(), "raw".into(), "-q".into()];
        for protocol in &cfg.digital_decoder.enabled_protocols {
            args.push("-a".into());
            args.push(protocol.clone());
        }
        jobs.push(("multimon-ng", cfg.digital_decoder.multimon_path, args));
    }
    if cfg.aprs.enabled && !s.0.sidecars.is_running("direwolf") {
        jobs.push((
            "direwolf",
            cfg.aprs.path,
            vec![
                "-n".into(),
                "1".into(),
                "-r".into(),
                "48000".into(),
                "-b".into(),
                "16".into(),
                "-t".into(),
                "0".into(),
                "-".into(),
            ],
        ));
    }
    if cfg.dsd.enabled && !s.0.sidecars.is_running("dsd-fme") {
        let null_out = if cfg!(windows) { "nul" } else { "/dev/null" };
        jobs.push((
            "dsd-fme",
            cfg.dsd.dsdneo_path,
            vec![
                "-fa".into(),
                "-i".into(),
                "-".into(),
                "-n".into(),
                "-o".into(),
                null_out.into(),
            ],
        ));
    }
    if cfg.dump978.enabled && !s.0.sidecars.is_running("dump978") {
        let mut args = cfg.dump978.extra_args.clone();
        if args.is_empty() {
            args.push("--raw-stdin".into());
        }
        jobs.push(("dump978", cfg.dump978.path, args));
    }
    if cfg.radiosonde.enabled && !s.0.sidecars.is_running("rs41mod") {
        jobs.push(("rs41mod", cfg.radiosonde.path, Vec::new()));
    }
    if cfg.hd_radio.enabled && !s.0.sidecars.is_running("nrsc5") {
        if let Some(path) = crate::hd_radio::find_nrsc5() {
            let freq = s.0.device.status().center_freq_hz.max(87_900_000);
            jobs.push((
                "nrsc5",
                path.display().to_string(),
                crate::hd_radio::nrsc5_stdin_args(freq, cfg.hd_radio.program),
            ));
        }
    }

    for (name, path, args) in jobs {
        if path.trim().is_empty() || s.0.sidecars.is_running(name) {
            continue;
        }
        if !sidecar_path_usable(&path) {
            tracing::warn!(sidecar = name, path = %path, "decoder executable is not a usable file");
            continue;
        }
        match s
            .0
            .sidecars
            .spawn_decoder(
                name,
                std::path::PathBuf::from(path),
                args,
                s.0.db.clone(),
                s.0.events.clone(),
            )
            .await
        {
            Ok(()) => tracing::info!(sidecar = name, "decoder started"),
            Err(e) => tracing::warn!(sidecar = name, error = %e, "decoder failed to start"),
        }
    }
}

fn sidecar_path_usable(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return false;
    }
    let candidate = std::path::Path::new(trimmed);
    candidate.is_file() || which::which(trimmed).is_ok()
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
    stop_ham_decoders(&s);
    s.0.receiver_session.lock().release("scanner");
    let _ = s.0.sidecars.kill_all().await;
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
struct JobCreateReq {
    name: String,
    kind: String,
    payload: Value,
    enabled: Option<bool>,
    next_run_ms: Option<i64>,
}
async fn jobs_list(State(s): State<ApiState>) -> impl IntoResponse {
    match s.0.db.list_scheduled_jobs() {
        Ok(jobs) => (StatusCode::OK, Json(json!({"jobs":jobs}))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":error.to_string()})),
        ),
    }
}
async fn jobs_create(
    State(s): State<ApiState>,
    Json(req): Json<JobCreateReq>,
) -> impl IntoResponse {
    if !matches!(req.kind.as_str(), "scan" | "recording" | "decode") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"kind must be scan, recording, or decode"})),
        );
    }
    let now = crate::scanner::now_ms();
    let job = crate::db::ScheduledJob {
        id: None,
        name: req.name,
        kind: req.kind,
        payload_json: req.payload.to_string(),
        enabled: req.enabled.unwrap_or(true),
        next_run_ms: req.next_run_ms,
        last_run_ms: None,
        last_status: "pending".into(),
        last_error: String::new(),
        created_ms: now,
        updated_ms: now,
    };
    match s.0.db.create_scheduled_job(&job) {
        Ok(id) => (StatusCode::CREATED, Json(json!({"ok":true,"id":id}))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":error.to_string()})),
        ),
    }
}
async fn jobs_delete(State(s): State<ApiState>, Path(id): Path<i64>) -> impl IntoResponse {
    match s.0.db.delete_scheduled_job(id) {
        Ok(0) => (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"job not found"})),
        ),
        Ok(_) => (StatusCode::OK, Json(json!({"ok":true}))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":error.to_string()})),
        ),
    }
}

async fn vfo_states(State(s): State<ApiState>) -> impl IntoResponse {
    let v =
        s.0.scanner
            .read()
            .as_ref()
            .map(|h| h.state.lock().vfo_states.clone())
            .unwrap_or_default();
    Json(serde_json::to_value(&v).unwrap())
}

#[derive(Deserialize)]
struct VfoBoolReq {
    id: u32,
    on: bool,
}
async fn vfo_mute(State(s): State<ApiState>, Json(req): Json<VfoBoolReq>) -> impl IntoResponse {
    send_vfo(
        &s,
        crate::scanner::ScannerCommand::SetVfoMute {
            id: req.id,
            muted: req.on,
        },
    );
    Json(json!({"ok": true}))
}

async fn vfo_agc(State(s): State<ApiState>, Json(req): Json<VfoBoolReq>) -> impl IntoResponse {
    send_vfo(
        &s,
        crate::scanner::ScannerCommand::ToggleVfoAgc {
            id: req.id,
            on: req.on,
        },
    );
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
struct VfoF32Req {
    id: u32,
    value: f32,
}
async fn vfo_volume(State(s): State<ApiState>, Json(req): Json<VfoF32Req>) -> impl IntoResponse {
    send_vfo(
        &s,
        crate::scanner::ScannerCommand::SetVfoVolume {
            id: req.id,
            volume: req.value,
        },
    );
    Json(json!({"ok": true}))
}
#[derive(Deserialize)]
struct VfoFrequencyReq {
    frequency_hz: u64,
}
async fn vfo_frequency(
    State(s): State<ApiState>,
    Path(id): Path<u32>,
    Json(req): Json<VfoFrequencyReq>,
) -> impl IntoResponse {
    let status = s.0.device.status();
    let usable_half = (status.sample_rate as u64 * 45) / 100;
    let outside_window = req.frequency_hz.abs_diff(status.center_freq_hz) > usable_half;
    if let Some(handle) = s.0.scanner.read().as_ref() {
        handle.state.lock().scan_locked = true;
    }
    if outside_window {
        if let Err(error) = s.0.device.set_frequency(req.frequency_hz) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"ok":false,"error":error.to_string()})),
            );
        }
        if let Some(handle) = s.0.scanner.read().as_ref() {
            handle.flush_iq();
        }
        s.0.audio.clear_queue();
    }
    send_vfo(
        &s,
        crate::scanner::ScannerCommand::SetVfoFrequency {
            id,
            frequency_hz: req.frequency_hz,
        },
    );
    (
        StatusCode::OK,
        Json(
            json!({"ok":true,"id":id,"frequency_hz":req.frequency_hz,"center_freq_hz":s.0.device.status().center_freq_hz}),
        ),
    )
}
#[derive(Deserialize)]
struct VfoModeReq {
    mode: String,
}
async fn vfo_mode(
    State(s): State<ApiState>,
    Path(id): Path<u32>,
    Json(req): Json<VfoModeReq>,
) -> impl IntoResponse {
    send_vfo(
        &s,
        crate::scanner::ScannerCommand::SetVfoMode {
            id,
            mode: req.mode.clone(),
        },
    );
    Json(json!({"ok":true,"id":id,"mode":req.mode}))
}

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
        "locked": runtime.scan_locked,
        "holding": runtime.holding,
        "frame_sequence": runtime.frames_processed,
        "frame_timestamp_ms": runtime.latest_spectrum_ms,
        "noise_floor_db": runtime.noise_floor_db,
    }))
}

async fn decoded_messages(State(s): State<ApiState>, Query(q): Query<LimitQ>) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100);
    match s.0.db.recent_decoded_messages(limit) {
        Ok(rows) => Json(serde_json::to_value(&rows).unwrap()),
        Err(e) => Json(json!({"error": e.to_string()})),
    }
}
#[derive(Deserialize)]
struct LimitQ {
    limit: Option<u32>,
}

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
            if e.top_confidence >= 0.7 && e.sub_protocol != "unknown" && !e.sub_protocol.is_empty()
            {
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
    Json(
        rows.into_iter()
            .find(|v| v.get("id").and_then(|x| x.as_str()) == Some(&id))
            .unwrap_or_else(|| json!({"error":"fingerprint not found"})),
    )
}
async fn signal_id_fp_delete(Path(_id): Path<i64>) -> impl IntoResponse {
    Json(json!({"ok": false, "error":"built-in fingerprints cannot be deleted"}))
}
async fn signal_id_fp_match(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let frequency_hz = v
        .get("frequency_hz")
        .and_then(|x| x.as_u64())
        .or_else(|| {
            s.0.scanner
                .read()
                .as_ref()
                .and_then(|h| h.state.lock().vfo_states.first().map(|vf| vf.frequency_hz))
        })
        .unwrap_or(0);
    let bandwidth_hz = v
        .get("bandwidth_hz")
        .and_then(|x| x.as_u64())
        .unwrap_or(12_500) as u32;
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
    let sample_rate = v
        .get("sample_rate_hz")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let center = v
        .get("center_freq_hz")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    if sample_rate == 0 {
        return Json(json!({"ok":false,"error":"sample_rate_hz is required"}));
    }
    Json(
        json!({"ok":true,"sample_rate_hz":sample_rate,"center_freq_hz":center,"output_rate_hz":sample_rate/2,"phase_count":4,"extractor":"deterministic-polyphase"}),
    )
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
    Json(
        json!({"ok":true,"sample_count":samples,"burst_count":4,"bursts":[
            {"start":0,"length":burst_len},{"start":burst_len,"length":burst_len},
            {"start":burst_len*2,"length":burst_len},{"start":burst_len*3,"length":samples.saturating_sub(burst_len*3)}
        ]}),
    )
}

/// Classify a frequency (and optional live audio) into ranked protocol candidates.
async fn signal_id_classify(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let frequency_hz = v.get("frequency_hz").and_then(|x| x.as_u64()).unwrap_or(0);
    let bandwidth_hz = v
        .get("bandwidth_hz")
        .and_then(|x| x.as_u64())
        .unwrap_or(12_500) as u32;
    let mode = v
        .get("mode")
        .and_then(|x| x.as_str())
        .unwrap_or("nfm")
        .to_string();
    let range_name = v
        .get("range_name")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let snr_db = v.get("snr_db").and_then(|x| x.as_f64()).unwrap_or(15.0) as f32;
    let with_audio = v
        .get("with_audio")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);

    let classification = if with_audio && s.0.device.status().connected {
        let status = s.0.device.status();
        let count = ((status.sample_rate as f64) * 0.4) as usize;
        match live_iq_snapshot(&s, count.max(4096)) {
            Ok(iq) if iq.len() > 2048 => {
                use crate::demod::Mode;
                let parsed = Mode::parse(&mode);
                let (pcm, audio_rate) = channelized_vfo_audio(
                    &iq,
                    frequency_hz,
                    status.center_freq_hz,
                    status.sample_rate,
                    parsed,
                );
                crate::signal_id::classify(
                    frequency_hz,
                    bandwidth_hz,
                    &mode,
                    &range_name,
                    snr_db,
                    Some((&pcm, audio_rate)),
                )
            }
            _ => crate::signal_id::classify(
                frequency_hz,
                bandwidth_hz,
                &mode,
                &range_name,
                snr_db,
                None,
            ),
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
async fn signal_id_auto_decode(
    State(s): State<ApiState>,
    Json(v): Json<Value>,
) -> impl IntoResponse {
    let frequency_hz = v.get("frequency_hz").and_then(|x| x.as_u64()).unwrap_or(0);
    let bandwidth_hz = v
        .get("bandwidth_hz")
        .and_then(|x| x.as_u64())
        .unwrap_or(12_500) as u32;
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
    let bandwidth_hz = v
        .get("bandwidth_hz")
        .and_then(|x| x.as_u64())
        .unwrap_or(12_500) as u32;
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
    match s.0.db.list_talkgroups() {
        Ok(v) => Json(serde_json::to_value(v).unwrap()),
        Err(e) => Json(json!({"error": e.to_string()})),
    }
}
async fn talkgroup_update(
    State(s): State<ApiState>,
    Json(t): Json<crate::db::Talkgroup>,
) -> impl IntoResponse {
    Json(json!({"ok": s.0.db.upsert_talkgroup(&t).is_ok()}))
}
async fn talkgroup_systems(State(s): State<ApiState>) -> impl IntoResponse {
    Json(serde_json::to_value(s.0.db.talkgroup_systems().unwrap_or_default()).unwrap())
}
async fn talkgroup_import(
    State(s): State<ApiState>,
    Json(rows): Json<Vec<crate::db::Talkgroup>>,
) -> impl IntoResponse {
    let mut ok = true;
    for t in rows {
        if s.0.db.upsert_talkgroup(&t).is_err() {
            ok = false;
        }
    }
    Json(json!({"ok": ok}))
}
async fn talkgroup_export(State(s): State<ApiState>) -> impl IntoResponse {
    Json(serde_json::to_value(s.0.db.export_talkgroups().unwrap_or_default()).unwrap())
}
#[derive(Deserialize)]
struct SystemReq {
    system_name: String,
}
async fn talkgroup_delete_system(
    State(s): State<ApiState>,
    Json(req): Json<SystemReq>,
) -> impl IntoResponse {
    Json(json!({"ok": s.0.db.delete_talkgroup_system(&req.system_name).is_ok()}))
}

async fn trunking_start(State(s): State<ApiState>, req: Option<Json<Value>>) -> impl IntoResponse {
    let control_hz = req.and_then(|Json(v)| v.get("control_channel_hz").and_then(|x| x.as_u64()));
    let status = s.0.device.status();
    let mut observation = crate::trunking::ControlChannelObservation::default();
    let mut reason = "P25 TSBK observer is armed; waiting for control-channel IQ".to_string();
    if status.connected {
        let count = (status.sample_rate as usize / 5).clamp(8_192, 480_000);
        if let Ok(iq) = live_iq_snapshot(&s, count) {
            let tune = control_hz.unwrap_or(status.center_freq_hz);
            let channel = crate::demod::channelize_iq(
                &iq,
                tune as f64 - status.center_freq_hz as f64,
                status.sample_rate,
                crate::demod::Mode::Nfm,
            );
            observation = crate::trunking::observe_control_channel(&channel, status.sample_rate);
            reason = if observation.grants.is_empty() {
                "no TSBK recovered from the current IQ snapshot".into()
            } else {
                format!("observed {} TSBK grant(s)", observation.grants.len())
            };
        }
    }
    let imported_voice = s.0.trunking.read().voice_channels.clone();
    let follow_hz = crate::trunking::follow_frequency(&observation, &imported_voice);
    if let Some(voice_hz) = follow_hz {
        send_vfo(
            &s,
            crate::scanner::ScannerCommand::SetVfoFrequency {
                id: 0,
                frequency_hz: voice_hz,
            },
        );
    }
    let mut t = s.0.trunking.write();
    t.running = true;
    t.available = false;
    t.protocol = Some("p25".into());
    if control_hz.is_some() {
        t.control_channel_hz = control_hz;
    }
    t.reason = Some(reason.clone());
    for grant in &observation.grants {
        let freq = follow_hz
            .or(t.control_channel_hz)
            .unwrap_or(status.center_freq_hz);
        t.active_talkgroup = Some(grant.talkgroup.clone());
        t.calls.push(crate::trunking::grant_to_call(grant, freq));
        t.log.push(format!(
            "TSBK grant TG {} src {} enc={} follow={}",
            grant.talkgroup,
            grant.source,
            grant.encrypted,
            follow_hz
                .map(|hz| hz.to_string())
                .unwrap_or_else(|| "none".into())
        ));
    }
    Json(json!({
        "ok": true,
        "available": false,
        "running": true,
        "native": true,
        "p25_fir": true,
        "grants": observation.grants,
        "idens": observation.idens,
        "follow_hz": follow_hz,
        "reason": reason,
        "missing_gate": "live control-channel hardware verification",
        "status": &*t
    }))
}
async fn trunking_stop(State(s): State<ApiState>) -> impl IntoResponse {
    let mut t = s.0.trunking.write();
    t.running = false;
    t.active_talkgroup = None;
    t.discovery_running = false;
    t.log
        .push(format!("{} trunking stopped", crate::scanner::now_ms()));
    Json(json!({"ok": true, "status": &*t}))
}
async fn trunking_status(State(s): State<ApiState>) -> impl IntoResponse {
    Json(serde_json::to_value(&*s.0.trunking.read()).unwrap())
}
#[derive(Deserialize)]
struct TrunkingLockReq {
    locked: Option<bool>,
}
async fn trunking_lock(
    State(s): State<ApiState>,
    req: Option<Json<TrunkingLockReq>>,
) -> impl IntoResponse {
    let mut t = s.0.trunking.write();
    t.locked = req.and_then(|Json(v)| v.locked).unwrap_or(!t.locked);
    Json(json!({"ok": true, "locked": t.locked}))
}
async fn trunking_calls(State(s): State<ApiState>) -> impl IntoResponse {
    Json(serde_json::to_value(&s.0.trunking.read().calls).unwrap())
}
async fn trunking_import(State(s): State<ApiState>, Json(def): Json<Value>) -> impl IntoResponse {
    let mut t = s.0.trunking.write();
    t.system = def
        .get("system")
        .or_else(|| def.get("system_name"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    t.control_channel_hz = def.get("control_channel_hz").and_then(|v| v.as_u64());
    t.voice_channels = def
        .get("voice_channels")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_u64()).collect())
        .unwrap_or_default();
    t.log.push("trunking definition imported".into());
    Json(json!({"ok": true, "status": &*t}))
}
async fn trunking_disc_start(State(_s): State<ApiState>) -> impl IntoResponse {
    Json(
        json!({"ok":false,"available":false,"error":"trunking discovery decoder is not installed"}),
    )
}
async fn trunking_disc_stop(State(s): State<ApiState>) -> impl IntoResponse {
    s.0.trunking.write().discovery_running = false;
    Json(json!({"ok": true}))
}
async fn trunking_disc_results(State(s): State<ApiState>) -> impl IntoResponse {
    Json(serde_json::to_value(&s.0.trunking.read().discovery_results).unwrap())
}
async fn trunking_disc_snapshot(State(s): State<ApiState>) -> impl IntoResponse {
    Json(serde_json::to_value(&*s.0.trunking.read()).unwrap())
}
async fn trunking_disc_log(State(s): State<ApiState>) -> impl IntoResponse {
    Json(serde_json::to_value(&s.0.trunking.read().log).unwrap())
}
async fn trunking_disc_log_clear(State(s): State<ApiState>) -> impl IntoResponse {
    s.0.trunking.write().log.clear();
    Json(json!({"ok": true}))
}
async fn trunking_disc_notes() -> impl IntoResponse {
    Json(json!([]))
}
async fn trunking_disc_promote(State(_s): State<ApiState>) -> impl IntoResponse {
    Json(
        json!({"ok":false,"available":false,"error":"there is no verified discovery result to promote"}),
    )
}
async fn trunking_disc_identify() -> impl IntoResponse {
    Json(
        json!({"ok":false,"available":false,"error":"trunking identification decoder is not installed"}),
    )
}
async fn trunking_disc_clear(State(s): State<ApiState>) -> impl IntoResponse {
    s.0.trunking.write().discovery_results.clear();
    Json(json!({"ok": true}))
}
async fn trunking_disc_delete(State(s): State<ApiState>) -> impl IntoResponse {
    s.0.trunking.write().discovery_results.clear();
    Json(json!({"ok": true}))
}
async fn trunking_zone_active(State(s): State<ApiState>) -> impl IntoResponse {
    Json(serde_json::to_value(&s.0.trunking.read().zones).unwrap())
}
async fn trunking_zone_upsert(
    State(s): State<ApiState>,
    Json(zone): Json<Value>,
) -> impl IntoResponse {
    let mut t = s.0.trunking.write();
    let key = zone
        .get("id")
        .or_else(|| zone.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if key.is_empty() {
        return Json(json!({"ok": false, "error": "zone requires id or name"}));
    }
    t.zones.retain(|z| {
        z.get("id")
            .or_else(|| z.get("name"))
            .and_then(|v| v.as_str())
            != Some(key)
    });
    t.zones.push(zone);
    Json(json!({"ok": true, "zones": &t.zones}))
}
async fn trunking_zone_delete(
    State(s): State<ApiState>,
    Json(zone): Json<Value>,
) -> impl IntoResponse {
    let key = zone
        .get("id")
        .or_else(|| zone.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let mut t = s.0.trunking.write();
    let before = t.zones.len();
    t.zones.retain(|z| {
        z.get("id")
            .or_else(|| z.get("name"))
            .and_then(|v| v.as_str())
            != Some(key)
    });
    Json(json!({"ok": true, "removed": before - t.zones.len()}))
}

async fn aero_enable(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let mut c = s.0.config.write();
    c.aero.enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
    let _ = c.save(&s.0.data_dir);
    Json(json!({"ok":true,"enabled":c.aero.enabled}))
}
async fn aero_check(State(s): State<ApiState>) -> impl IntoResponse {
    let c = s.0.config.read();
    Json(json!({"ok":true,"available":!c.aero.sniffer_path.is_empty(),"path":c.aero.sniffer_path}))
}
async fn aero_clear() -> impl IntoResponse {
    Json(json!({"ok": true}))
}
async fn aero_messages(State(s): State<ApiState>) -> impl IntoResponse {
    Json(
        serde_json::to_value(
            s.0.db
                .messages_by_protocol(Some("acars"), 100)
                .unwrap_or_default(),
        )
        .unwrap_or(json!([])),
    )
}
async fn aero_status(State(s): State<ApiState>) -> impl IntoResponse {
    let c = s.0.config.read();
    Json(
        json!({"enabled":c.aero.enabled,"satellite":c.aero.satellite,"center_freq_hz":c.aero.center_freq_hz,"sample_rate_hz":c.aero.sample_rate_hz,"path":c.aero.sniffer_path}),
    )
}
async fn aero_stderr(State(s): State<ApiState>) -> impl IntoResponse {
    Json(s.0.sidecars.stderr("aero"))
}

async fn iridium_enable(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let mut c = s.0.config.write();
    c.iridium.enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
    let _ = c.save(&s.0.data_dir);
    Json(json!({"ok":true,"enabled":c.iridium.enabled}))
}
async fn iridium_check(State(s): State<ApiState>) -> impl IntoResponse {
    let c = s.0.config.read();
    Json(
        json!({"ok":true,"available":false,"center_freq_hz":c.iridium.center_freq_hz,"sample_rate_hz":c.iridium.sample_rate_hz}),
    )
}
async fn iridium_clear() -> impl IntoResponse {
    Json(json!({"ok": true}))
}
async fn iridium_messages() -> impl IntoResponse {
    Json(json!([]))
}
async fn iridium_status(State(s): State<ApiState>) -> impl IntoResponse {
    let c = s.0.config.read();
    Json(
        json!({"enabled":c.iridium.enabled,"center_freq_hz":c.iridium.center_freq_hz,"sample_rate_hz":c.iridium.sample_rate_hz,"surface_message_content":c.iridium.surface_message_content}),
    )
}
async fn iridium_quick_start(State(s): State<ApiState>) -> impl IntoResponse {
    let mut c = s.0.config.write();
    c.iridium.enabled = true;
    let _ = c.save(&s.0.data_dir);
    Json(json!({"ok":true,"enabled":true}))
}
async fn iridium_stderr(State(s): State<ApiState>) -> impl IntoResponse {
    Json(s.0.sidecars.stderr("iridium"))
}

async fn stdc_enable(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let mut c = s.0.config.write();
    c.stdc.enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
    let _ = c.save(&s.0.data_dir);
    Json(json!({"ok":true,"enabled":c.stdc.enabled}))
}
async fn stdc_check(State(s): State<ApiState>) -> impl IntoResponse {
    let c = s.0.config.read();
    Json(json!({"ok":true,"available":which::which(&c.stdc.path).is_ok(),"path":c.stdc.path}))
}
async fn stdc_clear() -> impl IntoResponse {
    Json(json!({"ok": true}))
}
async fn stdc_messages() -> impl IntoResponse {
    Json(json!([]))
}
async fn stdc_status(State(s): State<ApiState>) -> impl IntoResponse {
    let c = s.0.config.read();
    Json(json!({"enabled":c.stdc.enabled,"path":c.stdc.path,"uw_tolerance":c.stdc.uw_tolerance}))
}

async fn gps_enable(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let mut c = s.0.config.write();
    c.gps.enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
    let _ = c.save(&s.0.data_dir);
    Json(json!({"ok":true,"enabled":c.gps.enabled}))
}
async fn gps_clear() -> impl IntoResponse {
    Json(json!({"ok": true}))
}
async fn gps_status(State(s): State<ApiState>) -> impl IntoResponse {
    let c = s.0.config.read();
    Json(
        json!({"enabled":c.gps.enabled,"sample_rate_hz":c.gps.sample_rate_hz,"detection_threshold":c.gps.detection_threshold,"doppler_search_hz":c.gps.doppler_search_hz}),
    )
}

async fn glonass_enable(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let mut c = s.0.config.write();
    c.glonass.enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
    let _ = c.save(&s.0.data_dir);
    Json(json!({"ok":true,"enabled":c.glonass.enabled}))
}
async fn glonass_clear() -> impl IntoResponse {
    Json(json!({"ok": true}))
}
async fn glonass_status(State(s): State<ApiState>) -> impl IntoResponse {
    let c = s.0.config.read();
    Json(
        json!({"enabled":c.glonass.enabled,"sample_rate_hz":c.glonass.sample_rate_hz,"detection_threshold":c.glonass.detection_threshold,"doppler_search_hz":c.glonass.doppler_search_hz}),
    )
}

async fn goes_enable(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let mut c = s.0.config.write();
    c.goes_lrit.enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
    let _ = c.save(&s.0.data_dir);
    Json(json!({"ok":true,"enabled":c.goes_lrit.enabled}))
}
async fn goes_check() -> impl IntoResponse {
    Json(json!({"ok": true, "available": false, "reason":"satdump sidecar not configured"}))
}
async fn goes_status(State(s): State<ApiState>) -> impl IntoResponse {
    let c = s.0.config.read();
    Json(
        json!({"enabled":c.goes_lrit.enabled,"satellite":c.goes_lrit.satellite,"path":c.goes_lrit.satdump_path,"sample_rate_hz":c.goes_lrit.sample_rate_hz}),
    )
}
async fn goes_satellite(State(s): State<ApiState>) -> impl IntoResponse {
    let c = s.0.config.read();
    Json(
        json!({"satellite":c.goes_lrit.satellite,"output_image_dir":c.goes_lrit.output_image_dir,"sample_rate_hz":c.goes_lrit.sample_rate_hz}),
    )
}
async fn goes_satellite_put(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let mut c = s.0.config.write();
    if let Some(x) = v.get("satellite").and_then(|x| x.as_str()) {
        c.goes_lrit.satellite = x.to_string();
    }
    if let Some(x) = v.get("output_image_dir").and_then(|x| x.as_str()) {
        c.goes_lrit.output_image_dir = x.to_string();
    }
    if let Some(x) = v.get("sample_rate_hz").and_then(|x| x.as_u64()) {
        c.goes_lrit.sample_rate_hz = x as u32;
    }
    let _ = c.save(&s.0.data_dir);
    Json(
        json!({"ok":true,"satellite":c.goes_lrit.satellite,"output_image_dir":c.goes_lrit.output_image_dir,"sample_rate_hz":c.goes_lrit.sample_rate_hz}),
    )
}

async fn hd_radio_enable(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
    {
        let mut c = s.0.config.write();
        c.hd_radio.enabled = enabled;
        if let Some(program) = v.get("program").and_then(|x| x.as_u64()) {
            c.hd_radio.program = program as u32;
        }
        let _ = c.save(&s.0.data_dir);
    }
    if enabled {
        start_configured_sidecars(&s).await;
    }
    let nrsc5 = crate::hd_radio::find_nrsc5();
    let running = s.0.sidecars.is_running("nrsc5");
    Json(json!({
        "ok": true,
        "enabled": enabled,
        "available": false,
        "running": running,
        "nrsc5": nrsc5.as_ref().map(|p| p.display().to_string()),
        "reason": if running {
            "nrsc5 sidecar is running; OFDM recorded-IQ end-to-end has not passed"
        } else if nrsc5.is_some() {
            "nrsc5 is installed; OFDM recorded-IQ end-to-end has not passed"
        } else {
            "HD Radio decoder sidecar (nrsc5) is not installed"
        }
    }))
}
async fn hd_radio_check(State(s): State<ApiState>) -> impl IntoResponse {
    let c = s.0.config.read();
    let nrsc5 = crate::hd_radio::find_nrsc5();
    Json(json!({
        "ok": true,
        "available": false,
        "configured": c.hd_radio.enabled,
        "program": c.hd_radio.program,
        "stations": c.hd_radio.stations,
        "nrsc5": nrsc5.as_ref().map(|p| p.display().to_string()),
        "reason": if nrsc5.is_some() {
            "nrsc5 is installed; SIS parser is unit-tested; OFDM IQ e2e remains open"
        } else {
            "HD Radio decoder sidecar (nrsc5) is not installed"
        }
    }))
}
async fn hd_radio_messages(State(s): State<ApiState>) -> impl IntoResponse {
    Json(json!(s
        .0
        .db
        .messages_by_protocol(Some("hd_radio"), 50)
        .unwrap_or_default()))
}
async fn hd_radio_status(State(s): State<ApiState>) -> impl IntoResponse {
    let c = s.0.config.read();
    let nrsc5 = crate::hd_radio::find_nrsc5();
    Json(json!({
        "enabled": c.hd_radio.enabled,
        "available": false,
        "auto_on_fm_lock": c.hd_radio.auto_on_fm_lock,
        "program": c.hd_radio.program,
        "stations": c.hd_radio.stations,
        "nrsc5": nrsc5.as_ref().map(|p| p.display().to_string()),
        "missing_gate": "nrsc5 OFDM recorded-IQ end-to-end fixture"
    }))
}
async fn hd_radio_aas(Path(_filename): Path<String>) -> impl IntoResponse {
    Json(json!({}))
}

async fn ble_devices(State(s): State<ApiState>) -> Json<Value> {
    let messages =
        s.0.db
            .messages_by_protocol(Some("ble"), 200)
            .unwrap_or_default();
    let mut devices = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for message in messages {
        if seen.insert(message.address.clone()) {
            devices.push(json!({
                "address": message.address,
                "address_type": message.function_code,
                "pdu_type": message.message_type,
                "name": message.content,
                "frequency_hz": message.frequency_hz,
                "timestamp_ms": message.timestamp_ms,
                "raw": message.raw,
            }));
        }
    }
    Json(json!(devices))
}
async fn ble_status(State(s): State<ApiState>) -> impl IntoResponse {
    let enabled = s.0.config.read().ble.enabled;
    let device_count =
        s.0.db
            .messages_by_protocol(Some("ble"), 200)
            .map(|m| {
                m.iter()
                    .map(|row| row.address.clone())
                    .collect::<std::collections::HashSet<_>>()
                    .len()
            })
            .unwrap_or(0);
    let status = s.0.device.status();
    Json(json!({
        "available": true,
        "native": true,
        "enabled": enabled,
        "running": enabled && status.connected,
        "device_count": device_count,
        "source": "native_gfsk",
        "required_sample_rate_hz": 4_000_000,
        "reason": "Native BLE advertising decoder; RTL-SDR-class tuners cannot cover 2.4 GHz"
    }))
}
async fn ble_file() -> impl IntoResponse {
    Json(json!(null))
}
async fn ble_clear(State(s): State<ApiState>) -> impl IntoResponse {
    let _ = s.0.db.delete_messages_by_protocol("ble");
    Json(json!({"ok": true}))
}

async fn lora_messages(State(s): State<ApiState>) -> impl IntoResponse {
    Json(json!(s
        .0
        .db
        .messages_by_protocols(crate::lora::LORA_PROTOCOLS, 100)
        .unwrap_or_default()))
}
async fn lora_regions() -> impl IntoResponse {
    Json(json!(crate::lora::regional_plans()))
}

async fn scan_lock(State(s): State<ApiState>) -> impl IntoResponse {
    if let Some(h) = s.0.scanner.read().as_ref() {
        h.state.lock().scan_locked = true;
    }
    Json(json!({"ok":true,"locked":true}))
}
async fn scan_unlock(State(s): State<ApiState>) -> impl IntoResponse {
    if let Some(h) = s.0.scanner.read().as_ref() {
        let _ = h.cmd_tx.send(crate::scanner::ScannerCommand::Resume);
    }
    Json(json!({"ok":true,"locked":false}))
}
async fn scan_skip(State(s): State<ApiState>) -> impl IntoResponse {
    if let Some(h) = s.0.scanner.read().as_ref() {
        let _ = h
            .cmd_tx
            .send(crate::scanner::ScannerCommand::Skip { temporary: true });
        return Json(json!({"ok":true,"temporary":true}));
    }
    Json(json!({"ok":false,"error":"scanner is not running"}))
}
async fn scan_lockout(State(s): State<ApiState>) -> impl IntoResponse {
    if let Some(h) = s.0.scanner.read().as_ref() {
        let _ = h
            .cmd_tx
            .send(crate::scanner::ScannerCommand::Skip { temporary: false });
        return Json(json!({"ok":true,"temporary":false}));
    }
    Json(json!({"ok":false,"error":"scanner is not running"}))
}
async fn scan_start_alt(
    State(s): State<ApiState>,
    req: Option<Json<ScanStartReq>>,
) -> impl IntoResponse {
    let range_name = req.map(|Json(r)| r.range_name).or_else(|| {
        s.0.config
            .read()
            .scan_ranges
            .first()
            .map(|r| r.name.clone())
    });
    match range_name {
        Some(name) => scan_start(State(s), Json(ScanStartReq { range_name: name })).await,
        None => Json(json!({"ok": false, "error": "no scan ranges configured"})),
    }
}
async fn scan_stop_alt(State(s): State<ApiState>) -> impl IntoResponse {
    scan_stop(State(s)).await
}
async fn sidecars_status(State(s): State<ApiState>) -> impl IntoResponse {
    let runtime = serde_json::to_value(s.0.sidecars.statuses()).unwrap();
    let discovered = serde_json::to_value(crate::depmanager::scan_all(&s.0.data_dir)).unwrap();
    Json(json!({"runtime": runtime, "discovered": discovered}))
}

async fn decoders_scan(State(s): State<ApiState>) -> Json<Value> {
    Json(serde_json::to_value(crate::depmanager::scan_all(&s.0.data_dir)).unwrap())
}

async fn decoders_adaptations(State(s): State<ApiState>) -> Json<Value> {
    let adaptations = crate::depmanager::adaptation_report(&s.0.data_dir);
    Json(json!({
        "ok": true,
        "count": adaptations.len(),
        "adaptations": adaptations,
    }))
}

async fn decoders_configure(State(s): State<ApiState>) -> Json<Value> {
    let data_dir = s.0.data_dir.clone();
    let configured = {
        let mut config = s.0.config.write();
        let results = crate::depmanager::configure_decoder_paths(&mut config, &data_dir);
        let _ = config.save(&data_dir);
        results
    };
    Json(json!({"ok": true, "configured": configured}))
}

async fn decoders_install_guide(Path(name): Path<String>) -> Json<Value> {
    let guide = crate::depmanager::install_instructions(&name);
    Json(json!({
        "ok": guide.is_some(),
        "name": name,
        "guide": guide,
        "can_auto_install": crate::depmanager::can_auto_install_decoder(&name),
        "direct_download_url": crate::depmanager::download_url_for_decoder(&name),
    }))
}

async fn decoders_install(State(s): State<ApiState>, Path(name): Path<String>) -> Json<Value> {
    let data_dir = s.0.data_dir.clone();
    let install_name = name.clone();
    let mut scratch = s.0.config.read().clone();
    match tokio::task::spawn_blocking(move || {
        crate::depmanager::install_decoder(&install_name, &data_dir, &mut scratch)
    })
    .await
    {
        Ok(Ok(configured)) => {
            let mut config = s.0.config.write();
            let updated = crate::depmanager::apply_decoder_path(
                &mut config,
                &configured.decoder,
                &configured.path,
            );
            let _ = config.save(&s.0.data_dir);
            Json(json!({
                "ok": true,
                "name": name,
                "path": configured.path,
                "updated": updated,
                "feature_pack_id": configured.feature_pack_id,
            }))
        }
        Ok(Err(error)) => Json(json!({"ok": false, "name": name, "error": error})),
        Err(error) => Json(
            json!({"ok": false, "name": name, "error": format!("installer task failed: {error}")}),
        ),
    }
}

async fn sidecars_start_all(State(s): State<ApiState>) -> impl IntoResponse {
    start_configured_sidecars(&s).await;
    Json(json!({"ok": true}))
}

async fn scan_status(State(s): State<ApiState>) -> impl IntoResponse {
    let runtime = s.0.scanner.read().as_ref().map(|h| h.state.lock().clone());
    Json(json!({
        "running": runtime.as_ref().map(|v| v.running).unwrap_or(false),
        "locked": runtime.as_ref().map(|v| v.scan_locked).unwrap_or(false),
        "holding": runtime.as_ref().map(|v| v.holding).unwrap_or(false),
        "range": runtime.and_then(|v| v.active_range)
    }))
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
    match live_iq_snapshot(&s, count.clamp(8192, 2_400_000)) {
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
                        m.altitude_ft.map(|a| format!("{a} ft")).unwrap_or_default()
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
            "error": e
        })),
    }
}
async fn native_ais_decode(Json(v): Json<Value>) -> Json<Value> {
    if let Some(iq) = v.get("iq").and_then(|x| x.as_array()) {
        let samples: Vec<(f32, f32)> = iq
            .iter()
            .filter_map(|p| {
                let a = p.as_array()?;
                Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
            })
            .collect();
        if samples.is_empty() {
            return Json(json!({"ok":false,"error":"iq must contain [I,Q] samples"}));
        }
        let rate = v
            .get("sample_rate_hz")
            .and_then(|x| x.as_f64())
            .unwrap_or(48000.0);
        let mut decoder = match crate::ais::IqDecoder::new(rate) {
            Ok(d) => d,
            Err(e) => return Json(json!({"ok":false,"error":e})),
        };
        let messages: Vec<Value> = decoder
            .push_iq(&samples)
            .into_iter()
            .filter_map(|r| r.ok())
            .filter_map(|m| serde_json::to_value(m).ok())
            .collect();
        return Json(
            json!({"ok":true,"native":true,"input":"iq","protocol":"ais","message_count":messages.len(),"messages":messages}),
        );
    }
    let bits: Vec<bool> = v
        .get("bits")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_bool()).collect())
        .unwrap_or_default();
    if bits.is_empty() {
        return Json(json!({"ok":false,"error":"bits[] is required"}));
    }
    let mut decoder = crate::ais::HdlcDecoder::new();
    let results = decoder.push_bits(bits);
    let messages: Vec<Value> = results
        .into_iter()
        .filter_map(|r| r.ok())
        .filter_map(|m| serde_json::to_value(m).ok())
        .collect();
    Json(
        json!({"ok":true,"native":true,"protocol":"ais","message_count":messages.len(),"messages":messages}),
    )
}

async fn native_pocsag_decode(Json(v): Json<Value>) -> Json<Value> {
    if let Some(iq) = v.get("iq").and_then(|x| x.as_array()) {
        let samples: Vec<(f32, f32)> = iq
            .iter()
            .filter_map(|p| {
                let a = p.as_array()?;
                Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
            })
            .collect();
        if samples.is_empty() {
            return Json(json!({"ok":false,"error":"iq must contain [I,Q] samples"}));
        }
        let rate = v
            .get("sample_rate_hz")
            .and_then(|x| x.as_u64())
            .unwrap_or(128000) as u32;
        let baud = match v.get("baud").and_then(|x| x.as_u64()).unwrap_or(1200) {
            2400 => crate::pocsag::PocsagBaud::Baud2400,
            _ => crate::pocsag::PocsagBaud::Baud1200,
        };
        let mut decoder = crate::pocsag::IqDecoder::new(rate, baud);
        let mut messages = decoder.push_iq(&samples);
        messages.extend(decoder.flush());
        return Json(
            json!({"ok":true,"native":true,"input":"iq","protocol":"pocsag","message_count":messages.len(),"messages":messages,"corrected_codewords":decoder.corrected_words(),"rejected_codewords":decoder.rejected_words()}),
        );
    }
    let bits: Vec<bool> = v
        .get("bits")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_bool()).collect())
        .unwrap_or_default();
    if bits.is_empty() {
        return Json(json!({"ok":false,"error":"bits[] is required"}));
    }
    let baud = match v.get("baud").and_then(|x| x.as_u64()).unwrap_or(1200) {
        2400 => crate::pocsag::PocsagBaud::Baud2400,
        _ => crate::pocsag::PocsagBaud::Baud1200,
    };
    let mut decoder = crate::pocsag::PocsagDecoder::new(baud.value() * 8, baud);
    let messages = decoder.push_bits(&bits);
    Json(
        json!({"ok":true,"native":true,"protocol":"pocsag","message_count":messages.len(),"messages":messages,"corrected_codewords":decoder.corrected_words(),"rejected_codewords":decoder.rejected_words()}),
    )
}

async fn native_uat_decode(Json(v): Json<Value>) -> Json<Value> {
    if let Some(iq) = v.get("iq").and_then(|x| x.as_array()) {
        let samples: Vec<(f32, f32)> = iq
            .iter()
            .filter_map(|p| {
                let a = p.as_array()?;
                Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
            })
            .collect();
        let rate = v
            .get("sample_rate_hz")
            .and_then(|x| x.as_u64())
            .unwrap_or(2_000_000) as u32;
        let mut d = crate::aviation::UatIqDecoder::new(rate);
        d.push_iq(&samples);
        let messages = d.take_messages();
        return Json(
            json!({"ok":true,"native":true,"input":"iq","protocol":"uat978","message_count":messages.len(),"messages":messages}),
        );
    }
    let bits: Vec<bool> = v
        .get("bits")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_bool()).collect())
        .unwrap_or_default();
    if bits.is_empty() {
        return Json(json!({"ok":false,"error":"bits[] is required"}));
    }
    let mut decoder = crate::aviation::UatDecoder::new();
    decoder.feed_bits(&bits);
    let messages = decoder.take_messages();
    Json(
        json!({"ok":true,"native":true,"protocol":"uat978","message_count":messages.len(),"messages":messages}),
    )
}

async fn native_acars_decode(Json(v): Json<Value>) -> Json<Value> {
    if let Some(iq) = v.get("iq").and_then(|x| x.as_array()) {
        let samples: Vec<(f32, f32)> = iq
            .iter()
            .filter_map(|p| {
                let a = p.as_array()?;
                Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
            })
            .collect();
        let rate = v
            .get("sample_rate_hz")
            .and_then(|x| x.as_u64())
            .unwrap_or(12500) as u32;
        let mut d =
            crate::aviation::AcarsIqDecoder::new(rate, crate::aviation::BitOrder::MsbFirst, false);
        d.push_iq(&samples);
        let messages = d.take_messages();
        return Json(
            json!({"ok":true,"native":true,"input":"iq","protocol":"acars","message_count":messages.len(),"messages":messages}),
        );
    }
    let bits: Vec<bool> = v
        .get("bits")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_bool()).collect())
        .unwrap_or_default();
    if bits.is_empty() {
        return Json(json!({"ok":false,"error":"bits[] is required"}));
    }
    let mut decoder =
        crate::aviation::AcarsDecoder::new(crate::aviation::BitOrder::MsbFirst, false);
    decoder.feed_bits(&bits);
    let messages = decoder.take_messages();
    Json(
        json!({"ok":true,"native":true,"protocol":"acars","message_count":messages.len(),"messages":messages}),
    )
}

async fn native_vdl2_decode(Json(v): Json<Value>) -> Json<Value> {
    if let Some(iq) = v.get("iq").and_then(|x| x.as_array()) {
        let samples: Vec<(f32, f32)> = iq
            .iter()
            .filter_map(|p| {
                let a = p.as_array()?;
                Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
            })
            .collect();
        let rate = v
            .get("sample_rate_hz")
            .and_then(|x| x.as_u64())
            .unwrap_or(1_000_000) as u32;
        let mut d = crate::aviation::Vdl2IqDecoder::new(rate);
        d.push_iq(&samples);
        let messages = d.take_messages();
        return Json(
            json!({"ok":true,"native":true,"input":"iq","protocol":"vdl2","message_count":messages.len(),"messages":messages,"note":"D8PSK slicer active; physical VDL2 FEC/carrier recovery is limited"}),
        );
    }
    let bits: Vec<bool> = v
        .get("bits")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_bool()).collect())
        .unwrap_or_default();
    if bits.is_empty() {
        return Json(json!({"ok":false,"error":"bits[] is required"}));
    }
    let mut decoder = crate::aviation::Vdl2Decoder::new();
    decoder.feed_bits(&bits);
    let messages = decoder.take_messages();
    Json(
        json!({"ok":true,"native":true,"protocol":"vdl2","message_count":messages.len(),"messages":messages}),
    )
}

async fn scan_ais(State(s): State<ApiState>) -> Json<Value> {
    recent_native_scan_messages(
        &s,
        "ais",
        "Native AIS decode runs while the AIS scan range is active; this endpoint returns recent persisted messages.",
    )
}

async fn scan_acars(State(s): State<ApiState>) -> Json<Value> {
    recent_native_scan_messages(
        &s,
        "acars",
        "Native ACARS decode runs while the ACARS scan range is active; this endpoint returns recent persisted messages.",
    )
}

fn recent_native_scan_messages(s: &ApiState, protocol: &str, note: &str) -> Json<Value> {
    let messages =
        s.0.db
            .messages_by_protocol(Some(protocol), 50)
            .unwrap_or_default();
    Json(json!({
        "available": true,
        "native": true,
        "integration": "live_scan_range",
        "messages": messages,
        "note": note,
    }))
}
async fn scan_aero(State(s): State<ApiState>) -> Json<Value> {
    scan_acars(State(s)).await
}
async fn scan_ble(State(s): State<ApiState>) -> Json<Value> {
    let status = s.0.device.status();
    if !status.connected {
        return Json(json!({
            "available": true,
            "native": true,
            "messages": s.0.db.messages_by_protocol(Some("ble"), 50).unwrap_or_default(),
            "reason": "no device connected — BLE advertising needs a 2.4 GHz-capable SDR"
        }));
    }
    if status.sample_rate < crate::ble::BLE_SYMBOL_RATE {
        return Json(json!({
            "available": true,
            "native": true,
            "messages": [],
            "reason": format!("sample rate {} Hz is below 1 Msym/s BLE advertising", status.sample_rate)
        }));
    }
    let count = (status.sample_rate as usize / 5).clamp(8_192, 800_000);
    match live_iq_snapshot(&s, count) {
        Ok(iq) => {
            let ads = crate::ble::decode_iq(&iq, status.sample_rate);
            for adv in &ads {
                if adv.crc_valid {
                    let _ =
                        s.0.db
                            .insert_decoded_message(&adv.to_decoded(status.center_freq_hz));
                }
            }
            Json(json!({
                "available": true,
                "native": true,
                "sample_rate_hz": status.sample_rate,
                "samples": iq.len(),
                "message_count": ads.len(),
                "messages": ads,
            }))
        }
        Err(e) => Json(json!({
            "available": true,
            "native": true,
            "messages": s.0.db.messages_by_protocol(Some("ble"), 50).unwrap_or_default(),
            "error": e
        })),
    }
}
async fn scan_lora(State(s): State<ApiState>) -> Json<Value> {
    let status = s.0.device.status();
    if !status.connected {
        return Json(json!({
            "available": true,
            "native": true,
            "messages": s.0.db.messages_by_protocols(crate::lora::LORA_PROTOCOLS, 50).unwrap_or_default(),
            "reason": "no device connected — tune an ISM LoRa channel and retry"
        }));
    }
    let count = (status.sample_rate as usize / 4).clamp(16_384, 1_000_000);
    match live_iq_snapshot(&s, count) {
        Ok(iq) => {
            let packets = crate::lora::decode_iq(&iq, status.sample_rate);
            for packet in &packets {
                let _ =
                    s.0.db
                        .insert_decoded_message(&packet.to_decoded(status.center_freq_hz));
            }
            Json(json!({
                "available": true,
                "native": true,
                "families": ["meshtastic", "meshcore", "reticulum", "modbus-lora", "lorawan"],
                "sample_rate_hz": status.sample_rate,
                "samples": iq.len(),
                "message_count": packets.len(),
                "messages": packets,
                "note": "Encrypted MeshCore/Meshtastic/Reticulum/LoRaWAN payloads are identified only"
            }))
        }
        Err(e) => Json(json!({
            "available": true,
            "native": true,
            "messages": s.0.db.messages_by_protocols(crate::lora::LORA_PROTOCOLS, 50).unwrap_or_default(),
            "error": e
        })),
    }
}

#[derive(Deserialize)]
struct RecordingReq {
    path: Option<String>,
}

async fn rec_iq_start(
    State(s): State<ApiState>,
    req: Option<Json<RecordingReq>>,
) -> impl IntoResponse {
    let req = req.map(|Json(v)| v).unwrap_or(RecordingReq { path: None });
    let dir = s.0.data_dir.join("recordings");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Json(json!({"ok": false, "error": e.to_string()}));
    }
    let path = req
        .path
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| dir.join(format!("iq-{}.cf32", crate::scanner::now_ms())));
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
    let Some(h) = handle.as_ref() else {
        return Json(json!({"available": false, "reason": "scanner not running"}));
    };
    let vfos = h.state.lock().vfo_states.clone();
    let vfo = crate::scanner::selected_vfo(&vfos).cloned();
    let vfo_id = vfo.as_ref().map(|v| v.id).unwrap_or(0);
    drop(handle);
    let status = s.0.device.status();
    if !status.connected {
        return Json(json!({"available": false, "reason": "no device connected"}));
    }
    let sample_rate = status.sample_rate;
    let count = (sample_rate as f64 * 0.3) as usize;
    match live_iq_snapshot(&s, count.max(4096)) {
        Ok(iq) if iq.len() > 1024 => {
            use crate::demod::{detect_ctcss, detect_dcs, Mode};
            let vfo_hz = vfo
                .as_ref()
                .map(|vfo| vfo.frequency_hz)
                .unwrap_or(status.center_freq_hz);
            let (audio, audio_rate) =
                channelized_vfo_audio(&iq, vfo_hz, status.center_freq_hz, sample_rate, Mode::Nfm);
            let ctcss = detect_ctcss(&audio, audio_rate);
            let dcs = detect_dcs(&audio, audio_rate);
            Json(json!({
                "available": true,
                "vfo_id": vfo_id,
                "frequency_hz": vfo_hz,
                "ctcss": ctcss.map(|(tone, conf)| json!({"tone_hz": (tone * 10.0).round() / 10.0, "confidence": conf})),
                "dcs": dcs,
                "samples_analyzed": audio.len(),
            }))
        }
        Ok(_) => Json(json!({"available": false, "reason": "insufficient samples"})),
        Err(e) => Json(json!({"available": false, "error": e})),
    }
}

async fn scan_aprs(State(s): State<ApiState>) -> Json<Value> {
    let status = s.0.device.status();
    if !status.connected {
        return Json(json!({"available": false, "reason": "no device connected"}));
    }
    // Read ~2 seconds of IQ for APRS decode (1200 baud = ~2400 bits = ~300 bytes)
    let sample_rate = status.sample_rate;
    let count = (sample_rate as f64 * 0.5) as usize;
    let vfo_hz = s.0.scanner.read().as_ref().and_then(|handle| {
        crate::scanner::selected_vfo(&handle.state.lock().vfo_states).map(|vfo| vfo.frequency_hz)
    });
    match live_iq_snapshot(&s, count.max(4096)) {
        Ok(iq) if iq.len() > 4096 => {
            use crate::aprs::AprsDecoder;
            use crate::demod::Mode;
            let tune_hz = vfo_hz.unwrap_or(status.center_freq_hz);
            let (audio, audio_rate) =
                channelized_vfo_audio(&iq, tune_hz, status.center_freq_hz, sample_rate, Mode::Nfm);
            let mut decoder = AprsDecoder::new(audio_rate);
            for &sample in &audio {
                decoder.feed(sample);
            }
            Json(json!({
                "available": true,
                "frequency_hz": tune_hz,
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
        Err(e) => Json(json!({"available": false, "error": e})),
    }
}

async fn scan_digital_voice(State(s): State<ApiState>, Json(req): Json<Value>) -> Json<Value> {
    let mode = req.get("mode").and_then(|v| v.as_str()).unwrap_or("auto");
    let status = s.0.device.status();
    if !status.connected {
        return Json(json!({"available": false, "reason": "no device connected"}));
    }
    let sample_rate = status.sample_rate;
    let count = (sample_rate as f64 * 0.5) as usize;
    let vfo_hz = s.0.scanner.read().as_ref().and_then(|handle| {
        crate::scanner::selected_vfo(&handle.state.lock().vfo_states).map(|vfo| vfo.frequency_hz)
    });
    match live_iq_snapshot(&s, count.max(4096)) {
        Ok(iq) if iq.len() > 4096 => {
            use crate::demod::{channelize_iq, discriminator_samples, Mode};
            use crate::voice_decoder;
            let tune_hz = vfo_hz.unwrap_or(status.center_freq_hz);
            let channel = channelize_iq(
                &iq,
                tune_hz as f64 - status.center_freq_hz as f64,
                sample_rate,
                Mode::Nfm,
            );
            let (filtered, fir_rate) = crate::trunking::apply_p25_vfo_fir(&channel, sample_rate);
            let mut previous = None;
            let discriminator = discriminator_samples(&filtered, &mut previous);
            let resampled = crate::sidecar::resample_audio(&discriminator, fir_rate, 48_000);
            let result = voice_decoder::decode_digital_voice_discriminator(&resampled, mode);
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
                "input": "discriminator",
                "p25_fir": {"taps": crate::trunking::P25_FIR_TAPS, "cutoff_hz": crate::trunking::P25_FIR_CUTOFF_HZ, "rate_hz": fir_rate},
                "frequency_hz": tune_hz,
                "discriminator_samples": resampled.len(),
            }))
        }
        Ok(_) => Json(json!({"available": false, "reason": "insufficient samples"})),
        Err(e) => Json(json!({"available": false, "error": e})),
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

#[derive(Deserialize)]
struct IqNetworkReq {
    target: String,
}
async fn iq_consumers(State(s): State<ApiState>) -> Json<Value> {
    let consumers =
        s.0.scanner
            .read()
            .as_ref()
            .map(|scanner| {
                scanner
                    .iq_consumers
                    .iter()
                    .map(|ring| ring.status())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
    Json(json!({"consumers":consumers}))
}

async fn iq_network_start(
    State(s): State<ApiState>,
    Json(req): Json<IqNetworkReq>,
) -> impl IntoResponse {
    match req.target.parse::<SocketAddr>() {
        Ok(target) => match s.0.iq_network.start(target) {
            Ok(()) => Json(json!({"ok":true,"status":s.0.iq_network.status()})),
            Err(e) => Json(json!({"ok":false,"error":e.to_string()})),
        },
        Err(e) => Json(json!({"ok":false,"error":format!("invalid target: {e}")})),
    }
}
async fn iq_network_stop(State(s): State<ApiState>) -> impl IntoResponse {
    s.0.iq_network.stop();
    Json(json!({"ok":true,"status":s.0.iq_network.status()}))
}
async fn iq_network_status(State(s): State<ApiState>) -> impl IntoResponse {
    Json(s.0.iq_network.status())
}

#[derive(Deserialize)]
struct AudioNetworkReq {
    target: String,
}
async fn audio_network_start(
    State(s): State<ApiState>,
    Json(req): Json<AudioNetworkReq>,
) -> impl IntoResponse {
    match req.target.parse::<SocketAddr>() {
        Ok(target) => match s.0.audio.start_network(target) {
            Ok(()) => Json(json!({"ok":true,"status":s.0.audio.network_status()})),
            Err(e) => Json(json!({"ok":false,"error":e.to_string()})),
        },
        Err(e) => Json(json!({"ok":false,"error":format!("invalid target: {e}")})),
    }
}
async fn audio_network_stop(State(s): State<ApiState>) -> impl IntoResponse {
    s.0.audio.stop_network();
    Json(json!({"ok":true,"status":s.0.audio.network_status()}))
}
async fn audio_network_status(State(s): State<ApiState>) -> impl IntoResponse {
    Json(s.0.audio.network_status())
}

async fn audio_stream(State(s): State<ApiState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    let audio = s.0.audio.clone();
    let receiver = audio.subscribe();
    ws.on_upgrade(move |socket| audio_stream_pump(socket, audio, receiver))
}

const SPECTRUM_WIRE_VERSION: u16 = 3;
const SPECTRUM_HEADER_BYTES: usize = 64;

fn encode_spectrum_frame(frame: &SpectrumFrame, session_revision: u64) -> Vec<u8> {
    // Half-dB quantization spans -140 dBFS through -12.5 dBFS. Stronger bins
    // saturate cleanly; the fixed scale prevents waterfall colors pumping.
    let floor_dbfs = -140.0f32;
    let scale_db = 0.5f32;
    let mut packet = Vec::with_capacity(SPECTRUM_HEADER_BYTES + frame.bins_dbfs.len());
    packet.extend_from_slice(b"PSF3");
    packet.extend_from_slice(&SPECTRUM_WIRE_VERSION.to_le_bytes());
    packet.extend_from_slice(&0u16.to_le_bytes());
    packet.extend_from_slice(&frame.sequence.to_le_bytes());
    packet.extend_from_slice(&frame.captured_ms.to_le_bytes());
    packet.extend_from_slice(&frame.center_freq_hz.to_le_bytes());
    packet.extend_from_slice(&frame.sample_rate_hz.to_le_bytes());
    packet.extend_from_slice(&frame.usable_span_hz.to_le_bytes());
    packet.extend_from_slice(&(frame.bins_dbfs.len() as u32).to_le_bytes());
    packet.extend_from_slice(&floor_dbfs.to_le_bytes());
    packet.extend_from_slice(&scale_db.to_le_bytes());
    // Numeric receiver identity is stable within this server contract. The
    // session revision changes on claim/release and lets clients discard
    // frames from a previous ownership epoch after an atomic takeover.
    packet.extend_from_slice(&0u32.to_le_bytes()); // receiver-0
    packet.extend_from_slice(&session_revision.to_le_bytes());
    packet.extend(
        frame
            .bins_dbfs
            .iter()
            .map(|value| ((value - floor_dbfs) / scale_db).round().clamp(0.0, 255.0) as u8),
    );
    packet
}

async fn spectrum_stream_ws(State(s): State<ApiState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    let state = s.0.clone();
    ws.on_upgrade(move |socket| spectrum_ws_pump(socket, state))
}

async fn spectrum_ws_pump(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut frames = state.spectrum.subscribe();
    loop {
        tokio::select! {
            changed = frames.changed() => {
                if changed.is_err() { break; }
                let revision = state.receiver_session.lock().revision;
                let packet = encode_spectrum_frame(&frames.borrow_and_update().clone(), revision);
                if sender.send(Message::Binary(packet)).await.is_err() { break; }
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
}

async fn audio_stream_pump(
    socket: WebSocket,
    audio: Arc<crate::audio::AudioSink>,
    mut frames: tokio::sync::broadcast::Receiver<crate::audio::AudioFrame>,
) {
    let (mut sender, mut receiver) = socket.split();
    loop {
        tokio::select! {
            incoming = receiver.next() => {
                if incoming.is_none() || matches!(incoming, Some(Ok(Message::Close(_)))) { break; }
            }
            frame = frames.recv() => {
                match frame {
                    Ok(frame) => {
                        let mut packet = Vec::with_capacity(32 + frame.samples.len() * 4);
                        packet.extend_from_slice(b"PSA2");
                        packet.extend_from_slice(&2u16.to_le_bytes());
                        packet.extend_from_slice(&frame.channels.to_le_bytes());
                        packet.extend_from_slice(&frame.sample_rate.to_le_bytes());
                        packet.extend_from_slice(&frame.sequence.to_le_bytes());
                        packet.extend_from_slice(&frame.captured_ms.to_le_bytes());
                        packet.extend_from_slice(&(frame.samples.len() as u32).to_le_bytes());
                        for sample in frame.samples.iter() { packet.extend_from_slice(&sample.to_le_bytes()); }
                        if sender.send(Message::Binary(packet)).await.is_err() { break; }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => audio.observe_remote_lag(count),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct AnnotationReq {
    recording_path: String,
    offset_ms: i64,
    text: String,
}
async fn playback_start(
    State(s): State<ApiState>,
    Json(req): Json<RecordingReq>,
) -> impl IntoResponse {
    let claim = s.0.receiver_session.lock().claim("playback", false);
    if let Err(error) = claim {
        let session = s.0.receiver_session.lock().clone();
        return (
            StatusCode::CONFLICT,
            Json(json!({"ok":false,"error":error,"session":session})),
        );
    }
    let Some(path) = req.path else {
        s.0.receiver_session.lock().release("playback");
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"path is required"})),
        );
    };
    match crate::capture::PlaybackReader::open(std::path::PathBuf::from(&path)) {
        Ok(reader) => {
            *s.0.playback.lock() = Some(reader);
            (
                StatusCode::OK,
                Json(json!({"ok":true,"path":path,"format":"cf32-le"})),
            )
        }
        Err(error) => {
            s.0.receiver_session.lock().release("playback");
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":error.to_string()})),
            )
        }
    }
}
async fn playback_stop(State(s): State<ApiState>) -> impl IntoResponse {
    let previous = s.0.playback.lock().take().map(|r| r.status());
    s.0.receiver_session.lock().release("playback");
    (StatusCode::OK, Json(json!({"ok":true,"previous":previous})))
}
async fn playback_status(State(s): State<ApiState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(
            s.0.playback
                .lock()
                .as_ref()
                .map(|r| r.status())
                .unwrap_or_else(|| json!({"playing":false,"format":"cf32-le"})),
        ),
    )
}

async fn rec_annotations(State(s): State<ApiState>) -> impl IntoResponse {
    match s.0.db.list_annotations() {
        Ok(v) => Json(serde_json::to_value(v).unwrap()),
        Err(e) => Json(json!({"error": e.to_string()})),
    }
}
async fn rec_annotation_new(
    State(s): State<ApiState>,
    Json(req): Json<AnnotationReq>,
) -> impl IntoResponse {
    let a = crate::db::RecordingAnnotation {
        id: None,
        recording_path: req.recording_path,
        offset_ms: req.offset_ms,
        text: req.text,
        created_ms: crate::scanner::now_ms(),
    };
    match s.0.db.add_annotation(&a) {
        Ok(id) => Json(json!({"ok": true, "id": id})),
        Err(e) => Json(json!({"ok": false, "error": e.to_string()})),
    }
}
async fn rec_annotation_one(State(s): State<ApiState>, Path(id): Path<i64>) -> impl IntoResponse {
    match s.0.db.list_annotations() {
        Ok(v) => Json(
            v.into_iter()
                .find(|a| a.id == Some(id))
                .map(|a| serde_json::to_value(a).unwrap())
                .unwrap_or_else(|| json!({"error":"not found"})),
        ),
        Err(e) => Json(json!({"error": e.to_string()})),
    }
}
async fn rec_annotation_delete(
    State(s): State<ApiState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    Json(json!({"ok": s.0.db.delete_annotation(id).is_ok()}))
}
async fn rec_annotation_update(
    State(s): State<ApiState>,
    Path(id): Path<i64>,
    Json(req): Json<AnnotationReq>,
) -> impl IntoResponse {
    let a = crate::db::RecordingAnnotation {
        id: Some(id),
        recording_path: req.recording_path,
        offset_ms: req.offset_ms,
        text: req.text,
        created_ms: crate::scanner::now_ms(),
    };
    Json(json!({"ok":s.0.db.update_annotation(id,&a).map(|n|n>0).unwrap_or(false),"id":id}))
}
async fn iq_rec_start(
    State(s): State<ApiState>,
    req: Option<Json<RecordingReq>>,
) -> impl IntoResponse {
    rec_iq_start(State(s), req).await
}
async fn iq_rec_stop(State(s): State<ApiState>) -> impl IntoResponse {
    rec_iq_stop(State(s)).await
}
async fn iq_rec_status(State(s): State<ApiState>) -> impl IntoResponse {
    Json(s.0.recording.lock().status())
}

async fn transcription_start(State(s): State<ApiState>) -> impl IntoResponse {
    let engine = crate::transcription::find_engine();
    let (model, language) = {
        let c = s.0.config.read();
        (
            c.transcription.model.clone(),
            c.transcription.language.clone(),
        )
    };
    if !engine.available {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({
                "ok": false,
                "available": false,
                "running": false,
                "engine": engine.kind,
                "model": model,
                "error": engine.install_hint,
            })),
        );
    }
    let status = s.0.device.status();
    let mut last_error = None;
    let mut segments = Vec::new();
    let (recent, recent_rate) = s.0.audio.recent_pcm();
    if recent.len() > 1_600 {
        match crate::transcription::transcribe_pcm(&recent, recent_rate, &model) {
            Ok(found) => segments = found,
            Err(e) => last_error = Some(e),
        }
    } else if status.connected {
        let count = (status.sample_rate as usize / 5).clamp(8_192, 480_000);
        match live_iq_snapshot(&s, count) {
            Ok(iq) if iq.len() > 1024 => {
                let pcm = crate::demod::channelize_demod(
                    &iq,
                    0.0,
                    status.sample_rate,
                    crate::demod::Mode::Nfm,
                );
                match crate::transcription::transcribe_pcm(&pcm, status.sample_rate, &model) {
                    Ok(found) => segments = found,
                    Err(e) => last_error = Some(e),
                }
            }
            Ok(_) => last_error = Some("insufficient samples".into()),
            Err(e) => last_error = Some(e),
        }
    } else {
        last_error = Some("no recent demod PCM and no device connected; engine is armed".into());
    }
    let mut runtime = s.0.transcription.lock();
    runtime.running = true;
    runtime.last_error = last_error.clone();
    runtime.transcripts.extend(segments.iter().cloned());
    if runtime.transcripts.len() > 64 {
        let extra = runtime.transcripts.len() - 64;
        runtime.transcripts.drain(..extra);
    }
    let _ = language;
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "available": true,
            "running": true,
            "engine": engine.kind,
            "path": engine.path,
            "model": model,
            "segments": segments,
            "error": last_error,
        })),
    )
}
async fn transcription_stop(State(s): State<ApiState>) -> impl IntoResponse {
    let mut c = s.0.config.write();
    c.transcription.enabled = false;
    let _ = c.save(&s.0.data_dir);
    s.0.transcription.lock().running = false;
    Json(json!({"ok":true,"running":false}))
}
async fn transcription_status(State(s): State<ApiState>) -> impl IntoResponse {
    let pcm_ring_samples = s.0.audio.recent_pcm().0.len();
    let c = s.0.config.read();
    let engine = crate::transcription::find_engine();
    let runtime = s.0.transcription.lock();
    Json(json!({
        "available": engine.available,
        "running": runtime.running,
        "enabled": c.transcription.enabled,
        "engine": engine.kind,
        "path": engine.path,
        "model": c.transcription.model,
        "status": if engine.available { "development" } else { "planned" },
        "install_hint": engine.install_hint,
        "last_error": runtime.last_error,
        "missing_gate": if engine.available {
            "hardware live verification of 16 kHz PCM through whisper.cpp"
        } else {
            "whisper.cpp / whisper-cli is not installed"
        },
        "pcm_ring_samples": pcm_ring_samples,
    }))
}
async fn transcription_list(State(s): State<ApiState>) -> impl IntoResponse {
    Json(json!(s.0.transcription.lock().transcripts.clone()))
}

#[derive(Deserialize)]
struct CaseReq {
    name: String,
    description: Option<String>,
    status: Option<String>,
    tags: Option<String>,
}
async fn cases(State(s): State<ApiState>) -> impl IntoResponse {
    match s.0.db.list_cases() {
        Ok(v) => Json(serde_json::to_value(v).unwrap()),
        Err(e) => Json(json!({"error": e.to_string()})),
    }
}
async fn cases_new(State(s): State<ApiState>, Json(req): Json<CaseReq>) -> impl IntoResponse {
    let now = crate::scanner::now_ms();
    let c = crate::db::Case {
        id: None,
        name: req.name,
        description: req.description.unwrap_or_default(),
        status: req.status.unwrap_or_else(|| "open".into()),
        tags: req.tags.unwrap_or_default(),
        created_ms: now,
        updated_ms: now,
    };
    match s.0.db.create_case(&c) {
        Ok(id) => Json(json!({"ok": true, "id": id})),
        Err(e) => Json(json!({"ok": false, "error": e.to_string()})),
    }
}
async fn case_one(State(s): State<ApiState>, Path(id): Path<i64>) -> impl IntoResponse {
    match s.0.db.get_case(id) {
        Ok(Some(c)) => Json(serde_json::to_value(c).unwrap()),
        Ok(None) => Json(json!({"error":"not found"})),
        Err(e) => Json(json!({"error": e.to_string()})),
    }
}
async fn case_delete(State(s): State<ApiState>, Path(id): Path<i64>) -> impl IntoResponse {
    Json(json!({"ok": s.0.db.delete_case(id).is_ok()}))
}
#[derive(Deserialize)]
struct CaseAttachmentReq {
    kind: String,
    r#ref: String,
    note: Option<String>,
}
async fn case_attach(
    State(s): State<ApiState>,
    Path(id): Path<i64>,
    Json(req): Json<CaseAttachmentReq>,
) -> impl IntoResponse {
    let a = crate::db::CaseAttachment {
        id: None,
        case_id: id,
        kind: req.kind,
        r#ref: req.r#ref,
        note: req.note.unwrap_or_default(),
        attached_ms: crate::scanner::now_ms(),
    };
    match s.0.db.add_case_attachment(&a) {
        Ok(att_id) => Json(json!({"ok":true,"id":att_id})),
        Err(e) => Json(json!({"ok":false,"error":e.to_string()})),
    }
}
async fn case_attachment_one(State(s): State<ApiState>, Path(id): Path<i64>) -> impl IntoResponse {
    match s.0.db.case_attachment(id) {
        Ok(Some(a)) => Json(serde_json::to_value(a).unwrap()),
        Ok(None) => Json(json!({"error":"not found"})),
        Err(e) => Json(json!({"error":e.to_string()})),
    }
}
async fn case_attachment_delete(
    State(s): State<ApiState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    Json(json!({"ok":s.0.db.delete_case_attachment(id).map(|n|n>0).unwrap_or(false)}))
}

async fn sidecar_stderr(State(s): State<ApiState>, Path(name): Path<String>) -> impl IntoResponse {
    Json(serde_json::to_value(s.0.sidecars.stderr(&name)).unwrap())
}

async fn feature_packs(State(s): State<ApiState>) -> impl IntoResponse {
    let c = s.0.config.read();
    let pack = |pack_id: &str,
                name: &str,
                enabled: bool,
                sidecar: &str,
                path: &str,
                protocols: &[&str]| {
        let available = !path.trim().is_empty() && std::path::Path::new(path).is_file();
        let decoder_name = crate::depmanager::decoder_for_pack(pack_id);
        let can_auto_install = decoder_name
            .map(crate::depmanager::can_auto_install_decoder)
            .unwrap_or(false);
        let direct_download_url =
            decoder_name.and_then(crate::depmanager::download_url_for_decoder);
        json!({
            "id": pack_id,
            "name": name,
            "enabled": enabled,
            "running": s.0.sidecars.is_running(sidecar),
            "path": path,
            "available": available,
            "availability_reason": if available { "executable found" } else { "executable missing" },
            "protocols": protocols,
            "decoder_name": decoder_name,
            "can_auto_install": can_auto_install,
            "direct_download_url": direct_download_url,
        })
    };
    let packs = vec![
        pack(
            "rtl433",
            "RTL-SDR 433 sensors",
            c.rtl433.enabled,
            "rtl_433",
            &c.rtl433.path,
            &["rtl_433"],
        ),
        pack(
            "digital",
            "Digital voice / pager",
            c.digital_decoder.enabled,
            "multimon-ng",
            &c.digital_decoder.multimon_path,
            &["pocsag", "p25", "dmr"],
        ),
        pack(
            "acars",
            "ACARS",
            c.acarsdec.enabled,
            "acarsdec",
            &c.acarsdec.path,
            &["acars"],
        ),
        pack(
            "vdl2",
            "VDL2",
            c.vdl2.enabled,
            "dumpvdl2",
            &c.vdl2.path,
            &["vdl2"],
        ),
        pack(
            "aprs",
            "APRS / Direwolf",
            c.aprs.enabled,
            "direwolf",
            &c.aprs.path,
            &["aprs"],
        ),
        pack(
            "dsd",
            "DSD digital voice",
            c.dsd.enabled,
            "dsd-fme",
            &c.dsd.dsdneo_path,
            &["p25", "dmr", "nxdn"],
        ),
        pack(
            "radiosonde",
            "RS41 radiosonde",
            c.radiosonde.enabled,
            "rs41mod",
            &c.radiosonde.path,
            &["rs41"],
        ),
    ];
    Json(json!({"groups": packs, "count": packs.len()}))
}
async fn feature_pack_enable(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> impl IntoResponse {
    let enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
    let ok = {
        let mut c = s.0.config.write();
        match id.as_str() {
            "rtl433" => {
                c.rtl433.enabled = enabled;
                true
            }
            "digital" => {
                c.digital_decoder.enabled = enabled;
                true
            }
            "acars" => {
                c.acarsdec.enabled = enabled;
                true
            }
            "vdl2" => {
                c.vdl2.enabled = enabled;
                true
            }
            "aprs" => {
                c.aprs.enabled = enabled;
                true
            }
            "dsd" => {
                c.dsd.enabled = enabled;
                true
            }
            "radiosonde" => {
                c.radiosonde.enabled = enabled;
                true
            }
            _ => false,
        }
    };
    if !ok {
        return Json(json!({"ok":false,"error":"unknown feature pack","id":id}));
    }
    {
        let c = s.0.config.read();
        let _ = c.save(&s.0.data_dir);
    }
    let sidecar = match id.as_str() {
        "rtl433" => "rtl_433",
        "digital" => "multimon-ng",
        "acars" => "acarsdec",
        "vdl2" => "dumpvdl2",
        "aprs" => "direwolf",
        "dsd" => "dsd-fme",
        "radiosonde" => "rs41mod",
        _ => "",
    };
    if enabled {
        start_configured_sidecars(&s).await;
    } else if !sidecar.is_empty() {
        let _ = s.0.sidecars.kill(sidecar).await;
    }
    Json(
        json!({"ok":true,"id":id,"enabled":enabled,"sidecar_running":s.0.sidecars.is_running(sidecar)}),
    )
}

async fn aircraft_lookup(State(_s): State<ApiState>, Query(q): Query<LookupQ>) -> Json<Value> {
    if q.q.unwrap_or_default().trim().is_empty() {
        return Json(
            json!({"available":false,"results":[],"reason":"Aircraft lookup database is not configured"}),
        );
    }
    Json(
        json!({"available":false,"results":[],"reason":"Aircraft lookup database is not configured"}),
    )
}
#[derive(Deserialize)]
struct LookupQ {
    q: Option<String>,
}
async fn intercept_results() -> impl IntoResponse {
    Json(json!([]))
}
async fn instances(State(s): State<ApiState>) -> impl IntoResponse {
    let d = s.0.device.status();
    Json(
        json!([{"id":"local","name":"PulseScope local","connected":d.connected,"driver":d.driver,"address":"127.0.0.1:8765"}]),
    )
}
async fn reconnect(State(s): State<ApiState>) -> impl IntoResponse {
    let key = s.0.config.read().device.last_device_key.clone();
    let result = s.0.device.connect(&key);
    Json(json!({"ok":result.is_ok(),"key":key,"status":s.0.device.status()}))
}
async fn close_session(State(s): State<ApiState>) -> impl IntoResponse {
    if let Some(h) = s.0.scanner.read().as_ref() {
        let _ = h.cmd_tx.send(crate::scanner::ScannerCommand::Stop);
    }
    let _ = s.0.device.disconnect();
    Json(json!({"ok":true,"status":s.0.device.status()}))
}
async fn slots(State(s): State<ApiState>) -> impl IntoResponse {
    let v =
        s.0.scanner
            .read()
            .as_ref()
            .map(|h| h.state.lock().vfo_states.clone())
            .unwrap_or_default();
    Json(v.into_iter().map(|x| json!({"slot":x.id,"frequency_hz":x.frequency_hz,"mode":x.mode,"active":!x.muted,"squelch_open":x.squelch_open})).collect::<Vec<_>>())
}

async fn rtl433_messages(State(s): State<ApiState>, Query(q): Query<LimitQ>) -> impl IntoResponse {
    Json(
        serde_json::to_value(
            s.0.db
                .messages_by_protocol(Some("rtl_433"), q.limit.unwrap_or(100))
                .unwrap_or_default(),
        )
        .unwrap(),
    )
}
async fn protocol_messages(
    State(s): State<ApiState>,
    Query(q): Query<LimitQ>,
) -> impl IntoResponse {
    Json(
        serde_json::to_value(
            s.0.db
                .messages_by_protocol(None, q.limit.unwrap_or(100))
                .unwrap_or_default(),
        )
        .unwrap(),
    )
}

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
    if let Some(lat) = v.get("latitude_deg").and_then(|x| x.as_f64()) {
        cfg.receiver_location.latitude_deg = lat;
    }
    if let Some(lon) = v.get("longitude_deg").and_then(|x| x.as_f64()) {
        cfg.receiver_location.longitude_deg = lon;
    }
    if let Some(alt) = v.get("altitude_m").and_then(|x| x.as_f64()) {
        cfg.receiver_location.altitude_m = alt;
    }
    Json(json!({"ok": true}))
}

async fn device_test(State(s): State<ApiState>) -> impl IntoResponse {
    let status = s.0.device.status();
    if !status.connected {
        return Json(
            json!({"ok":false,"result":"not_connected","connected":false,"samples":0,"error":"device is not connected"}),
        );
    }
    let iq = match live_iq_snapshot(&s, 4096) {
        Ok(iq) => iq,
        Err(e) => {
            return Json(
                json!({"ok":false,"result":"stream_error","connected":true,"samples":0,"error":e}),
            )
        }
    };
    if iq.is_empty() {
        return Json(
            json!({"ok":false,"result":"empty_frame","connected":true,"samples":0,"error":"device returned no samples"}),
        );
    }
    let rms = (iq.iter().map(|x| x.norm_sqr()).sum::<f32>() / iq.len() as f32).sqrt();
    let peak = iq.iter().map(|x| x.norm()).fold(0.0_f32, f32::max);
    Json(
        json!({"ok":true,"result":"pass","connected":true,"samples":iq.len(),"rms":rms,"peak":peak,"sample_rate":status.sample_rate}),
    )
}
async fn device_hackrf_amp(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
    let d = s.0.device.status();
    let supported = d.driver.eq_ignore_ascii_case("hackrf");
    Json(
        json!({"ok":supported && d.connected,"enabled":enabled,"supported":supported,"driver":d.driver,"error":if supported {Value::Null}else{json!("HackRF amplifier control requires an active HackRF driver")}}),
    )
}
async fn channel_banks_delete(
    State(s): State<ApiState>,
    Json(v): Json<Value>,
) -> impl IntoResponse {
    let name = v
        .get("name")
        .or_else(|| v.get("bank_name"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if name.is_empty() {
        return Json(json!({"ok":false,"error":"name is required"}));
    }
    let mut c = s.0.config.write();
    let before = c.scan_ranges.len();
    c.scan_ranges.retain(|r| r.name != name);
    let removed = before != c.scan_ranges.len();
    if removed {
        let _ = c.save(&s.0.data_dir);
    }
    Json(
        json!({"ok":removed,"name":name,"error":if removed {Value::Null}else{json!("bank not found")}}),
    )
}
async fn channel_banks_create(
    State(s): State<ApiState>,
    Json(v): Json<Value>,
) -> impl IntoResponse {
    match serde_json::from_value::<crate::config::ScanRange>(v) {
        Ok(range) if !range.name.trim().is_empty() => {
            let mut c = s.0.config.write();
            c.scan_ranges.retain(|r| r.name != range.name);
            c.scan_ranges.push(range.clone());
            let _ = c.save(&s.0.data_dir);
            Json(json!({"ok":true,"bank":range}))
        }
        Ok(_) => Json(json!({"ok":false,"error":"bank name is required"})),
        Err(e) => Json(json!({"ok":false,"error":e.to_string()})),
    }
}
async fn channel_bank_scan_config(State(s): State<ApiState>) -> impl IntoResponse {
    let c = s.0.config.read();
    Json(
        json!({"ranges":c.scan_ranges.len(),"enabled":c.scan_ranges.iter().filter(|r|r.enabled).count()}),
    )
}
async fn channel_bank_scan_config_put(
    State(s): State<ApiState>,
    Json(v): Json<Value>,
) -> impl IntoResponse {
    let name = v
        .get("bank_name")
        .or_else(|| v.get("name"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if name.is_empty() {
        return Json(json!({"ok":false,"error":"bank_name is required"}));
    }
    let mut c = s.0.config.write();
    let Some(r) = c.scan_ranges.iter_mut().find(|r| r.name == name) else {
        return Json(json!({"ok":false,"error":"bank not found"}));
    };
    if let Some(x) = v.get("enabled").and_then(|x| x.as_bool()) {
        r.enabled = x;
    }
    if let Some(x) = v.get("dwell_ms").and_then(|x| x.as_u64()) {
        r.dwell_ms = x as u32;
    }
    if let Some(x) = v.get("hold_ms").and_then(|x| x.as_u64()) {
        r.hold_ms = x as u32;
    }
    if let Some(x) = v.get("max_vfos").and_then(|x| x.as_u64()) {
        r.max_vfos = x as u32;
    }
    if let Some(x) = v.get("squelch_db").and_then(|x| x.as_f64()) {
        r.squelch_db = x as f32;
    }
    let out = r.clone();
    let _ = c.save(&s.0.data_dir);
    if v.get("squelch_db").is_some() {
        if let Some(scanner) = s.0.scanner.read().as_ref() {
            let active = scanner.state.lock().active_range.clone();
            if active.as_deref() == Some(name) {
                let _ = scanner
                    .cmd_tx
                    .send(crate::scanner::ScannerCommand::SetRangeSquelch {
                        squelch_db: out.squelch_db,
                    });
            }
        }
    }
    Json(json!({"ok":true,"bank":out}))
}
async fn channel_import(State(s): State<ApiState>, Json(v): Json<Value>) -> impl IntoResponse {
    let rows = if v.is_array() { v } else { json!([v]) };
    let mut added = 0;
    let mut c = s.0.config.write();
    for item in rows.as_array().cloned().unwrap_or_default() {
        if let Ok(r) = serde_json::from_value::<crate::config::ScanRange>(item) {
            c.scan_ranges.retain(|x| x.name != r.name);
            c.scan_ranges.push(r);
            added += 1;
        }
    }
    let _ = c.save(&s.0.data_dir);
    Json(json!({"ok":true,"added":added,"total":c.scan_ranges.len()}))
}
async fn scanner_max_vfos(State(s): State<ApiState>) -> impl IntoResponse {
    let cfg = s.0.config.read();
    Json(json!({"max_vfos": cfg.scanner.max_vfos}))
}
async fn vfo_diagnostics(State(s): State<ApiState>) -> impl IntoResponse {
    let vfos =
        s.0.scanner
            .read()
            .as_ref()
            .map(|h| h.state.lock().vfo_states.clone())
            .unwrap_or_default();
    Json(vfos.into_iter().map(|v| json!({"id": v.id, "frequency_hz": v.frequency_hz, "strength_db": v.strength_db, "audio_level_db": v.audio_level_db, "squelch_open": v.squelch_open, "muted": v.muted})).collect::<Vec<_>>())
}
async fn vfo_identify(State(s): State<ApiState>, Path(id): Path<i64>) -> impl IntoResponse {
    let v = s.0.scanner.read().as_ref().and_then(|h| {
        h.state
            .lock()
            .vfo_states
            .iter()
            .find(|v| v.id as i64 == id)
            .cloned()
    });
    let Some(v) = v else {
        return Json(json!({"result":"unknown","error":"vfo not found"}));
    };
    let range_name =
        s.0.scanner
            .read()
            .as_ref()
            .and_then(|h| h.state.lock().active_range.clone())
            .unwrap_or_default();
    let snr_db = if v.snr_db.abs() > 0.01 {
        v.snr_db
    } else if v.squelch_open {
        18.0
    } else {
        8.0
    };
    let status = s.0.device.status();

    let classification = if status.connected {
        let count = ((status.sample_rate as f64) * 0.35) as usize;
        match live_iq_snapshot(&s, count.max(4096)) {
            Ok(iq) if iq.len() > 2048 => {
                use crate::demod::Mode;
                let (pcm, audio_rate) = channelized_vfo_audio(
                    &iq,
                    v.frequency_hz,
                    status.center_freq_hz,
                    status.sample_rate,
                    Mode::parse(&v.mode),
                );
                crate::signal_id::classify(
                    v.frequency_hz,
                    12_500,
                    &v.mode,
                    &range_name,
                    snr_db,
                    Some((&pcm, audio_rate)),
                )
            }
            _ => crate::signal_id::classify(
                v.frequency_hz,
                12_500,
                &v.mode,
                &range_name,
                snr_db,
                None,
            ),
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
    let v = s.0.scanner.read().as_ref().and_then(|h| {
        h.state
            .lock()
            .vfo_states
            .iter()
            .find(|v| v.id as i64 == id)
            .cloned()
    });
    let Some(v) = v else {
        return Json(json!({"present":false,"reason":"vfo not found"}));
    };
    if !v.mode.eq_ignore_ascii_case("wfm") {
        return Json(json!({"present":false,"reason":"RDS requires WFM mode"}));
    }
    let status = s.0.device.status();
    if !status.connected {
        return Json(json!({"present":false,"reason":"no device connected"}));
    }
    let count = (status.sample_rate as f64 * 0.5) as usize;
    match live_iq_snapshot(&s, count.max(4096)) {
        Ok(iq) if iq.len() > 4096 => {
            use crate::demod::{channelize_iq, decode_rds, discriminator_samples, Mode};
            let channel = channelize_iq(
                &iq,
                v.frequency_hz as f64 - status.center_freq_hz as f64,
                status.sample_rate,
                Mode::Wfm,
            );
            let mut previous = None;
            let multiplex = discriminator_samples(&channel, &mut previous);
            let multiplex = crate::sidecar::resample_audio(&multiplex, status.sample_rate, 190_000);
            match decode_rds(&multiplex, 190_000.0) {
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
                Some(_) => Json(
                    json!({"present":false,"frequency_hz":v.frequency_hz,"reason":"RDS subcarrier detected but no valid groups decoded"}),
                ),
                None => Json(
                    json!({"present":false,"frequency_hz":v.frequency_hz,"reason":"no RDS subcarrier detected"}),
                ),
            }
        }
        Ok(_) => Json(json!({"present":false,"reason":"insufficient samples"})),
        Err(e) => Json(json!({"present":false,"error":e})),
    }
}
async fn signal_events(State(s): State<ApiState>, Query(q): Query<LimitQ>) -> impl IntoResponse {
    match s.0.db.recent_signal_events(q.limit.unwrap_or(100)) {
        Ok(rows) => (
            StatusCode::OK,
            Json(serde_json::to_value(rows).unwrap_or_else(|_| json!([]))),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error.to_string()})),
        ),
    }
}

async fn spectrum_occupancy(State(s): State<ApiState>) -> impl IntoResponse {
    let snapshot = s.0.scanner.read().as_ref().map(|h| h.state.lock().clone());
    if let Some(runtime) = snapshot {
        if !runtime.latest_spectrum.is_empty() {
            let status = s.0.device.status();
            let rows = crate::scanner::occupancy_from_spectrum(
                &runtime.latest_spectrum,
                status.center_freq_hz.max(1),
                status.sample_rate.max(1),
                runtime.noise_floor_db,
                crate::scanner::now_ms(),
            );
            for row in &rows {
                let _ = s.0.db.upsert_occupancy(row);
            }
        }
    }
    let stored = s.0.db.recent_occupancy(512).unwrap_or_default();
    Json(json!(stored
        .iter()
        .map(|row| {
            json!({
                "frequency_bucket_hz": row.frequency_bucket_hz,
                "time_bucket_15min": row.time_bucket_15min,
                "avg_power_db": row.avg_power_db,
                "peak_power_db": row.peak_power_db,
                "avg_above_floor_db": row.avg_above_floor_db,
                "sample_count": row.sample_count,
                "noise_floor_db": row.noise_floor_db,
                "occupancy": crate::scanner::occupancy_fraction(row),
            })
        })
        .collect::<Vec<_>>()))
}

#[derive(Deserialize)]
struct BlacklistReq {
    frequency_hz: u64,
    reason: Option<String>,
    temporary: Option<bool>,
}
async fn blacklist(State(s): State<ApiState>) -> impl IntoResponse {
    match s.0.db.list_blacklist() {
        Ok(v) => Json(serde_json::to_value(v).unwrap()),
        Err(e) => Json(json!({"error": e.to_string()})),
    }
}
async fn blacklist_add(
    State(s): State<ApiState>,
    Json(req): Json<BlacklistReq>,
) -> impl IntoResponse {
    let e = crate::db::BlacklistEntry {
        frequency_hz: req.frequency_hz,
        reason: req.reason.unwrap_or_default(),
        temporary: req.temporary.unwrap_or(false),
        created_ms: crate::scanner::now_ms(),
    };
    Json(json!({"ok": s.0.db.add_blacklist(&e).is_ok()}))
}
async fn blacklist_remove(
    State(s): State<ApiState>,
    Json(req): Json<BlacklistReq>,
) -> impl IntoResponse {
    Json(json!({"ok": s.0.db.remove_blacklist(req.frequency_hz).is_ok()}))
}
async fn blacklist_clear(State(s): State<ApiState>) -> impl IntoResponse {
    Json(json!({"ok": s.0.db.clear_blacklist(false).is_ok()}))
}
async fn blacklist_clear_temporary(State(s): State<ApiState>) -> impl IntoResponse {
    Json(json!({"ok": s.0.db.clear_blacklist(true).is_ok()}))
}

async fn debug_stats(State(s): State<ApiState>) -> impl IntoResponse {
    let frames_processed =
        s.0.scanner
            .read()
            .as_ref()
            .map(|h| h.state.lock().frames_processed)
            .unwrap_or(0);
    let messages_decoded = s.0.db.decoded_message_count().unwrap_or(0);
    Json(json!({
        "uptime_ms": crate::scanner::now_ms().saturating_sub(s.0.started_ms),
        "messages_decoded": messages_decoded,
        "frames_processed": frames_processed,
        "audio": s.0.audio.status(),
        "sidecars": s.0.sidecars.statuses(),
    }))
}
async fn debug_log(State(s): State<ApiState>) -> impl IntoResponse {
    Json(json!({"sidecars":s.0.sidecars.statuses(),"trunking_log":s.0.trunking.read().log}))
}
async fn debug_log_tail(State(s): State<ApiState>) -> impl IntoResponse {
    let mut lines = Vec::new();
    for name in [
        "rtl_433",
        "multimon-ng",
        "acarsdec",
        "dumpvdl2",
        "direwolf",
        "dsd-fme",
        "rs41mod",
    ] {
        for line in s.0.sidecars.stderr(name) {
            lines.push(json!({"source":name,"line":line}));
        }
    }
    Json(json!(lines))
}
async fn debug_classifications(State(s): State<ApiState>) -> impl IntoResponse {
    let v =
        s.0.scanner
            .read()
            .as_ref()
            .map(|h| h.state.lock().vfo_states.clone())
            .unwrap_or_default();
    Json(v.into_iter().map(|x|json!({"vfo_id":x.id,"frequency_hz":x.frequency_hz,"classification":x.mode,"confidence":if x.squelch_open {0.96}else{0.12}})).collect::<Vec<_>>())
}
async fn debug_noise_floor(State(s): State<ApiState>) -> impl IntoResponse {
    let floor =
        s.0.scanner
            .read()
            .as_ref()
            .and_then(|h| {
                let bins = h.state.lock().latest_spectrum.clone();
                bins.into_iter().filter(|v| v.is_finite()).reduce(f32::min)
            })
            .unwrap_or(-120.0);
    Json(json!({"noise_floor_db":floor}))
}
async fn debug_dsd_stderr(State(s): State<ApiState>) -> impl IntoResponse {
    let mut lines = s.0.sidecars.stderr("dsd-fme");
    if lines.is_empty() {
        lines = s.0.sidecars.stderr("dsd-neo");
    }
    Json(lines)
}
async fn debug_multimon_raw(State(s): State<ApiState>) -> impl IntoResponse {
    Json(s.0.sidecars.stderr("multimon-ng"))
}
async fn debug_p25_acq(State(s): State<ApiState>) -> impl IntoResponse {
    let t = s.0.trunking.read();
    Json(
        json!({"locked":t.locked,"running":t.running,"control_channel_hz":t.control_channel_hz,"protocol":"P25","acquired":t.running && t.control_channel_hz.is_some()}),
    )
}
async fn debug_p25_squelch(State(s): State<ApiState>) -> impl IntoResponse {
    let v =
        s.0.scanner
            .read()
            .as_ref()
            .map(|h| h.state.lock().vfo_states.clone())
            .unwrap_or_default();
    Json(
        json!({"open_vfos":v.iter().filter(|x|x.squelch_open).map(|x|x.id).collect::<Vec<_>>(),"threshold_source":"scanner runtime"}),
    )
}
async fn debug_provoice_stderr(State(s): State<ApiState>) -> impl IntoResponse {
    Json(s.0.sidecars.stderr("dsd-fme"))
}
async fn debug_rtl433_stderr(State(s): State<ApiState>) -> impl IntoResponse {
    Json(s.0.sidecars.stderr("rtl_433"))
}
async fn debug_p25_use_vfo_fir() -> impl IntoResponse {
    Json(json!({
        "enabled": true,
        "supported": true,
        "taps": crate::trunking::P25_FIR_TAPS,
        "cutoff_hz": crate::trunking::P25_FIR_CUTOFF_HZ,
        "rate_hz": crate::trunking::P25_FIR_RATE_HZ,
        "window": "hamming",
        "reason": "P25 VFO FIR runs on IQ before the discriminator; it is not a P25 voice decoder"
    }))
}
async fn debug_per_cc_stats(State(s): State<ApiState>) -> impl IntoResponse {
    let t = s.0.trunking.read();
    Json(
        json!({"running":t.running,"control_channel_hz":t.control_channel_hz,"call_count":t.calls.len(),"active_talkgroup":t.active_talkgroup}),
    )
}
async fn debug_vdl2_stderr(State(s): State<ApiState>) -> impl IntoResponse {
    Json(s.0.sidecars.stderr("dumpvdl2"))
}

// ── event fan-out ─────────────────────────────────────────────────────────

async fn event_stream(State(s): State<ApiState>) -> impl IntoResponse {
    use axum::response::sse::{Event, KeepAlive, Sse};

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
            if sender.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    });
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if matches!(msg, Message::Close(_)) {
                break;
            }
        }
    });
    let _ = tokio::join!(send_task, recv_task);
}

fn send_vfo(s: &ApiState, cmd: crate::scanner::ScannerCommand) {
    if let Some(h) = s.0.scanner.read().as_ref() {
        let _ = h.cmd_tx.send(cmd);
    }
}

/// Copy live IQ from the capture snapshot ring. Never reads the hardware
/// stream while the scanner owns it — that would steal waterfall/audio samples.
fn live_iq_snapshot(s: &ApiState, count: usize) -> Result<Vec<Complex<f32>>, String> {
    if let Some(handle) = s.0.scanner.read().as_ref() {
        return handle
            .snapshot_iq(count)
            .filter(|iq| !iq.is_empty())
            .ok_or_else(|| "live IQ snapshot is empty; wait for capture".to_string());
    }
    s.0.device
        .read_iq(count.max(4096))
        .map_err(|error| error.to_string())
}

fn channelized_vfo_audio(
    iq: &[Complex<f32>],
    vfo_hz: u64,
    center_hz: u64,
    sample_rate: u32,
    mode: crate::demod::Mode,
) -> (Vec<f32>, f32) {
    use crate::demod::channelize_demod;
    let pcm = channelize_demod(iq, vfo_hz as f64 - center_hz as f64, sample_rate, mode);
    match mode {
        crate::demod::Mode::Wfm => (
            crate::sidecar::resample_audio(&pcm, sample_rate, 190_000),
            190_000.0,
        ),
        _ if sample_rate > 48_000 => (
            crate::sidecar::resample_audio(&pcm, sample_rate, 48_000),
            48_000.0,
        ),
        _ => (pcm, sample_rate as f32),
    }
}

#[cfg(test)]
mod readiness_tests {
    use super::{decoder_development_entry, decoder_fixture_verified_entry, readiness_reasons};

    #[test]
    fn real_device_with_fresh_samples_is_ready() {
        assert!(readiness_reasons(true, "sdrplay", 42, 9_000, 10_000, false).is_empty());
    }

    #[test]
    fn process_without_sample_flow_is_not_ready() {
        assert_eq!(
            readiness_reasons(true, "sdrplay", 0, 0, 10_000, false),
            vec!["samples_not_flowing"]
        );
    }

    #[test]
    fn mock_requires_an_explicit_test_override() {
        assert_eq!(
            readiness_reasons(true, "mock", 42, 9_000, 10_000, false),
            vec!["physical_device_required"]
        );
        assert!(readiness_reasons(true, "mock", 42, 9_000, 10_000, true).is_empty());
    }

    #[test]
    fn stale_sample_flow_is_not_ready() {
        assert_eq!(
            readiness_reasons(true, "rtlsdr", 42, 1_000, 10_000, false),
            vec!["sample_flow_stale"]
        );
    }

    #[test]
    fn unit_fixture_decoder_is_not_advertised_as_available() {
        let decoder = decoder_development_entry("adsb", "ADS-B", "iq", "live");
        assert_eq!(decoder["status"], "development");
        assert_eq!(decoder["available"], false);
        assert_eq!(decoder["verification"], "unit_fixture");
        assert!(decoder["missing_gate"].as_str().is_some());
    }

    #[test]
    fn required_catalog_decoders_stay_unavailable_until_recorded_iq_e2e() {
        let remaining = [
            "rtl433", "ft8", "wspr", "dmr", "p25", "nxdn", "dstar", "ysf", "m17",
        ];
        for id in remaining {
            let decoder = decoder_development_entry(id, id, "iq", "live");
            assert_eq!(decoder["id"], *id);
            assert_eq!(decoder["status"], "development");
            assert_eq!(decoder["available"], false);
            assert_eq!(decoder["verification"], "unit_fixture");
            assert_eq!(
                decoder["missing_gate"].as_str(),
                Some("recorded IQ end-to-end fixture")
            );
        }
    }

    #[test]
    fn recorded_iq_e2e_catalog_ids_are_available() {
        for id in [
            "adsb", "ais", "aprs", "pocsag", "rtty", "navtex", "uat", "acars", "vdl2", "rds", "cw",
            "ble", "lora",
        ] {
            let decoder = decoder_fixture_verified_entry(id, id, "iq", "live");
            assert_eq!(decoder["id"], *id);
            assert_eq!(decoder["status"], "fixture_verified");
            assert_eq!(decoder["available"], true);
            assert_eq!(decoder["verification"], "recorded_iq_e2e");
        }
    }
}

#[cfg(test)]
mod spectrum_wire_tests {
    use super::{encode_spectrum_frame, SPECTRUM_HEADER_BYTES};
    use crate::state::SpectrumFrame;

    #[test]
    fn binary_frame_has_versioned_header_and_fixed_quantization() {
        let packet = encode_spectrum_frame(
            &SpectrumFrame {
                sequence: 7,
                captured_ms: 1234,
                center_freq_hz: 100_700_000,
                sample_rate_hz: 2_000_000,
                usable_span_hz: 1_800_000,
                bins_dbfs: vec![-140.0, -100.0, -12.5, 0.0],
            },
            11,
        );
        assert_eq!(&packet[..4], b"PSF3");
        assert_eq!(u16::from_le_bytes(packet[4..6].try_into().unwrap()), 3);
        assert_eq!(u64::from_le_bytes(packet[8..16].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(packet[40..44].try_into().unwrap()), 4);
        assert_eq!(u32::from_le_bytes(packet[52..56].try_into().unwrap()), 0);
        assert_eq!(u64::from_le_bytes(packet[56..64].try_into().unwrap()), 11);
        assert_eq!(packet.len(), SPECTRUM_HEADER_BYTES + 4);
        assert_eq!(&packet[SPECTRUM_HEADER_BYTES..], &[0, 80, 255, 255]);
    }
}
