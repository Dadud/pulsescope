//! Recording API route composition. Handler contracts live in the parent module.

use super::*;

pub(super) fn router() -> Router<ApiState> {
    Router::new()
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
        .route("/iq_recording/start", post(iq_rec_start))
        .route("/iq_recording/stop", post(iq_rec_stop))
        .route("/iq_recording/status", get(iq_rec_status))
        .route("/transcription/start", post(transcription_start))
        .route("/transcription/stop", post(transcription_stop))
        .route("/transcription/status", get(transcription_status))
        .route("/transcription/transcripts", get(transcription_list))
        .route("/cases", get(cases).post(cases_new))
        .route("/cases/:id", get(case_one).delete(case_delete))
        .route("/cases/:id/attach", post(case_attach))
        .route(
            "/cases/attachments/:att_id",
            get(case_attachment_one).delete(case_attachment_delete),
        )
}
