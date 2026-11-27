//! Convolutional subsampling front end and frame stacking.

// Ported from esp32/components/tinyasr/tasr_nemo.c:628-713 and 888-999 at
// commit c8d60325ba8bffd4bad65a9ac63c713ee64d1aae.

use alloc::vec::Vec;

use crate::features::Features;
use crate::mathx::rne;
use crate::model::{MEL_BANDS, Model, subsampled_mel_bands};
use crate::quant::quant_vec;
use crate::transcriber::{TranscribeError, zeroed};

/// Rows that one pass of a linear layer takes.
const ROW_BLOCK: usize = 64;

/// Rows of conv0 output that one conv2 output row reads.
const TAPS: usize = 3;

/// Padded columns of a conv0 output row: 40 mel positions and a zero on each side.
const CONV0_COLUMNS: usize = (MEL_BANDS - 1) / 2 + 1 + 2;

/// A `rows` by `d` matrix of encoder activations.
#[derive(Debug)]
pub(crate) struct Frames {
    pub rows: usize,
    pub d: usize,
    pub data: Vec<f32>,
}

/// Turns features into the input of the first conformer block.
pub(crate) fn subsample(model: &Model<'_>, features: &Features) -> Result<Frames, TranscribeError> {
    stack(model, convolve(model, features)?)
}

/// The two striding convolutions and the projection, scaled by `sqrt(d)`.
///
/// The result has a spare zero row, which pairs with an odd last frame.
fn convolve(model: &Model<'_>, features: &Features) -> Result<Frames, TranscribeError> {
    let d = model.config().d_model;
    let channels = model.config().subsampling_channels;
    let positions = subsampled_mel_bands();
    let halve = |n: usize| n.saturating_sub(1) / 2 + 1;
    let conv0_rows = halve(features.frames);
    let rows = if features.frames == 0 {
        0
    } else {
        halve(conv0_rows)
    };
    let mut data = zeroed::<f32>((rows + 1) * d)?;

    // Conv2 runs one GEMM per `batch` output frames, so its weights are read once for them.
    let batch = (ROW_BLOCK / positions).max(1);
    let (patch, project) = (model.c2.kp, model.sub.kp);
    let mut ring = Conv0Ring::new(channels)?;
    let mut patches = zeroed::<i8>(batch * positions * patch)?;
    let mut patch_scales = zeroed::<f32>(batch * positions)?;
    let mut conv2 = zeroed::<f32>(batch * positions * channels)?;
    let mut column = zeroed::<f32>(channels * positions)?;
    let mut projected = zeroed::<i8>(ROW_BLOCK * project)?;
    let mut projected_scales = [0.0f32; ROW_BLOCK];

    let mut next_conv0 = 0;
    let (mut batched, mut pending, mut written) = (0, 0, 0);
    for t in 0..rows {
        while next_conv0 < conv0_rows && next_conv0 <= 2 * t + 1 {
            ring.convolve(model, features, next_conv0);
            next_conv0 += 1;
        }
        let first = batched * positions;
        ring.patches(
            t,
            conv0_rows,
            &mut patches[first * patch..][..positions * patch],
            &mut patch_scales[first..][..positions],
        );
        batched += 1;
        if batched < batch && t + 1 < rows {
            continue;
        }

        model.c2.forward(
            &patches,
            patch,
            &patch_scales,
            batched * positions,
            &mut conv2,
            channels,
        );
        for (nth, frame) in conv2
            .chunks_exact(positions * channels)
            .take(batched)
            .enumerate()
        {
            // Reorder to channel-major and apply the ReLU.
            for (f, values) in frame.chunks_exact(channels).enumerate() {
                for (channel, &value) in values.iter().enumerate() {
                    column[channel * positions + f] = if value > 0.0 { value } else { 0.0 };
                }
            }
            let slot = &mut projected[pending * project..][..project];
            projected_scales[pending] = quant_vec(&column, slot);
            pending += 1;
            let last = t + 1 - batched + nth + 1 == rows;
            if pending == ROW_BLOCK || last {
                model.sub.forward(
                    &projected,
                    project,
                    &projected_scales,
                    pending,
                    &mut data[written * d..],
                    d,
                );
                written += pending;
                pending = 0;
            }
        }
        batched = 0;
    }

    let scale = libm::sqrtf(d as f32);
    for value in &mut data[..rows * d] {
        *value *= scale;
    }
    Ok(Frames { rows, d, data })
}

/// Joins pairs of frames when the model asks for it.
fn stack(model: &Model<'_>, mut frames: Frames) -> Result<Frames, TranscribeError> {
    let d = frames.d;
    let Some(projection) = &model.stk else {
        frames.data.truncate(frames.rows * d);
        return Ok(frames);
    };
    let rows = frames.rows.div_ceil(2);
    let kp = projection.kp;
    let mut xq = zeroed::<i8>(ROW_BLOCK * kp)?;
    let mut scales = [0.0f32; ROW_BLOCK];
    let mut output = zeroed::<f32>(ROW_BLOCK * d)?;
    // The stacked rows overwrite the pairs that they were read from.
    for first in (0..rows).step_by(ROW_BLOCK) {
        let count = ROW_BLOCK.min(rows - first);
        for (t, scale) in scales.iter_mut().enumerate().take(count) {
            let pair = &frames.data[(first + t) * 2 * d..][..2 * d];
            *scale = quant_vec(pair, &mut xq[t * kp..][..kp]);
        }
        projection.forward(&xq, kp, &scales, count, &mut output, d);
        frames.data[first * d..][..count * d].copy_from_slice(&output[..count * d]);
    }
    frames.rows = rows;
    frames.data.truncate(rows * d);
    Ok(frames)
}

/// The last three rows of conv0 output, `[channels][CONV0_COLUMNS]` each.
struct Conv0Ring {
    channels: usize,
    rows: Vec<f32>,
    /// Maximum over the channels of each column, per row.
    maxima: [[f32; CONV0_COLUMNS]; TAPS],
}

impl Conv0Ring {
    fn new(channels: usize) -> Result<Self, TranscribeError> {
        Ok(Self {
            channels,
            rows: zeroed(TAPS * channels * CONV0_COLUMNS)?,
            maxima: [[0.0; CONV0_COLUMNS]; TAPS],
        })
    }

    /// Computes conv0 row `r`: 3 by 3, stride 2, padding 1, `ReLU`.
    fn convolve(&mut self, model: &Model<'_>, features: &Features, r: usize) {
        let mut input = [[0.0f32; MEL_BANDS + 2]; TAPS];
        for (dt, line) in input.iter_mut().enumerate() {
            if let Some(t) = (2 * r + dt).checked_sub(1)
                && t < features.frames
            {
                line[1..=MEL_BANDS].copy_from_slice(&features.data[t * MEL_BANDS..][..MEL_BANDS]);
            }
        }
        let width = self.channels * CONV0_COLUMNS;
        let output = &mut self.rows[(r % TAPS) * width..][..width];
        let maxima = &mut self.maxima[r % TAPS];
        maxima.fill(0.0);
        let weights = model.c0_w.chunks_exact(TAPS * TAPS).zip(model.c0_b);
        for (out, (w, &bias)) in output.chunks_exact_mut(CONV0_COLUMNS).zip(weights) {
            out[0] = 0.0;
            out[CONV0_COLUMNS - 1] = 0.0;
            for f in 0..CONV0_COLUMNS - 2 {
                let mut acc = bias;
                for (line, w) in input.iter().zip(w.chunks_exact(TAPS)) {
                    for (&x, &weight) in line[2 * f..][..TAPS].iter().zip(w) {
                        acc += weight * x;
                    }
                }
                let value = if acc > 0.0 { acc } else { 0.0 };
                out[1 + f] = value;
                if value > maxima[1 + f] {
                    maxima[1 + f] = value;
                }
            }
        }
    }

    /// Gathers and quantizes the 3 by 3 patches of conv2 output frame `t`.
    ///
    /// Conv0 is not negative, so the column maxima give the maximum of a patch.
    fn patches(&self, t: usize, conv0_rows: usize, patches: &mut [i8], scales: &mut [f32]) {
        let sources: [Option<usize>; TAPS] =
            core::array::from_fn(|dt| (2 * t + dt).checked_sub(1).filter(|&r| r < conv0_rows));
        let kp = patches.len() / scales.len();
        let width = self.channels * CONV0_COLUMNS;
        for (f, (patch, scale)) in patches.chunks_exact_mut(kp).zip(scales).enumerate() {
            let mut max = 0.0f32;
            for r in sources.into_iter().flatten() {
                for &value in &self.maxima[r % TAPS][2 * f..][..TAPS] {
                    if value > max {
                        max = value;
                    }
                }
            }
            if max < 1e-30 {
                max = 1e-30;
            }
            let inverse = 127.0 / max;
            patch.fill(0);
            for (dt, source) in sources.into_iter().enumerate() {
                let Some(r) = source else {
                    continue;
                };
                let row = &self.rows[(r % TAPS) * width..][..width];
                for (x, q) in row
                    .chunks_exact(CONV0_COLUMNS)
                    .zip(patch.chunks_exact_mut(TAPS * TAPS))
                {
                    for (q, &value) in q[dt * TAPS..][..TAPS].iter_mut().zip(&x[2 * f..][..TAPS]) {
                        *q = rne(value * inverse) as i8;
                    }
                }
            }
            *scale = max / 127.0;
        }
    }
}
