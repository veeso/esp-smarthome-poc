//! The TLM1 GRU language model.

// Ported from esp32/components/tinyasr/tinyasr_lm.c:12-83 and 151-198 at
// commit c8d60325ba8bffd4bad65a9ac63c713ee64d1aae.

use alloc::vec::Vec;
use core::fmt;

use crate::mathx::{Tables, logf};
use crate::model::{Cursor, Truncated, size};
use crate::quant::{QLinear, quant_rows};
use crate::transcriber::{TranscribeError, zeroed};

/// Bytes of the header that precedes the tensors.
const HEADER_BYTES: usize = 32;

/// Why a blob is not a usable language model.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanguageModelError {
    /// The blob does not start with the TLM1 magic.
    BadMagic,
    /// The blob does not start at a 16-byte boundary.
    Misaligned,
    /// The blob is shorter than its contents.
    Truncated {
        /// Bytes that the next part of the model ends at.
        needed: usize,
        /// Bytes of the blob.
        len: usize,
    },
    /// The header describes a model that cannot run.
    Unsupported(&'static str),
}

impl fmt::Display for LanguageModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => f.write_str("not an Oído TLM1 language model"),
            Self::Misaligned => f.write_str("the language model is not aligned to 16 bytes"),
            Self::Truncated { needed, len } => write!(
                f,
                "the language model is truncated: it needs {needed} bytes and has {len}"
            ),
            Self::Unsupported(what) => write!(f, "unsupported language model: {what}"),
        }
    }
}

impl core::error::Error for LanguageModelError {}

impl From<Truncated> for LanguageModelError {
    fn from(error: Truncated) -> Self {
        Self::Truncated {
            needed: error.needed,
            len: error.len,
        }
    }
}

/// A GRU language model over the tokens of an acoustic model.
///
/// # Examples
///
/// ```
/// use oido::{Aligned, LanguageModel, LanguageModelError};
///
/// let error = LanguageModel::parse(&Aligned([0u8; 64]).0).unwrap_err();
/// assert_eq!(error, LanguageModelError::BadMagic);
/// ```
#[derive(Clone)]
pub struct LanguageModel<'a> {
    vocab: usize,
    embedding_size: usize,
    hidden: usize,
    weight_percent: u32,
    bonus_percent: u32,
    /// One row per token, whose scale dequantizes its embedding.
    embedding: QLinear<'a>,
    input: QLinear<'a>,
    recurrent: QLinear<'a>,
    output: QLinear<'a>,
}

impl fmt::Debug for LanguageModel<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LanguageModel")
            .field("vocab_size", &self.vocab)
            .field("hidden_size", &self.hidden)
            .finish_non_exhaustive()
    }
}

impl<'a> LanguageModel<'a> {
    /// Parses a TLM1 language model; the weights stay in `blob`.
    ///
    /// # Errors
    ///
    /// Returns [`LanguageModelError`] when `blob` is not 16-byte aligned or is
    /// not a complete TLM1 language model.
    pub fn parse(blob: &'a [u8]) -> Result<Self, LanguageModelError> {
        if !blob.as_ptr().addr().is_multiple_of(16) {
            return Err(LanguageModelError::Misaligned);
        }
        if blob.get(..4) != Some(b"TLM1") {
            return Err(LanguageModelError::BadMagic);
        }
        let mut cursor = Cursor { blob, pos: 0 };
        let header = cursor.bytes(HEADER_BYTES)?;
        let word = |index: usize| {
            let at = 4 + 4 * index;
            u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
        };
        let (vocab, embedding_size, hidden) =
            (word(0) as usize, word(1) as usize, word(2) as usize);
        // Bounding every dimension keeps the size arithmetic from overflowing.
        if [vocab, embedding_size, hidden]
            .iter()
            .any(|&n| n == 0 || n > usize::from(u16::MAX))
        {
            return Err(LanguageModelError::Unsupported("model dimensions"));
        }
        Ok(Self {
            vocab,
            embedding_size,
            hidden,
            weight_percent: word(3),
            bonus_percent: word(4),
            embedding: cursor.linear(vocab, embedding_size, 8)?,
            input: cursor.linear(size(hidden, 3), embedding_size, 8)?,
            recurrent: cursor.linear(size(hidden, 3), hidden, 8)?,
            output: cursor.linear(vocab, hidden, 8)?,
        })
    }

    /// Returns the number of tokens.
    #[must_use]
    pub fn vocab_size(&self) -> usize {
        self.vocab
    }

    /// Returns the beam search weight stored in the file, or 0 when it has none.
    #[must_use]
    pub fn recommended_weight(&self) -> f32 {
        self.weight_percent as f32 / 100.0
    }

    /// Returns the token bonus stored in the file, or 0 when it has none.
    #[must_use]
    pub fn recommended_token_bonus(&self) -> f32 {
        self.bonus_percent as f32 / 100.0
    }

    /// Width of one hidden state.
    pub(crate) fn hidden_size(&self) -> usize {
        self.hidden
    }

    /// One GRU step for each token.
    ///
    /// `previous` and `next` hold one hidden state per token and `logp` one row
    /// of log-probabilities per token. A zero state starts a sentence.
    pub(crate) fn steps(
        &self,
        tokens: &[u32],
        previous: &[f32],
        next: &mut [f32],
        logp: &mut [f32],
        scratch: &mut LmScratch,
    ) {
        let rows = tokens.len();
        if rows == 0 {
            return;
        }
        let (embedding_size, hidden) = (self.embedding_size, self.hidden);
        let weights: &[i8] = bytemuck::cast_slice(self.embedding.weights);
        for (row, &token) in scratch
            .embedded
            .chunks_exact_mut(embedding_size)
            .zip(tokens)
        {
            let token = token as usize;
            let stored = &weights[token * self.embedding.kp..][..embedding_size];
            for (value, &weight) in row.iter_mut().zip(stored) {
                *value = f32::from(weight) * self.embedding.scales[token];
            }
        }
        scratch.tile.apply(
            &self.input,
            &scratch.embedded,
            rows,
            embedding_size,
            &mut scratch.input_gates,
        );
        scratch.tile.apply(
            &self.recurrent,
            previous,
            rows,
            hidden,
            &mut scratch.hidden_gates,
        );

        let tables = &scratch.tables;
        let states = next
            .chunks_exact_mut(hidden)
            .zip(previous.chunks_exact(hidden));
        let gates = scratch
            .input_gates
            .chunks_exact(3 * hidden)
            .zip(scratch.hidden_gates.chunks_exact(3 * hidden));
        for ((next, previous), (input, recurrent)) in states.zip(gates).take(rows) {
            for (j, (next, &previous)) in next.iter_mut().zip(previous).enumerate() {
                let reset = tables.fast_sigmoid(input[j] + recurrent[j]);
                let update = tables.fast_sigmoid(input[hidden + j] + recurrent[hidden + j]);
                let candidate =
                    tables.fast_tanh(input[2 * hidden + j] + reset * recurrent[2 * hidden + j]);
                *next = (1.0 - update) * candidate + update * previous;
            }
        }

        let logp = &mut logp[..rows * self.vocab];
        scratch.tile.apply(&self.output, next, rows, hidden, logp);
        for row in logp.chunks_exact_mut(self.vocab) {
            let mut max = row[0];
            for &value in &row[1..] {
                if value > max {
                    max = value;
                }
            }
            let mut sum = 0.0f32;
            for &value in row.iter() {
                sum += tables.fast_expneg(max - value);
            }
            let log_sum = max + logf(sum);
            for value in row {
                *value -= log_sum;
            }
        }
    }
}

/// Working buffers of up to a fixed number of simultaneous steps.
#[derive(Debug)]
pub(crate) struct LmScratch {
    tables: Tables,
    tile: LmTile,
    embedded: Vec<f32>,
    input_gates: Vec<f32>,
    hidden_gates: Vec<f32>,
}

/// Quantized activations of one batch.
#[derive(Debug)]
struct LmTile {
    xq: Vec<i8>,
    ldq: usize,
    xs: Vec<f32>,
}

impl LmTile {
    fn apply(&mut self, linear: &QLinear<'_>, x: &[f32], rows: usize, width: usize, y: &mut [f32]) {
        quant_rows(
            x,
            rows,
            width,
            width,
            &mut self.xq,
            self.ldq,
            linear.kp,
            &mut self.xs,
        );
        linear.forward(&self.xq, self.ldq, &self.xs, rows, y, linear.n);
    }
}

impl LmScratch {
    pub(crate) fn new(lm: &LanguageModel<'_>, batch: usize) -> Result<Self, TranscribeError> {
        let ldq = lm.input.kp.max(lm.recurrent.kp).max(lm.output.kp);
        Ok(Self {
            tables: Tables::new(),
            tile: LmTile {
                xq: zeroed(batch * ldq)?,
                ldq,
                xs: zeroed(batch)?,
            },
            embedded: zeroed(batch * lm.embedding_size)?,
            input_gates: zeroed(batch * 3 * lm.hidden)?,
            hidden_gates: zeroed(batch * 3 * lm.hidden)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::mathx::tests::XorShift32;

    #[test]
    fn lm_should_parse_the_v2_header() {
        let lm = LanguageModel::parse(oido_reference::lm_blob()).unwrap();
        assert_eq!(lm.vocab_size(), 1025);
        assert!((lm.recommended_weight() - 0.30).abs() < 1e-6);
        assert!((lm.recommended_token_bonus() - 1.00).abs() < 1e-6);
    }

    #[test]
    fn lm_should_reject_bad_input() {
        let blob = oido_reference::lm_blob();
        let copy = oido_reference::aligned_copy(blob);
        assert_eq!(
            LanguageModel::parse(&copy[1..]).unwrap_err(),
            LanguageModelError::Misaligned
        );
        copy[0] = b'X';
        assert_eq!(
            LanguageModel::parse(copy).unwrap_err(),
            LanguageModelError::BadMagic
        );
        for len in [10, 32, blob.len() - 1] {
            let error = LanguageModel::parse(&blob[..len]).unwrap_err();
            assert!(
                matches!(error, LanguageModelError::Truncated { needed, len: actual }
                    if actual == len && needed > len),
                "length {len}: {error:?}"
            );
        }
        // A zero hidden width.
        let copy = oido_reference::aligned_copy(blob);
        copy[12..16].fill(0);
        assert_eq!(
            LanguageModel::parse(copy).unwrap_err(),
            LanguageModelError::Unsupported("model dimensions")
        );
    }

    /// Steps both engines from `previous` and returns the new states.
    #[track_caller]
    fn check_step(tokens: &[u32], previous: &[f32]) -> Vec<f32> {
        let lm = LanguageModel::parse(oido_reference::lm_blob()).unwrap();
        let reference = oido_reference::RefLm::load(oido_reference::lm_blob());
        let mut scratch = LmScratch::new(&lm, tokens.len()).unwrap();
        let mut h = vec![0.0; previous.len()];
        let mut logp = vec![0.0; tokens.len() * lm.vocab_size()];
        lm.steps(tokens, previous, &mut h, &mut logp, &mut scratch);
        let (expected_h, expected_logp) = reference.steps(tokens, previous);
        let bits = |x: &[f32]| x.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&h), bits(&expected_h));
        assert_eq!(bits(&logp), bits(&expected_logp));
        h
    }

    #[test]
    fn lm_steps_should_match_the_reference() {
        let lm = LanguageModel::parse(oido_reference::lm_blob()).unwrap();
        let width = lm.hidden_size();
        let begin = check_step(&[1024], &vec![0.0; width]);
        let three = check_step(&[5, 17, 1023], &begin.repeat(3));
        let mut rng = XorShift32::new();
        let tokens: Vec<u32> = (0..16).map(|_| rng.next_u32() % 1025).collect();
        let previous: Vec<f32> = three.iter().copied().cycle().take(16 * width).collect();
        check_step(&tokens, &previous);
    }
}
