//! Streaming SSTV automation over PulseScope's post-demodulated audio tap.
//!
//! `slowrx` accepts arbitrary audio chunks and performs its own resampling and
//! VIS detection.  Completed pictures are stored as portable PPM files so the
//! core appliance stays lightweight; the decoder event carries the file path.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::broadcast;

use crate::audio::AudioSink;
use crate::db::{Db, DecodedMessage};
use crate::state::ScannerEvent;

pub fn spawn(audio: Arc<AudioSink>, db: Db, events: broadcast::Sender<ScannerEvent>, data_dir: PathBuf, frequency_hz: u64) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut frames = audio.subscribe();
        let mut decoder = match slowrx::SstvDecoder::new(48_000) {
            Ok(decoder) => decoder,
            Err(error) => {
                tracing::warn!(%error, "SSTV decoder could not initialize");
                return;
            }
        };
        let image_dir = data_dir.join("recordings").join("sstv");
        let _ = fs::create_dir_all(&image_dir);
        loop {
            let frame = match frames.recv().await {
                Ok(frame) => frame,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    audio.observe_remote_lag(skipped);
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            };
            let mono = frame.samples.chunks(frame.channels.max(1) as usize)
                .map(|samples| samples.iter().copied().sum::<f32>() / samples.len().max(1) as f32)
                .collect::<Vec<_>>();
            for event in decoder.process(&mono) {
                match event {
                    slowrx::SstvEvent::VisDetected { mode, hedr_shift_hz, .. } => {
                        publish(&db, &events, frequency_hz, "sstv", "mode_detected", format!("SSTV {mode:?} detected (tuning offset {hedr_shift_hz:+.0} Hz)"));
                    }
                    slowrx::SstvEvent::UnknownVis { code, .. } => {
                        publish(&db, &events, frequency_hz, "sstv", "unknown_vis", format!("SSTV VIS {code} detected but is not supported by this decoder"));
                    }
                    slowrx::SstvEvent::ImageComplete { image, partial } => {
                        let filename = format!("sstv-{}-{}.ppm", crate::scanner::now_ms(), format!("{:?}", image.mode).to_lowercase());
                        let path = image_dir.join(filename);
                        match write_ppm(&path, image.width, image.height, &image.pixels) {
                            Ok(()) => publish(&db, &events, frequency_hz, "sstv", "image", format!("SSTV {:?} image decoded{}: {}", image.mode, if partial { " (partial)" } else { "" }, path.display())),
                            Err(error) => tracing::warn!(%error, "failed to save SSTV image"),
                        }
                    }
                    slowrx::SstvEvent::LineDecoded { .. } => {}
                    _ => {}
                }
            }
        }
    })
}

/// Decode successive, UTC-aligned 15 second FT8 periods using WSJT-X's
/// documented `jt9 -8 wav-file` command-line decoder.  It is deliberately a
/// file boundary rather than a raw-stdin sidecar: jt9 documents WAV and shared
/// memory inputs, not a generic audio pipe.
pub fn spawn_ft8(audio: Arc<AudioSink>, db: Db, events: broadcast::Sender<ScannerEvent>, data_dir: PathBuf, frequency_hz: u64, executable: PathBuf) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut frames = audio.subscribe();
        let mut period_start = None::<i64>;
        let mut samples = Vec::<f32>::with_capacity(12_000 * 15);
        let temp_dir = data_dir.join("decoders").join("ft8");
        let _ = fs::create_dir_all(&temp_dir);
        loop {
            let frame = match frames.recv().await {
                Ok(frame) => frame,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    audio.observe_remote_lag(skipped);
                    samples.clear();
                    period_start = None;
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            };
            let bucket = frame.captured_ms - frame.captured_ms.rem_euclid(15_000);
            if let Some(start) = period_start {
                if bucket != start {
                    if samples.len() >= 12_000 * 14 {
                        decode_ft8_period(&executable, &temp_dir, start, &samples, &db, &events, frequency_hz).await;
                    }
                    samples.clear();
                }
            }
            period_start = Some(bucket);
            // AudioSink produces 48 kHz float PCM on the appliance. Decimate
            // deterministically to the 12 kHz WAV rate used by WSJT-X.
            let channels = frame.channels.max(1) as usize;
            for (index, chunk) in frame.samples.chunks(channels).enumerate() {
                if index % 4 == 0 {
                    samples.push(chunk.iter().copied().sum::<f32>() / chunk.len().max(1) as f32);
                }
            }
        }
    })
}

async fn decode_ft8_period(executable: &PathBuf, temp_dir: &PathBuf, period_start: i64, samples: &[f32], db: &Db, events: &broadcast::Sender<ScannerEvent>, frequency_hz: u64) {
    let path = temp_dir.join(format!("ft8-{period_start}.wav"));
    if let Err(error) = write_wav_mono_16(&path, 12_000, samples) {
        tracing::warn!(%error, "failed to create FT8 WAV period");
        return;
    }
    let output = tokio::process::Command::new(executable)
        .args(["-8", "-p", "0.25"])
        .arg(&path)
        .output()
        .await;
    let _ = fs::remove_file(&path);
    match output {
        Ok(output) if output.status.success() => {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let line = line.trim();
                // jt9 also writes banners and diagnostics. FT8 decoded lines
                // carry a CQ or the FT8 `~` mode marker plus message fields.
                if line.contains("CQ ") || (line.contains('~') && line.split_whitespace().count() >= 5) {
                    publish(db, events, frequency_hz, "ft8", "message", line.to_string());
                }
            }
        }
        Ok(output) => tracing::warn!(status = ?output.status, stderr = %String::from_utf8_lossy(&output.stderr), "FT8 decoder exited unsuccessfully"),
        Err(error) => tracing::warn!(%error, "FT8 decoder could not start"),
    }
}

fn publish(db: &Db, events: &broadcast::Sender<ScannerEvent>, frequency_hz: u64, protocol: &str, message_type: &str, content: String) {
    let message = DecodedMessage {
        id: None,
        frequency_hz,
        protocol: protocol.into(),
        message_type: message_type.into(),
        address: String::new(),
        function_code: String::new(),
        raw: content.clone(),
        content,
        encryption: "none".into(),
        timestamp_ms: crate::scanner::now_ms(),
    };
    let _ = db.insert_decoded_message(&message);
    let _ = events.send(ScannerEvent::DecodedMessage(message));
}

fn write_ppm(path: &PathBuf, width: u32, height: u32, pixels: &[[u8; 3]]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = fs::File::create(path)?;
    write!(file, "P6\n{width} {height}\n255\n")?;
    for pixel in pixels { file.write_all(pixel)?; }
    Ok(())
}

fn write_wav_mono_16(path: &PathBuf, sample_rate: u32, samples: &[f32]) -> std::io::Result<()> {
    use std::io::Write;
    let bytes = (samples.len() * 2) as u32;
    let mut file = fs::File::create(path)?;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + bytes).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&(sample_rate * 2).to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&16u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&bytes.to_le_bytes())?;
    for sample in samples {
        file.write_all(&((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes())?;
    }
    Ok(())
}
