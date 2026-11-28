//! The TNM1 acoustic model, parsed in place.

// Ported from esp32/components/tinyasr/tasr_nemo.c:74-213 at commit
// c8d60325ba8bffd4bad65a9ac63c713ee64d1aae.

use alloc::vec::Vec;
use core::fmt;

use crate::conformer::MAX_HEAD;
use crate::quant::{Layout, QLinear};

/// Mel bands of the front end.
pub(crate) const MEL_BANDS: usize = 80;

/// Power spectrum bins of the 512-point transform.
pub(crate) const SPECTRUM_BINS: usize = 257;

/// Samples of the analysis window.
pub(crate) const WINDOW_SAMPLES: usize = 400;

/// Bytes of the header that precedes the tensors.
const HEADER_BYTES: usize = 64;

/// Alignment of the blob and of every tensor in it.
const ALIGNMENT: usize = 16;

/// Dimensions of an acoustic model.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelConfig {
    /// Width of the encoder.
    pub d_model: usize,
    /// Attention heads.
    pub heads: usize,
    /// Width of the feed-forward modules.
    pub ff_dim: usize,
    /// Taps of the depthwise convolution.
    pub conv_kernel: usize,
    /// Conformer blocks.
    pub layers: usize,
    /// Channels of the convolutional front end.
    pub subsampling_channels: usize,
    /// Tokens, without the CTC blank.
    pub vocab_size: usize,
    /// Width of the encoder weights, 4 or 8.
    pub weight_bits: usize,
    /// Front-end frames that make one encoder frame, 1 or 2.
    pub frame_stack: usize,
}

/// Why a blob is not a usable model.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelError {
    /// The blob does not start with the TNM1 magic.
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
    /// The model uses a feature that this port does not implement.
    Unsupported(&'static str),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => f.write_str("not an Oído TNM1 model"),
            Self::Misaligned => f.write_str("the model is not aligned to 16 bytes"),
            Self::Truncated { needed, len } => write!(
                f,
                "the model is truncated: it needs {needed} bytes and has {len}"
            ),
            Self::Unsupported(what) => write!(f, "unsupported model: {what}"),
        }
    }
}

impl core::error::Error for ModelError {}

impl From<Truncated> for ModelError {
    fn from(error: Truncated) -> Self {
        Self::Truncated {
            needed: error.needed,
            len: error.len,
        }
    }
}

/// An Oído acoustic model (`NeMo` Conformer CTC) that borrows its weights.
///
/// # Examples
///
/// ```
/// let error = oido::Model::parse(&oido::Aligned([0u8; 64]).0).unwrap_err();
/// assert_eq!(error, oido::ModelError::BadMagic);
/// ```
#[derive(Clone)]
pub struct Model<'a> {
    config: ModelConfig,
    pub(crate) window: &'a [f32],
    pub(crate) mel: MelFilters,
    pub(crate) c0_w: &'a [f32],
    pub(crate) c0_b: &'a [f32],
    pub(crate) c2: QLinear<'a>,
    pub(crate) sub: QLinear<'a>,
    pub(crate) stk: Option<QLinear<'a>>,
    pub(crate) layers: Vec<Layer<'a>>,
    pub(crate) head: QLinear<'a>,
    tokens: Vec<&'a str>,
    pub(crate) nmean: &'a [f32],
    pub(crate) nstd: &'a [f32],
}

impl fmt::Debug for Model<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Model")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// The mel filter bank without its zeros.
#[derive(Clone, Debug)]
pub(crate) struct MelFilters {
    /// First spectrum bin of each band.
    pub starts: Vec<usize>,
    /// Bins of each band.
    pub lens: Vec<usize>,
    /// The weights of every band, one band after the other.
    pub weights: Vec<f32>,
}

/// Gain and offset of one layer normalization.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LayerNorm<'a> {
    pub gamma: &'a [f32],
    pub beta: &'a [f32],
}

/// The tensors of one conformer block.
#[derive(Clone, Debug)]
pub(crate) struct Layer<'a> {
    pub ln_ff1: LayerNorm<'a>,
    pub ff1_1: QLinear<'a>,
    pub ff1_2: QLinear<'a>,
    pub ln_att: LayerNorm<'a>,
    pub qkv: QLinear<'a>,
    pub out: QLinear<'a>,
    pub pos: QLinear<'a>,
    pub pbu: &'a [f32],
    pub pbv: &'a [f32],
    pub ln_conv: LayerNorm<'a>,
    pub pw1: QLinear<'a>,
    /// Depthwise weights transposed to `[kernel][d_model]`.
    pub dw_w: Vec<f32>,
    pub dw_b: &'a [f32],
    pub pw2: QLinear<'a>,
    pub ln_ff2: LayerNorm<'a>,
    pub ff2_1: QLinear<'a>,
    pub ff2_2: QLinear<'a>,
    pub ln_out: LayerNorm<'a>,
}

impl<'a> Model<'a> {
    /// Parses a TNM1 model; the weights stay in `blob`.
    ///
    /// The format is little-endian, like every supported target.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError`] when `blob` is not 16-byte aligned, is not a
    /// complete TNM1 model, or uses a feature that the port does not implement.
    pub fn parse(blob: &'a [u8]) -> Result<Self, ModelError> {
        if !blob.as_ptr().addr().is_multiple_of(ALIGNMENT) {
            return Err(ModelError::Misaligned);
        }
        if blob.get(..4) != Some(b"TNM1") {
            return Err(ModelError::BadMagic);
        }
        let mut cursor = Cursor { blob, pos: 0 };
        let header = cursor.bytes(HEADER_BYTES)?;
        let word = |index: usize| {
            let at = 4 + 4 * index;
            u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
                as usize
        };
        let config = ModelConfig {
            d_model: word(1),
            heads: word(2),
            ff_dim: word(3),
            conv_kernel: word(4),
            layers: word(5),
            subsampling_channels: word(6),
            vocab_size: word(7),
            weight_bits: word(8),
            frame_stack: word(13),
        };
        check_supported(&config, word(9), word(10))?;

        let d = config.d_model;
        let channels = config.subsampling_channels;
        let bits = config.weight_bits;
        // Mel positions that remain after the two striding convolutions.
        let positions = subsampled_mel_bands();

        let window = cursor.floats(WINDOW_SAMPLES)?;
        let mel = MelFilters::new(cursor.floats(SPECTRUM_BINS * MEL_BANDS)?);
        let c0_w = cursor.floats(size(channels, 9))?;
        let c0_b = cursor.floats(channels)?;
        let c2 = cursor.linear(channels, size(channels, 9), 8)?;
        let sub = cursor.linear(d, size(channels, positions), 8)?;
        let stk = if config.frame_stack == 2 {
            Some(cursor.linear(d, size(d, 2), 8)?)
        } else {
            None
        };
        let mut layers = Vec::new();
        for _ in 0..config.layers {
            layers.push(cursor.layer(&config, bits)?);
        }
        let head = cursor.linear(config.vocab_size.saturating_add(1), d, 8)?;
        cursor.align();
        let mut tokens = Vec::new();
        for _ in 0..config.vocab_size {
            let len = usize::from(cursor.bytes(1)?[0]);
            let token = core::str::from_utf8(cursor.bytes(len)?)
                .map_err(|_error| ModelError::Unsupported("non-UTF-8 tokens"))?;
            tokens.push(token);
        }
        let nmean = cursor.floats(MEL_BANDS)?;
        let nstd = cursor.floats(MEL_BANDS)?;

        Ok(Self {
            config,
            window,
            mel,
            c0_w,
            c0_b,
            c2,
            sub,
            stk,
            layers,
            head,
            tokens,
            nmean,
            nstd,
        })
    }

    /// Returns the dimensions of the model.
    #[must_use]
    pub fn config(&self) -> &ModelConfig {
        &self.config
    }

    /// Returns the text of token `id`, or `None` outside the vocabulary.
    ///
    /// Control tokens are empty.
    #[must_use]
    pub fn token(&self, id: u32) -> Option<&'a str> {
        self.tokens.get(id as usize).copied()
    }
}

/// Mel positions after the front end halves the mel axis twice.
pub(crate) const fn subsampled_mel_bands() -> usize {
    let once = (MEL_BANDS - 1) / 2 + 1;
    (once - 1) / 2 + 1
}

fn check_supported(config: &ModelConfig, rnnt: usize, flags: usize) -> Result<(), ModelError> {
    let unsupported = |what| Err(ModelError::Unsupported(what));
    if rnnt != 0 {
        return unsupported("RNN-T models");
    }
    if flags & 1 == 0 {
        return unsupported("per-utterance feature normalisation");
    }
    if !matches!(config.frame_stack, 1 | 2) {
        return unsupported("frame stack");
    }
    if !matches!(config.weight_bits, 4 | 8) {
        return unsupported("weight bits");
    }
    let dimensions = [
        config.d_model,
        config.heads,
        config.ff_dim,
        config.conv_kernel,
        config.layers,
        config.subsampling_channels,
        config.vocab_size,
    ];
    // Bounding every dimension keeps the size arithmetic from overflowing.
    if dimensions
        .iter()
        .any(|&n| n == 0 || n > usize::from(u16::MAX))
        || !config.d_model.is_multiple_of(config.heads)
        || config.d_model / config.heads > MAX_HEAD
        || !config.d_model.is_multiple_of(2)
        || config.conv_kernel.is_multiple_of(2)
    {
        return unsupported("model dimensions");
    }
    Ok(())
}

/// A size that saturates, so that an absurd one reads as truncation.
pub(crate) fn size(a: usize, b: usize) -> usize {
    a.saturating_mul(b)
}

impl MelFilters {
    /// Keeps the span of nonzero weights of each band of `bank` (`[bins][bands]`).
    fn new(bank: &[f32]) -> Self {
        let mut starts = Vec::with_capacity(MEL_BANDS);
        let mut lens = Vec::with_capacity(MEL_BANDS);
        let mut weights = Vec::new();
        for band in 0..MEL_BANDS {
            let used = |bin: &usize| bank[bin * MEL_BANDS + band] != 0.0;
            let start = (0..SPECTRUM_BINS).find(used);
            let end = (0..SPECTRUM_BINS).rfind(used);
            let (start, len) = match (start, end) {
                (Some(start), Some(end)) => (start, end - start + 1),
                _ => (0, 0),
            };
            starts.push(start);
            lens.push(len);
            weights.extend((start..start + len).map(|bin| bank[bin * MEL_BANDS + band]));
        }
        Self {
            starts,
            lens,
            weights,
        }
    }
}

/// A blob ended before the part that was being read.
#[derive(Debug)]
pub(crate) struct Truncated {
    pub needed: usize,
    pub len: usize,
}

/// Reads the aligned parts of a blob in order.
pub(crate) struct Cursor<'a> {
    pub blob: &'a [u8],
    pub pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn align(&mut self) {
        self.pos = self.pos.saturating_add(ALIGNMENT - 1) & !(ALIGNMENT - 1);
    }

    pub(crate) fn bytes(&mut self, len: usize) -> Result<&'a [u8], Truncated> {
        let end = self.pos.saturating_add(len);
        let bytes = self.blob.get(self.pos..end).ok_or(Truncated {
            needed: end,
            len: self.blob.len(),
        })?;
        self.pos = end;
        Ok(bytes)
    }

    pub(crate) fn floats(&mut self, count: usize) -> Result<&'a [f32], Truncated> {
        self.align();
        let bytes = self.bytes(size(count, 4))?;
        // The blob and the position are aligned, and the length is a multiple of 4.
        Ok(bytemuck::cast_slice(bytes))
    }

    pub(crate) fn linear(
        &mut self,
        n: usize,
        k: usize,
        weight_bits: usize,
    ) -> Result<QLinear<'a>, Truncated> {
        let layout = match weight_bits {
            8 => Layout::Int8,
            _ if n.is_multiple_of(16) => Layout::Int4Blocked,
            _ => Layout::Int4Rows,
        };
        let block = if layout == Layout::Int4Rows { 32 } else { 16 };
        let kp = k.saturating_add(block - 1) / block * block;
        let weight_bytes = if layout == Layout::Int8 {
            size(n, kp)
        } else {
            size(n, kp / 2)
        };
        self.align();
        let weights = self.bytes(weight_bytes)?;
        let scales = self.floats(n)?;
        let bias = self.floats(n)?;
        Ok(QLinear {
            n,
            k,
            kp,
            layout,
            weights,
            scales,
            bias,
        })
    }

    fn norm(&mut self, d: usize) -> Result<LayerNorm<'a>, Truncated> {
        Ok(LayerNorm {
            gamma: self.floats(d)?,
            beta: self.floats(d)?,
        })
    }

    /// Reads one conformer block; the fields are read in their stored order.
    fn layer(&mut self, config: &ModelConfig, bits: usize) -> Result<Layer<'a>, Truncated> {
        let (d, ff, taps) = (config.d_model, config.ff_dim, config.conv_kernel);
        let ln_ff1 = self.norm(d)?;
        let ff1_1 = self.linear(ff, d, bits)?;
        let ff1_2 = self.linear(d, ff, bits)?;
        let ln_att = self.norm(d)?;
        let qkv = self.linear(size(d, 3), d, bits)?;
        let out = self.linear(d, d, bits)?;
        // The positional projection is always int8.
        let pos = self.linear(d, d, 8)?;
        let pbu = self.floats(d)?;
        let pbv = self.floats(d)?;
        let ln_conv = self.norm(d)?;
        let pw1 = self.linear(size(d, 2), d, bits)?;
        let stored = self.floats(size(d, taps))?;
        let dw_w = (0..taps)
            .flat_map(|tap| (0..d).map(move |channel| stored[channel * taps + tap]))
            .collect();
        let dw_b = self.floats(d)?;
        let pw2 = self.linear(d, d, bits)?;
        let ln_ff2 = self.norm(d)?;
        let ff2_1 = self.linear(ff, d, bits)?;
        let ff2_2 = self.linear(d, ff, bits)?;
        let ln_out = self.norm(d)?;
        Ok(Layer {
            ln_ff1,
            ff1_1,
            ff1_2,
            ln_att,
            qkv,
            out,
            pos,
            pbu,
            pbv,
            ln_conv,
            pw1,
            dw_w,
            dw_b,
            pw2,
            ln_ff2,
            ff2_1,
            ff2_2,
            ln_out,
        })
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    /// Returns an aligned copy of the model with header word `index` set to `value`.
    fn patched(index: usize, value: u32) -> &'static [u8] {
        let copy = oido_reference::aligned_copy(oido_reference::model_blob());
        copy[4 + 4 * index..][..4].copy_from_slice(&value.to_le_bytes());
        copy
    }

    #[test]
    fn should_parse_the_v2b_config() {
        let model = Model::parse(oido_reference::model_blob()).unwrap();
        assert_eq!(
            *model.config(),
            ModelConfig {
                d_model: 176,
                heads: 4,
                ff_dim: 704,
                conv_kernel: 31,
                layers: 12,
                subsampling_channels: 64,
                vocab_size: 1024,
                weight_bits: 4,
                frame_stack: 2,
            }
        );
    }

    #[test]
    fn should_read_the_same_tokens_as_the_reference() {
        let model = Model::parse(oido_reference::model_blob()).unwrap();
        let reference = oido_reference::RefModel::load(oido_reference::model_blob());
        for id in 0..1024 {
            let token = model.token(id).unwrap_or_default();
            assert_eq!(token.as_bytes(), reference.token(id), "token {id}");
        }
        assert_eq!(model.token(1024), None);
    }

    #[test]
    fn should_reject_a_truncated_blob() {
        let blob = oido_reference::model_blob();
        for len in [10, 64, blob.len() - 1] {
            let error = Model::parse(&blob[..len]).unwrap_err();
            assert!(
                matches!(error, ModelError::Truncated { needed, len: actual }
                    if actual == len && needed > len),
                "length {len}: {error:?}"
            );
        }
    }

    #[test]
    fn should_reject_unusable_models() {
        let bad_magic = oido_reference::aligned_copy(oido_reference::model_blob());
        bad_magic[0] = b'X';
        let misaligned = &oido_reference::aligned_copy(oido_reference::model_blob())[1..];
        let unsupported =
            |word, value, reason| (patched(word, value), ModelError::Unsupported(reason));
        for (blob, expected) in [
            (&*bad_magic, ModelError::BadMagic),
            (misaligned, ModelError::Misaligned),
            unsupported(9, 1, "RNN-T models"),
            unsupported(10, 2, "per-utterance feature normalisation"),
            unsupported(13, 3, "frame stack"),
            unsupported(8, 6, "weight bits"),
            unsupported(2, 0, "model dimensions"),
        ] {
            assert_eq!(Model::parse(blob).unwrap_err(), expected);
        }
    }

    #[test]
    fn model_error_should_display() {
        assert_eq!(ModelError::BadMagic.to_string(), "not an Oído TNM1 model");
    }
}
