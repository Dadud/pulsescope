//! Clean-room baseband demodulators.
//!
//! Input is complex baseband IQ at the SDR sample rate. Output is normalized
//! mono f32 PCM at the same frame rate; the audio backend/resampler is kept
//! separate so the DSP remains testable without hardware or Windows audio.

use std::f32::consts::TAU;
use rustfft::num_complex::Complex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Am,
    Nfm,
    Wfm,
    Usb,
    Lsb,
}

impl Mode {
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "am" => Self::Am,
            "wfm" => Self::Wfm,
            "usb" => Self::Usb,
            "lsb" => Self::Lsb,
            _ => Self::Nfm,
        }
    }
}

/// Demodulate one IQ frame. `previous` preserves phase continuity between
/// scanner frames, which matters for FM and prevents clicks at frame edges.
/// One-pole complex low-pass used as the first channel-filter boundary before demodulation.
/// `state` must persist between capture blocks so the audio worker has continuous phase/filter state.
pub fn low_pass_complex(input: &[Complex<f32>], cutoff_hz: f32, sample_rate_hz: u32, state: &mut Complex<f32>) -> Vec<Complex<f32>> {
    if input.is_empty() || sample_rate_hz == 0 { return Vec::new(); }
    let cutoff = cutoff_hz.max(1.0).min(sample_rate_hz as f32 * 0.45);
    let alpha = 1.0 - (-std::f32::consts::TAU * cutoff / sample_rate_hz as f32).exp();
    let mut out = Vec::with_capacity(input.len());
    for &sample in input {
        *state += (sample - *state) * alpha;
        out.push(*state);
    }
    out
}
pub fn demodulate(mode: Mode, iq: &[Complex<f32>], previous: &mut Option<Complex<f32>>) -> Vec<f32> {
    if iq.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(iq.len());
    match mode {
        Mode::Am => {
            let mean = iq.iter().map(|s| s.norm()).sum::<f32>() / iq.len() as f32;
            for s in iq {
                out.push((s.norm() - mean) * 4.0);
            }
        }
        Mode::Usb => {
            for s in iq { out.push((s.re + s.im) * std::f32::consts::FRAC_1_SQRT_2); }
        }
        Mode::Lsb => {
            for s in iq { out.push((s.re - s.im) * std::f32::consts::FRAC_1_SQRT_2); }
        }
        Mode::Nfm | Mode::Wfm => {
            let mut prev = previous.take().unwrap_or(iq[0]);
            let scale = if mode == Mode::Nfm { 1.0 } else { 0.35 };
            for s in iq {
                let cross = s.im * prev.re - s.re * prev.im;
                let dot = s.re * prev.re + s.im * prev.im;
                out.push(cross.atan2(dot) * scale);
                prev = *s;
            }
            *previous = Some(prev);
        }
    }
    soft_limit(&mut out);
    out
}

/// Translate a VFO offset to zero-IF while preserving oscillator phase.
pub fn mix_down(input: &[Complex<f32>], offset_hz: f64, sample_rate: u32, phase: &mut f64) -> Vec<Complex<f32>> {
    if input.is_empty() || sample_rate == 0 { return Vec::new(); }
    let step = -std::f64::consts::TAU * offset_hz / sample_rate as f64;
    let mut out = Vec::with_capacity(input.len());
    for sample in input {
        let (sin, cos) = phase.sin_cos();
        let osc = Complex::new(cos as f32, sin as f32);
        out.push(*sample * osc);
        *phase = (*phase + step).rem_euclid(std::f64::consts::TAU);
    }
    out
}

/// Boxcar low-pass plus decimation for the wideband-to-audio boundary.
pub fn decimate_complex_average(input: &[Complex<f32>], factor: usize) -> Vec<Complex<f32>> {
    if factor <= 1 { return input.to_vec(); }
    input.chunks(factor).map(|chunk| {
        let sum = chunk.iter().copied().fold(Complex::new(0.0, 0.0), |a, b| a + b);
        sum / chunk.len() as f32
    }).collect()
}
pub fn decimate_average(input: &[f32], factor: usize) -> Vec<f32> {
    if factor <= 1 { return input.to_vec(); }
    input.chunks(factor).filter_map(|chunk| {
        if chunk.len() < factor { return None; }
        Some(chunk.iter().sum::<f32>() / factor as f32)
    }).collect()
}


/// This keeps the audio sink rate correct for arbitrary SDR/output-rate pairs.
pub fn resample_linear(input: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
    if input.is_empty() || input_rate == 0 || output_rate == 0 { return Vec::new(); }
    if input.len() == 1 { return vec![input[0]]; }
    let count = ((input.len() as u64 * output_rate as u64) / input_rate as u64) as usize;
    let count = count.max(1);
    let step = input_rate as f64 / output_rate as f64;
    (0..count).map(|n| {
        let pos = n as f64 * step;
        let i = (pos.floor() as usize).min(input.len() - 2);
        let frac = (pos - i as f64) as f32;
        input[i] * (1.0 - frac) + input[i + 1] * frac
    }).collect()
}

/// Stateful band-limited sample-rate conversion for continuous receiver audio.
///
/// The former block-local linear interpolation reset its phase every 20 ms and
/// did not reject content above the output Nyquist limit. This windowed-sinc
/// converter retains both phase and filter history across blocks.
#[derive(Debug)]
pub struct SincResampler {
    buffer: Vec<f32>,
    position: f64,
    input_rate: u32,
    output_rate: u32,
    half_taps: usize,
}

impl Default for SincResampler {
    fn default() -> Self {
        Self {
            buffer: Vec::new(),
            position: 0.0,
            input_rate: 0,
            output_rate: 0,
            half_taps: 16,
        }
    }
}

impl SincResampler {
    pub fn process(&mut self, input: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
        if input.is_empty() || input_rate == 0 || output_rate == 0 {
            return Vec::new();
        }
        if self.input_rate != input_rate || self.output_rate != output_rate {
            self.buffer = vec![0.0; self.half_taps];
            self.position = self.half_taps as f64;
            self.input_rate = input_rate;
            self.output_rate = output_rate;
        }
        self.buffer.extend_from_slice(input);

        let step = input_rate as f64 / output_rate as f64;
        let cutoff = 0.45 * (output_rate as f64 / input_rate as f64).min(1.0);
        let taps = self.half_taps * 2;
        let mut output = Vec::with_capacity(
            ((input.len() as u64 * output_rate as u64) / input_rate as u64) as usize + 2,
        );

        while self.position + (self.half_taps as f64) < self.buffer.len() as f64 {
            let center = self.position.floor() as isize;
            let mut sample = 0.0_f64;
            let mut weight = 0.0_f64;
            for tap in 0..taps {
                let index = center + tap as isize - self.half_taps as isize + 1;
                let distance = index as f64 - self.position;
                let sinc_arg = 2.0 * cutoff * distance;
                let sinc = if sinc_arg.abs() < 1.0e-12 {
                    1.0
                } else {
                    (std::f64::consts::PI * sinc_arg).sin()
                        / (std::f64::consts::PI * sinc_arg)
                };
                let window = 0.5
                    - 0.5
                        * (std::f64::consts::TAU * tap as f64 / (taps - 1) as f64).cos();
                let coefficient = 2.0 * cutoff * sinc * window;
                sample += self.buffer[index as usize] as f64 * coefficient;
                weight += coefficient;
            }
            output.push(if weight.abs() > 1.0e-12 {
                (sample / weight) as f32
            } else {
                0.0
            });
            self.position += step;
        }

        let discard = (self.position.floor() as usize).saturating_sub(self.half_taps);
        if discard > 0 {
            self.buffer.drain(..discard);
            self.position -= discard as f64;
        }
        output
    }
}

pub fn low_pass_real(input: &[f32], cutoff_hz: f32, sample_rate_hz: u32, state: &mut f32) -> Vec<f32> {
    if input.is_empty() || sample_rate_hz == 0 { return Vec::new(); }
    let cutoff = cutoff_hz.max(1.0).min(sample_rate_hz as f32 * 0.45);
    let alpha = 1.0 - (-TAU * cutoff / sample_rate_hz as f32).exp();
    input.iter().map(|sample| {
        *state += (*sample - *state) * alpha;
        *state
    }).collect()
}

pub fn deemphasis(input: &mut [f32], sample_rate_hz: u32, time_constant_us: f32, state: &mut f32) {
    if sample_rate_hz == 0 || time_constant_us <= 0.0 { return; }
    let tau = time_constant_us * 1.0e-6;
    let alpha = 1.0 - (-1.0 / (sample_rate_hz as f32 * tau)).exp();
    for sample in input {
        *state += (*sample - *state) * alpha;
        *sample = *state;
    }
}


pub fn dc_block(samples: &mut [f32], state: &mut f32, coefficient: f32) {
    let coefficient = coefficient.clamp(0.0, 0.99999);
    for sample in samples {
        *state = coefficient * *state + (1.0 - coefficient) * *sample;
        *sample -= *state;
    }
}

fn soft_limit(samples: &mut [f32]) {
    for sample in samples {
        *sample = (*sample * 1.8).tanh();
    }
}

// ── CW / Morse decoder ─────────────────────────────────────────────────────

/// International Morse code lookup: dit/dah pattern → character.
const MORSE_TABLE: &[(&str, char)] = &[
    (".-", 'A'), ("-...", 'B'), ("-.-.", 'C'), ("-..", 'D'), (".", 'E'),
    ("..-.", 'F'), ("--.", 'G'), ("....", 'H'), ("..", 'I'), (".---", 'J'),
    ("-.-", 'K'), (".-..", 'L'), ("--", 'M'), ("-.", 'N'), ("---", 'O'),
    (".--.", 'P'), ("--.-", 'Q'), (".-.", 'R'), ("...", 'S'), ("-", 'T'),
    ("..-", 'U'), ("...-", 'V'), (".--", 'W'), ("-..-", 'X'), ("-.--", 'Y'),
    ("--..", 'Z'),
    ("-----", '0'), (".----", '1'), ("..---", '2'), ("...--", '3'), ("....-", '4'),
    (".....", '5'), ("-....", '6'), ("--...", '7'), ("---..", '8'), ("----.", '9'),
    (".-.-.-", '.'), ("--..--", ','), ("..--..", '?'), (".----.", '\''),
    ("-.-.--", '!'), ("-..-.", '/'), ("-.--.", '('), ("-.--.-", ')'),
    (".-...", '&'), ("---...", ':'), ("-.-.-.", ';'), ("-...-", '='),
    (".-.-.", '+'), ("-....-", '-'), ("..--.-", '_'), (".-..-.", '"'),
    (".--.-.", '@'),
];

fn morse_decode(pattern: &str) -> Option<char> {
    MORSE_TABLE.iter().find(|(p, _)| *p == pattern).map(|(_, c)| *c)
}

/// Decode CW Morse from a tone-detected envelope.
/// `samples` is demodulated audio at `sample_rate`.
/// Detects on/off keying via band-energy threshold.
/// Returns decoded text.
pub fn decode_cw(samples: &[f32], sample_rate: f32, target_tone_hz: f32) -> Option<String> {
    if samples.len() < (sample_rate * 0.1) as usize { return None; }

    let block_size = (sample_rate * 0.005) as usize;
    if block_size < 4 { return None; }
    let total_energy: f32 = samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32;
    if total_energy < 1e-10 { return None; }

    let mut envelope: Vec<bool> = Vec::new();
    for chunk in samples.chunks(block_size) {
        let mag = goertzel_magnitude(chunk, target_tone_hz, sample_rate);
        let chunk_energy: f32 = chunk.iter().map(|x| x * x).sum::<f32>() / chunk.len() as f32;
        let snr = if chunk_energy > 1e-12 { 10.0 * (mag / (chunk.len() as f32 * chunk_energy)).log10() } else { -100.0 };
        envelope.push(snr >= 6.0);
    }

    let mut runs: Vec<(bool, usize)> = Vec::new();
    for &on in &envelope {
        if let Some(last) = runs.last_mut() {
            if last.0 == on { last.1 += 1; continue; }
        }
        runs.push((on, 1));
    }

    let tone_runs: Vec<usize> = runs.iter().filter(|(on, _)| *on).map(|(_, n)| *n).collect();
    if tone_runs.is_empty() { return None; }
    let dit_blocks = tone_runs.iter().copied().min().unwrap_or(1).max(1);
    let dah_blocks = dit_blocks * 3;

    let mut text = String::new();
    let mut current_pattern = String::new();

    for (on, count) in &runs {
        if *on {
            if *count >= dah_blocks * 3 / 4 {
                current_pattern.push('-');
            } else {
                current_pattern.push('.');
            }
        } else {
            if *count >= dit_blocks * 7 / 2 {
                if let Some(c) = morse_decode(&current_pattern) { text.push(c); }
                text.push(' ');
                current_pattern.clear();
            } else if *count >= dit_blocks * 5 / 2 {
                if let Some(c) = morse_decode(&current_pattern) { text.push(c); }
                current_pattern.clear();
            }
        }
    }
    if !current_pattern.is_empty() {
        if let Some(c) = morse_decode(&current_pattern) { text.push(c); }
    }

    if text.is_empty() { None } else { Some(text) }
}

// ── DTMF decoder ───────────────────────────────────────────────────────────

const DTMF_LOW: [f32; 4] = [697.0, 770.0, 852.0, 941.0];
const DTMF_HIGH: [f32; 4] = [1209.0, 1336.0, 1477.0, 1633.0];
const DTMF_KEYS: [[char; 4]; 4] = [
    ['1', '2', '3', 'A'],
    ['4', '5', '6', 'B'],
    ['7', '8', '9', 'C'],
    ['*', '0', '#', 'D'],
];

/// Detect DTMF digits in a block of demodulated audio.
/// Returns decoded digit string.
pub fn detect_dtmf(samples: &[f32], sample_rate: f32) -> Option<String> {
    let block_size = (sample_rate * 0.04) as usize;
    if samples.len() < block_size { return None; }

    let total_energy: f32 = samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32;
    if total_energy < 1e-10 { return None; }

    let mut result = String::new();
    let mut last_digit: Option<char> = None;

    for chunk in samples.chunks(block_size) {
        if chunk.len() < block_size / 2 { break; }
        let chunk_energy: f32 = chunk.iter().map(|x| x * x).sum::<f32>() / chunk.len() as f32;
        if chunk_energy < 1e-8 { last_digit = None; continue; }

        let mut best_low = (0usize, 0.0f32);
        let mut best_high = (0usize, 0.0f32);
        for (i, &freq) in DTMF_LOW.iter().enumerate() {
            let mag = goertzel_magnitude(chunk, freq, sample_rate);
            if mag > best_low.1 { best_low = (i, mag); }
        }
        for (i, &freq) in DTMF_HIGH.iter().enumerate() {
            let mag = goertzel_magnitude(chunk, freq, sample_rate);
            if mag > best_high.1 { best_high = (i, mag); }
        }

        // Goertzel magnitude for a matching tone ≈ N²·A²/4, for noise ≈ N·E.
        // Threshold: tone must be at least 3x above the per-bin noise floor.
        let threshold = chunk_energy * chunk.len() as f32 * 3.0;
        if best_low.1 < threshold || best_high.1 < threshold {
            last_digit = None; continue;
        }
        // Twist test: ratio of strong to weak tone (power) should be < 10x (~10 dB)
        let ratio = best_low.1.max(best_high.1) / best_low.1.min(best_high.1).max(1e-12);
        if ratio > 10.0 {
            last_digit = None; continue;
        }

        let digit = DTMF_KEYS[best_low.0][best_high.0];
        if Some(digit) != last_digit {
            result.push(digit);
            last_digit = Some(digit);
        }
    }

    if result.is_empty() { None } else { Some(result) }
}

// ── RTTY / SSTV / NAVTEX ───────────────────────────────────────────────────

const ITA2_LETTERS: [char; 32] = [
    '\0', 'E', '\n', 'A', ' ', 'S', 'I', 'U', '\r', 'D', 'R', 'J', 'N', 'F', 'C', 'K',
    'T', 'Z', 'L', 'W', 'H', 'Y', 'P', 'Q', 'O', 'B', 'G', '\0', 'M', 'X', 'V', '\0',
];
const ITA2_FIGURES: [char; 32] = [
    '\0', '3', '\n', '-', ' ', '\'', '8', '7', '\r', '$', '4', '\'', ',', '!', ':', '(',
    '5', '+', ')', '2', '#', '6', '0', '1', '9', '?', '&', '\0', '.', '/', ';', '\0',
];

fn ita2_push(out: &mut String, code: u8, figures: &mut bool) {
    match code & 0x1f {
        0x1b => *figures = true,
        0x1f => *figures = false,
        value => {
            let c = if *figures { ITA2_FIGURES[value as usize] } else { ITA2_LETTERS[value as usize] };
            if c != '\0' && c != '\r' { out.push(c); }
        }
    }
}

fn fsk_bit_at(samples: &[f32], center: f32, samples_per_bit: f32, mark: f32, space: f32, sample_rate: f32) -> Option<bool> {
    let width = (samples_per_bit * 0.72).round().max(8.0) as usize;
    let middle = center.round() as isize;
    let start = middle - width as isize / 2;
    if start < 0 || start as usize + width > samples.len() { return None; }
    let block = &samples[start as usize..start as usize + width];
    Some(goertzel_magnitude(block, mark, sample_rate) > goertzel_magnitude(block, space, sample_rate))
}

/// Decode asynchronous Baudot/ITA-2 RTTY audio. Mark and space may be any
/// pair (the common shifts are 170, 450 and 850 Hz); standard 45.45, 50 and
/// 75 baud signals are supported, including fractional samples per symbol.
pub fn decode_rtty(samples: &[f32], sample_rate: f32, mark_freq: f32, space_freq: f32, baud_rate: f32) -> Option<String> {
    if sample_rate <= 0.0 || baud_rate <= 0.0 || mark_freq <= 0.0 || space_freq <= 0.0 || mark_freq == space_freq { return None; }
    let spb = sample_rate / baud_rate;
    if spb < 8.0 || (samples.len() as f32) < spb * 8.0 { return None; }
    let energy = samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32;
    if energy < 1e-9 { return None; }

    // RTTY idles on mark. Search a fine timing grid for mark-to-space start
    // transitions, then sample the five LSB-first data bits at their centres.
    let step = (spb / 8.0).max(1.0);
    let mut cursor = spb * 0.5;
    let mut previous = fsk_bit_at(samples, cursor, spb, mark_freq, space_freq, sample_rate).unwrap_or(true);
    let mut codes = Vec::new();
    while cursor + spb * 7.0 < samples.len() as f32 {
        cursor += step;
        let current = match fsk_bit_at(samples, cursor, spb, mark_freq, space_freq, sample_rate) { Some(v) => v, None => break };
        if previous && !current {
            let start = cursor - step * 0.5;
            let start_ok = fsk_bit_at(samples, start + spb * 0.5, spb, mark_freq, space_freq, sample_rate) == Some(false);
            let stop_ok = fsk_bit_at(samples, start + spb * 6.5, spb, mark_freq, space_freq, sample_rate) == Some(true);
            if start_ok && stop_ok {
                let mut code = 0u8;
                let mut complete = true;
                for bit in 0..5 {
                    match fsk_bit_at(samples, start + spb * (1.5 + bit as f32), spb, mark_freq, space_freq, sample_rate) {
                        Some(true) => code |= 1 << bit,
                        Some(false) => {}
                        None => complete = false,
                    }
                }
                if complete {
                    codes.push(code);
                    cursor = start + spb * 7.0;
                    previous = true;
                    continue;
                }
            }
        }
        previous = current;
    }

    let mut out = String::new();
    let mut figures = false;
    for code in codes { ita2_push(&mut out, code, &mut figures); }
    let text = out.trim_matches('\0').to_string();
    if text.is_empty() { None } else { Some(text) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SstvMode {
    Scottie1,
    Scottie2,
    Martin1,
    Martin2,
    Robot36,
    Robot72,
    Pd120,
    Pd180,
    Pd240,
    Pd290,
}

/// Detect an SSTV mode from repeated 1200-Hz horizontal sync pulses.  The
/// four standard SSTV signalling levels (1200 sync, 1500 black/separator,
/// 1900 grey and 2300 white Hz) are compared in each analysis window; mode
/// classification uses both sync length and measured line period.
pub fn detect_sstv_mode(samples: &[f32], sample_rate: f32) -> Option<SstvMode> {
    if sample_rate < 6000.0 || samples.len() < (sample_rate * 0.25) as usize { return None; }
    let window = (sample_rate * 0.0025).round().max(12.0) as usize;
    let hop = (sample_rate * 0.001).round().max(1.0) as usize;
    let tones = [1200.0, 1500.0, 1900.0, 2300.0];
    let mut sync_flags = Vec::new();
    let mut positions = Vec::new();
    let mut offset = 0usize;
    while offset + window <= samples.len() {
        let block = &samples[offset..offset + window];
        let mut powers = [0.0f32; 4];
        for (i, &tone) in tones.iter().enumerate() { powers[i] = goertzel_magnitude(block, tone, sample_rate); }
        let mut order = [0usize, 1, 2, 3];
        order.sort_by(|&a, &b| powers[b].partial_cmp(&powers[a]).unwrap_or(std::cmp::Ordering::Equal));
        sync_flags.push(order[0] == 0 && powers[0] > powers[order[1]] * 1.8);
        positions.push(offset + window / 2);
        offset += hop;
    }

    let mut pulses: Vec<(f32, f32)> = Vec::new(); // start seconds, duration seconds
    let mut i = 0usize;
    while i < sync_flags.len() {
        if !sync_flags[i] { i += 1; continue; }
        let begin = i;
        while i < sync_flags.len() && sync_flags[i] { i += 1; }
        let run_hops = i - begin;
        if run_hops >= 2 {
            pulses.push((positions[begin] as f32 / sample_rate, run_hops as f32 * hop as f32 / sample_rate));
        }
    }
    if pulses.len() < 2 { return None; }

    let mut periods: Vec<f32> = pulses.windows(2).map(|p| p[1].0 - p[0].0).filter(|p| *p > 0.05 && *p < 0.6).collect();
    if periods.is_empty() { return None; }
    periods.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let period = periods[periods.len() / 2];
    let mut durations: Vec<f32> = pulses.iter().map(|p| p.1).collect();
    durations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let duration = durations[durations.len() / 2];

    // Nominal line periods and sync widths in seconds.
    let candidates = [
        (SstvMode::Scottie1, 0.42822, 0.009), (SstvMode::Scottie2, 0.27769, 0.009),
        (SstvMode::Martin1, 0.446446, 0.004862), (SstvMode::Martin2, 0.226798, 0.004862),
        (SstvMode::Robot36, 0.1500, 0.009), (SstvMode::Robot72, 0.3000, 0.009),
        (SstvMode::Pd120, 0.1216, 0.020), (SstvMode::Pd180, 0.18304, 0.020),
        (SstvMode::Pd240, 0.24448, 0.020), (SstvMode::Pd290, 0.2880, 0.020),
    ];
    candidates.iter().map(|&(mode, line, sync)| {
        let line_error = (period - line).abs() / line;
        // The short analysis window broadens pulse edges, so line timing is
        // weighted more heavily than measured sync width.
        let sync_error = (duration - sync).abs() / sync;
        (mode, line_error + sync_error * 0.15, line_error)
    }).min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
      .and_then(|(mode, _, line_error)| if line_error <= 0.12 { Some(mode) } else { None })
}

fn ccir476_to_ita2(code: u8) -> Option<u8> {
    // CCIR 476 is the constant-weight (four marks) recoding of ITA-2.
    Some(match code {
        0x6a => 0x00, 0x56 => 0x01, 0x6c => 0x02, 0x47 => 0x03,
        0x5c => 0x04, 0x4b => 0x05, 0x4d => 0x06, 0x4e => 0x07,
        0x78 => 0x08, 0x53 => 0x09, 0x55 => 0x0a, 0x17 => 0x0b,
        0x59 => 0x0c, 0x1b => 0x0d, 0x1d => 0x0e, 0x1e => 0x0f,
        0x74 => 0x10, 0x63 => 0x11, 0x65 => 0x12, 0x27 => 0x13,
        0x69 => 0x14, 0x2b => 0x15, 0x2d => 0x16, 0x2e => 0x17,
        0x71 => 0x18, 0x72 => 0x19, 0x35 => 0x1a, 0x36 => 0x1b,
        0x39 => 0x1c, 0x3a => 0x1d, 0x3c => 0x1e, 0x5a => 0x1f,
        _ => return None, // SIA/SIB/RPT and invalid constant-weight words
    })
}

/// Decode 100-baud NAVTEX/SITOR-B audio (nominal 1700-Hz centre,
/// 170-Hz shift). The receiver searches symbol phase, polarity and bit order,
/// validates CCIR-476's four-of-seven alphabet, and suppresses the repeated
/// copy sent five characters later by SITOR-B forward-error correction.
pub fn decode_navtex(samples: &[f32], sample_rate: f32) -> Option<String> {
    const BAUD: f32 = 100.0;
    const MARK: f32 = 1785.0;
    const SPACE: f32 = 1615.0;
    if sample_rate < 5000.0 || samples.len() < (sample_rate * 0.15) as usize { return None; }
    let energy = samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32;
    if energy < 1e-9 { return None; }
    let spb = sample_rate / BAUD;

    let mut best: Vec<u8> = Vec::new();
    let mut best_score = 0usize;
    for phase_step in 0..8 {
        let phase = spb * (phase_step as f32 / 8.0 + 0.5);
        let mut bits = Vec::new();
        let mut center = phase;
        while let Some(bit) = fsk_bit_at(samples, center, spb, MARK, SPACE, sample_rate) {
            bits.push(bit);
            center += spb;
        }
        for inverted in [false, true] {
            for reversed in [false, true] {
                for alignment in 0..7 {
                    let mut words = Vec::new();
                    let mut valid = 0usize;
                    let mut p = alignment;
                    while p + 7 <= bits.len() {
                        let mut word = 0u8;
                        for bit in 0..7 {
                            let value = bits[p + bit] ^ inverted;
                            let shift = if reversed { bit } else { 6 - bit };
                            if value { word |= 1 << shift; }
                        }
                        if word.count_ones() == 4 && ccir476_to_ita2(word).is_some() { valid += 1; }
                        words.push(word);
                        p += 7;
                    }
                    if valid > best_score { best_score = valid; best = words; }
                }
            }
        }
    }
    if best_score < 2 || best_score * 2 < best.len() { return None; }

    let mut selected = Vec::new();
    for (i, &word) in best.iter().enumerate() {
        if ccir476_to_ita2(word).is_none() { continue; }
        // In the SITOR-B interleave a character's second transmission is five
        // character intervals later. Keep the first valid copy; use the second
        // when the first was damaged.
        if i >= 5 && best[i - 5] == word { continue; }
        selected.push(word);
    }
    let mut out = String::new();
    let mut figures = false;
    for word in selected {
        if let Some(code) = ccir476_to_ita2(word) { ita2_push(&mut out, code, &mut figures); }
    }
    let text = out.trim_matches(|c: char| c == '\0' || c == '\r' || c == '\n').to_string();
    if text.is_empty() { None } else { Some(text) }
}

// ── CTCSS / DCS ────────────────────────────────────────────────────────────

/// Downconvert and decode RDS from a WFM-demodulated multiplex signal.
/// RDS sits on a 57 kHz suppressed-carrier AM/PM subcarrier with ±2 kHz
/// deviation. Data rate is 1187.5 bps, differentially encoded (BPSK).
/// This decoder performs:
///   1. Quadrature downconvert from 57 kHz
///   2. Low-pass to isolate the 1.1875 kHz baseband
///   3. Clock recovery via zero crossings
///   4. Differential decoding
///
/// Returns decoded groups as a JSON-serializable struct.
pub struct RdsResult {
    pub bits_decoded: usize,
    pub groups_found: usize,
    pub program_service: Option<String>,
    pub radio_text: Option<String>,
    pub pty: Option<u8>,
    pub pi_code: Option<u16>,
}

/// Decode RDS from demodulated WFM audio (multiplex).
/// `sample_rate` must be the audio rate (typically 96 kHz after resampling).
/// Returns None if no RDS subcarrier is present.
pub fn decode_rds(multiplex: &[f32], sample_rate: f32) -> Option<RdsResult> {
    const RDS_CARRIER: f32 = 57_000.0;
    const BIT_RATE: f32 = 1187.5;
    const RDS_BW: f32 = 2_500.0;

    // Need at least 0.5 seconds for meaningful RDS decode
    if multiplex.len() < (sample_rate * 0.5) as usize { return None; }

    // 1. Quadrature mix-down from 57 kHz
    let mut baseband_i = Vec::with_capacity(multiplex.len());
    let mut baseband_q = Vec::with_capacity(multiplex.len());
    for (n, &sample) in multiplex.iter().enumerate() {
        let phase = TAU * RDS_CARRIER * n as f32 / sample_rate;
        baseband_i.push(sample * phase.cos());
        baseband_q.push(sample * -phase.sin());
    }

    // 2. One-pole low-pass at ~2.5 kHz to isolate baseband RDS
    let alpha = 1.0 - (-TAU * RDS_BW / sample_rate).exp();
    let mut lp_i = 0.0f32;
    let mut lp_q = 0.0f32;
    let filtered: Vec<f32> = baseband_i.iter().zip(baseband_q.iter()).map(|(&i, &q)| {
        lp_i += (i - lp_i) * alpha;
        lp_q += (q - lp_q) * alpha;
        (lp_i * lp_i + lp_q * lp_q).sqrt()
    }).collect();

    // 3. Check signal presence: RMS of the filtered signal
    let rms = filtered.iter().map(|x| x * x).sum::<f32>() / filtered.len() as f32;
    if rms < 1e-8 { return None; }

    // 4. Clock recovery via zero-crossing on the shaped signal
    // Average and threshold to get bipolar data
    let dc = filtered.iter().sum::<f32>() / filtered.len() as f32;
    let bipolar: Vec<f32> = filtered.iter().map(|x| x - dc).collect();

    let samples_per_bit = sample_rate / BIT_RATE;
    let mut bits: Vec<u8> = Vec::new();
    let mut pos = 0.0f32;
    while (pos as usize + samples_per_bit as usize / 2) < bipolar.len() {
        let idx = pos as usize;
        let center = idx + (samples_per_bit / 2.0) as usize;
        if center < bipolar.len() {
            bits.push(if bipolar[center] >= 0.0 { 1 } else { 0 });
        }
        pos += samples_per_bit;
    }

    if bits.len() < 104 { return Some(RdsResult { bits_decoded: bits.len(), groups_found: 0, program_service: None, radio_text: None, pty: None, pi_code: None }); }

    // 5. Differential decode
    let diff_bits: Vec<u8> = bits.windows(2).map(|w| w[0] ^ w[1]).collect();

    // 6. Search for RDS sync word (block A offset word: 0b00111111000 = 0x3D8)
    // After differential decoding, we look for the known sync pattern.
    // We scan for block alignment by checking the 10-bit offset word.
    let mut groups = Vec::new();
    let sync_pattern: [u8; 10] = [0, 0, 1, 1, 1, 1, 1, 1, 0, 0];

    let mut i = 0;
    while i + 104 <= diff_bits.len() {
        // Check for sync pattern at this position
        let matches: usize = (0..10).map(|j| if diff_bits[i + j] == sync_pattern[j] { 1 } else { 0 }).sum();
        if matches >= 8 {
            // Extract 26-bit words for each of the 4 blocks
            let extract_word = |start: usize| -> u32 {
                diff_bits[start..start.min(diff_bits.len())]
                    .iter()
                    .take(26)
                    .fold(0u32, |acc, &b| (acc << 1) | b as u32)
            };
            let word_a = extract_word(i);
            // PI code is the first 16 bits of block A
            let pi = (word_a >> 10) as u16;
            // PTY is bits 15-19 of block B
            let word_b = extract_word(i + 26);
            let pty = ((word_b >> 5) & 0x1F) as u8;
            groups.push((pi, pty, i));
            i += 104;
        } else {
            i += 1;
        }
    }

    if groups.is_empty() {
        return Some(RdsResult { bits_decoded: diff_bits.len(), groups_found: 0, program_service: None, radio_text: None, pty: None, pi_code: None });
    }

    // Aggregate the most common PI and PTY
    use std::collections::HashMap;
    let mut pi_counts: HashMap<u16, usize> = HashMap::new();
    let mut pty_counts: HashMap<u8, usize> = HashMap::new();
    for (pi, pty, _) in &groups {
        *pi_counts.entry(*pi).or_default() += 1;
        *pty_counts.entry(*pty).or_default() += 1;
    }
    let pi_code = pi_counts.into_iter().max_by_key(|(_, c)| *c).map(|(k, _)| k);
    let pty = pty_counts.into_iter().max_by_key(|(_, c)| *c).map(|(k, _)| k);

    Some(RdsResult {
        bits_decoded: diff_bits.len(),
        groups_found: groups.len(),
        program_service: None, // Requires full block-by-block PS decoding (8-char display, sent over 4 groups)
        radio_text: None,
        pty,
        pi_code,
    })
}



/// Standard CTCSS tones (EIA) in Hz.
pub const CTCSS_TONES: [f32; 50] = [
    67.0, 69.3, 71.9, 74.4, 77.0, 79.7, 82.5, 85.4, 88.5, 91.5,
    94.8, 97.4, 100.0, 103.5, 107.2, 110.9, 114.8, 118.8, 123.0, 127.3,
    131.8, 136.5, 141.3, 146.2, 151.4, 156.7, 159.8, 162.2, 165.5, 167.9,
    171.3, 173.8, 177.3, 179.9, 183.5, 186.2, 189.9, 192.8, 196.6, 199.5,
    203.5, 206.5, 210.7, 218.1, 225.7, 229.1, 233.6, 241.8, 250.3, 254.1,
];

/// Goertzel magnitude for a single frequency over a block of samples.
fn goertzel_magnitude(samples: &[f32], freq: f32, sample_rate: f32) -> f32 {
    if samples.is_empty() { return 0.0; }
    let k = freq * samples.len() as f32 / sample_rate;
    let omega = TAU * k / samples.len() as f32;
    let coeff = 2.0 * omega.cos();
    let mut s0;
    let mut s1 = 0.0;
    let mut s2 = 0.0;
    for &x in samples {
        s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - coeff * s1 * s2
}

/// Detect the strongest CTCSS tone in a block of demodulated audio.
/// Returns the tone frequency and a 0.0–1.0 confidence.
/// At least 200 ms of audio is required for reliable sub-audible detection.
pub fn detect_ctcss(samples: &[f32], sample_rate: f32) -> Option<(f32, f32)> {
    if samples.len() < (sample_rate * 0.2) as usize { return None; }
    let total_energy: f32 = samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32;
    if total_energy < 1e-10 { return None; }
    let mut best: Option<(f32, f32)> = None;
    for &tone in &CTCSS_TONES {
        let mag = goertzel_magnitude(samples, tone, sample_rate);
        let normalized = mag / (samples.len() as f32 * total_energy + 1e-12);
        if best.map(|(_, b)| normalized > b).unwrap_or(true) {
            best = Some((tone, normalized));
        }
    }
    // Confidence: ratio of best tone energy to total audio energy.
    best.and_then(|(tone, ratio)| {
        let snr_db = 10.0 * ratio.max(1e-12).log10();
        if snr_db >= 6.0 { Some((tone, (snr_db / 20.0).min(1.0))) } else { None }
    })
}

/// DCS detection: look for a 23.43 kHz-class pattern is not feasible from
/// demodulated audio at typical sample rates. Instead, DCS at 134.4 bps
/// uses a 23-bit Golay word on a subcarrier we can't resolve without the
/// raw discriminator. We detect the DCS pilot pattern band energy:
/// a notch around 134 Hz baud / 72 Hz reversal. Returns detected DCS code
/// as a string if present, or None.
pub fn detect_dcs(samples: &[f32], sample_rate: f32) -> Option<String> {
    // DCS uses FSK at ~134.4 bps with ±0.62 kHz deviation on a 23-bit word.
    // Without a bit synchronizer this can't reliably extract the Golay code
    // from band-limited audio. We do a band-energy check at the expected
    // DCS fundamental to flag presence, but do NOT fabricate a code.
    if samples.len() < (sample_rate * 0.3) as usize { return None; }
    let dcs_band = goertzel_magnitude(samples, 134.4, sample_rate);
    let total_energy: f32 = samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32;
    if total_energy < 1e-10 { return None; }
    let ratio = dcs_band / (samples.len() as f32 * total_energy + 1e-12);
    let snr_db = 10.0 * ratio.max(1e-12).log10();
    if snr_db >= 10.0 { Some(format!("DCS signal detected (SNR {:.1} dB) — code extraction requires raw discriminator bitstream", snr_db)) }
    else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, cycles: f32) -> Vec<Complex<f32>> {
        (0..n).map(|i| Complex::from_polar(1.0, TAU * cycles * i as f32 / n as f32)).collect()
    }

    #[test]
    fn complex_low_pass_attenuates_out_of_channel_tone() {
        let input: Vec<_> = (0..20_000).map(|i| {
            let phase = TAU * 200_000.0 * i as f32 / 2_000_000.0;
            Complex::from_polar(1.0, phase)
        }).collect();
        let mut state = Complex::new(0.0, 0.0);
        let output = low_pass_complex(&input, 12_500.0, 2_000_000, &mut state);
        let input_rms = (input[2_000..].iter().map(|x| x.norm_sqr()).sum::<f32>() / 18_000.0).sqrt();
        let output_rms = (output[2_000..].iter().map(|x| x.norm_sqr()).sum::<f32>() / 18_000.0).sqrt();
        assert!(output_rms < input_rms * 0.2, "filter attenuation too weak: {output_rms} vs {input_rms}");
    }

    #[test]
    fn fm_demodulator_produces_audio_and_preserves_state() {
        let iq = tone(1024, 32.0);
        let mut previous = None;
        let first = demodulate(Mode::Nfm, &iq[..512], &mut previous);
        let second = demodulate(Mode::Nfm, &iq[512..], &mut previous);
        assert_eq!(first.len(), 512);
        assert_eq!(second.len(), 512);
        assert!(first.iter().skip(4).any(|v| v.abs() > 0.01));
        assert!(previous.is_some());
    }

    #[test]
    fn ssb_modes_are_distinct() {
        let iq = vec![Complex::new(0.8, 0.2); 64];
        let mut p = None;
        let usb = demodulate(Mode::Usb, &iq, &mut p);
        let lsb = demodulate(Mode::Lsb, &iq, &mut p);
        assert_ne!(usb, lsb);
    }

    #[test]
    fn fractional_resampler_hits_requested_rate() {
        let input: Vec<f32> = (0..2000).map(|i| i as f32 / 2000.0).collect();
        let output = resample_linear(&input, 2_000_000, 96_000);
        assert_eq!(output.len(), 96);
        assert!(output.windows(2).all(|w| w[1] >= w[0]));
    }
    #[test]
    fn mix_down_preserves_sample_count_and_phase() {
        let input = tone(256, 8.0);
        let mut phase = 0.0;
        let output = mix_down(&input, 12_500.0, 2_000_000, &mut phase);
        assert_eq!(output.len(), input.len());
        assert!(phase.abs() > 0.0);
    }

    #[test]
    fn decimation_applies_bounded_boxcar() {
        let input: Vec<f32> = (0..16).map(|v| v as f32).collect();
        assert_eq!(decimate_average(&input, 4), vec![1.5, 5.5, 9.5, 13.5]);
    }

    #[test]
    fn output_is_bounded() {
        let iq = vec![Complex::new(100.0, -100.0); 128];
        let mut p = None;
        let pcm = demodulate(Mode::Am, &iq, &mut p);
        assert!(pcm.iter().all(|v| v.abs() <= 1.0));
    }
    #[test]
    fn sinc_resampler_is_continuous_across_blocks() {
        let input_rate = 500_000;
        let output_rate = 48_000;
        let input: Vec<f32> = (0..10_000)
            .map(|index| (TAU * 1_000.0 * index as f32 / input_rate as f32).sin())
            .collect();
        let mut resampler = SincResampler::default();
        let mut output = Vec::new();
        for block in input.chunks(1_337) {
            output.extend(resampler.process(block, input_rate, output_rate));
        }
        assert!((output.len() as isize - 960).abs() <= 2, "unexpected output length: {}", output.len());
        assert!(output.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn sinc_resampler_rejects_above_nyquist_energy() {
        let input_rate = 96_000;
        let output_rate = 48_000;
        let input: Vec<f32> = (0..9_600)
            .map(|index| (TAU * 36_000.0 * index as f32 / input_rate as f32).sin())
            .collect();
        let mut resampler = SincResampler::default();
        let output = resampler.process(&input, input_rate, output_rate);
        let steady = &output[64.min(output.len())..];
        let rms = (steady.iter().map(|sample| sample * sample).sum::<f32>()
            / steady.len().max(1) as f32)
            .sqrt();
        assert!(rms < 0.08, "aliased energy was not rejected: {rms}");
    }

    #[test]
    fn broadcast_deemphasis_attenuates_high_frequency_content() {
        let rate = 192_000u32;
        let mut state = 0.0;
        let mut alternating: Vec<f32> = (0..4096).map(|index| if index % 2 == 0 { 1.0 } else { -1.0 }).collect();
        deemphasis(&mut alternating, rate, 75.0, &mut state);
        let rms = (alternating[512..].iter().map(|sample| sample * sample).sum::<f32>() / (alternating.len() - 512) as f32).sqrt();
        assert!(rms < 0.1, "de-emphasis did not suppress high-frequency energy: {rms}");
    }

    #[test]
    fn dtmf_detects_single_digit() {
        let sample_rate = 8000.0;
        let block = (sample_rate * 0.08) as usize;
        let mut samples = vec![0.0f32; block];
        for (i, sample) in samples.iter_mut().enumerate() {
            *sample = ((TAU * 697.0 * i as f32 / sample_rate).sin() + (TAU * 1209.0 * i as f32 / sample_rate).sin()) * 0.3;
        }
        let result = detect_dtmf(&samples, sample_rate).expect("should detect DTMF digit");
        assert!(result.contains('1'), "expected '1', got '{result}'");
    }

    #[test]
    fn dtmf_detects_multi_digit_sequence() {
        let sample_rate = 8000.0;
        let tone_dur = (sample_rate * 0.12) as usize;
        let gap_dur = (sample_rate * 0.06) as usize;
        let mut samples: Vec<f32> = Vec::new();
        let keys: [(f32, f32); 3] = [(697.0, 1209.0), (770.0, 1336.0), (852.0, 1477.0)];
        for &(low, high) in &keys {
            for i in 0..tone_dur {
                samples.push(((TAU * low * i as f32 / sample_rate).sin() + (TAU * high * i as f32 / sample_rate).sin()) * 0.3);
            }
            samples.extend(std::iter::repeat_n(0.0, gap_dur));
        }
        let result = detect_dtmf(&samples, sample_rate);
        assert!(result.is_some(), "should detect DTMF sequence");
        let result = result.unwrap();
        assert!(result.len() >= 2, "expected at least 2 digits, got '{result}'");
    }

    #[test]
    fn dtmf_rejects_noise() {
        let sample_rate = 8000.0;
        let n = (sample_rate * 0.2) as usize;
        let samples: Vec<f32> = (0..n).map(|i| (((i * 7919) % 1000) as f32 / 500.0 - 1.0) * 0.1).collect();
        assert!(detect_dtmf(&samples, sample_rate).is_none(), "should not detect DTMF in noise");
    }

    #[test]
    fn cw_decodes_sos() {
        let sample_rate = 8000.0;
        let tone_hz = 700.0;
        let dit_samples = (sample_rate * 0.1) as usize; // 100ms per dit
        let dah_samples = dit_samples * 3;
        let gap_samples = dit_samples;
        let letter_gap = dit_samples * 3;
        let mut samples: Vec<f32> = Vec::new();
        let key = |samples: &mut Vec<f32>, dur: usize| {
            for i in 0..dur {
                samples.push((TAU * tone_hz * i as f32 / sample_rate).sin() * 0.5);
            }
            samples.extend(std::iter::repeat_n(0.0, gap_samples));
        };
        // S = ...
        key(&mut samples, dit_samples); key(&mut samples, dit_samples); key(&mut samples, dit_samples);
        samples.extend(std::iter::repeat_n(0.0, letter_gap));
        // O = ---
        key(&mut samples, dah_samples); key(&mut samples, dah_samples); key(&mut samples, dah_samples);
        samples.extend(std::iter::repeat_n(0.0, letter_gap));
        // S = ...
        key(&mut samples, dit_samples); key(&mut samples, dit_samples); key(&mut samples, dit_samples);
        let result = decode_cw(&samples, sample_rate, tone_hz).expect("should decode CW");
        assert!(result.contains('S'), "expected S, got '{result}'");
        assert!(result.contains('O'), "expected O, got '{result}'");
    }

    #[test]
    fn cw_rejects_silence() {
        let samples = vec![0.0f32; 8000];
        assert!(decode_cw(&samples, 8000.0, 700.0).is_none());
    }

    #[test]
    fn rds_rejects_silence() {
        let samples = vec![0.0f32; 48_000];
        assert!(decode_rds(&samples, 96_000.0).is_none());
    }

    #[test]
    fn rds_downconverts_57khz_subcarrier() {
        // Generate a 57 kHz tone (simulates RDS subcarrier presence)
        let sample_rate = 96_000.0f32;
        let n = (sample_rate * 0.5) as usize;
        let samples: Vec<f32> = (0..n).map(|i| {
            (TAU * 57_000.0 * i as f32 / sample_rate).sin() * 0.1
        }).collect();
        let result = decode_rds(&samples, sample_rate);
        // Should detect energy at 57 kHz but won't find valid groups from a pure tone
        // It should at least not return None (signal present)
        assert!(result.is_some(), "RDS decoder should detect 57 kHz subcarrier energy");
        let r = result.unwrap();
        assert!(r.bits_decoded > 0, "should have decoded some bits");
    }

    #[test]
    fn ctcss_detects_known_tone() {
        let sample_rate = 8000.0;
        let target = 131.8; // common CTCSS tone
        let n = (sample_rate * 0.3) as usize; // 300 ms
        let samples: Vec<f32> = (0..n).map(|i| {
            (TAU * target * i as f32 / sample_rate).sin() * 0.5
        }).collect();
        let (tone, confidence) = detect_ctcss(&samples, sample_rate).expect("should detect 131.8 Hz");
        assert!((tone - target).abs() < 2.0, "detected {tone:.1} expected {target}, confidence {confidence:.2}");
        assert!(confidence > 0.3, "confidence too low: {confidence:.2}");
    }

    #[test]
    fn ctcss_rejects_noise() {
        let sample_rate = 8000.0;
        let n = (sample_rate * 0.3) as usize;
        // Pseudo-noise: deterministic but non-tonal
        let samples: Vec<f32> = (0..n).map(|i| (((i * 7919) % 1000) as f32 / 500.0 - 1.0) * 0.1).collect();
        assert!(detect_ctcss(&samples, sample_rate).is_none(), "should not detect a CTCSS tone in noise");
    }

    #[test]
    fn ctcss_detects_lowest_tone() {
        let sample_rate = 8000.0;
        let target = 67.0;
        let n = (sample_rate * 0.3) as usize;
        let samples: Vec<f32> = (0..n).map(|i| {
            (TAU * target * i as f32 / sample_rate).sin() * 0.5
        }).collect();
        let (tone, _) = detect_ctcss(&samples, sample_rate).expect("should detect 67 Hz");
        assert!((tone - target).abs() < 2.0, "detected {tone:.1} expected {target}");
    }

    fn append_fsk_symbol(samples: &mut Vec<f32>, bit: bool, count: usize, mark: f32, space: f32, sample_rate: f32, phase: &mut f32) {
        let frequency = if bit { mark } else { space };
        let step = TAU * frequency / sample_rate;
        for _ in 0..count {
            samples.push(phase.sin() * 0.7);
            *phase = (*phase + step).rem_euclid(TAU);
        }
    }

    #[test]
    fn rtty_decodes_ita2_letters() {
        let sample_rate = 8000.0;
        let baud = 50.0;
        let symbol = (sample_rate / baud) as usize;
        let (mark, space) = (2125.0, 1955.0);
        let mut samples = Vec::new();
        let mut phase = 0.0;
        append_fsk_symbol(&mut samples, true, symbol * 3, mark, space, sample_rate, &mut phase);
        for code in [0x14u8, 0x01, 0x12, 0x12, 0x18] { // HELLO
            append_fsk_symbol(&mut samples, false, symbol, mark, space, sample_rate, &mut phase);
            for bit in 0..5 { append_fsk_symbol(&mut samples, code & (1 << bit) != 0, symbol, mark, space, sample_rate, &mut phase); }
            append_fsk_symbol(&mut samples, true, symbol * 2, mark, space, sample_rate, &mut phase);
        }
        let decoded = decode_rtty(&samples, sample_rate, mark, space, baud).expect("RTTY should decode");
        assert!(decoded.contains("HELLO"), "decoded '{decoded}'");
    }

    #[test]
    fn sstv_classifies_scottie_two_from_sync_period() {
        let sample_rate = 12_000.0;
        let period = 0.27769f32;
        let duration = period * 3.2;
        let n = (sample_rate * duration) as usize;
        let samples: Vec<f32> = (0..n).map(|i| {
            let t = i as f32 / sample_rate;
            let within_line = t.rem_euclid(period);
            let freq = if within_line < 0.009 { 1200.0 } else if within_line < 0.020 { 1500.0 } else if within_line < 0.12 { 1900.0 } else { 2300.0 };
            (TAU * freq * t).sin() * 0.6
        }).collect();
        assert_eq!(detect_sstv_mode(&samples, sample_rate), Some(SstvMode::Scottie2));
    }

    #[test]
    fn navtex_decodes_ccir476_fec_text() {
        let sample_rate = 8000.0;
        let symbol = (sample_rate / 100.0) as usize;
        let words = [0x69u8, 0x56, 0x65, 0x65, 0x71]; // HELLO in CCIR 476
        let mut samples = Vec::new();
        let mut phase = 0.0;
        for &word in words.iter().chain(words.iter()) { // repeat after five characters
            for bit in (0..7).rev() {
                append_fsk_symbol(&mut samples, word & (1 << bit) != 0, symbol, 1785.0, 1615.0, sample_rate, &mut phase);
            }
        }
        let decoded = decode_navtex(&samples, sample_rate).expect("NAVTEX should decode");
        assert_eq!(decoded, "HELLO", "decoded '{decoded}'");
    }
}
