// lib.rs — PulseScope library entry point.
//
// The binary target is `src/main.rs`; this file exposes the library
// crate so the Tauri macro and integration tests can reach the modules.

pub mod adsb;
pub mod ais;
pub mod api;
pub mod aprs;
pub mod audio;
pub mod aviation;
pub mod capture;
pub mod config;
pub mod db;
pub mod demod;
pub mod depmanager;
pub mod device;
pub mod pocsag;
pub mod scanner;
pub mod security;
pub mod sidecar;
pub mod signal_id;
pub mod state;
pub mod voice_decoder;
