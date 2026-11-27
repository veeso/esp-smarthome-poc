//! `no_std` Rust port of the Oído on-device English speech-to-text engine.
//!
//! The port follows <https://github.com/lokutor-ai/oido> at commit
//! `c8d60325ba8bffd4bad65a9ac63c713ee64d1aae`, Copyright (C) 2026 Hashing
//! Works, S.L.L. (Lokutor).
//!
//! The engine transcribes whole utterances of 16 kHz mono PCM16 audio with a
//! `NeMo` Conformer CTC model:
//!
//! - [`Model`] and [`LanguageModel`] parse the TNM1 and TLM1 files in place;
//! - [`Transcriber`] turns an utterance into text, greedily or with a beam
//!   search that the language model steers;
//! - [`Segmenter`] cuts live audio into utterances.
//!
//! On the same computer, transcripts and logits match the C engine bit for
//! bit, and segmenter samples match it within one unit.
//!
//! # Examples
//!
//! ```
//! use oido::{BeamConfig, Decoding, LanguageModel, Model, Transcriber};
//!
//! fn transcribe(
//!     model_blob: &[u8],
//!     lm_blob: &[u8],
//!     pcm: &[i16],
//! ) -> Result<String, Box<dyn core::error::Error>> {
//!     let model = Model::parse(model_blob)?;
//!     let lm = LanguageModel::parse(lm_blob)?;
//!     let mut transcriber = Transcriber::new(
//!         &model,
//!         Decoding::Beam {
//!             language_model: Some(&lm),
//!             config: BeamConfig::default(),
//!         },
//!     )?;
//!     Ok(transcriber.transcribe(pcm)?)
//! }
//! ```
//!
//! The parsers need 16-byte aligned bytes. A firmware can embed the models at
//! that alignment; the file must exist at build time:
//!
//! ```ignore
//! static MODEL: &oido::Aligned<[u8]> = oido::include_aligned_bytes!("model.tnm");
//! let model = oido::Model::parse(&MODEL.0)?;
//! ```
//!
//! # License
//!
//! The code is licensed under the GNU GPL version 3, so a program that links it
//! is distributed under that license. The model files are not part of the
//! crate; they are licensed under CC BY-NC-SA 4.0 with Lokutor's terms of use,
//! which exclude commercial use without a written license.

#![no_std]
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "the port keeps the integer and float conversions of the C engine"
)]

extern crate alloc;

mod aligned;
mod beam;
mod conformer;
mod features;
mod frontend;
mod head;
mod lm;
mod mathx;
mod model;
mod posenc;
mod quant;
mod segment;
#[cfg(test)]
mod stage_hash;
mod transcriber;

#[doc(inline)]
pub use aligned::Aligned;
#[doc(inline)]
pub use beam::{BeamConfig, MAX_BEAM_WIDTH, MAX_TOP_K};
#[doc(inline)]
pub use lm::{LanguageModel, LanguageModelError};
#[doc(inline)]
pub use model::{Model, ModelConfig, ModelError};
#[doc(inline)]
pub use segment::{BLOCK_SAMPLES, PREROLL_SAMPLES, SegmentEvent, Segmenter};
#[doc(inline)]
pub use transcriber::{Decoding, TranscribeError, Transcriber};

/// Sample rate of the audio that the engine transcribes, in hertz.
pub const SAMPLE_RATE_HZ: u32 = 16_000;
