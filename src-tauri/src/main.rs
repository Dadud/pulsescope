// -*- mode: rust -*-
//
// PulseScope — open-source SDR scanner, MIT-licensed core.
//
// Modes of operation:
//   1. `--server`           headless HTTP+WS API + bundled UI on port (default 8765)
//   2. `PULSESCOPE_API_ONLY=1` legacy alias for `--server`
//   3. `--tauri` (default)  desktop Tauri app shell that runs the same API locally
//
// The same `crate::api` serves both modes, the only difference is whether `tower_http`
// also serves the SvelteKit `ui/build/` static bundle and whether the Tauri webview
// is launched.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod audio;
mod capture;
mod config;
mod db;
mod demod;
mod depmanager;
mod device;
mod scanner;
mod sidecar;
mod aprs;
mod voice_decoder;
mod state;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use futures_util::future::pending;
use tracing_subscriber::EnvFilter;

use crate::api::ServeConfig;
use crate::state::AppState;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("PulseScope starting up");

    let app_state = AppState::new();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let server_mode = args.iter().any(|a| a == "--server" || a == "--api") || std::env::var("PULSESCOPE_API_ONLY").is_ok();
    let desktop_mode = !server_mode;
    let port: u16 = std::env::var("PULSESCOPE_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8765);
    let bind: String = std::env::var("PULSESCOPE_BIND").unwrap_or_else(|_| "127.0.0.1".to_string());

    let auth_token = std::env::var("PULSESCOPE_AUTH_TOKEN").ok();
    let ui_dir = if server_mode { static_ui_dir() } else { None };
    let tls = load_tls_from_env();
    let has_auth = auth_token.is_some();
    let has_ui = ui_dir.is_some();

    let cfg = ServeConfig {
        addr: SocketAddr::from((
            bind.parse::<std::net::IpAddr>().unwrap_or(std::net::IpAddr::from([127, 0, 0, 1])),
            port,
        )),
        ui_dir,
        auth_token,
        tls,
    };

    let app_clone: Arc<AppState> = app_state.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::api::serve(cfg, app_clone.clone()).await {
            tracing::error!(error = %e, "API server error");
        }
    });

    tracing::info!(
        bind = %bind,
        port,
        mode = if server_mode { "server" } else { "desktop" },
        auth = has_auth,
        static_ui = has_ui,
        "PulseScope ready"
    );

    if desktop_mode {
        tauri::run(app_state);
    } else {
        // Server mode: keep the binary alive, never returns.
        let _: () = pending().await;
    }
}

fn static_ui_dir() -> Option<PathBuf> {
    // Production layout: the Svelte build lives next to the binary at `../ui/build/`
    // or under `ui/build/` of the workspace. Allow override.
    if let Ok(p) = std::env::var("PULSESCOPE_UI_DIR") {
        return Some(PathBuf::from(p));
    }
    let candidates = [
        PathBuf::from("./ui/build"),
        PathBuf::from("../ui/build"),
        PathBuf::from("./build"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

/// Load PEM certificate chain and private key from environment variables when
/// both are present. Returns `None` if either is missing (run-time TLS is opt-in).
fn load_tls_from_env() -> Option<crate::api::TlsConfig> {
    let cert = std::env::var("PULSESCOPE_TLS_CERT").ok()
        .map(std::path::PathBuf::from)
        .and_then(|p| std::fs::read(&p).ok());
    let key = std::env::var("PULSESCOPE_TLS_KEY").ok()
        .map(std::path::PathBuf::from)
        .and_then(|p| std::fs::read(&p).ok());
    match (cert, key) {
        (Some(certificate_chain_pem), Some(private_key_pem)) => Some(crate::api::TlsConfig { certificate_chain_pem, private_key_pem }),
        (None, None) => None,
        _ => {
            tracing::warn!("both PULSESCOPE_TLS_CERT and PULSESCOPE_TLS_KEY must be set; TLS disabled");
            None
        }
    }
}

// -------- desktop-only Tauri shell --------
//
// Kept under a cfg so server builds can drop the Tauri dependency if desired.
#[cfg(not(feature = "headless"))]
mod tauri {
    use std::sync::Arc;
    use super::AppState;
    use tauri::Manager;

    pub fn run(state: Arc<AppState>) {
        tauri::Builder::default()
            .manage(state)
            .setup(|app| {
                let state = app.state::<Arc<AppState>>().inner().clone();
                let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 8765));
                let cfg = crate::api::ServeConfig { addr, ui_dir: None, auth_token: None, tls: None };
                tokio::spawn(async move {
                    if let Err(e) = crate::api::serve(cfg, state).await {
                        tracing::error!(error = %e, "Tauri API server error");
                    }
                });
                Ok(())
            })
            .invoke_handler(tauri::generate_handler![])
            .run(tauri::generate_context!())
            .expect("Tauri runtime error");
    }
}

#[cfg(feature = "headless")]
mod tauri {
    use std::sync::Arc;
    use super::AppState;
    pub fn run(_state: Arc<AppState>) {
        tracing::info!("headless feature: tauri shell disabled");
    }
}
