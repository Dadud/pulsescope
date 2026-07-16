//! Dependency manager — discovers, downloads, and manages external decoder
//! sidecar binaries. Each decoder has a manifest entry describing where to
//! find it, what format it ships in, and how PulseScope should invoke it.

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
    /// Direct download URL for auto-install (zip or single exe)
    pub download_url: Option<&'static str>,
    /// Subdirectory within the extracted archive where the exe lives
    pub extract_subdir: Option<&'static str>,
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
        download_url: None,
        extract_subdir: None,
    },
    // multimon-ng: c0ne fork ships pre-built Windows x64 binary
    DecoderManifest {
        name: "multimon-ng",
        exe_name: "multimon-ng.exe",
        description: "POCSAG512/1200/2400, FLEX, EAS, AFSK, ZVEI, DTMF, MORSE_CW from audio",
        github: Some(("EliasOenal", "multimon-ng")),
        search_dirs: &["", "bin", "multimon-ng-win"],
        input_type: InputType::StdinAudioS16,
        protocol: "pocsag",
        download_url: Some("https://github.com/c0ne/multimon-ng/raw/master/multimon-ng_1.1.8_x64.zip"),
        extract_subdir: Some("multimon-ng-win"),
    },
    // acarsdec: recovered from Nyx Scope MSI (v3.7), Thierry Leconte upstream
    DecoderManifest {
        name: "acarsdec",
        exe_name: "acarsdec.exe",
        description: "ACARS aircraft messaging (131 MHz)",
        github: Some(("TLeconte", "acarsdec")),
        search_dirs: &["", "bin", "acarsdec"],
        input_type: InputType::FileIq,
        protocol: "acars",
        download_url: Some("https://github.com/Dadud/pulsescope/releases/download/decoder-deps-v1/acarsdec.zip"),
        extract_subdir: Some("acarsdec"),
    },
    // direwolf: official wb2osz 1.8.1 x64 release
    DecoderManifest {
        name: "direwolf",
        exe_name: "direwolf.exe",
        description: "AX.25 / APRS packet TNC — 300/1200/2400/4800/9600 baud (144 MHz)",
        github: Some(("wb2osz", "direwolf")),
        search_dirs: &["", "bin", "direwolf", "direwolf/direwolf-1.8.1-a231971_x86_64"],
        input_type: InputType::StdinAudioS16,
        protocol: "aprs",
        download_url: Some("https://github.com/wb2osz/direwolf/releases/download/1.8.1/direwolf-1.8.1-a231971_x86_64.zip"),
        extract_subdir: Some("direwolf"),
    },
    // nrsc5: HD Radio / NRSC-5 decoder, Windows binary from LTCAshraven fork
    DecoderManifest {
        name: "nrsc5",
        exe_name: "nrsc5.exe",
        description: "HD Radio (NRSC-5) FM band IBOC decoder",
        github: Some(("theori-io", "nrsc5")),
        search_dirs: &["", "bin", "nrsc5"],
        input_type: InputType::FileIq,
        protocol: "hd_radio",
        download_url: Some("https://github.com/Dadud/pulsescope/releases/download/decoder-deps-v1/nrsc5.zip"),
        extract_subdir: Some("nrsc5"),
    },
    // dump978-fa: UAT 978 MHz, Cygwin build from ImagoTrigger
    DecoderManifest {
        name: "dump978",
        exe_name: "dump978-fa.exe",
        description: "UAT 978 MHz ADS-B aircraft tracking",
        github: Some(("mutability", "dump978")),
        search_dirs: &["", "bin", "dump978/adsb_uat_win-main/new/978-fa"],
        input_type: InputType::StdinU8Iq,
        protocol: "uat978",
        download_url: Some("https://github.com/ImagoTrigger/adsb_uat_win/archive/refs/heads/main.zip"),
        extract_subdir: Some("dump978/adsb_uat_win-main/new/978-fa"),
    },
    // dumpvdl2: VDL Mode 2, recovered from Nyx Scope MSI (v2.6.0)
    DecoderManifest {
        name: "dumpvdl2",
        exe_name: "dumpvdl2.exe",
        description: "VDL Mode 2 aircraft datalink (136 MHz)",
        github: Some(("szpajder", "dumpvdl2")),
        search_dirs: &["", "bin", "dumpvdl2"],
        input_type: InputType::FileIq,
        protocol: "vdl2",
        download_url: Some("https://github.com/Dadud/pulsescope/releases/download/decoder-deps-v1/dumpvdl2.zip"),
        extract_subdir: Some("dumpvdl2"),
    },
    // dump1090: ADS-B 1090 MHz, Windows-native from gvanem fork
    DecoderManifest {
        name: "dump1090",
        exe_name: "dump1090.exe",
        description: "1090 MHz ADS-B Mode-S receiver and decoder",
        github: Some(("gvanem", "Dump1090")),
        search_dirs: &["", "bin", "dump1090/Dump1090-main"],
        input_type: InputType::Direct,
        protocol: "adsb",
        download_url: Some("https://github.com/gvanem/Dump1090/archive/refs/heads/main.zip"),
        extract_subdir: Some("dump1090/Dump1090-main"),
    },
    // readsb: ADS-B 1090 MHz, Cygwin build from ImagoTrigger
    DecoderManifest {
        name: "readsb",
        exe_name: "readsb.exe",
        description: "ADS-B Mode-S/Beast receiver (Cygwin build, supports RTL-SDR)",
        github: Some(("wiedehopf", "readsb")),
        search_dirs: &["", "bin", "dump978/adsb_uat_win-main/new/readsb"],
        input_type: InputType::Direct,
        protocol: "adsb",
        download_url: Some("https://github.com/ImagoTrigger/adsb_uat_win/archive/refs/heads/main.zip"),
        extract_subdir: Some("dump978/adsb_uat_win-main/new/readsb"),
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
        download_url: Some("https://github.com/lwvmobile/dsd-fme/releases/download/20260715/dsd-fme-x86-64-cygwin-portable-20260715.zip"),
        extract_subdir: Some("dsd-fme/dsd-fme-portable"),
    },
    DecoderManifest {
        name: "AIS-catcher",
        exe_name: "AIS-catcher.exe",
        description: "AIS ship tracking (162 MHz)",
        github: Some(("jvde-github", "AIS-catcher")),
        search_dirs: &["", "bin"],
        input_type: InputType::Direct,
        protocol: "ais",
        download_url: Some("https://github.com/jvde-github/AIS-catcher/releases/download/v0.70/AIS-catcher.SDRPLAY.x64.zip"),
        extract_subdir: Some("AIS-catcher"),
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
        download_url: None,
        extract_subdir: None,
    },
    // SatDump: multi-satellite decoder (NOAA APT, GOES HRIT/LRIT, Meteor-M, Iridium, etc.)
    DecoderManifest {
        name: "satdump",
        exe_name: "satdump.exe",
        description: "Multi-satellite decoder: NOAA APT, GOES HRIT/LRIT, Meteor-M, Iridium, Inmarsat",
        github: Some(("SatDump", "SatDump")),
        search_dirs: &["", "satdump"],
        input_type: InputType::FileIq,
        protocol: "satellite",
        download_url: Some("https://github.com/SatDump/SatDump/releases/download/1.2.2/SatDump-Windows_x64_Portable.zip"),
        extract_subdir: Some("satdump"),
    },
    // noaa-apt: dedicated NOAA APT weather satellite image decoder
    DecoderManifest {
        name: "noaa-apt",
        exe_name: "noaa-apt-console.exe",
        description: "NOAA APT weather satellite image decoder (137 MHz, WAV input)",
        github: Some(("martinber", "noaa-apt")),
        search_dirs: &["", "noaa-apt"],
        input_type: InputType::StdinAudioS16,
        protocol: "noaa_apt",
        download_url: Some("https://github.com/martinber/noaa-apt/releases/download/v1.4.1/noaa-apt-1.4.1-x86_64-windows-gnu.zip"),
        extract_subdir: Some("noaa-apt"),
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
    if let Some(subdir) = decoder.extract_subdir {
        let exe = data_dir.join("decoders").join(subdir).join(decoder.exe_name);
        if exe.exists() { return (true, Some(exe), "pulsescope/decoders".into()); }
    }
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

/// Download and install a decoder archive into the PulseScope data directory.
///
/// Archives are extracted below the first component of `extract_subdir`. This
/// keeps each upstream archive isolated while preserving its internal layout.
pub fn download_decoder(name: &str, data_dir: &Path) -> Result<String, String> {
    let decoder = KNOWN_DECODERS
        .iter()
        .find(|decoder| decoder.name == name)
        .ok_or_else(|| format!("unknown decoder: {name}"))?;
    let url = decoder
        .download_url
        .ok_or_else(|| format!("automatic installation is not available for {name}"))?;
    let extract_subdir = decoder
        .extract_subdir
        .ok_or_else(|| format!("no extraction directory is configured for {name}"))?;

    if !url.to_ascii_lowercase().split(['?', '#']).next().unwrap_or(url).ends_with(".zip") {
        return Err(format!("unsupported decoder archive format for {name}: {url}"));
    }

    let response = reqwest::blocking::get(url)
        .map_err(|error| format!("failed to download {name}: {error}"))?
        .error_for_status()
        .map_err(|error| format!("failed to download {name}: {error}"))?;
    let bytes = response
        .bytes()
        .map_err(|error| format!("failed to read {name} download: {error}"))?;
    let reader = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|error| format!("invalid zip archive for {name}: {error}"))?;

    let archive_root = extract_subdir
        .split(['/', '\\'])
        .next()
        .filter(|component| !component.is_empty())
        .ok_or_else(|| format!("invalid extraction directory for {name}"))?;
    let destination = data_dir.join("decoders").join(archive_root);
    std::fs::create_dir_all(&destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("failed to read zip entry {index} for {name}: {error}"))?;
        let Some(relative_path) = entry.enclosed_name() else {
            return Err(format!("unsafe path in {name} archive: {}", entry.name()));
        };
        let output_path = destination.join(relative_path);
        if entry.is_dir() {
            std::fs::create_dir_all(&output_path)
                .map_err(|error| format!("failed to create {}: {error}", output_path.display()))?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        let mut output = std::fs::File::create(&output_path)
            .map_err(|error| format!("failed to create {}: {error}", output_path.display()))?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|error| format!("failed to extract {}: {error}", output_path.display()))?;
    }

    let expected_exe_path = data_dir
        .join("decoders")
        .join(extract_subdir)
        .join(decoder.exe_name);
    let exe_path = if expected_exe_path.is_file() {
        expected_exe_path
    } else {
        // Some release zips add a versioned top-level folder (for example,
        // direwolf-1.8.1-...). Accept that packaging detail while still
        // requiring the manifest's exact executable to be present.
        find_executable(&destination, decoder.exe_name).ok_or_else(|| {
            format!(
                "downloaded {name}, but {} was not found after extraction",
                decoder.exe_name
            )
        })?
    };
    Ok(exe_path.to_string_lossy().into_owned())
}

fn find_executable(directory: &Path, exe_name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(directory).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path.file_name().and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(exe_name))
        {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = find_executable(&path, exe_name) {
                return Some(found);
            }
        }
    }
    None
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
