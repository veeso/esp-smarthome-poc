//! Cuts live audio into utterances with a slow AGC and an energy detector.

// Ported from esp32/components/tinyasr/tasr_seg.c:5-85 at commit
// c8d60325ba8bffd4bad65a9ac63c713ee64d1aae.

use core::fmt;

/// Samples of one input block (20 ms).
pub const BLOCK_SAMPLES: usize = 320;

/// Samples kept from before the speech onset (0.2 s).
pub const PREROLL_SAMPLES: usize = 3_200;

/// Silent blocks after which an utterance ends (0.8 s).
const HANG_BLOCKS: usize = 40;

/// Speech blocks below which a burst is a click, not an utterance.
const MIN_VOICED_BLOCKS: usize = 10;

/// Energy over the noise floor that starts speech (9 dB).
const START_RATIO: f32 = 8.0;

/// Energy over the noise floor that keeps speech going (4.8 dB).
const STAY_RATIO: f32 = 3.0;

/// What one block changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentEvent {
    /// Nothing to act on.
    Idle,
    /// Speech began.
    SpeechStarted,
    /// The speech was too short and is discarded.
    SpeechDropped,
    /// The utterance is complete.
    UtteranceReady,
}

/// Collects one utterance from live audio into a caller-owned buffer.
///
/// An utterance ends after 0.8 s of silence or when the capacity is reached.
///
/// # Examples
///
/// ```
/// use oido::{BLOCK_SAMPLES, SegmentEvent, Segmenter};
///
/// let mut buffer = [0; 16_000];
/// let mut segmenter = Segmenter::new(&mut buffer);
/// assert_eq!(segmenter.feed(&[0; BLOCK_SAMPLES]), SegmentEvent::Idle);
/// assert_eq!(segmenter.utterance().len(), BLOCK_SAMPLES);
/// ```
pub struct Segmenter<'b> {
    buffer: &'b mut [i16],
    capacity: usize,
    len: usize,
    gain: f32,
    noise_floor: f32,
    /// Whether the noise floor has its first value.
    primed: bool,
    speech: bool,
    silent_blocks: usize,
    voiced_blocks: usize,
    ready: bool,
}

impl fmt::Debug for Segmenter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Segmenter")
            .field("capacity", &self.capacity)
            .field("len", &self.len)
            .field("speech", &self.speech)
            .field("ready", &self.ready)
            .finish_non_exhaustive()
    }
}

impl<'b> Segmenter<'b> {
    /// A segmenter whose utterances fill at most `buffer`.
    #[must_use]
    pub fn new(buffer: &'b mut [i16]) -> Self {
        let capacity = buffer.len();
        let mut segmenter = Self {
            buffer,
            capacity,
            len: 0,
            gain: 0.0,
            noise_floor: 0.0,
            primed: false,
            speech: false,
            silent_blocks: 0,
            voiced_blocks: 0,
            ready: false,
        };
        segmenter.restart(capacity);
        segmenter
    }

    /// Forgets everything, including the gain and the noise floor.
    ///
    /// `capacity` bounds the next utterance and is clamped to the buffer.
    pub fn restart(&mut self, capacity: usize) {
        self.capacity = capacity.min(self.buffer.len());
        self.len = 0;
        self.gain = 4.0;
        self.noise_floor = 0.0;
        self.primed = false;
        self.speech = false;
        self.silent_blocks = 0;
        self.voiced_blocks = 0;
        self.ready = false;
    }

    /// Adds up to [`BLOCK_SAMPLES`] samples of 16 kHz audio.
    ///
    /// A longer block is truncated. After [`SegmentEvent::UtteranceReady`]
    /// the audio is ignored until [`Segmenter::restart`].
    pub fn feed(&mut self, block: &[i16]) -> SegmentEvent {
        if self.ready || block.is_empty() {
            return SegmentEvent::Idle;
        }
        let block = &block[..block.len().min(BLOCK_SAMPLES)];
        let voiced = self.detect(block);
        let mut amplified = [0i16; BLOCK_SAMPLES];
        for (out, &sample) in amplified.iter_mut().zip(block) {
            // The conversion saturates and truncates toward zero.
            *out = (f32::from(sample) * self.gain) as i16;
        }
        let amplified = &amplified[..block.len()];

        let mut event = SegmentEvent::Idle;
        if voiced {
            if !self.speech {
                event = SegmentEvent::SpeechStarted;
            }
            self.speech = true;
            self.silent_blocks = 0;
            self.voiced_blocks += 1;
        } else {
            self.silent_blocks += 1;
        }
        if !self.speech {
            // Keep a short pre-roll so that the first phoneme is not cut.
            self.keep_last(
                PREROLL_SAMPLES
                    .min(self.capacity)
                    .saturating_sub(amplified.len()),
            );
            self.append(amplified);
            return event;
        }
        self.append(amplified);
        if self.silent_blocks <= HANG_BLOCKS && self.len < self.capacity {
            return event;
        }
        if self.voiced_blocks >= MIN_VOICED_BLOCKS {
            self.ready = true;
            return SegmentEvent::UtteranceReady;
        }
        // Too short to be speech: back to idle with the tail as pre-roll.
        self.speech = false;
        self.silent_blocks = 0;
        self.voiced_blocks = 0;
        self.keep_last(PREROLL_SAMPLES);
        SegmentEvent::SpeechDropped
    }

    /// Returns the audio collected so far, after gain.
    #[must_use]
    pub fn utterance(&self) -> &[i16] {
        &self.buffer[..self.len]
    }

    /// Updates the noise floor and the gain, and tells whether `block` is speech.
    fn detect(&mut self, block: &[i16]) -> bool {
        let mut energy = 0.0f32;
        for &sample in block {
            let x = f32::from(sample) / 32_768.0;
            energy += x * x;
        }
        energy /= block.len() as f32;
        // The floor follows dips quickly and creeps up slowly, so that speech
        // never teaches it its own level.
        if !self.primed {
            self.noise_floor = if energy > 1e-9 { energy } else { 1e-9 };
            self.primed = true;
        }
        if energy < self.noise_floor {
            self.noise_floor = 0.9 * self.noise_floor + 0.1 * energy;
        } else {
            self.noise_floor *= 1.002;
        }
        if self.noise_floor < 1e-9 {
            self.noise_floor = 1e-9;
        }
        let ratio = if self.speech { STAY_RATIO } else { START_RATIO };
        let voiced = energy > 1e-8 && energy > ratio * self.noise_floor;
        if voiced {
            // Slow gain control towards about -20 dBFS.
            let level = libm::sqrtf(energy) * self.gain;
            if level > 1e-5 {
                self.gain *= libm::powf(0.1 / level, 0.02);
            }
            self.gain = self.gain.clamp(0.25, 64.0);
        }
        voiced
    }

    /// Drops all but the last `keep` samples.
    fn keep_last(&mut self, keep: usize) {
        if self.len > keep {
            self.buffer.copy_within(self.len - keep..self.len, 0);
            self.len = keep;
        }
    }

    /// Appends what fits within the capacity.
    fn append(&mut self, samples: &[i16]) {
        let take = samples.len().min(self.capacity - self.len);
        self.buffer[self.len..self.len + take].copy_from_slice(&samples[..take]);
        self.len += take;
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::*;

    /// A block that alternates between `amplitude` and `-amplitude`.
    const fn square(amplitude: i16) -> [i16; BLOCK_SAMPLES] {
        let mut block = [amplitude; BLOCK_SAMPLES];
        let mut i = 1;
        while i < BLOCK_SAMPLES {
            block[i] = -amplitude;
            i += 2;
        }
        block
    }

    const NOISE: [i16; BLOCK_SAMPLES] = square(30);
    const LOUD: [i16; BLOCK_SAMPLES] = square(8_000);

    /// Feeds `count` copies of `block` and returns the events.
    fn feed(segmenter: &mut Segmenter<'_>, block: &[i16], count: usize) -> Vec<SegmentEvent> {
        (0..count).map(|_| segmenter.feed(block)).collect()
    }

    /// The samples are compared within one unit instead of bit for bit.
    ///
    /// The gain control raises to the power 0.02 once per speech block.
    /// `libm::powf` and the `powf` of the computer's C library differ there by
    /// one unit in the last place, which now and then moves a truncated sample
    /// by one. With the computer's `powf` the buffers are identical. Lengths and
    /// speech decisions are exact.
    #[test]
    fn should_match_the_reference_segmenter() {
        let capacity = 16_000 * 20;
        let mut buffer = vec![0; capacity];
        let mut segmenter = Segmenter::new(&mut buffer);
        let mut reference = oido_reference::RefSegmenter::new(capacity);

        let mut audio = NOISE.repeat(50);
        audio.extend(oido_reference::clip("b05"));
        audio.resize(audio.len() + 16_000, 0);
        audio.extend(LOUD.repeat(5));
        audio.extend(NOISE.repeat(100));
        audio.extend(oido_reference::clip("b14"));
        audio.extend(NOISE.repeat(100));

        let mut utterances = 0;
        for (nth, block) in audio.chunks(BLOCK_SAMPLES).enumerate() {
            let event = segmenter.feed(block);
            let returned = reference.feed(block);
            let (actual, expected) = (segmenter.utterance(), reference.samples());
            assert_eq!(actual.len(), expected.len(), "block {nth}");
            let apart = actual
                .iter()
                .zip(expected)
                .position(|(&a, &b)| a.abs_diff(b) > 1);
            assert_eq!(apart, None, "block {nth}");
            let ready = event == SegmentEvent::UtteranceReady;
            assert_eq!(ready, returned != 0, "block {nth}");
            if ready {
                utterances += 1;
                segmenter.restart(capacity);
                reference.init();
            }
        }
        assert!(utterances >= 1, "no utterance was completed");
    }

    #[test]
    fn should_report_speech_onset_once() {
        let mut buffer = vec![0; 32_000];
        let mut segmenter = Segmenter::new(&mut buffer);
        assert_eq!(feed(&mut segmenter, &NOISE, 20), [SegmentEvent::Idle; 20]);
        assert_eq!(segmenter.feed(&LOUD), SegmentEvent::SpeechStarted);
        assert_eq!(feed(&mut segmenter, &LOUD, 4), [SegmentEvent::Idle; 4]);
    }

    #[test]
    fn should_report_a_dropped_burst() {
        let mut buffer = vec![0; 32_000];
        let mut segmenter = Segmenter::new(&mut buffer);
        feed(&mut segmenter, &NOISE, 20);
        feed(&mut segmenter, &LOUD, 5);
        assert_eq!(feed(&mut segmenter, &NOISE, 40), [SegmentEvent::Idle; 40]);
        assert_eq!(segmenter.feed(&NOISE), SegmentEvent::SpeechDropped);
        assert_eq!(segmenter.utterance().len(), PREROLL_SAMPLES);
    }

    #[test]
    fn should_latch_ready_until_restart() {
        let mut buffer = vec![0; 32_000];
        let mut segmenter = Segmenter::new(&mut buffer);
        feed(&mut segmenter, &NOISE, 20);
        feed(&mut segmenter, &LOUD, 20);
        let events = feed(&mut segmenter, &NOISE, 41);
        assert_eq!(events.last(), Some(&SegmentEvent::UtteranceReady));
        let len = segmenter.utterance().len();
        assert_eq!(feed(&mut segmenter, &LOUD, 3), [SegmentEvent::Idle; 3]);
        assert_eq!(segmenter.utterance().len(), len);

        segmenter.restart(32_000);
        assert_eq!(segmenter.feed(&NOISE), SegmentEvent::Idle);
        assert_eq!(segmenter.utterance().len(), BLOCK_SAMPLES);
    }

    #[test]
    fn should_ignore_an_empty_block() {
        let mut buffer = vec![0; 32_000];
        let mut segmenter = Segmenter::new(&mut buffer);
        assert_eq!(segmenter.feed(&[]), SegmentEvent::Idle);
        assert!(segmenter.utterance().is_empty());
    }

    #[test]
    fn should_complete_an_utterance_at_its_capacity() {
        let capacity = PREROLL_SAMPLES + 20 * BLOCK_SAMPLES;
        let mut buffer = vec![0; 32_000];
        let mut segmenter = Segmenter::new(&mut buffer);
        segmenter.restart(capacity);
        feed(&mut segmenter, &NOISE, 20);
        let events = feed(&mut segmenter, &LOUD, 20);
        assert_eq!(events[0], SegmentEvent::SpeechStarted);
        assert_eq!(events[1..19], [SegmentEvent::Idle; 18]);
        assert_eq!(events[19], SegmentEvent::UtteranceReady);
        assert_eq!(segmenter.utterance().len(), capacity);
    }

    #[test]
    fn should_drop_a_burst_that_reaches_its_capacity() {
        let capacity = PREROLL_SAMPLES + 5 * BLOCK_SAMPLES;
        let mut buffer = vec![0; 32_000];
        let mut segmenter = Segmenter::new(&mut buffer);
        segmenter.restart(capacity);
        feed(&mut segmenter, &NOISE, 20);
        let events = feed(&mut segmenter, &LOUD, 5);
        assert_eq!(events[4], SegmentEvent::SpeechDropped);
        assert_eq!(segmenter.utterance().len(), PREROLL_SAMPLES);
    }

    #[test]
    fn should_stay_within_a_buffer_shorter_than_the_preroll() {
        let mut buffer = vec![0; 700];
        let mut segmenter = Segmenter::new(&mut buffer);
        segmenter.restart(usize::MAX);
        feed(&mut segmenter, &NOISE, 20);
        assert_eq!(segmenter.utterance().len(), 700);
        feed(&mut segmenter, &LOUD, 60);
        assert_eq!(segmenter.utterance().len(), 700);
    }
}
