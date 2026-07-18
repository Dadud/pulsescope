//! Authentication primitives shared by the HTTP and WebSocket transports.
//!
//! Long-lived tokens are deliberately accepted only in the Authorization
//! header.  Browser WebSockets exchange that credential for a single-use,
//! short-lived ticket first, keeping credentials out of URLs and proxy logs.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{extract::Extension, http::StatusCode, response::IntoResponse, Json};
use rand::{distributions::Alphanumeric, Rng};
use serde_json::json;
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct SecurityState(Arc<Mutex<SecurityInner>>);

struct SecurityInner {
    tokens: Vec<Vec<u8>>,
    tickets: HashMap<String, Instant>,
}

impl SecurityState {
    pub fn new(token: Option<String>) -> Self {
        Self(Arc::new(Mutex::new(SecurityInner {
            tokens: token
                .filter(|v| !v.is_empty())
                .map(|v| vec![v.into_bytes()])
                .unwrap_or_default(),
            tickets: HashMap::new(),
        })))
    }

    pub fn enabled(&self) -> bool {
        !self.0.lock().expect("security lock").tokens.is_empty()
    }

    pub fn authenticate(&self, candidate: &str) -> bool {
        let inner = self.0.lock().expect("security lock");
        inner.tokens.iter().any(|expected| {
            expected.len() == candidate.len()
                && expected.as_slice().ct_eq(candidate.as_bytes()).into()
        })
    }

    pub fn issue_ticket(&self) -> String {
        let ticket: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(48)
            .map(char::from)
            .collect();
        let mut inner = self.0.lock().expect("security lock");
        inner.tickets.retain(|_, expiry| *expiry > Instant::now());
        inner
            .tickets
            .insert(ticket.clone(), Instant::now() + Duration::from_secs(30));
        ticket
    }

    /// Consumes a ticket. Tickets cannot be replayed, even inside their TTL.
    pub fn consume_ticket(&self, candidate: &str) -> bool {
        let mut inner = self.0.lock().expect("security lock");
        inner
            .tickets
            .remove(candidate)
            .is_some_and(|expiry| expiry > Instant::now())
    }

    pub fn rotate(&self) -> String {
        let token: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(64)
            .map(char::from)
            .collect();
        self.0.lock().expect("security lock").tokens = vec![token.as_bytes().to_vec()];
        tracing::info!(target: "security_audit", action = "token.rotate", outcome = "success");
        token
    }

    pub fn revoke_all(&self) {
        let mut inner = self.0.lock().expect("security lock");
        inner.tokens.clear();
        inner.tickets.clear();
        tracing::info!(target: "security_audit", action = "token.revoke_all", outcome = "success");
    }
}

pub async fn ws_ticket(Extension(security): Extension<SecurityState>) -> impl IntoResponse {
    let ticket = security.issue_ticket();
    tracing::info!(target: "security_audit", action = "websocket.ticket.issue", outcome = "success");
    (
        StatusCode::CREATED,
        Json(json!({ "ticket": ticket, "expires_in": 30 })),
    )
}

pub async fn rotate_token(Extension(security): Extension<SecurityState>) -> impl IntoResponse {
    let token = security.rotate();
    (StatusCode::OK, Json(json!({ "token": token })))
}

pub async fn revoke_tokens(Extension(security): Extension<SecurityState>) -> impl IntoResponse {
    security.revoke_all();
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_match_and_rotate() {
        let state = SecurityState::new(Some("old-secret".into()));
        assert!(state.authenticate("old-secret"));
        assert!(!state.authenticate("old-secreu"));
        let fresh = state.rotate();
        assert!(!state.authenticate("old-secret"));
        assert!(state.authenticate(&fresh));
    }

    #[test]
    fn tickets_are_single_use() {
        let state = SecurityState::new(Some("secret".into()));
        let ticket = state.issue_ticket();
        assert!(state.consume_ticket(&ticket));
        assert!(!state.consume_ticket(&ticket));
    }
}
