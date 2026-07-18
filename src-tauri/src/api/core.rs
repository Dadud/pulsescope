//! Core API route composition. Handler contracts live in the parent module.

use super::*;

pub(super) fn router() -> Router<ApiState> {
    Router::new()
        .route("/health", get(health))
        .route("/settings", get(get_settings).put(put_settings))
        .route("/digital_voice/check", get(digital_voice_check))
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
        .route("/instances", get(instances))
        .route("/reconnect", post(reconnect))
        .route("/close", post(close_session))
        .route("/slots", get(slots))
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
        .route("/event-stream", get(event_stream))
        .route("/events", get(events_ws))
}
