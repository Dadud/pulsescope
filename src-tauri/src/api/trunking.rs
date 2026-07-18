//! Trunking API route composition. Handler contracts live in the parent module.

use super::*;

pub(super) fn router() -> Router<ApiState> {
    Router::new()
        .route("/talkgroups", get(talkgroups).post(talkgroup_update))
        .route("/talkgroups/systems", get(talkgroup_systems))
        .route("/talkgroups/import", post(talkgroup_import))
        .route("/talkgroups/export", get(talkgroup_export))
        .route("/talkgroups/update", post(talkgroup_update))
        .route("/talkgroups/delete-system", post(talkgroup_delete_system))
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
}
