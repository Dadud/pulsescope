//! Dependency manager — discovers, downloads, and manages external decoder
//! sidecar binaries. Each decoder has a manifest entry describing where to
//! find it, what format it ships in, and how PulseScope should invoke it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A decoder tool descriptor.
#[derive(Clone, Debug)]
pub struct DecoderManifest {
    pub name: &'static str,
    pub exe_name: &'static str,
    pub description: &'static str,
    /// GitHub release source: (owner, repo)
    pub github: Option<(&'static str, &'static str)>,
    /// Search paths relative to common roots (PothosSDR, data_dir, PATH)
    pub search_dirs: &'static [&'static str],
    /// Arguments template. {rate} and {freq} are substituted at spawn time.
    /// Input is via stdin unless noted.
    pub input_type: InputType,
    pub protocol: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputType {
    /// Reads interleaved u8 IQ from stdin (`-r -`)
    StdinU8Iq,
    /// Reads raw audio samples from stdin (s16le)
    StdinAudioS16,
    /// Reads from a TCP socket (rtl_tcp style)
    TcpSocket,
    /// File-based IQ input
    FileIq,
    /// Needs a real soundcard / SDR API directly (can't be piped)
    Direct,
}

pub const KNOWN_DECODERS: &[DecoderManifest] = &[
    DecoderManifest {
        name: "rtl_433",
        exe_name: "rtl_433.exe",
        description: "433/868/915 MHz ISM band sensors (weather, TPMS, remotes)",
        github: Some(("merbanan", "rtl_433")),
        search_dirs: &["", "bin"],
        input_type: InputType::StdinU8Iq,
        protocol: "rtl_433",
    },
    // multimon-ng: modern community fork ships Windows binaries
    DecoderManifest {
        name: "multimon-ng",
        exe_name: "multimon-ng.exe",
        description: "POCSAG, FLEX, AFSK, ZVEI, DTMF, EAS, Flex from audio",
        github: Some(("ZanoroyAo", "multimon-ng")),
        search_dirs: &["", "bin"],
        input_type: InputType::StdinAudioS16,
        protocol: "pocsag",
    },
    // acarsdec: Eddware builds Windows binaries from TLeconte source
    DecoderManifest {
        name: "acarsdec",
        exe_name: "acarsdec.exe",
        description: "ACARS aircraft messaging (131 MHz)",
        github: Some(("Eddware", "acarsdec-windows")),
        search_dirs: &["", "bin"],
        input_type: InputType::StdinU8Iq,
        protocol: "acars",
    },
    // direwolf: official wb2osz repo ships x86_64 Windows zip
    DecoderManifest {
        name: "direwolf",
        exe_name: "direwolf.exe",
        description: "AX.25 / APRS packet TNC (144 MHz)",
        github: Some(("wb2osz", "direwolf")),
        search_dirs: &["", "bin"],
        input_type: InputType::StdinAudioS16,
        protocol: "aprs",
    },
    // nrsc5: no Windows release official; build from source
    DecoderManifest {
        name: "nrsc5",
        exe_name: "nrsc5.exe",
        description: "HD Radio (NRSC-5) FM band IBOC decoder",
        github: Some(("theori-io", "nrsc5")),
        search_dirs: &["", "bin"],
        input_type: InputType::FileIq,
        protocol: "hd_radio",
    },
    // dump978: mutability ships Linux-only; community WinBuild exists
    DecoderManifest {
        name: "dump978",
        exe_name: "dump978.exe",
        description: "UAT 978 MHz ADS-B aircraft tracking",
        github: Some(("mutability", "dump978")),
        search_dirs: &["", "bin"],
        input_type: InputType::StdinU8Iq,
        protocol: "uat978",
    },
    // dumpvdl2: jacketizer Win build
    DecoderManifest {
        name: "dumpvdl2",
        exe_name: "dumpvdl2.exe",
        description: "VDL Mode 2 aircraft datalink (136 MHz)",
        github: Some(("szpajder", "dumpvdl2")),
        search_dirs: &["", "bin"],
        input_type: InputType::StdinU8Iq,
        protocol: "vdl2",
    },
    // dsd-fme: lwvmobile fork ships Windows Cygwin builds with all vocoders
    DecoderManifest {
        name: "dsd-fme",
        exe_name: "dsd-fme.exe",
        description: "P25/DMR/NXDN/D-STAR/YSF/M17 digital voice",
        github: Some(("lwvmobile", "dsd-fme")),
        search_dirs: &["", "bin", "dsd-fme-portable/dsd-fme"],
        input_type: InputType::StdinAudioS16,
        protocol: "p25",
    },
    DecoderManifest {
        name: "AIS-catcher",
        exe_name: "AIS-catcher.exe",
        description: "AIS ship tracking (162 MHz)",
        github: Some(("jvde-github", "AIS-catcher")),
        search_dirs: &["", "bin"],
        input_type: InputType::Direct,
        protocol: "ais",
    },
    // rtl_adsb is bundled with PothosSDR — no install needed
    DecoderManifest {
        name: "rtl_adsb",
        exe_name: "rtl_adsb.exe",
        description: "1090 MHz ADS-B aircraft tracking (bundled with PothosSDR)",
        github: None,
        search_dirs: &["bin"],
        input_type: InputType::StdinU8Iq,
        protocol: "adsb",
    },
];

/// Result of probing for a decoder.
#[derive(Clone, Debug, serde::Serialize)]
pub struct DecoderStatus {
    pub name: String,
    pub description: String,
    pub protocol: String,
    pub found: bool,
    pub path: Option<String>,
    pub source: String,
    pub input_type: String,
    pub github_url: Option<String>,
    pub install_url: Option<String>,
}

/// Scan all known decoder search locations and return status for each.
pub fn scan_all(data_dir: &Path) -> Vec<DecoderStatus> {
    // Cross-platform bin directory of the system SDR/Soapy install.
    let default_root = std::env::var("SOAPY_SDR_ROOT")
        .ok()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(windows) {
                std::path::PathBuf::from(r"C:\Program Files\PothosSDR")
            // last-ditch PothosSDR install
            } else {
                std::path::PathBuf::from("/usr/local")
            }
        });
    let pothos_bin = default_root.join("bin");

    KNOWN_DECODERS.iter().map(|decoder| {
        let (found, path, source) = find_decoder(decoder, data_dir, &pothos_bin);
        DecoderStatus {
            name: decoder.name.to_string(),
            description: decoder.description.to_string(),
            protocol: decoder.protocol.to_string(),
            found,
            path: path.as_ref().map(|p| p.to_string_lossy().to_string()),
            source,
            input_type: format!("{:?}", decoder.input_type),
            github_url: decoder.github.map(|(owner, repo)| format!("https://github.com/{owner}/{repo}")),
            install_url: decoder.github.map(|(owner, repo)| format!("https://github.com/{owner}/{repo}/releases/latest")),
        }
    }).collect()
}

fn find_decoder(decoder: &DecoderManifest, data_dir: &Path, pothos_bin: &Path) -> (bool, Option<PathBuf>, String) {
    // 1. Data dir (downloaded decoders)
    for subdir in decoder.search_dirs {
        let dir = if subdir.is_empty() { data_dir.join("decoders") } else { data_dir.join("decoders").join(subdir) };
        let exe = dir.join(decoder.exe_name);
        if exe.exists() { return (true, Some(exe), "pulsescope/decoders".into()); }
    }

    // 2. PothosSDR / SoapySDR bin directory (SOAPY_SDR_ROOT or platform default)
    let exe = pothos_bin.join(decoder.exe_name);
    if exe.exists() { return (true, Some(exe), "SoapySDR bin".into()); }

    // 3. Standard *nix system binaries, plus PATH.
    let standard_paths: &[&str] = if cfg!(windows) {
        &[
            r"C:\Program Files\rtl-sdr",
            r"C:\SDR",
        ]
    } else {
        &[
            "/usr/bin",
            "/usr/local/bin",
            "/opt/homebrew/bin",
        ]
    };
    for dir in standard_paths {
        let exe = std::path::PathBuf::from(dir).join(decoder.exe_name);
        if exe.exists() { return (true, Some(exe), format!("{dir}/")); }
    }

    // 4. PATH lookup via `where`/`which`.
    let bare = decoder.exe_name.trim_end_matches(".exe");
    if let Ok(output) = Command::new(if cfg!(windows) { "where" } else { "which" }).arg(bare).output() {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout);
            if let Some(first_line) = path_str.lines().next() {
                let path = PathBuf::from(first_line.trim());
                if path.exists() { return (true, Some(path), "PATH".into()); }
            }
        }
    }

    (true, None, "not_found".into())
}

/// Generate download instructions for a decoder.
pub fn install_instructions(name: &str) -> Option<String> {
    let decoder = KNOWN_DECODERS.iter().find(|d| d.name == name)?;
    let (owner, repo) = decoder.github?;
    Some(format!(
        "Download from https://github.com/{owner}/{repo}/releases/latest\n\
         Extract the .exe to: <pulsescope_data>/decoders/\n\
         Input type: {:?}\n\
         Protocol: {}",
        decoder.input_type, decoder.protocol
    ))
}
