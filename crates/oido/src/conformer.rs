//! One conformer block: feed-forward, attention, convolution, feed-forward.

// Ported from esp32/components/tinyasr/tasr_nemo.c:332-342, 423-523 and
// 1029-1174 at commit c8d60325ba8bffd4bad65a9ac63c713ee64d1aae.

use alloc::vec::Vec;

use crate::frontend::Frames;
use crate::mathx::{NEXP_STEPS, Tables, rne};
use crate::model::{Layer, LayerNorm, Model};
use crate::posenc::QuantRows;
use crate::quant::{QLinear, quant_rows, quant_vec};
use crate::transcriber::{TranscribeError, zeroed};

/// Rows that one pass of a position-wise module takes.
const ROW_BLOCK: usize = 64;

/// Working buffers that every layer of one utterance shares.
#[derive(Debug)]
pub(crate) struct Scratch {
    /// Quantizes activations for the linear layers.
    tile: Tile,
    /// Layer normalization and module outputs of one row block.
    hb: Vec<f32>,
    /// Expanded activations of one row block.
    h2: Vec<f32>,
    attention: Attention,
    /// Gated linear units of every frame, with a zero halo of half a kernel.
    glu: Vec<f32>,
    /// Convolution output, and before it the attention output.
    cv: Vec<f32>,
    /// Positional projection of one row block.
    prow: Vec<f32>,
    /// CTC logits of one row block.
    logits: Vec<f32>,
}

impl Scratch {
    pub(crate) fn new(model: &Model<'_>, frames: usize) -> Result<Self, TranscribeError> {
        let config = model.config();
        let (d, ff) = (config.d_model, config.ff_dim);
        let halo = config.conv_kernel / 2;
        // The widest quantized input, rounded up like the widest row layout.
        let ldq = ff.max(2 * d).next_multiple_of(32);
        Ok(Self {
            tile: Tile {
                xq: zeroed(ROW_BLOCK * ldq)?,
                ldq,
                xs: [0.0; ROW_BLOCK],
            },
            hb: zeroed(ROW_BLOCK * d)?,
            h2: zeroed(ROW_BLOCK * ff.max(3 * d))?,
            attention: Attention::new(config.heads, d / config.heads, frames)?,
            glu: zeroed((frames + 2 * halo) * d)?,
            cv: zeroed(frames * d)?,
            prow: zeroed(ROW_BLOCK * d)?,
            logits: zeroed(ROW_BLOCK * (config.vocab_size + 1))?,
        })
    }

    /// Applies the CTC head to one row block and returns the raw logits.
    pub(crate) fn logits(&mut self, head: &QLinear<'_>, x: &[f32], rows: usize) -> &[f32] {
        self.tile.apply(head, x, rows, head.k, &mut self.logits);
        &self.logits[..rows * head.n]
    }
}

/// Quantized activations of up to one row block.
#[derive(Debug)]
struct Tile {
    xq: Vec<i8>,
    ldq: usize,
    xs: [f32; ROW_BLOCK],
}

impl Tile {
    /// `y = linear(x)` for `rows` rows of `width` values.
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

/// Quantized queries, keys, values, and positions of every head and frame.
#[derive(Debug)]
struct Attention {
    heads: usize,
    /// Width of one head.
    dh: usize,
    /// `dh` rounded up to 16.
    dhp: usize,
    frames: usize,
    /// `frames` rounded up to 16.
    tp: usize,
    /// Queries with the content bias, `[heads][frames][dhp]`.
    qu: Vec<i8>,
    /// Queries with the position bias, `[heads][frames][dhp]`.
    qv: Vec<i8>,
    /// Keys, `[heads][frames][dhp]`.
    keys: Vec<i8>,
    /// Values transposed, `[heads][dh][tp]`.
    values: Vec<i8>,
    /// Positional projections, `[heads][2 * frames - 1][dhp]`.
    positions: Vec<i8>,
    qu_scales: Vec<f32>,
    qv_scales: Vec<f32>,
    key_scales: Vec<f32>,
    value_scales: Vec<f32>,
    position_scales: Vec<f32>,
    /// Scores, then probabilities, of one query.
    scores: Vec<f32>,
    probabilities: Vec<i8>,
}

impl Attention {
    fn new(heads: usize, dh: usize, frames: usize) -> Result<Self, TranscribeError> {
        let dhp = dh.next_multiple_of(16);
        let tp = frames.next_multiple_of(16);
        let rows = (2 * frames).saturating_sub(1);
        Ok(Self {
            heads,
            dh,
            dhp,
            frames,
            tp,
            qu: zeroed(heads * frames * dhp)?,
            qv: zeroed(heads * frames * dhp)?,
            keys: zeroed(heads * frames * dhp)?,
            values: zeroed(heads * dh * tp)?,
            positions: zeroed(heads * rows * dhp)?,
            qu_scales: zeroed(heads * frames)?,
            qv_scales: zeroed(heads * frames)?,
            key_scales: zeroed(heads * frames)?,
            value_scales: zeroed(heads * frames)?,
            position_scales: zeroed(heads * rows)?,
            scores: zeroed(tp)?,
            probabilities: zeroed(tp)?,
        })
    }

    /// Quantizes the `[q, k, v]` row of frame `t` for every head.
    fn store(&mut self, t: usize, qkv: &[f32], block: &Layer<'_>) {
        let (dh, dhp, frames, tp) = (self.dh, self.dhp, self.frames, self.tp);
        let width = self.heads * dh;
        let (queries, rest) = qkv.split_at(width);
        let (keys, values) = rest.split_at(width);
        // A head is at most `MAX_HEAD` wide, which the model parser enforces.
        let mut biased = [0.0f32; MAX_HEAD];
        let mut quantized = [0i8; MAX_HEAD];
        for head in 0..self.heads {
            let at = head * frames + t;
            let span = head * dh..(head + 1) * dh;
            let row = at * dhp..(at + 1) * dhp;
            add(
                &queries[span.clone()],
                &block.pbu[span.clone()],
                &mut biased,
            );
            self.qu_scales[at] = quant_vec(&biased[..dh], &mut self.qu[row.clone()]);
            add(
                &queries[span.clone()],
                &block.pbv[span.clone()],
                &mut biased,
            );
            self.qv_scales[at] = quant_vec(&biased[..dh], &mut self.qv[row.clone()]);
            self.key_scales[at] = quant_vec(&keys[span.clone()], &mut self.keys[row]);
            self.value_scales[at] = quant_vec(&values[span], &mut quantized[..dh]);
            for (e, &value) in quantized.iter().enumerate().take(dh) {
                self.values[(head * dh + e) * tp + t] = value;
            }
        }
    }

    /// Writes the attention of every query of every head into `out` (`[frames][d]`).
    fn attend(&mut self, tables: &Tables, out: &mut [f32]) {
        let (dh, dhp, frames, tp) = (self.dh, self.dhp, self.frames, self.tp);
        let d = self.heads * dh;
        let rows = 2 * frames - 1;
        let scale = 1.0 / libm::sqrtf(dh as f32);
        let steps = NEXP_STEPS as f32;
        for head in 0..self.heads {
            let keys = &self.keys[head * frames * dhp..][..frames * dhp];
            let positions = &self.positions[head * rows * dhp..][..rows * dhp];
            let values = &self.values[head * dh * tp..][..dh * tp];
            let key_scales = &self.key_scales[head * frames..][..frames];
            let value_scales = &self.value_scales[head * frames..][..frames];
            let position_scales = &self.position_scales[head * rows..][..rows];
            for i in 0..frames {
                let at = head * frames + i;
                let qu = &self.qu[at * dhp..][..dhp];
                let qv = &self.qv[at * dhp..][..dhp];
                let content = self.qu_scales[at] * scale;
                let position = self.qv_scales[at] * scale;
                // Key `j` pairs with positional row `first + j`.
                let first = frames - 1 - i;
                let scores = &mut self.scores[..frames];
                let mut max = -1e30f32;
                for (j, score) in scores.iter_mut().enumerate() {
                    let a = dot(qu, &keys[j * dhp..][..dhp]);
                    let b = dot(qv, &positions[(first + j) * dhp..][..dhp]);
                    let value = a as f32 * content * key_scales[j]
                        + b as f32 * position * position_scales[first + j];
                    *score = value;
                    if value > max {
                        max = value;
                    }
                }

                // Softmax through the `exp` table, already weighted by the value scales.
                let mut sum = 0.0f32;
                let mut peak = 0.0f32;
                let max_steps = max * steps + 0.5;
                for (score, &value_scale) in scores.iter_mut().zip(value_scales) {
                    let index = (max_steps - *score * steps) as usize;
                    let exp = tables.nexp(index);
                    sum += exp;
                    let weighted = exp * value_scale;
                    *score = weighted;
                    if weighted > peak {
                        peak = weighted;
                    }
                }
                if peak < 1e-30 {
                    peak = 1e-30;
                }
                let inverse = 127.0 / peak;
                self.probabilities.fill(0);
                for (p, &score) in self.probabilities.iter_mut().zip(scores.iter()) {
                    *p = rne(score * inverse) as i8;
                }

                let output_scale = peak / 127.0 / sum;
                let out = &mut out[i * d + head * dh..][..dh];
                for (out, row) in out.iter_mut().zip(values.chunks_exact(tp)) {
                    *out = dot(&self.probabilities, row) as f32 * output_scale;
                }
            }
        }
    }
}

/// `out = x + bias`.
fn add(x: &[f32], bias: &[f32], out: &mut [f32]) {
    for ((out, &x), &bias) in out.iter_mut().zip(x).zip(bias) {
        *out = x + bias;
    }
}

/// Widest attention head that [`Attention::store`] supports.
pub(crate) const MAX_HEAD: usize = 64;

#[inline]
fn dot(a: &[i8], b: &[i8]) -> i32 {
    a.iter()
        .zip(b)
        .map(|(&a, &b)| i32::from(a) * i32::from(b))
        .sum()
}

/// Runs one conformer block over `x` in place.
pub(crate) fn layer(
    block: &Layer<'_>,
    x: &mut Frames,
    positions: &QuantRows,
    scratch: &mut Scratch,
    tables: &Tables,
) {
    feed_forward(block, x, scratch, tables);
    self_attention(block, x, positions, scratch, tables);
    convolution(block, x, scratch, tables);
    final_feed_forward(block, x, scratch, tables);
}

/// The first feed-forward module, added with half weight.
fn feed_forward(block: &Layer<'_>, x: &mut Frames, scratch: &mut Scratch, tables: &Tables) {
    let d = x.d;
    for rows in x.data.chunks_mut(ROW_BLOCK * d) {
        expand(
            &block.ln_ff1,
            &block.ff1_1,
            &block.ff1_2,
            rows,
            d,
            scratch,
            tables,
        );
        for (x, &y) in rows.iter_mut().zip(&scratch.hb) {
            *x += 0.5 * y;
        }
    }
}

/// Layer norm, expansion, `SiLU`, and projection of one row block, into `scratch.hb`.
fn expand(
    norm: &LayerNorm<'_>,
    up: &QLinear<'_>,
    down: &QLinear<'_>,
    x: &[f32],
    d: usize,
    scratch: &mut Scratch,
    tables: &Tables,
) {
    let rows = x.len() / d;
    normalize_rows(norm, x, d, &mut scratch.hb);
    scratch
        .tile
        .apply(up, &scratch.hb, rows, d, &mut scratch.h2);
    let hidden = &mut scratch.h2[..rows * up.n];
    for value in hidden.iter_mut() {
        *value *= tables.nsig(*value);
    }
    scratch
        .tile
        .apply(down, &scratch.h2, rows, up.n, &mut scratch.hb);
}

/// Multi-head self-attention with relative positions.
fn self_attention(
    block: &Layer<'_>,
    x: &mut Frames,
    positions: &QuantRows,
    scratch: &mut Scratch,
    tables: &Tables,
) {
    let d = x.d;
    for (nth, rows) in x.data.chunks(ROW_BLOCK * d).enumerate() {
        let count = rows.len() / d;
        normalize_rows(&block.ln_att, rows, d, &mut scratch.hb);
        scratch
            .tile
            .apply(&block.qkv, &scratch.hb, count, d, &mut scratch.h2);
        for (t, qkv) in scratch.h2.chunks_exact(3 * d).take(count).enumerate() {
            scratch.attention.store(nth * ROW_BLOCK + t, qkv, block);
        }
    }
    let attention = &mut scratch.attention;
    let frames = attention.frames;
    for values in attention.values.chunks_exact_mut(attention.tp) {
        values[frames..].fill(0);
    }

    let (dh, dhp) = (attention.dh, attention.dhp);
    for first in (0..positions.rows).step_by(ROW_BLOCK) {
        let count = ROW_BLOCK.min(positions.rows - first);
        block.pos.forward(
            &positions.data[first * positions.kp..],
            positions.kp,
            &positions.scales[first..],
            count,
            &mut scratch.prow,
            d,
        );
        for (r, row) in scratch.prow.chunks_exact(d).take(count).enumerate() {
            for (head, projected) in row.chunks_exact(dh).enumerate() {
                let at = head * positions.rows + first + r;
                attention.position_scales[at] =
                    quant_vec(projected, &mut attention.positions[at * dhp..][..dhp]);
            }
        }
    }

    attention.attend(tables, &mut scratch.cv);
    for (rows, attended) in x
        .data
        .chunks_mut(ROW_BLOCK * d)
        .zip(scratch.cv.chunks(ROW_BLOCK * d))
    {
        scratch
            .tile
            .apply(&block.out, attended, rows.len() / d, d, &mut scratch.hb);
        for (x, &y) in rows.iter_mut().zip(&scratch.hb) {
            *x += y;
        }
    }
}

/// Pointwise convolution with GLU, depthwise convolution, swish, projection.
fn convolution(block: &Layer<'_>, x: &mut Frames, scratch: &mut Scratch, tables: &Tables) {
    let d = x.d;
    let kernel = block.dw_w.len() / d;
    let halo = kernel / 2;
    let outputs = &mut scratch.glu[halo * d..][..x.rows * d];
    for (rows, gated) in x
        .data
        .chunks(ROW_BLOCK * d)
        .zip(outputs.chunks_mut(ROW_BLOCK * d))
    {
        let count = rows.len() / d;
        normalize_rows(&block.ln_conv, rows, d, &mut scratch.hb);
        scratch
            .tile
            .apply(&block.pw1, &scratch.hb, count, d, &mut scratch.h2);
        for (out, pair) in gated
            .chunks_exact_mut(d)
            .zip(scratch.h2.chunks_exact(2 * d))
        {
            let (values, gates) = pair.split_at(d);
            for ((out, &value), &gate) in out.iter_mut().zip(values).zip(gates) {
                *out = value * tables.nsig(gate);
            }
        }
    }

    // The rows outside the utterance stay zero, which pads the convolution.
    for (t, out) in scratch.cv.chunks_exact_mut(d).take(x.rows).enumerate() {
        out.copy_from_slice(block.dw_b);
        let window = &scratch.glu[t * d..][..kernel * d];
        for (weights, input) in block.dw_w.chunks_exact(d).zip(window.chunks_exact(d)) {
            for ((out, &w), &x) in out.iter_mut().zip(weights).zip(input) {
                *out += w * x;
            }
        }
        for value in out.iter_mut() {
            *value *= tables.nsig(*value);
        }
    }

    for (rows, convolved) in x
        .data
        .chunks_mut(ROW_BLOCK * d)
        .zip(scratch.cv.chunks(ROW_BLOCK * d))
    {
        scratch
            .tile
            .apply(&block.pw2, convolved, rows.len() / d, d, &mut scratch.hb);
        for (x, &y) in rows.iter_mut().zip(&scratch.hb) {
            *x += y;
        }
    }
}

/// The second feed-forward module and the final layer norm.
fn final_feed_forward(block: &Layer<'_>, x: &mut Frames, scratch: &mut Scratch, tables: &Tables) {
    let d = x.d;
    for rows in x.data.chunks_mut(ROW_BLOCK * d) {
        expand(
            &block.ln_ff2,
            &block.ff2_1,
            &block.ff2_2,
            rows,
            d,
            scratch,
            tables,
        );
        for (row, update) in rows.chunks_exact_mut(d).zip(scratch.hb.chunks_exact(d)) {
            for (x, &y) in row.iter_mut().zip(update) {
                *x += 0.5 * y;
            }
            let (mean, inverse) = statistics(row);
            for ((x, &gamma), &beta) in row
                .iter_mut()
                .zip(block.ln_out.gamma)
                .zip(block.ln_out.beta)
            {
                *x = (*x - mean) * inverse * gamma + beta;
            }
        }
    }
}

/// Layer normalization of every row of `x` into `out`.
fn normalize_rows(norm: &LayerNorm<'_>, x: &[f32], d: usize, out: &mut [f32]) {
    for (row, out) in x.chunks_exact(d).zip(out.chunks_exact_mut(d)) {
        let (mean, inverse) = statistics(row);
        for (((out, &x), &gamma), &beta) in out.iter_mut().zip(row).zip(norm.gamma).zip(norm.beta) {
            *out = (x - mean) * inverse * gamma + beta;
        }
    }
}

/// Mean and inverse standard deviation of a row.
fn statistics(row: &[f32]) -> (f32, f32) {
    let width = row.len() as f32;
    let mut mean = 0.0f32;
    for &value in row {
        mean += value;
    }
    mean /= width;
    let mut variance = 0.0f32;
    for &value in row {
        let centered = value - mean;
        variance += centered * centered;
    }
    variance /= width;
    (mean, 1.0 / libm::sqrtf(variance + 1e-5))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::features;
    use crate::frontend::subsample;
    use crate::posenc::positions;
    use crate::stage_hash::{fnv_f32, reference_hashes, stage, tone};

    #[test]
    fn should_handle_a_single_frame() {
        let model = Model::parse(oido_reference::model_blob()).unwrap();
        let tables = Tables::new();
        // The shortest audio that gives any output.
        let pcm = tone(320);
        let mut x = subsample(&model, &features(&model, &pcm).unwrap()).unwrap();
        assert_eq!(x.rows, 1);
        let (_, pos) = positions(x.rows, x.d).unwrap();
        let mut scratch = Scratch::new(&model, x.rows).unwrap();
        for block in &model.layers {
            layer(block, &mut x, &pos, &mut scratch, &tables);
        }
        assert_eq!(
            fnv_f32(&x.data),
            stage(&reference_hashes(&pcm), "layer", 11)
        );
    }
}
