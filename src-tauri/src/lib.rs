// lib.rs — PulseScope library entry point.
//
// The binary target is `src/main.rs`; this file exposes the library
// crate so the Tauri macro and integration tests can reach the modules.

pub mod adsb;
pub mod ais;
pub mod api;
pub mod arrl_bandplan;
pub mod auto_decode;
pub mod aprs;
pub mod audio;
pub mod aviation;
pub mod ble;
pub mod capture;
pub mod config;
pub mod db;
pub mod decoder_fixtures;
pub mod decoder_manifest;
pub mod decoder_scheduler;
pub mod demod;
pub mod depmanager;
pub mod device;
pub mod hd_radio;
pub mod lora;
pub mod operations;
pub mod paths;
pub mod pocsag;
pub mod protocols;
pub mod scanner;
pub mod sidecar;
pub mod signal_id;
pub mod sstv;
pub mod state;
pub mod transcription;
pub mod trunking;
pub mod voice_decoder;
pub mod webrtc;
