//! Speech-to-text with the embedded Oído models.

use alloc::boxed::Box;
use alloc::string::String;
use core::fmt;

use embassy_time::Instant;
use log::{info, warn};
use oido::{BeamConfig, Decoding, LanguageModel, Model, Transcriber};

static MODEL: &oido::Aligned<[u8]> = oido::include_aligned_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../models/oido/oido_v2b.tnm"
));

static LANGUAGE_MODEL: &oido::Aligned<[u8]> = oido::include_aligned_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../models/oido/oido_v2.tlm"
));

/// The speech engine, which transcribes one utterance at a time.
pub struct Stt {
    transcriber: Transcriber<'static>,
}

impl fmt::Debug for Stt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stt").finish_non_exhaustive()
    }
}

impl Stt {
    /// Parses the models and sets up the beam search.
    ///
    /// Must be called once, because the models live as long as the program.
    ///
    /// # Panics
    ///
    /// Panics when a model is invalid or the decoder does not fit in the heap.
    pub fn load() -> Self {
        let model: &'static Model<'static> = Box::leak(Box::new(
            Model::parse(&MODEL.0).unwrap_or_else(|error| failed(&error)),
        ));
        let language_model: &'static LanguageModel<'static> = Box::leak(Box::new(
            LanguageModel::parse(&LANGUAGE_MODEL.0).unwrap_or_else(|error| failed(&error)),
        ));
        let decoding = Decoding::Beam {
            language_model: Some(language_model),
            config: BeamConfig::default(),
        };
        let transcriber = Transcriber::new(model, decoding).unwrap_or_else(|error| failed(&error));
        info!("speech engine ready");
        Self { transcriber }
    }

    /// Transcribes 16 kHz audio and logs how long it took.
    ///
    /// Returns an empty text when the transcription fails.
    #[expect(
        clippy::cast_precision_loss,
        reason = "the durations are only logged, with two decimals"
    )]
    pub fn transcribe(&mut self, pcm: &[i16]) -> String {
        let seconds = pcm.len() as f32 / oido::SAMPLE_RATE_HZ as f32;
        // The chip is slow at this, so say what it is busy with.
        info!("transcribing {seconds:.1} s of audio");
        let start = Instant::now();
        match self.transcriber.transcribe(pcm) {
            Ok(text) => {
                let millis = start.elapsed().as_millis();
                // Real-time factor: processing time over audio time.
                let rtf = millis as f32 / 1_000.0 / seconds;
                info!("transcribed {seconds:.1} s of audio in {millis} ms (RTF {rtf:.2})");
                text
            }
            Err(error) => {
                warn!("transcription failed: {error}");
                String::new()
            }
        }
    }
}

fn failed(error: &dyn fmt::Display) -> ! {
    panic!("speech engine failed to load: {error}")
}
