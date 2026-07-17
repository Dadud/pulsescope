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
mod adsb;
mod pocsag;
mod ais;
mod aviation;
mod voice_decoder;
mod signal_id;
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

    // Bundled SDR runtime — ships with the app, no PothosSDR install needed
    setup_sdr_runtime();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let server_mode = args.iter().any(|a| a == "--server" || a == "--api") || std::env::var("PULSESCOPE_API_ONLY").is_ok();
    // A LAN/headless server must not open physical speakers merely because a
    // remote browser starts a scan. Desktop mode retains local audio; server
    // audio is opt-in for explicit lab use.
    if server_mode && std::env::var("PULSESCOPE_AUDIO_OUTPUT").is_err() {
        std::env::set_var("PULSESCOPE_AUDIO_OUTPUT", "0");
    }
    let app_state = AppState::new();
    app_state.start_job_scheduler();
    let desktop_mode = !server_mode;
    // Both desktop and LAN open onto a visible, muted monitor. Server mode
    // still hard-disables CPAL output above, so this cannot start speaker noise.
    app_state.start_default_monitor();
    let port: u16 = std::env::var("PULSESCOPE_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8765);
    let bind: String = std::env::var("PULSESCOPE_BIND").unwrap_or_else(|_| "127.0.0.1".to_string());

    let auth_token = std::env::var("PULSESCOPE_AUTH_TOKEN").ok();
    let ui_dir = if server_mode { static_ui_dir() } else { None };
    let tls = load_tls_from_env();
    let has_auth = auth_token.is_some();
    let has_ui = ui_dir.is_some();

    if server_mode {
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
            if let Err(e) = crate::api::serve(cfg, app_clone).await {
                tracing::error!(error = %e, "API server error");
            }
        });
    }

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
    // Desktop and LAN deliberately serve the same Svelte build. A separate
    // LAN skin guarantees feature drift and is retained only as a diagnostic
    // artifact, never as the production UI.
    let candidates = [
        PathBuf::from("./ui/build"),
        PathBuf::from("../ui/build"),
        PathBuf::from("./build"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

/// Locate the bundled SDR runtime directory.
///
/// PulseScope ships `sdr-runtime/` next to the binary containing SoapySDR.dll,
/// all device module DLLs (RTL-SDR, Airspy, HackRF, bladeRF, LimeSDR, SDRplay,
/// UHD, etc.), driver DLLs, and rtl_433/rtl_adsb. This eliminates the need for
/// users to install PothosSDR separately.
///
/// In dev mode the directory is at `<workspace>/sdr-runtime/`. In a packaged
/// install it's next to the .exe.
fn bundled_sdr_root() -> Option<PathBuf> {
    // 1. Explicit override
    if let Ok(p) = std::env::var("PULSESCOPE_SDR_ROOT") {
        let pb = PathBuf::from(p);
        if pb.join("bin/SoapySDR.dll").exists() || pb.join("SoapySDR.dll").exists() {
            return Some(pb);
        }
    }
    // 2. Next to the running executable (packaged install)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("sdr-runtime");
            if candidate.join("bin/SoapySDR.dll").exists() {
                return Some(candidate);
            }
            // Also check without bin/ prefix
            if candidate.join("SoapySDR.dll").exists() {
                return Some(candidate);
            }
        }
    }
    // 3. Relative to CWD (dev mode)
    for rel in &["./sdr-runtime", "../sdr-runtime", "../../sdr-runtime"] {
        let candidate = PathBuf::from(rel);
        if candidate.join("bin/SoapySDR.dll").exists() || candidate.join("SoapySDR.dll").exists() {
            return Some(candidate);
        }
    }
    // 4. Fall back to system PothosSDR if installed
    if std::path::Path::new(r"C:\Program Files\PothosSDR\bin\SoapySDR.dll").exists() {
        return Some(PathBuf::from(r"C:\Program Files\PothosSDR"));
    }
    None
}

/// Set up environment variables so SoapySDR finds the bundled runtime.
/// Called at startup before any SDR operations.
fn setup_sdr_runtime() {
    if let Some(root) = bundled_sdr_root() {
        let bin = if root.join("bin").exists() { root.join("bin") } else { root.clone() };

        // Point SoapySDR at the bundled root
        if std::env::var("SOAPY_SDR_ROOT").is_err() {
            std::env::set_var("SOAPY_SDR_ROOT", &root);
            tracing::info!(sdr_root = %root.display(), "using bundled SDR runtime");
        }

        // Explicitly register bundled Soapy modules. This matters on Windows:
        // the core DLL can load while the module directory remains invisible.
        let modules = root.join("lib/SoapySDR/modules0.8");
        if modules.exists() { std::env::set_var("SOAPY_SDR_MODULE_PATH", &modules); }

        // SDRplay API is an external prerequisite for RSP1B/RSP2. Keep the
        // bundled runtime self-contained for other hardware, but add the
        // installed API directory when present so sdrPlaySupport.dll loads.
        let sdrplay_api = std::path::PathBuf::from(r"C:\Program Files\SDRplay\API\x64");
        let mut extra_path = bin.display().to_string();
        if sdrplay_api.exists() { extra_path.push(';'); extra_path.push_str(&sdrplay_api.display().to_string()); }

        // Prepend runtime DLL paths to PATH.
        if let Ok(path) = std::env::var("PATH") {
            let new_path = format!("{};{}", extra_path, path);
            std::env::set_var("PATH", &new_path);
        }

        // Set PKG_CONFIG_PATH for build-time discovery (harmless at runtime)
        let pkgconfig = root.join("lib/pkgconfig");
        if pkgconfig.exists() {
            std::env::set_var("PKG_CONFIG_PATH", &pkgconfig);
        }
    } else {
        tracing::warn!("No SDR runtime found — install PothosSDR or bundle sdr-runtime/");
    }
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
