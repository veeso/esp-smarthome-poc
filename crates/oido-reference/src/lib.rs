//! Test-only C oracle built from the unmodified upstream Oído engine.

#![cfg_attr(target_os = "none", no_std)]
#![cfg(not(target_os = "none"))]

use std::ffi::{CStr, c_char, c_int};
use std::path::PathBuf;
use std::ptr::{self, NonNull};
use std::sync::{Mutex, OnceLock, PoisonError};

mod ffi;
mod kernels;
mod math;
mod segment;

pub use kernels::{QLinear, quant_rows, qvec};
pub use math::{expf, fast_expneg, fast_sigmoid, fast_tanh, logf, nexp_tab, nsig};
pub use segment::RefSegmenter;

/// Number of CTC classes of the pinned model, the blank included.
const CLASSES: usize = 1025;

/// Bytes of the transcript buffer handed to the C engine.
const TEXT_BYTES: usize = 4096;

/// Bytes of the buffer that receives the captured stage hashes.
const DEBUG_BYTES: usize = 1 << 16;

/// The C engine keeps lazily built tables and the hash capture in statics.
static ENGINE: Mutex<()> = Mutex::new(());

/// Returns the pinned acoustic model, aligned to 16 bytes.
///
/// # Panics
///
/// Panics when the file is missing.
#[must_use]
pub fn model_blob() -> &'static [u8] {
    static BLOB: OnceLock<&'static [u8]> = OnceLock::new();
    BLOB.get_or_init(|| aligned_copy(&read_asset("oido_v2b.tnm")))
}

/// Returns the pinned language model, aligned to 16 bytes.
///
/// # Panics
///
/// Panics when the file is missing.
#[must_use]
pub fn lm_blob() -> &'static [u8] {
    static BLOB: OnceLock<&'static [u8]> = OnceLock::new();
    BLOB.get_or_init(|| aligned_copy(&read_asset("oido_v2.tlm")))
}

/// Returns a leaked copy of `bytes` that starts at a 16-byte boundary.
#[must_use]
pub fn aligned_copy(bytes: &[u8]) -> &'static mut [u8] {
    let storage = vec![0u8; bytes.len() + 16].leak();
    let offset = storage.as_ptr().align_offset(16);
    let copy = &mut storage[offset..offset + bytes.len()];
    copy.copy_from_slice(bytes);
    copy
}

/// Returns the samples of a test clip, which is a 16 kHz mono PCM16 WAV file.
///
/// # Panics
///
/// Panics when the file is missing or has another format.
#[must_use]
pub fn clip(name: &str) -> Vec<i16> {
    let wav = read_asset(&format!("clips/{name}.wav"));
    assert!(
        wav.get(..4) == Some(b"RIFF") && wav.get(8..12) == Some(b"WAVE"),
        "{name}.wav is not a WAV file"
    );

    let mut rest = &wav[12..];
    let mut format = None;
    while rest.len() >= 8 {
        let size = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
        let body = &rest[8..(8 + size).min(rest.len())];
        match &rest[..4] {
            b"fmt " => {
                let word = |at: usize| u16::from_le_bytes([body[at], body[at + 1]]);
                let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                format = Some((word(0), word(2), rate, word(14)));
            }
            b"data" => {
                assert_eq!(
                    format,
                    Some((1, 1, 16_000, 16)),
                    "{name}.wav is not 16 kHz mono PCM16"
                );
                return body
                    .chunks_exact(2)
                    .map(|s| i16::from_le_bytes([s[0], s[1]]))
                    .collect();
            }
            _ => {}
        }
        // Chunks are padded to an even size.
        rest = &rest[(8 + size + size % 2).min(rest.len())..];
    }
    panic!("{name}.wav has no data chunk");
}

/// The acoustic model of the C engine.
#[derive(Debug)]
pub struct RefModel {
    ptr: NonNull<ffi::Model>,
}

impl RefModel {
    /// Loads a TNM1 model that the C engine reads in place.
    ///
    /// # Panics
    ///
    /// Panics when the C engine rejects the blob.
    #[must_use]
    pub fn load(blob: &'static [u8]) -> Self {
        // SAFETY: the blob lives forever and the C engine only reads it.
        let ptr = unsafe { ffi::tasr_nemo_load(blob.as_ptr(), blob.len()) };
        Self {
            ptr: NonNull::new(ptr).expect("the C engine should load the model"),
        }
    }

    /// Returns the normalized log-mel features, 80 values per frame.
    ///
    /// # Panics
    ///
    /// Panics when `pcm` holds fewer than 320 samples.
    #[must_use]
    pub fn features(&self, pcm: &[i16]) -> Vec<f32> {
        assert!(pcm.len() >= 320, "the engine needs 320 samples");
        let _engine = ENGINE.lock().unwrap_or_else(PoisonError::into_inner);
        let capacity = pcm.len() / 160 + 8;
        let mut out = vec![0f32; capacity * 80];
        // SAFETY: the model is loaded and `out` holds `capacity` frames.
        let frames = unsafe {
            ffi::ref_features(
                self.ptr.as_ptr(),
                pcm.as_ptr(),
                to_c_int(pcm.len()),
                out.as_mut_ptr(),
                to_c_int(capacity),
            )
        };
        out.truncate(usize::try_from(frames).expect("the frame count is not negative") * 80);
        out
    }

    /// Returns the bytes of vocabulary token `id`.
    ///
    /// # Panics
    ///
    /// Panics when `id` is not a token of the model.
    #[must_use]
    pub fn token(&self, id: u32) -> Vec<u8> {
        // A token is at most 255 bytes long.
        let mut out = vec![0u8; 255];
        // SAFETY: the model is loaded and `out` holds `out.len()` bytes.
        let len = unsafe {
            ffi::ref_token(
                self.ptr.as_ptr(),
                to_c_int(id as usize),
                out.as_mut_ptr().cast::<c_char>(),
                to_c_int(out.len()),
            )
        };
        out.truncate(usize::try_from(len).expect("the id is in the vocabulary"));
        out
    }
}

impl Drop for RefModel {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `tasr_nemo_load` and is freed once.
        unsafe { ffi::tasr_nemo_free(self.ptr.as_ptr()) }
    }
}

/// The language model of the C engine.
#[derive(Debug)]
pub struct RefLm {
    ptr: NonNull<ffi::Lm>,
}

impl RefLm {
    /// Loads a TLM1 language model that the C engine reads in place.
    ///
    /// # Panics
    ///
    /// Panics when the C engine rejects the blob.
    #[must_use]
    pub fn load(blob: &'static [u8]) -> Self {
        // SAFETY: the blob lives forever and the C engine only reads it.
        let ptr = unsafe { ffi::tasr_lm_load(blob.as_ptr(), blob.len()) };
        Self {
            ptr: NonNull::new(ptr).expect("the C engine should load the language model"),
        }
    }

    /// Advances the state of each token by one step.
    ///
    /// `previous` holds one hidden state per token. Returns the new states and
    /// the log-probabilities over the 1025 tokens.
    ///
    /// # Panics
    ///
    /// Panics when there are more than 16 tokens or the states do not match.
    #[must_use]
    pub fn steps(&self, tokens: &[u32], previous: &[f32]) -> (Vec<f32>, Vec<f32>) {
        math::init_tables();
        let _engine = ENGINE.lock().unwrap_or_else(PoisonError::into_inner);
        // SAFETY: the language model is loaded.
        let hidden = unsafe { ffi::ref_lm_hidden_size(self.ptr.as_ptr()) };
        let hidden = usize::try_from(hidden).expect("the width is not negative");
        assert!(tokens.len() <= 16 && previous.len() == tokens.len() * hidden);
        assert!(tokens.iter().all(|&token| (token as usize) < CLASSES));
        let ids: Vec<c_int> = tokens.iter().map(|&t| to_c_int(t as usize)).collect();
        let mut states = vec![0f32; previous.len()];
        let mut logp = vec![0f32; tokens.len() * CLASSES];
        // SAFETY: the buffers hold one row of the checked width per token.
        let () = unsafe {
            ffi::ref_lm_steps(
                self.ptr.as_ptr(),
                ids.as_ptr(),
                previous.as_ptr(),
                to_c_int(tokens.len()),
                states.as_mut_ptr(),
                logp.as_mut_ptr(),
            );
        };
        (states, logp)
    }
}

impl Drop for RefLm {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `tasr_lm_load` and is freed once.
        unsafe { ffi::tasr_lm_free(self.ptr.as_ptr()) }
    }
}

/// Result of one transcription by the C engine.
#[derive(Clone, Debug)]
pub struct RefTranscript {
    /// Decoded text.
    pub text: String,
    /// Raw logits, `frames` rows of 1025 values with the blank last.
    pub logits: Vec<f32>,
    /// Number of encoder frames.
    pub frames: usize,
    /// Name, layer, and FNV-1a hash of each intermediate tensor.
    pub stage_hashes: Vec<(String, i32, u32)>,
}

/// Transcribes `pcm` with the C engine.
///
/// `lm: None` with `beam == 0` decodes greedily. Every other combination runs
/// the beam search with the weights that the language model recommends.
///
/// # Panics
///
/// Panics when the engine returns invalid text.
#[must_use]
pub fn transcribe(
    model: &RefModel,
    lm: Option<&RefLm>,
    beam: i32,
    top_k: i32,
    pcm: &[i16],
) -> RefTranscript {
    math::init_tables();
    let _engine = ENGINE.lock().unwrap_or_else(PoisonError::into_inner);
    let lm_ptr = lm.map_or(ptr::null(), |lm| lm.ptr.as_ptr().cast_const());
    let decoder = (lm.is_some() || beam != 0).then(|| {
        // SAFETY: `lm_ptr` is null or a loaded language model that outlives the decoder.
        unsafe {
            ffi::tasr_decoder_create(
                lm_ptr,
                to_c_int(CLASSES),
                beam,
                top_k,
                ffi::tasr_lm_weight(lm_ptr, 0.6),
                ffi::tasr_lm_bonus(lm_ptr, 2.5),
            )
        }
    });

    let max_frames = pcm.len() / 160 + 8;
    let mut logits = vec![0f32; max_frames * CLASSES];
    let mut text = vec![0u8; TEXT_BYTES];
    let mut frames: c_int = 0;
    drop(take_stage_hashes());
    // SAFETY: every pointer is valid for the length passed with it, and the
    // logit sink holds `max_frames` rows.
    unsafe {
        ffi::tasr_nemo_transcribe(
            model.ptr.as_ptr(),
            pcm.as_ptr(),
            to_c_int(pcm.len()),
            decoder.unwrap_or(ptr::null_mut()),
            text.as_mut_ptr().cast::<c_char>(),
            to_c_int(TEXT_BYTES),
            logits.as_mut_ptr(),
            to_c_int(max_frames),
            &raw mut frames,
        )
    };
    let stage_hashes = take_stage_hashes();
    if let Some(decoder) = decoder {
        // SAFETY: the decoder came from `tasr_decoder_create` and is freed once.
        unsafe { ffi::tasr_decoder_free(decoder) }
    }

    let frames = usize::try_from(frames).expect("the frame count is not negative");
    logits.truncate(frames * CLASSES);
    let text = CStr::from_bytes_until_nul(&text)
        .expect("the C engine terminates the text")
        .to_str()
        .expect("the transcript is UTF-8")
        .to_owned();
    RefTranscript {
        text,
        logits,
        frames,
        stage_hashes,
    }
}

/// The log-softmax that the engine feeds to the beam search for one logit row.
///
/// A blank-dominated row writes `out[0]` only.
///
/// # Panics
///
/// Panics when the buffers differ in length.
pub fn ctc_log_probs(row: &[f32], out: &mut [f32]) {
    assert_eq!(row.len(), out.len());
    math::init_tables();
    let _engine = ENGINE.lock().unwrap_or_else(PoisonError::into_inner);
    // SAFETY: both buffers hold `row.len()` values.
    unsafe { ffi::ref_ctc_log_probs(row.as_ptr(), to_c_int(row.len()), out.as_mut_ptr()) }
}

/// Parses and clears the `DBG <name> <layer> <hex>` lines of the C engine.
fn take_stage_hashes() -> Vec<(String, i32, u32)> {
    let mut buffer = vec![0u8; DEBUG_BYTES];
    // SAFETY: `ref_dbg_take` writes at most `DEBUG_BYTES` bytes, NUL included.
    unsafe { ffi::ref_dbg_take(buffer.as_mut_ptr().cast::<c_char>(), DEBUG_BYTES) }
    let lines = CStr::from_bytes_until_nul(&buffer)
        .expect("the capture is terminated")
        .to_string_lossy()
        .into_owned();
    lines
        .lines()
        .filter_map(|line| {
            let mut words = line.strip_prefix("DBG ")?.split(' ');
            let name = words.next()?.to_owned();
            let layer = words.next()?.parse().ok()?;
            let hash = u32::from_str_radix(words.next()?, 16).ok()?;
            Some((name, layer, hash))
        })
        .collect()
}

/// Converts a length to the C `int` that the engine takes.
fn to_c_int(value: usize) -> c_int {
    c_int::try_from(value).expect("the length fits in a C int")
}

/// Reads a downloaded model or clip from `models/oido/`.
fn read_asset(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../models/oido")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "cannot read {path}: {error}; run just download_oido_assets",
            path = path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_capture_every_layer_and_frame_of_a_clip() {
        let model = RefModel::load(model_blob());
        let lm = RefLm::load(lm_blob());
        let out = transcribe(&model, Some(&lm), 2, 3, &clip("b05"));
        assert_eq!(out.logits.len(), out.frames * 1025);
        assert_eq!(
            out.stage_hashes
                .iter()
                .filter(|(stage, _, _)| stage == "layer")
                .count(),
            12
        );
    }
}
