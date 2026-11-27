//! Transcription of one utterance.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::beam::{BeamConfig, BeamDecoder};
use crate::conformer::{Scratch, layer};
use crate::features::features;
use crate::frontend::subsample;
use crate::head::{ctc_log_probs, detokenize, greedy, head_blocks};
use crate::lm::LanguageModel;
use crate::mathx::Tables;
use crate::model::Model;
use crate::posenc::positions;

/// How the CTC output turns into tokens.
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub enum Decoding<'m> {
    /// The most likely class of each frame, with repeats and blanks removed.
    Greedy,
    /// A prefix beam search, which a language model can steer.
    Beam {
        /// The language model, over the tokens of the acoustic model.
        language_model: Option<&'m LanguageModel<'m>>,
        /// Widths and weights of the search.
        config: BeamConfig,
    },
}

/// Why a transcription cannot run.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscribeError {
    /// A working buffer does not fit in the heap.
    OutOfMemory,
    /// The beam width or the candidate count is out of range.
    InvalidBeamConfig,
    /// The language model and the acoustic model have different classes.
    VocabularyMismatch {
        /// Classes of the acoustic model, the blank included.
        model: usize,
        /// Tokens of the language model.
        language_model: usize,
    },
}

impl fmt::Display for TranscribeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfMemory => f.write_str("not enough memory to transcribe"),
            Self::InvalidBeamConfig => f.write_str("invalid beam search configuration"),
            Self::VocabularyMismatch {
                model,
                language_model,
            } => write!(
                f,
                "the language model has {language_model} tokens for {model} classes"
            ),
        }
    }
}

impl core::error::Error for TranscribeError {}

/// Transcribes utterances with one acoustic model.
///
/// # Examples
///
/// ```
/// use oido::{Decoding, Model, TranscribeError, Transcriber};
///
/// fn transcribe(model: &Model<'_>, pcm: &[i16]) -> Result<String, TranscribeError> {
///     Transcriber::new(model, Decoding::Greedy)?.transcribe(pcm)
/// }
/// ```
pub struct Transcriber<'m> {
    model: &'m Model<'m>,
    tables: Tables,
    beam: Option<Beam<'m>>,
}

/// The beam search and the frame that it reads.
#[derive(Debug)]
struct Beam<'m> {
    decoder: BeamDecoder<'m>,
    /// Log-probabilities of one frame, the blank first.
    logp: Vec<f32>,
}

impl fmt::Debug for Transcriber<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transcriber")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl<'m> Transcriber<'m> {
    /// Prepares the lookup tables of `model` and the state of the decoding.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError::InvalidBeamConfig`] for a width or `top_k`
    /// out of range, [`TranscribeError::VocabularyMismatch`] for a language
    /// model of other tokens, and [`TranscribeError::OutOfMemory`] when the
    /// decoder state does not fit in the heap.
    pub fn new(model: &'m Model<'m>, decoding: Decoding<'m>) -> Result<Self, TranscribeError> {
        let beam = match decoding {
            Decoding::Greedy => None,
            Decoding::Beam {
                language_model,
                config,
            } => {
                let classes = model.config().vocab_size + 1;
                Some(Beam {
                    decoder: BeamDecoder::new(language_model, classes, config)?,
                    logp: zeroed(classes)?,
                })
            }
        };
        Ok(Self {
            model,
            tables: Tables::new(),
            beam,
        })
    }

    /// Transcribes one utterance of 16 kHz mono PCM16 audio.
    ///
    /// Audio shorter than 320 samples (20 ms) gives an empty text.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError::OutOfMemory`] when the working buffers do not
    /// fit in the heap. Their size grows with the length of `pcm`.
    pub fn transcribe(&mut self, pcm: &[i16]) -> Result<String, TranscribeError> {
        let mut tokens = Vec::new();
        let (model, tables) = (self.model, &self.tables);
        if let Some(beam) = &mut self.beam {
            beam.decoder.reset();
            encode(model, tables, pcm, &mut |row| {
                ctc_log_probs(row, tables, &mut beam.logp);
                beam.decoder.step(&beam.logp);
            })?;
            beam.decoder.best(&mut tokens);
        } else {
            let blank = model.config().vocab_size as u32;
            let mut previous = None;
            encode(model, tables, pcm, &mut |row| {
                let best = greedy(row);
                if previous != Some(best) && best != blank {
                    tokens.push(best);
                }
                previous = Some(best);
            })?;
        }
        let mut text = String::new();
        detokenize(self.model, &tokens, &mut text);
        Ok(text)
    }

    /// The raw logits of `pcm`, for the comparison with the C engine.
    #[cfg(test)]
    fn logits(&mut self, pcm: &[i16]) -> Result<Vec<f32>, TranscribeError> {
        let mut logits = Vec::new();
        encode(self.model, &self.tables, pcm, &mut |row| {
            logits.extend_from_slice(row);
        })?;
        Ok(logits)
    }
}

/// Runs the encoder and the CTC head, handing each logit row to `on_row`.
fn encode(
    model: &Model<'_>,
    tables: &Tables,
    pcm: &[i16],
    on_row: &mut dyn FnMut(&[f32]),
) -> Result<(), TranscribeError> {
    let features = features(model, pcm)?;
    if features.frames == 0 {
        return Ok(());
    }
    let mut x = subsample(model, &features)?;
    drop(features);
    let (_, positions) = positions(x.rows, x.d)?;
    let mut scratch = Scratch::new(model, x.rows)?;
    for block in &model.layers {
        layer(block, &mut x, &positions, &mut scratch, tables);
    }
    head_blocks(model, &x, &mut scratch, on_row);
    Ok(())
}

/// A default-filled buffer, or [`TranscribeError::OutOfMemory`].
pub(crate) fn zeroed<T>(len: usize) -> Result<Vec<T>, TranscribeError>
where
    T: Copy + Default,
{
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(len)
        .map_err(|_error| TranscribeError::OutOfMemory)?;
    buffer.resize(len, T::default());
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model<'static> {
        Model::parse(oido_reference::model_blob()).unwrap()
    }

    fn reference(pcm: &[i16]) -> oido_reference::RefTranscript {
        let model = oido_reference::RefModel::load(oido_reference::model_blob());
        oido_reference::transcribe(&model, None, 0, 0, pcm)
    }

    #[test]
    fn should_match_reference_logits_on_the_clips() {
        let model = model();
        let mut transcriber = Transcriber::new(&model, Decoding::Greedy).unwrap();
        for name in ["b05", "b14", "b16"] {
            let pcm = oido_reference::clip(name);
            let actual = transcriber.logits(&pcm).unwrap();
            let expected = reference(&pcm);
            assert_eq!(actual.len(), expected.frames * 1025, "{name}");
            let differing = actual
                .iter()
                .zip(&expected.logits)
                .position(|(a, b)| a.to_bits() != b.to_bits());
            assert_eq!(differing, None, "{name}");
        }
    }

    #[test]
    fn should_return_empty_text_for_short_audio() {
        let model = model();
        let mut transcriber = Transcriber::new(&model, Decoding::Greedy).unwrap();
        for len in [0, 319] {
            let text = transcriber.transcribe(&alloc::vec![1_000; len]).unwrap();
            assert_eq!(text, "", "length {len}");
        }
    }

    fn beam<'a>(language_model: Option<&'a LanguageModel<'a>>, config: BeamConfig) -> Decoding<'a> {
        Decoding::Beam {
            language_model,
            config,
        }
    }

    fn language_model() -> LanguageModel<'static> {
        LanguageModel::parse(oido_reference::lm_blob()).unwrap()
    }

    #[test]
    fn should_match_reference_transcripts() {
        let model = model();
        let lm = language_model();
        let reference_model = oido_reference::RefModel::load(oido_reference::model_blob());
        let reference_lm = oido_reference::RefLm::load(oido_reference::lm_blob());
        let wide = BeamConfig {
            beam_width: 4,
            top_k: 6,
            ..BeamConfig::default()
        };
        let clips: &[&str] = &["b05", "b14", "b16"];
        // Decoding, the matching arguments of the C engine, and the clips.
        for (decoding, reference_lm, beam_width, top_k, names) in [
            (Decoding::Greedy, None, 0, 0, clips),
            (
                beam(Some(&lm), BeamConfig::default()),
                Some(&reference_lm),
                2,
                3,
                clips,
            ),
            (beam(None, wide), None, 4, 6, &clips[..1]),
        ] {
            let mut transcriber = Transcriber::new(&model, decoding).unwrap();
            for name in names {
                let pcm = oido_reference::clip(name);
                let expected = oido_reference::transcribe(
                    &reference_model,
                    reference_lm,
                    beam_width,
                    top_k,
                    &pcm,
                );
                assert_eq!(
                    transcriber.transcribe(&pcm).unwrap(),
                    expected.text,
                    "{name}, beam {beam_width}"
                );
            }
        }
    }

    #[test]
    fn should_reject_invalid_beam_configs() {
        let model = model();
        for (beam_width, top_k) in [(0, 3), (17, 3), (2, 0), (2, 17)] {
            let config = BeamConfig {
                beam_width,
                top_k,
                ..BeamConfig::default()
            };
            assert_eq!(
                Transcriber::new(&model, beam(None, config)).unwrap_err(),
                TranscribeError::InvalidBeamConfig,
                "width {beam_width}, top-k {top_k}"
            );
        }
    }

    #[test]
    fn should_reject_a_mismatched_language_model() {
        let model = model();
        let copy = oido_reference::aligned_copy(oido_reference::lm_blob());
        copy[4..8].copy_from_slice(&1_000u32.to_le_bytes());
        // The shorter tables of the patched header still fit in the file.
        let lm = LanguageModel::parse(copy).unwrap();
        assert_eq!(
            Transcriber::new(&model, beam(Some(&lm), BeamConfig::default())).unwrap_err(),
            TranscribeError::VocabularyMismatch {
                model: 1025,
                language_model: 1000,
            }
        );
    }
}
