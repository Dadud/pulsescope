//! Device API route composition. Handler contracts live in the parent module.

use super::*;

pub(super) fn router() -> Router<ApiState> {
    Router::new()
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
        .route("/receiver_location", get(rx_location).put(rx_location_put))
}
