//! Dependency manager — discovers, downloads, and manages external decoder
//! sidecar binaries. Each decoder has a manifest entry describing where to
//! find it, what format it ships in, and how PulseScope should invoke it.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Config;

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
    // TetraEar: TETRA decoder with voice decoding
    DecoderManifest {
        name: "tetraear",
        exe_name: "TETRA_Decoder_Modern.exe",
        description: "TETRA digital trunked radio decoder with voice (GUI app)",
        github: Some(("syrex1013", "TetraEar")),
        search_dirs: &["", "tetraear"],
        input_type: InputType::Direct,
        protocol: "tetra",
        download_url: Some("https://github.com/syrex1013/TetraEar/releases/download/v2.2/TetraEar-v2.2-Windows-x64.zip"),
        extract_subdir: Some("tetraear"),
    },
    // iridiumlive: Iridium satellite burst decoder
    DecoderManifest {
        name: "iridiumlive",
        exe_name: "IridiumLive.exe",
        description: "Iridium satellite burst detector and demodulator (1616 MHz)",
        github: Some(("microp11", "iridiumlive")),
        search_dirs: &["", "windows-x64"],
        input_type: InputType::Direct,
        protocol: "iridium",
        download_url: Some("https://github.com/microp11/iridiumlive/releases/download/v1.3/windows-x64.zip"),
        extract_subdir: Some("iridiumlive/windows-x64"),
    },
];

/// Maps depmanager decoder names to `feature_packs` ids and config path fields.
const FEATURE_PACK_BINDINGS: [(&str, &str); 6] = [
    ("rtl_433", "rtl433"),
    ("multimon-ng", "digital"),
    ("acarsdec", "acars"),
    ("dumpvdl2", "vdl2"),
    ("direwolf", "aprs"),
    ("dsd-fme", "dsd"),
];

#[derive(Clone, Debug, serde::Serialize)]
pub struct ConfiguredDecoder {
    pub decoder: String,
    pub feature_pack_id: Option<String>,
    pub path: String,
    pub updated: bool,
}

fn feature_pack_for_decoder(name: &str) -> Option<&'static str> {
    FEATURE_PACK_BINDINGS
        .iter()
        .find(|(decoder, _)| *decoder == name)
        .map(|(_, pack)| *pack)
}

/// Feature-pack id for a depmanager decoder name (inverse of `feature_pack_for_decoder`).
pub fn decoder_for_pack(pack_id: &str) -> Option<&'static str> {
    FEATURE_PACK_BINDINGS
        .iter()
        .find(|(_, pack)| *pack == pack_id)
        .map(|(decoder, _)| *decoder)
}

pub fn manifest_for_decoder(name: &str) -> Option<&'static DecoderManifest> {
    KNOWN_DECODERS.iter().find(|decoder| decoder.name == name)
}

pub fn can_auto_install_decoder(name: &str) -> bool {
    manifest_for_decoder(name)
        .is_some_and(|decoder| decoder.download_url.is_some())
}

pub fn download_url_for_decoder(name: &str) -> Option<&'static str> {
    manifest_for_decoder(name).and_then(|decoder| decoder.download_url)
}

fn executable_candidates(exe_name: &str) -> Vec<String> {
    let base = exe_name.trim_end_matches(".exe");
    if cfg!(windows) {
        vec![format!("{}.exe", base), base.to_string()]
    } else {
        vec![base.to_string(), format!("{}.exe", base)]
    }
}

fn file_is_executable(path: &Path) -> bool {
    path.is_file() && {
        if cfg!(windows) {
            true
        } else {
            use std::os::unix::fs::PermissionsExt;
            path.metadata()
                .map(|meta| meta.permissions().mode() & 0o111 != 0)
                .unwrap_or(true)
        }
    }
}
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
    /// PulseScope can download and extract this decoder into the data directory.
    pub can_auto_install: bool,
    /// Feature-pack id when this decoder backs a normal UI pack.
    pub feature_pack_id: Option<String>,
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

    KNOWN_DECODERS
        .iter()
        .map(|decoder| {
            let (found, path, source) = find_decoder(decoder, data_dir, &pothos_bin);
            DecoderStatus {
                name: decoder.name.to_string(),
                description: decoder.description.to_string(),
                protocol: decoder.protocol.to_string(),
                found,
                path: path.as_ref().map(|p| p.to_string_lossy().to_string()),
                source,
                input_type: format!("{:?}", decoder.input_type),
                github_url: decoder
                    .github
                    .map(|(owner, repo)| format!("https://github.com/{owner}/{repo}")),
                install_url: decoder
                    .download_url
                    .map(str::to_string)
                    .or_else(|| {
                        decoder.github.map(|(owner, repo)| {
                            format!("https://github.com/{owner}/{repo}/releases/latest")
                        })
                    }),
                can_auto_install: decoder.download_url.is_some(),
                feature_pack_id: feature_pack_for_decoder(decoder.name).map(str::to_string),
            }
        })
        .collect()
}

fn find_decoder(
    decoder: &DecoderManifest,
    data_dir: &Path,
    pothos_bin: &Path,
) -> (bool, Option<PathBuf>, String) {
    let candidates = executable_candidates(decoder.exe_name);
    // 1. Data dir (downloaded decoders)
    if let Some(subdir) = decoder.extract_subdir {
        for exe_name in &candidates {
            let exe = data_dir
                .join("decoders")
                .join(subdir)
                .join(exe_name);
            if file_is_executable(&exe) {
                return (true, Some(exe), "pulsescope/decoders".into());
            }
        }
    }
    for subdir in decoder.search_dirs {
        let dir = if subdir.is_empty() {
            data_dir.join("decoders")
        } else {
            data_dir.join("decoders").join(subdir)
        };
        for exe_name in &candidates {
            let exe = dir.join(exe_name);
            if file_is_executable(&exe) {
                return (true, Some(exe), "pulsescope/decoders".into());
            }
        }
    }

    // 2. PothosSDR / SoapySDR bin directory (SOAPY_SDR_ROOT or platform default)
    for exe_name in &candidates {
        let exe = pothos_bin.join(exe_name);
        if file_is_executable(&exe) {
            return (true, Some(exe), "SoapySDR bin".into());
        }
    }

    // 3. Standard *nix system binaries, plus PATH.
    let standard_paths: &[&str] = if cfg!(windows) {
        &[r"C:\Program Files\rtl-sdr", r"C:\SDR"]
    } else {
        &["/usr/bin", "/usr/local/bin", "/opt/homebrew/bin"]
    };
    for dir in standard_paths {
        for exe_name in &candidates {
            let exe = std::path::PathBuf::from(dir).join(exe_name);
            if file_is_executable(&exe) {
                return (true, Some(exe), format!("{dir}/"));
            }
        }
    }

    // 4. PATH lookup via `where`/`which`.
    for exe_name in &candidates {
        let bare = exe_name.trim_end_matches(".exe");
        if let Ok(output) = Command::new(if cfg!(windows) { "where" } else { "which" })
            .arg(bare)
            .output()
        {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout);
                if let Some(first_line) = path_str.lines().next() {
                    let path = PathBuf::from(first_line.trim());
                    if file_is_executable(&path) {
                        return (true, Some(path), "PATH".into());
                    }
                }
            }
        }
    }

    (false, None, "not_found".into())
}

/// Write discovered decoder executables into config path fields.
pub fn configure_decoder_paths(config: &mut Config, data_dir: &Path) -> Vec<ConfiguredDecoder> {
    let mut results = Vec::new();
    for status in scan_all(data_dir) {
        if !status.found {
            continue;
        }
        let Some(path) = status.path else {
            continue;
        };
        let updated = apply_decoder_path(config, &status.name, &path);
        results.push(ConfiguredDecoder {
            decoder: status.name.clone(),
            feature_pack_id: status.feature_pack_id.clone(),
            path,
            updated,
        });
    }
    results
}

/// Apply one discovered or installed executable path to config.
pub fn apply_decoder_path(config: &mut Config, decoder_name: &str, path: &str) -> bool {
    match decoder_name {
        "rtl_433" => update_path(&mut config.rtl433.path, path),
        "multimon-ng" => update_path(&mut config.digital_decoder.multimon_path, path),
        "acarsdec" => update_path(&mut config.acarsdec.path, path),
        "direwolf" => update_path(&mut config.aprs.path, path),
        "dumpvdl2" => update_path(&mut config.vdl2.path, path),
        "dsd-fme" | "dsd-neo" => update_path(&mut config.dsd.dsdneo_path, path),
        "dump978" | "dump978-fa" => update_path(&mut config.dump978.path, path),
        _ => false,
    }
}

fn update_path(current: &mut String, path: &str) -> bool {
    if current == path {
        return false;
    }
    *current = path.to_string();
    true
}

/// Download, install when possible, and configure the matching config path.
pub fn install_decoder(name: &str, data_dir: &Path, config: &mut Config) -> Result<ConfiguredDecoder, String> {
    let path = if let Some(decoder) = KNOWN_DECODERS.iter().find(|d| d.name == name) {
        if decoder.download_url.is_some() {
            download_decoder(name, data_dir)?
        } else {
            let (found, discovered, _) = find_decoder(
                decoder,
                data_dir,
                &std::env::var("SOAPY_SDR_ROOT")
                    .ok()
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| {
                        if cfg!(windows) {
                            std::path::PathBuf::from(r"C:\Program Files\PothosSDR")
                        } else {
                            std::path::PathBuf::from("/usr/local")
                        }
                    })
                    .join("bin"),
            );
            if !found {
                return Err(format!(
                    "{name} is not bundled; install manually or choose a decoder with automatic download"
                ));
            }
            discovered
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default()
        }
    } else {
        return Err(format!("unknown decoder: {name}"));
    };
    let updated = apply_decoder_path(config, name, &path);
    Ok(ConfiguredDecoder {
        decoder: name.to_string(),
        feature_pack_id: feature_pack_for_decoder(name).map(str::to_string),
        path,
        updated,
    })
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

    if !url
        .to_ascii_lowercase()
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .ends_with(".zip")
    {
        return Err(format!(
            "unsupported decoder archive format for {name}: {url}"
        ));
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
            && path
                .file_name()
                .and_then(|name| name.to_str())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn touch_executable(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, b"").unwrap();
        if !cfg!(windows) {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn find_decoder_returns_not_found_when_missing() {
        let data_dir = std::env::temp_dir().join(format!(
            "pulsescope-depmanager-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&data_dir).unwrap();
        let decoder = manifest_for_decoder("rtl_433").expect("rtl_433 manifest");
        let pothos_bin = data_dir.join("empty-pothos-bin");
        std::fs::create_dir_all(&pothos_bin).unwrap();
        let (found, path, source) = find_decoder(decoder, &data_dir, &pothos_bin);
        assert!(!found);
        assert!(path.is_none());
        assert_eq!(source, "not_found");
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn configure_applies_discovered_multimon_path() {
        let data_dir = std::env::temp_dir().join(format!(
            "pulsescope-depmanager-configure-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&data_dir);
        let decoder = manifest_for_decoder("multimon-ng").expect("multimon-ng manifest");
        let subdir = decoder.extract_subdir.expect("multimon extract subdir");
        let exe_path = data_dir.join("decoders").join(subdir).join(decoder.exe_name);
        touch_executable(&exe_path);

        let mut config = Config::default();
        let results = configure_decoder_paths(&mut config, &data_dir);
        assert!(
            results
                .iter()
                .any(|entry| entry.decoder == "multimon-ng" && entry.updated),
            "expected multimon-ng to be configured: {:?}",
            results
        );
        assert_eq!(config.digital_decoder.multimon_path, exe_path.to_string_lossy());

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn feature_pack_bindings_are_consistent() {
        for (decoder, pack) in FEATURE_PACK_BINDINGS {
            assert_eq!(feature_pack_for_decoder(decoder), Some(pack));
            assert_eq!(decoder_for_pack(pack), Some(decoder));
        }
    }
}
