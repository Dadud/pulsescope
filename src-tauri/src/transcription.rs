//! Local speech transcription transport (whisper.cpp / whisper-cli).
//!
//! PCM is resampled to 16 kHz mono. The engine is optional; when it is not
//! installed the API stays `available: false` with an install hint. This never
//! claims catalog availability from unit tests alone.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::demod::resample_linear;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TranscriptionRuntime {
    pub running: bool,
    pub last_error: Option<String>,
    pub transcripts: Vec<TranscriptSegment>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub timestamp_ms: i64,
    pub start_s: f32,
    pub end_s: f32,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranscriptionEngine {
    pub available: bool,
    pub path: Option<String>,
    pub kind: String,
    pub install_hint: String,
}

pub fn find_engine() -> TranscriptionEngine {
    let hint = "Install whisper.cpp (whisper-cli) and place it on PATH, or set PULSESCOPE_WHISPER to the binary.".to_string();
    if let Ok(p) = std::env::var("PULSESCOPE_WHISPER") {
        let path = PathBuf::from(&p);
        if path.exists() {
            return TranscriptionEngine {
                available: true,
                path: Some(path.display().to_string()),
                kind: "whisper".into(),
                install_hint: hint,
            };
        }
    }
    for name in ["whisper-cli", "whisper", "main"] {
        if let Ok(path) = which::which(name) {
            let kind = if name == "main" {
                "whisper.cpp-main"
            } else {
                "whisper"
            };
            return TranscriptionEngine {
                available: true,
                path: Some(path.display().to_string()),
                kind: kind.into(),
                install_hint: hint,
            };
        }
    }
    TranscriptionEngine {
        available: false,
        path: None,
        kind: "none".into(),
        install_hint: hint,
    }
}

pub fn resample_to_16k(samples: &[f32], sample_rate_hz: u32) -> Vec<f32> {
    if sample_rate_hz == 0 {
        return Vec::new();
    }
    if sample_rate_hz == 16_000 {
        return samples.to_vec();
    }
    resample_linear(samples, sample_rate_hz, 16_000)
}

pub fn parse_whisper_stdout(text: &str) -> Vec<TranscriptSegment> {
    let now = crate::scanner::now_ms();
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(segment) = parse_timestamped_line(trimmed, now) {
            out.push(segment);
        } else if !trimmed.starts_with("whisper_")
            && !trimmed.starts_with("system_")
            && !trimmed.contains("loading")
        {
            out.push(TranscriptSegment {
                timestamp_ms: now,
                start_s: 0.0,
                end_s: 0.0,
                text: trimmed.to_string(),
            });
        }
    }
    out
}

fn parse_timestamped_line(line: &str, now: i64) -> Option<TranscriptSegment> {
    // [00:00:00.000 --> 00:00:05.200]  hello
    let rest = line.strip_prefix('[')?;
    let (times, text) = rest.split_once(']')?;
    let (start, end) = times.split_once("-->")?;
    Some(TranscriptSegment {
        timestamp_ms: now,
        start_s: parse_hms(start.trim())?,
        end_s: parse_hms(end.trim())?,
        text: text.trim().to_string(),
    })
}

fn parse_hms(value: &str) -> Option<f32> {
    let parts: Vec<&str> = value.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let hours: f32 = parts[0].parse().ok()?;
    let minutes: f32 = parts[1].parse().ok()?;
    let seconds: f32 = parts[2].parse().ok()?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

fn write_wav_16k(path: &Path, samples: &[f32]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    let data_size = (samples.len() * 2) as u32;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_size).to_le_bytes())?;
    file.write_all(b"WAVE")?;
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&16_000u32.to_le_bytes())?;
    file.write_all(&32_000u32.to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&16u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_size.to_le_bytes())?;
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        file.write_all(&v.to_le_bytes())?;
    }
    Ok(())
}

pub fn transcribe_pcm(
    samples: &[f32],
    sample_rate_hz: u32,
    model: &str,
) -> Result<Vec<TranscriptSegment>, String> {
    let engine = find_engine();
    let path = engine.path.ok_or_else(|| engine.install_hint.clone())?;
    let pcm = resample_to_16k(samples, sample_rate_hz);
    if pcm.is_empty() {
        return Err("no audio samples to transcribe".into());
    }
    let wav = std::env::temp_dir().join(format!("pulsescope_whisper_{}.wav", std::process::id()));
    write_wav_16k(&wav, &pcm).map_err(|e| e.to_string())?;
    let output = Command::new(&path)
        .args(["-m", model, "-f", &wav.display().to_string(), "-nt"])
        .output();
    let _ = std::fs::remove_file(&wav);
    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let combined = format!("{stdout}\n{stderr}");
            Ok(parse_whisper_stdout(&combined))
        }
        Err(e) => Err(format!("failed to spawn whisper: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_hits_16k() {
        let input: Vec<f32> = (0..4800).map(|i| (i as f32 / 4800.0).sin()).collect();
        let out = resample_to_16k(&input, 48_000);
        assert_eq!(out.len(), 1600);
    }

    #[test]
    fn parse_timestamped_whisper_line() {
        let segs = parse_whisper_stdout("[00:00:00.000 --> 00:00:01.500]  hello radio");
        assert_eq!(segs.len(), 1);
        assert!((segs[0].end_s - 1.5).abs() < 0.01);
        assert_eq!(segs[0].text, "hello radio");
    }

    #[test]
    fn missing_engine_is_unavailable() {
        std::env::set_var("PULSESCOPE_WHISPER", "/nonexistent/whisper-cli");
        let engine = find_engine();
        if engine.path.as_deref() == Some("/nonexistent/whisper-cli") {
            panic!("nonexistent path must not count as available");
        }
        std::env::remove_var("PULSESCOPE_WHISPER");
    }
}
