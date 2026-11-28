//! Log-mel features: pre-emphasis, centered STFT, mel bank, and normalization.

// Ported from esp32/components/tinyasr/tasr_nemo.c:203-211, 526-620 at commit
// c8d60325ba8bffd4bad65a9ac63c713ee64d1aae.

use alloc::vec::Vec;

use crate::mathx::logf;
use crate::model::{MEL_BANDS, Model, SPECTRUM_BINS, WINDOW_SAMPLES};
use crate::transcriber::{TranscribeError, zeroed};

/// Samples between two frames (10 ms).
const HOP: usize = 160;

/// Samples of the transform, which pads the window with zeros.
const FFT_SAMPLES: usize = 512;

/// Points of the complex transform of the packed real signal.
const HALF: usize = FFT_SAMPLES / 2;

/// `2^-24`, which keeps the logarithm of silence finite.
const LOG_GUARD: f32 = 5.960_464_5e-8;

/// Normalized log-mel features, `frames` rows of 80 values.
#[derive(Debug)]
pub(crate) struct Features {
    pub frames: usize,
    pub data: Vec<f32>,
}

/// Computes the features of `pcm`; fewer than 320 samples give no frames.
pub(crate) fn features(model: &Model<'_>, pcm: &[i16]) -> Result<Features, TranscribeError> {
    if pcm.len() < 2 * HOP {
        return Ok(Features {
            frames: 0,
            data: Vec::new(),
        });
    }
    let valid = pcm.len() / HOP;
    let frames = valid + 1;
    let mut data = zeroed::<f32>(frames * MEL_BANDS)?;
    let transform = Transform::new();
    for (t, bands) in data.chunks_exact_mut(MEL_BANDS).enumerate() {
        let mut windowed = [0.0f32; FFT_SAMPLES];
        for (i, (value, &weight)) in windowed.iter_mut().zip(model.window).enumerate() {
            // Frame `t` is centered on sample `160 t`; outside the clip it is zero.
            let at = (HOP * t + i).checked_sub(WINDOW_SAMPLES / 2);
            *value = at.map_or(0.0, |at| pre_emphasized(pcm, at)) * weight;
        }
        let power = transform.power(&windowed);
        let mut weights = model.mel.weights.as_slice();
        for (band, (&start, &len)) in bands
            .iter_mut()
            .zip(model.mel.starts.iter().zip(&model.mel.lens))
        {
            let (own, rest) = weights.split_at(len);
            weights = rest;
            let mut energy = 0.0;
            for (&p, &w) in power[start..start + len].iter().zip(own) {
                energy += p * w;
            }
            *band = logf(energy + LOG_GUARD);
        }
    }
    normalize(model, &mut data, valid);
    Ok(Features { frames, data })
}

/// Sample `at` after the 0.97 pre-emphasis filter, or zero past the end.
#[inline]
fn pre_emphasized(pcm: &[i16], at: usize) -> f32 {
    let Some(&sample) = pcm.get(at) else {
        return 0.0;
    };
    let now = f32::from(sample) / 32_768.0;
    match at.checked_sub(1) {
        Some(previous) => now - 0.97 * (f32::from(pcm[previous]) / 32_768.0),
        None => now,
    }
}

/// Applies the fixed per-band statistics; the frame past the audio becomes zero.
fn normalize(model: &Model<'_>, data: &mut [f32], valid: usize) {
    for (band, (&mean, &deviation)) in model.nmean.iter().zip(model.nstd).enumerate() {
        let inverse = 1.0 / deviation;
        for t in 0..valid {
            let value = &mut data[t * MEL_BANDS + band];
            *value = (*value - mean) * inverse;
        }
        data[valid * MEL_BANDS + band] = 0.0;
    }
}

/// Twiddle factors and the bit reversal of the 512-point real transform.
struct Transform {
    re: [f32; HALF],
    im: [f32; HALF],
    reversed: [u8; HALF],
}

impl Transform {
    fn new() -> Self {
        let angle = |i: usize| -2.0 * core::f64::consts::PI * i as f64 / FFT_SAMPLES as f64;
        Self {
            re: core::array::from_fn(|i| libm::cos(angle(i)) as f32),
            im: core::array::from_fn(|i| libm::sin(angle(i)) as f32),
            reversed: core::array::from_fn(|i| (i as u8).reverse_bits()),
        }
    }

    /// Power spectrum through a 256-point complex transform of the sample pairs.
    fn power(&self, x: &[f32; FFT_SAMPLES]) -> [f32; SPECTRUM_BINS] {
        let mut re = [0.0f32; HALF];
        let mut im = [0.0f32; HALF];
        for (pair, &to) in x.chunks_exact(2).zip(&self.reversed) {
            re[usize::from(to)] = pair[0];
            im[usize::from(to)] = pair[1];
        }
        let mut len = 2;
        while len <= HALF {
            let half = len / 2;
            let step = 2 * (HALF / len);
            for i in (0..HALF).step_by(len) {
                for j in 0..half {
                    let (wr, wi) = (self.re[j * step], self.im[j * step]);
                    let (a, b) = (i + j, i + j + half);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len *= 2;
        }

        let mut power = [0.0f32; SPECTRUM_BINS];
        power[0] = (re[0] + im[0]) * (re[0] + im[0]);
        power[HALF] = (re[0] - im[0]) * (re[0] - im[0]);
        for k in 1..HALF {
            let (zr, zi, cr, ci) = (re[k], im[k], re[HALF - k], -im[HALF - k]);
            let (er, ei) = (0.5 * (zr + cr), 0.5 * (zi + ci));
            let (dr, di) = (0.5 * (zr - cr), 0.5 * (zi - ci));
            let (wr, wi) = (self.re[k], self.im[k]);
            let or = er + (wr * di + wi * dr);
            let oi = ei - (wr * dr - wi * di);
            power[k] = or * or + oi * oi;
        }
        power
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_hash::tone;

    fn bits(x: &[f32]) -> Vec<u32> {
        x.iter().map(|v| v.to_bits()).collect()
    }

    fn model() -> Model<'static> {
        Model::parse(oido_reference::model_blob()).unwrap()
    }

    fn reference_features(pcm: &[i16]) -> Vec<f32> {
        oido_reference::RefModel::load(oido_reference::model_blob()).features(pcm)
    }

    #[test]
    fn should_match_the_reference_features() {
        let model = model();
        // The tone is the shortest input that gives frames.
        let inputs = ["b05", "b14", "b16"]
            .map(oido_reference::clip)
            .into_iter()
            .chain([tone(320)]);
        for pcm in inputs {
            let len = pcm.len();
            let actual = features(&model, &pcm).unwrap();
            let expected = reference_features(&pcm);
            assert_eq!(actual.frames, len / 160 + 1, "{len} samples");
            assert_eq!(bits(&actual.data), bits(&expected), "{len} samples");
        }
    }

    #[test]
    fn should_return_no_frames_for_audio_shorter_than_320_samples() {
        let model = model();
        for len in [0, 319] {
            let actual = features(&model, &alloc::vec![100; len]).unwrap();
            assert_eq!((actual.frames, actual.data.len()), (0, 0), "length {len}");
        }
    }
}
