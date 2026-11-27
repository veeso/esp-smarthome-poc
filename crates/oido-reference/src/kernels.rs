//! Quantization and linear layers of the C engine.

use std::ffi::c_int;
use std::marker::PhantomData;

use crate::{ffi, to_c_int};

/// `tasr_quant_rows`.
///
/// # Panics
///
/// Panics when a buffer is shorter than the shape requires.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the C signature that the port is compared against"
)]
pub fn quant_rows(
    x: &[f32],
    rows: usize,
    k: usize,
    ldx: usize,
    xq: &mut [i8],
    ldq: usize,
    kp: usize,
    xs: &mut [f32],
) {
    assert!(rows > 0 && k <= ldx && k <= kp && kp <= ldq);
    assert!(x.len() >= (rows - 1) * ldx + k);
    assert!(xq.len() >= (rows - 1) * ldq + kp && xs.len() >= rows);
    // SAFETY: the assertions keep every access inside the buffers.
    unsafe {
        ffi::tasr_quant_rows(
            x.as_ptr(),
            to_c_int(rows),
            to_c_int(k),
            to_c_int(ldx),
            xq.as_mut_ptr(),
            to_c_int(ldq),
            to_c_int(kp),
            xs.as_mut_ptr(),
        );
    }
}

/// `qvec` of the utterance engine: quantizes `x` into `q` and returns the scale.
///
/// # Panics
///
/// Panics when `q` is shorter than `x`.
#[must_use]
pub fn qvec(x: &[f32], q: &mut [i8]) -> f32 {
    assert!(q.len() >= x.len());
    // SAFETY: `x` holds `x.len()` values and `q` holds `q.len()` values.
    unsafe {
        ffi::ref_qvec(
            x.as_ptr(),
            to_c_int(x.len()),
            q.as_mut_ptr(),
            to_c_int(q.len()),
        )
    }
}

/// A `tasr_qlin_t` that borrows its tensors.
#[derive(Clone, Copy, Debug)]
pub struct QLinear<'a> {
    /// Outputs.
    pub n: usize,
    /// Inputs.
    pub k: usize,
    /// Inputs rounded up to the block size.
    pub kp: usize,
    /// Weight width, 4 or 8.
    pub bits: u8,
    /// Whether int4 weights use the tiles of 16 outputs.
    pub blocked: bool,
    /// Packed weights.
    pub weights: &'a [u8],
    /// Scale of each output.
    pub scales: &'a [f32],
    /// Bias of each output.
    pub bias: &'a [f32],
}

impl QLinear<'_> {
    /// `tasr_qlin_range` over every output.
    ///
    /// # Panics
    ///
    /// Panics when a buffer is shorter than the shape requires.
    pub fn forward(
        &self,
        xq: &[i8],
        ldq: usize,
        xs: &[f32],
        rows: usize,
        y: &mut [f32],
        ldy: usize,
    ) {
        let weight_bytes = self.n * self.kp * usize::from(self.bits) / 8;
        assert!(self.weights.len() >= weight_bytes);
        assert!(self.scales.len() >= self.n && self.bias.len() >= self.n);
        assert!(rows > 0 && self.kp <= ldq && self.n <= ldy);
        assert!(xq.len() >= (rows - 1) * ldq + self.kp && xs.len() >= rows);
        assert!(y.len() >= (rows - 1) * ldy + self.n);
        let raw = ffi::QLinear {
            n: to_c_int(self.n),
            k: to_c_int(self.k),
            kp: to_c_int(self.kp),
            bits: c_int::from(self.bits),
            blocked: c_int::from(self.blocked),
            w: self.weights.as_ptr().cast(),
            s: self.scales.as_ptr(),
            b: self.bias.as_ptr(),
            _borrow: PhantomData,
        };
        let mut scratch = vec![0i8; 16 * self.kp + 16];
        let mut accumulators = vec![0i32; 64 * 16];
        // SAFETY: the assertions and the scratch sizes keep the kernel in bounds.
        unsafe {
            ffi::tasr_qlin_range(
                &raw const raw,
                xq.as_ptr(),
                xs.as_ptr(),
                to_c_int(rows),
                to_c_int(ldq),
                y.as_mut_ptr(),
                to_c_int(ldy),
                scratch.as_mut_ptr(),
                accumulators.as_mut_ptr(),
                0,
                to_c_int(self.n),
            );
        }
    }
}
