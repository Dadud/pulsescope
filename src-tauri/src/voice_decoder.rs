//! Digital voice decoder sidecar (dsd-fme).
//!
//! Feeds demodulated 48kHz mono S16LE audio to dsd-fme via temp WAV file,
//! parses stdout for decoded call/nac/tg info, and persists results.
//!
//! dsd-fme supports: P25 Phase 1/2, DMR, NXDN48/96, D-STAR, YSF, M17, ProVoice, X2-TDMA.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const DSD_FME_DIRS: &[&str] = {
    // Can't do vec! in const, so use a match in the function
    &[]
};

/// Search common locations for dsd-fme.exe
pub fn find_dsd_fme() -> Option<PathBuf> {
    // Check env var first
    if let Ok(p) = std::env::var("PULSESCOPE_DSD_FME") {
        let pb = PathBuf::from(&p);
        if pb.exists() {
            return Some(pb);
        }
    }

    // Known locations
    let candidates: Vec<PathBuf> = if cfg!(windows) {
        vec![
            PathBuf::from(
                r"C:\Users\Dadud\pulsescope\decoders\dsd-fme\dsd-fme-portable\dsd-fme\dsd-fme.exe",
            ),
            PathBuf::from("dsd-fme.exe"),
        ]
    } else {
        vec![
            PathBuf::from("/usr/bin/dsd-fme"),
            PathBuf::from("/usr/local/bin/dsd-fme"),
            PathBuf::from("dsd-fme"),
        ]
    };

    // Check PothosSDR
    if let Ok(sdr_root) = std::env::var("SOAPY_SDR_ROOT") {
        let p = PathBuf::from(&sdr_root).join("bin").join(if cfg!(windows) {
            "dsd-fme.exe"
        } else {
            "dsd-fme"
        });
        if p.exists() {
            return Some(p);
        }
    }

    for c in &candidates {
        if c.exists() {
            return Some(c.clone());
        }
    }

    // PATH search
    which::which(if cfg!(windows) {
        "dsd-fme.exe"
    } else {
        "dsd-fme"
    })
    .ok()
}

/// Write demodulated audio samples (f32) as a 48kHz mono S16LE WAV file.
pub fn write_wav_48k(path: &PathBuf, samples: &[f32]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    // WAV header
    let num_samples = samples.len() as u32;
    let data_size = num_samples * 2;
    let file_size = 36 + data_size;

    file.write_all(b"RIFF")?;
    file.write_all(&file_size.to_le_bytes())?;
    file.write_all(b"WAVE")?;
    // fmt chunk
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?; // chunk size
    file.write_all(&1u16.to_le_bytes())?; // PCM
    file.write_all(&1u16.to_le_bytes())?; // mono
    file.write_all(&48000u32.to_le_bytes())?; // sample rate
    file.write_all(&96000u32.to_le_bytes())?; // byte rate (48k * 2)
    file.write_all(&2u16.to_le_bytes())?; // block align
    file.write_all(&16u16.to_le_bytes())?; // bits per sample
                                           // data chunk
    file.write_all(b"data")?;
    file.write_all(&data_size.to_le_bytes())?;

    // Write samples as S16LE
    let mut buf = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        buf.extend_from_slice(&v.to_le_bytes());
    }
    file.write_all(&buf)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DsdResult {
    pub available: bool,
    pub decoder_path: Option<String>,
    pub mode: String,
    pub frames_decoded: usize,
    pub calls: Vec<String>,
    pub talkgroups: Vec<String>,
    pub nacs: Vec<String>,
    pub errors: usize,
    pub raw_output: Vec<String>,
    pub error_message: Option<String>,
}

impl Default for DsdResult {
    fn default() -> Self {
        Self {
            available: false,
            decoder_path: None,
            mode: String::new(),
            frames_decoded: 0,
            calls: Vec::new(),
            talkgroups: Vec::new(),
            nacs: Vec::new(),
            errors: 0,
            raw_output: Vec::new(),
            error_message: None,
        }
    }
}

/// Run dsd-fme on demodulated audio samples.
/// `mode` can be: "auto", "p25p1", "p25p2", "dmr", "nxdn48", "nxdn96", "dstar", "ysf", "m17", "provoice"
pub fn decode_digital_voice(samples: &[f32], mode: &str) -> DsdResult {
    let exe =
        match find_dsd_fme() {
            Some(p) => p,
            None => return DsdResult {
                available: false,
                error_message: Some(
                    "dsd-fme not found. Install from https://github.com/lwvmobile/dsd-fme/releases"
                        .into(),
                ),
                ..Default::default()
            },
        };

    // Write temp WAV
    let temp_dir = std::env::temp_dir();
    let wav_path = temp_dir.join(format!("pulsescope_dsd_{}.wav", std::process::id()));

    if let Err(e) = write_wav_48k(&wav_path, samples) {
        return DsdResult {
            available: true,
            decoder_path: Some(exe.display().to_string()),
            error_message: Some(format!("Failed to write temp WAV: {e}")),
            ..Default::default()
        };
    }

    // Build mode flag
    let mode_flag = match mode.to_lowercase().as_str() {
        "auto" | "" => "-fa",
        "p25p1" | "p25" | "p25-1" => "-f1",
        "p25p2" | "p25-2" => "-f2",
        "dmr" => "-fs",
        "nxdn48" | "nxdn-48" | "idas" => "-fi",
        "nxdn96" | "nxdn-96" => "-fn",
        "dstar" => "-fd",
        "ysf" => "-fy",
        "m17" => "-fz",
        "provoice" => "-fp",
        _ => "-fa", // default auto
    };

    let wav_str = wav_path.to_string_lossy().to_string();

    let output = Command::new(&exe)
        .args([mode_flag, "-i", &wav_str, "-o", "/dev/null"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    // Clean up temp file
    let _ = std::fs::remove_file(&wav_path);

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            return DsdResult {
                available: true,
                decoder_path: Some(exe.display().to_string()),
                error_message: Some(format!("Failed to spawn dsd-fme: {e}")),
                ..Default::default()
            }
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{stdout}\n{stderr}");

    let mut result = DsdResult {
        available: true,
        decoder_path: Some(exe.display().to_string()),
        mode: mode.to_string(),
        ..Default::default()
    };

    // Parse output line by line
    for line in combined.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Track relevant output lines
        if trimmed.contains("Audio In Device")
            || trimmed.contains("Decoding")
            || trimmed.contains("End of")
            || trimmed.contains("Total audio errors")
            || trimmed.contains("Inbound")
            || trimmed.contains("Outbound")
            || trimmed.contains("VOICE")
            || trimmed.contains("Project 25")
            || trimmed.contains("DMR")
            || trimmed.contains("NXDN")
            || trimmed.contains("Call")
            || trimmed.contains("TG")
            || trimmed.contains("NAC")
            || trimmed.contains("RIC")
            || trimmed.contains("SYNC")
            || trimmed.contains("Slot")
        {
            result.raw_output.push(trimmed.to_string());
            result.frames_decoded += 1;
        }

        // Extract calls, TGs, NACs
        if trimmed.contains("Call:") || trimmed.contains("SRC:") {
            if let Some(call) = trimmed
                .split("Call:")
                .nth(1)
                .or_else(|| trimmed.split("SRC:").nth(1))
            {
                let call = call
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(',')
                    .to_string();
                if !call.is_empty() && !result.calls.contains(&call) {
                    result.calls.push(call);
                }
            }
        }
        if trimmed.contains("TG:") || trimmed.contains("TGID:") {
            if let Some(tg) = trimmed
                .split("TG:")
                .nth(1)
                .or_else(|| trimmed.split("TGID:").nth(1))
            {
                let tg = tg
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(',')
                    .to_string();
                if !tg.is_empty() && !result.talkgroups.contains(&tg) {
                    result.talkgroups.push(tg);
                }
            }
        }
        if trimmed.contains("NAC:") {
            if let Some(nac) = trimmed.split("NAC:").nth(1) {
                let nac = nac
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(',')
                    .to_string();
                if !nac.is_empty() && !result.nacs.contains(&nac) {
                    result.nacs.push(nac);
                }
            }
        }
        if trimmed.contains("audio errors") {
            if let Some(num) = trimmed
                .split("errors")
                .next()
                .and_then(|s| s.split_whitespace().last())
            {
                result.errors = num.parse().unwrap_or(0);
            }
        }
    }

    // Cap raw output
    if result.raw_output.len() > 50 {
        result.raw_output.truncate(50);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_writer_produces_valid_header() {
        let path = std::env::temp_dir().join("pulsescope_test_wav.wav");
        let samples = vec![0.0; 480]; // 10ms at 48kHz
        write_wav_48k(&path, &samples).expect("write");

        let data = std::fs::read(&path).expect("read");
        assert_eq!(&data[0..4], b"RIFF");
        assert_eq!(&data[8..12], b"WAVE");
        assert_eq!(&data[12..16], b"fmt ");
        assert_eq!(&data[22..24], &[1, 0]); // mono
        assert_eq!(&data[24..28], &[0x80, 0xBB, 0, 0]); // 48000 LE
        assert_eq!(&data[34..36], &[16, 0]); // 16-bit
        assert_eq!(&data[36..40], b"data");
        let data_size = u32::from_le_bytes([data[40], data[41], data[42], data[43]]);
        assert_eq!(data_size, 480 * 2); // 480 samples * 2 bytes
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn decode_handles_missing_decoder() {
        // Temporarily hide the decoder by setting a bogus env var
        std::env::set_var("PULSESCOPE_DSD_FME", "/nonexistent/dsd-fme");
        let result = decode_digital_voice(&[0.0; 100], "auto");
        // Should either find it elsewhere or report unavailable
        // Just verify it doesn't panic
        let _ = result;
        std::env::remove_var("PULSESCOPE_DSD_FME");
    }
}
