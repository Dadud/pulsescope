// lib.rs — PulseScope library entry point.
//
// The binary target is `src/main.rs`; this file exposes the library
// crate so the Tauri macro and integration tests can reach the modules.

pub mod api;
pub mod audio;
pub mod capture;
pub mod config;
pub mod db;
pub mod demod;
pub mod pocsag;
pub mod device;
pub mod scanner;
pub mod sidecar;
pub mod depmanager;
pub mod aprs;
pub mod adsb;
pub mod ais;
pub mod aviation;
pub mod voice_decoder;
pub mod signal_id;
pub mod state;
