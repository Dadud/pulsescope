//! Native, clean-room POCSAG decoder.
//!
//! The audio input is demodulated 2-FSK discriminator/baseband audio: mark and
//! space must have opposite polarity.  The decoder performs adaptive DC
//! removal, integrate-and-dump bit slicing, polarity acquisition from the
//! POCSAG sync word, BCH(31,21) correction of up to two bad bits, and batch /
//! frame / message parsing.  It intentionally does not manufacture messages
//! when synchronization or error correction fails.

use serde::{Deserialize, Serialize};

/// POCSAG synchronization codeword (transmitted most-significant bit first).
pub const SYNC_CODEWORD: u32 = 0x7CD2_15D8;
/// POCSAG idle codeword.
pub const IDLE_CODEWORD: u32 = 0x7A89_C197;
const BCH_GENERATOR: u32 = 0x769; // x^10+x^9+x^8+x^6+x^5+x^3+1
const WORDS_PER_BATCH: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PocsagEncoding {
    Numeric,
    Alphanumeric,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PocsagMessage {
    /// Receiver identity code (capcode), including the three frame bits.
    pub ric: u32,
    /// The two function bits from the address codeword.
    pub function: u8,
    /// POCSAG frame in which the address was received (0..=7).
    pub frame: u8,
    pub encoding: PocsagEncoding,
    pub text: String,
    /// Number of corrected data/address codewords contributing to this message.
    pub corrected_codewords: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PocsagBaud {
    Baud1200,
    Baud2400,
}

impl PocsagBaud {
    pub const fn value(self) -> u32 {
        match self {
            Self::Baud1200 => 1_200,
            Self::Baud2400 => 2_400,
        }
    }
}

#[derive(Debug)]
struct PendingMessage {
    ric: u32,
    function: u8,
    frame: u8,
    data: Vec<u32>,
    corrected_codewords: u32,
}

/// Stateful decoder.  Both audio and already-sliced bits may be supplied in
/// arbitrary chunk sizes.  Call [`flush`](Self::flush) at end of stream.
#[derive(Debug)]
pub struct PocsagDecoder {
    sample_rate: u32,
    baud: u32,
    sample_clock: u32,
    symbol_sum: f32,
    symbol_samples: u32,
    dc: f32,

    hunt_shift: u32,
    hunt_bits: u8,
    inverted: bool,
    synchronized: bool,
    word: u32,
    word_bits: u8,
    batch_word: usize,
    pending: Option<PendingMessage>,
    corrected_words: u64,
    rejected_words: u64,
}

/// Complex-IQ front end for POCSAG FSK captures. This performs phase
/// discrimination and hands the resulting baseband stream to the native
/// symbol slicer; clock recovery remains in `PocsagDecoder`.
#[derive(Debug)]
pub struct IqDecoder {
    decoder: PocsagDecoder,
    previous: Option<(f32, f32)>,
}

impl IqDecoder {
    pub fn new(sample_rate: u32, baud: PocsagBaud) -> Self {
        Self {
            decoder: PocsagDecoder::new(sample_rate, baud),
            previous: None,
        }
    }

    pub fn push_iq(&mut self, samples: &[(f32, f32)]) -> Vec<PocsagMessage> {
        let mut audio = Vec::with_capacity(samples.len());
        for &(i, q) in samples {
            if !i.is_finite() || !q.is_finite() {
                self.previous = None;
                continue;
            }
            if let Some((pi, pq)) = self.previous {
                audio.push((q * pi - i * pq).atan2(i * pi + q * pq));
            }
            self.previous = Some((i, q));
        }
        self.decoder.push_audio(&audio)
    }

    pub fn flush(&mut self) -> Vec<PocsagMessage> {
        self.decoder.flush()
    }
    pub fn corrected_words(&self) -> u64 {
        self.decoder.corrected_words()
    }
    pub fn rejected_words(&self) -> u64 {
        self.decoder.rejected_words()
    }
}

impl PocsagDecoder {
    pub fn new(sample_rate: u32, baud: PocsagBaud) -> Self {
        assert!(sample_rate > 0, "sample rate must be non-zero");
        Self {
            sample_rate,
            baud: baud.value(),
            sample_clock: 0,
            symbol_sum: 0.0,
            symbol_samples: 0,
            dc: 0.0,
            hunt_shift: 0,
            hunt_bits: 0,
            inverted: false,
            synchronized: false,
            word: 0,
            word_bits: 0,
            batch_word: 0,
            pending: None,
            corrected_words: 0,
            rejected_words: 0,
        }
    }

    /// Feed discriminator/baseband PCM.  A positive and negative level may
    /// represent either polarity; sync acquisition detects and corrects it.
    pub fn push_audio(&mut self, samples: &[f32]) -> Vec<PocsagMessage> {
        let mut messages = Vec::new();
        for &sample in samples {
            if !sample.is_finite() {
                continue;
            }
            // Slow enough not to follow ordinary data runs, but removes sound-
            // card / discriminator offset across a long-lived stream.
            self.dc += (sample - self.dc) * 0.0005;
            self.symbol_sum += sample - self.dc;
            self.symbol_samples += 1;
            self.sample_clock += self.baud;
            if self.sample_clock >= self.sample_rate {
                self.sample_clock -= self.sample_rate;
                let bit = self.symbol_sum >= 0.0;
                self.symbol_sum = 0.0;
                self.symbol_samples = 0;
                self.push_bit_internal(bit, &mut messages);
            }
        }
        messages
    }

    /// Feed hard-decision bits, in over-the-air order (MSB first per codeword).
    /// This is also a useful integration seam for a higher quality external
    /// timing recovery loop.
    pub fn push_bits(&mut self, bits: &[bool]) -> Vec<PocsagMessage> {
        let mut messages = Vec::new();
        for &bit in bits {
            self.push_bit_internal(bit, &mut messages);
        }
        messages
    }

    /// Emit a message still open at end of a recording/stream.
    pub fn flush(&mut self) -> Vec<PocsagMessage> {
        self.finish_pending().into_iter().collect()
    }

    pub fn corrected_words(&self) -> u64 {
        self.corrected_words
    }

    pub fn rejected_words(&self) -> u64 {
        self.rejected_words
    }

    fn push_bit_internal(&mut self, raw_bit: bool, out: &mut Vec<PocsagMessage>) {
        if !self.synchronized {
            self.hunt_shift = (self.hunt_shift << 1) | raw_bit as u32;
            self.hunt_bits = self.hunt_bits.saturating_add(1);
            if self.hunt_bits >= 32 {
                let normal_distance = (self.hunt_shift ^ SYNC_CODEWORD).count_ones();
                let inverse_distance = ((!self.hunt_shift) ^ SYNC_CODEWORD).count_ones();
                if normal_distance <= 2 || inverse_distance <= 2 {
                    self.inverted = inverse_distance < normal_distance;
                    self.synchronized = true;
                    self.word = 0;
                    self.word_bits = 0;
                    self.batch_word = 0;
                }
            }
            return;
        }

        let bit = raw_bit ^ self.inverted;
        self.word = (self.word << 1) | bit as u32;
        self.word_bits += 1;
        if self.word_bits != 32 {
            return;
        }

        let raw_word = self.word;
        self.word = 0;
        self.word_bits = 0;
        self.process_codeword(raw_word, out);
        self.batch_word += 1;
        if self.batch_word == WORDS_PER_BATCH {
            // The next 32 transmitted bits are a new sync word.  Return to the
            // rolling hunt so a dropped bit does not permanently misalign us.
            self.synchronized = false;
            self.hunt_shift = 0;
            self.hunt_bits = 0;
        }
    }

    fn process_codeword(&mut self, raw: u32, out: &mut Vec<PocsagMessage>) {
        let Some((word, corrected_bits)) = correct_codeword(raw) else {
            self.rejected_words += 1;
            return;
        };
        if corrected_bits != 0 {
            self.corrected_words += 1;
        }

        if word == IDLE_CODEWORD {
            if let Some(message) = self.finish_pending() {
                out.push(message);
            }
            return;
        }

        if word & 0x8000_0000 == 0 {
            if let Some(message) = self.finish_pending() {
                out.push(message);
            }
            let frame = (self.batch_word / 2) as u8;
            let address = (word >> 13) & 0x3ffff;
            self.pending = Some(PendingMessage {
                ric: (address << 3) | frame as u32,
                function: ((word >> 11) & 0x03) as u8,
                frame,
                data: Vec::new(),
                corrected_codewords: u32::from(corrected_bits != 0),
            });
        } else if let Some(pending) = self.pending.as_mut() {
            pending.data.push((word >> 11) & 0x000f_ffff);
            pending.corrected_codewords += u32::from(corrected_bits != 0);
        }
    }

    fn finish_pending(&mut self) -> Option<PocsagMessage> {
        let pending = self.pending.take()?;
        if pending.data.is_empty() {
            return None;
        }
        let encoding = if pending.function == 0 {
            PocsagEncoding::Numeric
        } else {
            PocsagEncoding::Alphanumeric
        };
        let text = match encoding {
            PocsagEncoding::Numeric => decode_numeric(&pending.data),
            PocsagEncoding::Alphanumeric => decode_alphanumeric(&pending.data),
        };
        if text.is_empty() {
            return None;
        }
        Some(PocsagMessage {
            ric: pending.ric,
            function: pending.function,
            frame: pending.frame,
            encoding,
            text,
            corrected_codewords: pending.corrected_codewords,
        })
    }
}

/// Validate the extended BCH(31,21) codeword and correct up to two flipped
/// bits.  Brute force is deliberately used here: at only 529 candidates per
/// bad word it is small, obvious, dependency-free, and avoids syndrome-table
/// mistakes.  The final POCSAG bit is the even-parity extension.
fn correct_codeword(word: u32) -> Option<(u32, u8)> {
    if valid_codeword(word) {
        return Some((word, 0));
    }
    for bit in 0..32 {
        let candidate = word ^ (1u32 << bit);
        if valid_codeword(candidate) {
            return Some((candidate, 1));
        }
    }
    for first in 0..31 {
        for second in (first + 1)..32 {
            let candidate = word ^ (1u32 << first) ^ (1u32 << second);
            if valid_codeword(candidate) {
                return Some((candidate, 2));
            }
        }
    }
    None
}

fn valid_codeword(word: u32) -> bool {
    word.count_ones() & 1 == 0 && bch_remainder(word >> 1) == 0
}

fn bch_remainder(mut value: u32) -> u32 {
    for bit in (10..=30).rev() {
        if value & (1u32 << bit) != 0 {
            value ^= BCH_GENERATOR << (bit - 10);
        }
    }
    value & 0x03ff
}

fn decode_numeric(words: &[u32]) -> String {
    const TABLE: &[u8; 16] = b"084 2.6]195-3U7[";
    let mut text = String::with_capacity(words.len() * 5);
    for &data in words {
        for shift in (0..20).step_by(4) {
            text.push(TABLE[((data >> shift) & 0x0f) as usize] as char);
        }
    }
    text.trim_end_matches([' ', '\0']).to_owned()
}

fn decode_alphanumeric(words: &[u32]) -> String {
    let mut accumulator = 0u64;
    let mut available = 0usize;
    let mut text = String::new();
    for &data in words {
        accumulator |= (data as u64) << available;
        available += 20;
        while available >= 7 {
            let character = (accumulator & 0x7f) as u8;
            accumulator >>= 7;
            available -= 7;
            match character {
                0x00 | 0x03 => return text.trim_end().to_owned(),
                0x0a | 0x0d => text.push('\n'),
                0x20..=0x7e => text.push(character as char),
                _ => text.push('\u{fffd}'),
            }
        }
    }
    text.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_codeword(data21: u32) -> u32 {
        let mut bch = (data21 & 0x1f_ffff) << 10;
        bch |= bch_remainder(bch);
        let mut word = bch << 1;
        if word.count_ones() & 1 != 0 {
            word |= 1;
        }
        assert!(valid_codeword(word));
        word
    }

    fn address_word(ric: u32, function: u8) -> u32 {
        encode_codeword(((ric >> 3) << 2) | (function as u32 & 3))
    }

    fn message_word(data: u32) -> u32 {
        encode_codeword((1 << 20) | (data & 0x000f_ffff))
    }

    fn word_bits(word: u32, output: &mut Vec<bool>) {
        output.extend((0..32).rev().map(|bit| word & (1 << bit) != 0));
    }

    fn alpha_words(text: &str) -> Vec<u32> {
        let mut bits = 0u128;
        let mut count = 0usize;
        for byte in text.bytes().chain(std::iter::once(0x03)) {
            bits |= (byte as u128) << count;
            count += 7;
        }
        (0..count.div_ceil(20))
            .map(|index| message_word(((bits >> (index * 20)) & 0x000f_ffff) as u32))
            .collect()
    }

    fn numeric_word(text: &str) -> u32 {
        const TABLE: &[u8; 16] = b"084 2.6]195-3U7[";
        let mut data = 0u32;
        for (index, byte) in text
            .bytes()
            .chain(std::iter::repeat(b' '))
            .take(5)
            .enumerate()
        {
            let nibble = TABLE.iter().position(|&item| item == byte).unwrap() as u32;
            data |= nibble << (index * 4);
        }
        message_word(data)
    }

    fn batch(ric: u32, function: u8, message_words: &[u32]) -> Vec<bool> {
        let frame = (ric & 7) as usize;
        let address_index = frame * 2;
        let mut words = [IDLE_CODEWORD; WORDS_PER_BATCH];
        words[address_index] = address_word(ric, function);
        for (destination, source) in words[address_index + 1..].iter_mut().zip(message_words) {
            *destination = *source;
        }
        let mut bits = Vec::new();
        word_bits(SYNC_CODEWORD, &mut bits);
        for word in words {
            word_bits(word, &mut bits);
        }
        bits
    }

    #[test]
    fn bch_corrects_one_and_two_bits_and_rejects_three() {
        let original = message_word(0x5a55a);
        assert_eq!(correct_codeword(original), Some((original, 0)));
        assert_eq!(correct_codeword(original ^ (1 << 17)), Some((original, 1)));
        assert_eq!(
            correct_codeword(original ^ (1 << 2) ^ (1 << 29)),
            Some((original, 2))
        );
        // The extended code has minimum distance six, so any three-bit error
        // cannot be mistaken for a codeword within correction distance two.
        assert_eq!(correct_codeword(original ^ 1 ^ (1 << 8) ^ (1 << 19)), None);
    }

    #[test]
    fn parses_numeric_message_and_frame_derived_ric() {
        let ric = 123_453; // frame 5
        let bits = batch(ric, 0, &[numeric_word("12345")]);
        let mut decoder = PocsagDecoder::new(24_000, PocsagBaud::Baud1200);
        let messages = decoder.push_bits(&bits);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].ric, ric);
        assert_eq!(messages[0].frame, 5);
        assert_eq!(messages[0].encoding, PocsagEncoding::Numeric);
        assert_eq!(messages[0].text, "12345");
    }

    #[test]
    fn parses_alphanumeric_across_codewords() {
        let ric = 42_002;
        let bits = batch(ric, 3, &alpha_words("HELLO"));
        let mut decoder = PocsagDecoder::new(48_000, PocsagBaud::Baud2400);
        let messages = decoder.push_bits(&bits);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].ric, ric);
        assert_eq!(messages[0].function, 3);
        assert_eq!(messages[0].text, "HELLO");
    }

    #[test]
    fn streaming_audio_decodes_both_baud_rates_and_inverted_polarity() {
        for baud in [PocsagBaud::Baud1200, PocsagBaud::Baud2400] {
            let bits = batch(777_216, 3, &alpha_words("PAGE"));
            let samples_per_bit = 24_000 / baud.value();
            let mut audio = Vec::new();
            for bit in bits {
                let level = if bit { -0.8 } else { 0.8 }; // deliberately inverted
                audio.extend(std::iter::repeat(level).take(samples_per_bit as usize));
            }
            let mut decoder = PocsagDecoder::new(24_000, baud);
            let mut messages = Vec::new();
            // Chunk boundaries deliberately do not coincide with symbols.
            for chunk in audio.chunks(137) {
                messages.extend(decoder.push_audio(chunk));
            }
            assert_eq!(messages.len(), 1, "baud {}", baud.value());
            assert_eq!(messages[0].text, "PAGE");
        }
    }

    #[test]
    fn correction_is_reported_on_decoded_message() {
        let ric = 88_888;
        let mut words = alpha_words("OK");
        words[0] ^= (1 << 4) | (1 << 25);
        let bits = batch(ric, 1, &words);
        let mut decoder = PocsagDecoder::new(24_000, PocsagBaud::Baud1200);
        let messages = decoder.push_bits(&bits);
        assert_eq!(messages[0].text, "OK");
        assert_eq!(messages[0].corrected_codewords, 1);
        assert_eq!(decoder.corrected_words(), 1);
    }
}
