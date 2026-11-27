//! Row quantization and the quantized linear layer.

// Ported from esp32/components/tinyasr/kernels.c:77-91, 183-211 and 338-533,
// and tasr_nemo.c:322-331 at commit c8d60325ba8bffd4bad65a9ac63c713ee64d1aae.

use crate::mathx::rne;

/// Outputs of one tile of the blocked int4 layout.
const TILE: usize = 16;

/// Weights that one 16-byte group of packed int4 holds.
const UNPACKED: usize = 32;

/// Rows that share one pass over the weights.
const ROW_BLOCK: usize = 64;

/// Quantizes `x` symmetrically to int8, zero-pads `xq`, and returns the scale.
pub(crate) fn quant_vec(x: &[f32], xq: &mut [i8]) -> f32 {
    let mut max = 0.0f32;
    for value in x {
        let magnitude = value.abs();
        if magnitude > max {
            max = magnitude;
        }
    }
    if max < 1e-30 {
        max = 1e-30;
    }
    let inverse = 127.0 / max;
    let (head, padding) = xq.split_at_mut(x.len());
    for (q, value) in head.iter_mut().zip(x) {
        *q = rne(value * inverse) as i8;
    }
    padding.fill(0);
    max / 127.0
}

/// Quantizes `rows` rows of `k` values, each with its own scale.
///
/// Rows start `ldx` values apart in `x` and `ldq` apart in `xq`, and are
/// zero-padded to `kp`.
#[expect(
    clippy::too_many_arguments,
    reason = "the shape of the source and the destination are independent"
)]
pub(crate) fn quant_rows(
    x: &[f32],
    rows: usize,
    k: usize,
    ldx: usize,
    xq: &mut [i8],
    ldq: usize,
    kp: usize,
    xs: &mut [f32],
) {
    for (t, scale) in xs.iter_mut().enumerate().take(rows) {
        *scale = quant_vec(&x[t * ldx..][..k], &mut xq[t * ldq..][..kp]);
    }
}

/// Weight storage of a [`QLinear`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    /// `[n][kp]` bytes.
    Int8,
    /// Tiles of 16 outputs; a byte holds two consecutive inputs of one output.
    Int4Blocked,
    /// `[n][kp / 2]` bytes; byte `j` of a group holds inputs `j` and `j + 16`.
    Int4Rows,
}

/// A linear layer with int8 or int4 weights and quantized inputs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct QLinear<'a> {
    pub n: usize,
    pub k: usize,
    pub kp: usize,
    pub layout: Layout,
    pub weights: &'a [u8],
    pub scales: &'a [f32],
    pub bias: &'a [f32],
}

impl QLinear<'_> {
    /// Computes `y[t][n] = sum_k(xq[t][k] * w[n][k]) * xs[t] * scales[n] + bias[n]`.
    ///
    /// Each input row holds at least `kp` values.
    pub(crate) fn forward(
        &self,
        xq: &[i8],
        ldq: usize,
        xs: &[f32],
        rows: usize,
        y: &mut [f32],
        ldy: usize,
    ) {
        match self.layout {
            Layout::Int8 => {
                let weights: &[i8] = bytemuck::cast_slice(self.weights);
                for n in 0..self.n {
                    let w = &weights[n * self.kp..][..self.kp];
                    for t in 0..rows {
                        let acc = dot(&xq[t * ldq..][..self.kp], w);
                        y[t * ldy + n] = self.output(n, acc, xs[t]);
                    }
                }
            }
            Layout::Int4Rows => {
                let row_bytes = self.kp / 2;
                for n in 0..self.n {
                    let packed = &self.weights[n * row_bytes..][..row_bytes];
                    for first in (0..rows).step_by(ROW_BLOCK) {
                        let mut acc = [0i32; ROW_BLOCK];
                        let count = ROW_BLOCK.min(rows - first);
                        for (group, bytes) in packed.chunks_exact(UNPACKED / 2).enumerate() {
                            let w = unpack_split(bytes);
                            for (t, acc) in acc.iter_mut().enumerate().take(count) {
                                let start = (first + t) * ldq + group * UNPACKED;
                                *acc += dot(&xq[start..][..UNPACKED], &w);
                            }
                        }
                        for (t, &acc) in acc.iter().enumerate().take(count) {
                            y[(first + t) * ldy + n] = self.output(n, acc, xs[first + t]);
                        }
                    }
                }
            }
            Layout::Int4Blocked => self.forward_blocked(xq, ldq, xs, rows, y, ldy),
        }
    }

    fn forward_blocked(
        &self,
        xq: &[i8],
        ldq: usize,
        xs: &[f32],
        rows: usize,
        y: &mut [f32],
        ldy: usize,
    ) {
        let tile_bytes = self.kp * TILE / 2;
        for first_output in (0..self.n).step_by(TILE) {
            let tile = &self.weights[first_output / TILE * tile_bytes..][..tile_bytes];
            for first_row in (0..rows).step_by(ROW_BLOCK) {
                let count = ROW_BLOCK.min(rows - first_row);
                let mut acc = [[0i32; TILE]; ROW_BLOCK];
                // Eight packed lines at a time are 16 inputs of all 16 outputs.
                for (group, bytes) in tile.chunks_exact(8 * TILE).enumerate() {
                    let mut w = [[0i8; TILE]; TILE];
                    for (pair, line) in w.chunks_exact_mut(2).zip(bytes.chunks_exact(TILE)) {
                        for (j, &byte) in line.iter().enumerate() {
                            pair[0][j] = low_nibble(byte);
                            pair[1][j] = high_nibble(byte);
                        }
                    }
                    for (t, acc) in acc.iter_mut().enumerate().take(count) {
                        let start = (first_row + t) * ldq + group * TILE;
                        for (&x, weights) in xq[start..][..TILE].iter().zip(&w) {
                            let x = i32::from(x);
                            for (sum, &weight) in acc.iter_mut().zip(weights) {
                                *sum += x * i32::from(weight);
                            }
                        }
                    }
                }
                for (t, acc) in acc.iter().enumerate().take(count) {
                    let row = first_row + t;
                    let out = &mut y[row * ldy + first_output..][..TILE];
                    for (j, (value, &sum)) in out.iter_mut().zip(acc).enumerate() {
                        *value = self.output(first_output + j, sum, xs[row]);
                    }
                }
            }
        }
    }

    /// Float epilogue of an integer sum, in the evaluation order of the C engine.
    #[inline]
    fn output(&self, n: usize, acc: i32, input_scale: f32) -> f32 {
        acc as f32 * input_scale * self.scales[n] + self.bias[n]
    }
}

#[inline]
fn dot(x: &[i8], w: &[i8]) -> i32 {
    x.iter()
        .zip(w)
        .map(|(&a, &b)| i32::from(a) * i32::from(b))
        .sum()
}

/// Removes the offset of 8 from the low half of a packed byte.
#[inline]
fn low_nibble(byte: u8) -> i8 {
    (byte & 15) as i8 - 8
}

/// Removes the offset of 8 from the high half of a packed byte.
#[inline]
fn high_nibble(byte: u8) -> i8 {
    (byte >> 4) as i8 - 8
}

/// Unpacks 16 bytes of the row layout into 32 weights.
#[inline]
fn unpack_split(bytes: &[u8]) -> [i8; UNPACKED] {
    let mut w = [0i8; UNPACKED];
    let (low, high) = w.split_at_mut(UNPACKED / 2);
    for ((low, high), &byte) in low.iter_mut().zip(high).zip(bytes) {
        *low = low_nibble(byte);
        *high = high_nibble(byte);
    }
    w
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::*;
    use crate::mathx::tests::XorShift32;

    fn floats(rng: &mut XorShift32, len: usize) -> Vec<f32> {
        (0..len).map(|_| rng.uniform(-4.0, 4.0)).collect()
    }

    /// An integer uniformly drawn from `[low, high]`.
    fn integer(rng: &mut XorShift32, low: i32, high: i32) -> i8 {
        let span = (high - low + 1) as u32;
        (low + (rng.next_u32() % span) as i32) as i8
    }

    #[test]
    fn quant_rows_should_match_the_reference() {
        let mut rng = XorShift32::new();
        for rows in [1, 3, 64] {
            for k in [9, 176, 352, 1280] {
                let kp = usize::next_multiple_of(k, 16);
                let mut x = floats(&mut rng, rows * k);
                // The last row has a single nonzero value, the one before it none.
                x[(rows - 1) * k..].fill(0.0);
                x[(rows - 1) * k + k / 2] = -3.5;
                if rows > 1 {
                    x[(rows - 2) * k..(rows - 1) * k].fill(0.0);
                }
                for x in [x, vec![0.0; rows * k]] {
                    let (mut xq, mut xs) = (vec![1i8; rows * kp], vec![0.0; rows]);
                    quant_rows(&x, rows, k, k, &mut xq, kp, kp, &mut xs);
                    let (mut expected_rows, mut expected_scales) =
                        (vec![2i8; rows * kp], vec![0.0; rows]);
                    oido_reference::quant_rows(
                        &x,
                        rows,
                        k,
                        k,
                        &mut expected_rows,
                        kp,
                        kp,
                        &mut expected_scales,
                    );
                    assert_eq!(xq, expected_rows, "rows {rows}, k {k}");
                    assert_eq!(bits(&xs), bits(&expected_scales), "rows {rows}, k {k}");
                }
            }
        }
    }

    fn bits(x: &[f32]) -> Vec<u32> {
        x.iter().map(|v| v.to_bits()).collect()
    }

    /// Packs `w[n][k]` (rows of `kp` values) as the model exporter does.
    fn pack(w: &[i8], n: usize, kp: usize, layout: Layout) -> Vec<u8> {
        let nibbles = |low: i8, high: i8| (low + 8) as u8 | (((high + 8) as u8) << 4);
        match layout {
            Layout::Int8 => w.iter().map(|&v| v as u8).collect(),
            Layout::Int4Blocked => {
                let mut out = Vec::new();
                for tile in 0..n / 16 {
                    for k in (0..kp).step_by(2) {
                        for j in 0..16 {
                            let row = (tile * 16 + j) * kp;
                            out.push(nibbles(w[row + k], w[row + k + 1]));
                        }
                    }
                }
                out
            }
            Layout::Int4Rows => w
                .chunks_exact(32)
                .flat_map(|block| (0..16).map(|j| nibbles(block[j], block[j + 16])))
                .collect(),
        }
    }

    fn check_qlinear(shapes: &[(usize, usize)], layout: Layout) {
        let mut rng = XorShift32::new();
        for &(n, k) in shapes {
            let bits4 = layout != Layout::Int8;
            let kp = k.next_multiple_of(if layout == Layout::Int4Rows { 32 } else { 16 });
            let (low, high) = if bits4 { (-8, 7) } else { (-127, 127) };
            let mut w = vec![0i8; n * kp];
            for row in w.chunks_exact_mut(kp) {
                row[..k].fill_with(|| integer(&mut rng, low, high));
            }
            let weights = pack(&w, n, kp, layout);
            let scales = floats(&mut rng, n);
            let bias = floats(&mut rng, n);
            let rows = 70;
            let mut xq = vec![0i8; rows * kp];
            for row in xq.chunks_exact_mut(kp) {
                row[..k].fill_with(|| integer(&mut rng, -127, 127));
            }
            let xs: Vec<f32> = (0..rows).map(|_| rng.uniform(0.001, 0.1)).collect();

            let linear = QLinear {
                n,
                k,
                kp,
                layout,
                weights: &weights,
                scales: &scales,
                bias: &bias,
            };
            let ldy = n + 3;
            let mut y = vec![0.0; rows * ldy];
            linear.forward(&xq, kp, &xs, rows, &mut y, ldy);

            let mut expected = vec![0.0; rows * ldy];
            oido_reference::QLinear {
                n,
                k,
                kp,
                bits: if bits4 { 4 } else { 8 },
                blocked: layout == Layout::Int4Blocked,
                weights: &weights,
                scales: &scales,
                bias: &bias,
            }
            .forward(&xq, kp, &xs, rows, &mut expected, ldy);
            assert_eq!(bits(&y), bits(&expected), "n {n}, k {k}");
        }
    }

    #[test]
    fn qlinear_should_match_the_reference() {
        let int8: &[(usize, usize)] = &[(64, 576), (176, 1280), (1025, 176), (176, 352)];
        let blocked: &[(usize, usize)] = &[(704, 176), (176, 704), (528, 176), (352, 176)];
        for (shapes, layout) in [
            (int8, Layout::Int8),
            (blocked, Layout::Int4Blocked),
            (&[(40, 96)], Layout::Int4Rows),
        ] {
            check_qlinear(shapes, layout);
        }
    }

    #[test]
    fn quant_vec_should_match_the_reference() {
        let mut rng = XorShift32::new();
        for x in [floats(&mut rng, 44), vec![0.0; 44]] {
            let (mut q, mut expected) = ([1i8; 48], [2i8; 48]);
            let scale = quant_vec(&x, &mut q);
            let expected_scale = oido_reference::qvec(&x, &mut expected);
            assert_eq!(q, expected);
            assert_eq!(scale.to_bits(), expected_scale.to_bits());
        }
    }
}
