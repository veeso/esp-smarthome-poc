//! Rounding, exponential, logarithm, and the lookup tables of the engine.

// Ported from esp32/components/tinyasr/tasr_math.h:15-40, kernels.c:46-51 and
// kernels.c:295-320, and tasr_nemo.c:294-321 at commit
// c8d60325ba8bffd4bad65a9ac63c713ee64d1aae.

use alloc::boxed::Box;
use core::f32::consts::{LN_2, LOG2_E, SQRT_2};

/// `1.5 * 2^23`: adding and subtracting it rounds a float to the nearest even integer.
const ROUND_MAGIC: f32 = 12_582_912.0;

/// Entries of the `exp(-i / 64)` table, which covers `[0, 20)`.
pub(crate) const NEXP_LEN: usize = 20 * NEXP_STEPS;

/// Table steps per unit of the `exp(-u)` table of the attention softmax.
pub(crate) const NEXP_STEPS: usize = 64;

/// Intervals of the sigmoid tables over `[-12, 12)`, in steps of 1/16.
const SIGMOID_STEPS: usize = 384;

/// Rounds to the nearest even integer, for `|v| < 2^22`.
#[inline]
pub(crate) fn rne(v: f32) -> i32 {
    ((v + ROUND_MAGIC) - ROUND_MAGIC) as i32
}

/// Deterministic `exp`, with the same bits on every target.
#[inline]
pub(crate) fn expf(x: f32) -> f32 {
    if x < -87.0 {
        return 0.0;
    }
    let x = if x > 88.0 { 88.0 } else { x };
    let scaled = x * LOG2_E;
    let n = (scaled + if scaled >= 0.0 { 0.5 } else { -0.5 }) as i32;
    // `ln 2` in two parts keeps the reduced argument exact.
    let r = (x - n as f32 * 0.693_145_75) - n as f32 * 1.428_606_8e-6;
    let p = 1.0
        + r * (1.0
            + r * (0.5
                + r * (0.166_666_67
                    + r * (0.041_666_668 + r * (0.008_333_334 + r * 0.001_388_888_9)))));
    f32::from_bits(p.to_bits().wrapping_add((n as u32).wrapping_shl(23)))
}

/// Deterministic natural logarithm; at or below `f32::MIN_POSITIVE` it saturates.
#[inline]
pub(crate) fn logf(x: f32) -> f32 {
    if x.is_nan() || x <= f32::MIN_POSITIVE {
        return -87.3365;
    }
    let bits = x.to_bits();
    let mut exponent = ((bits >> 23) & 0xff) as i32 - 127;
    let mut m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    if m > SQRT_2 {
        m *= 0.5;
        exponent += 1;
    }
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let log_m = 2.0
        * s
        * (1.0
            + s2 * (0.333_333_34
                + s2 * (0.2 + s2 * (0.142_857_15 + s2 * (0.111_111_11 + s2 * 0.090_909_09)))));
    log_m + exponent as f32 * LN_2
}

/// Lookup tables of the sigmoid, the tanh, and `exp(-u)`.
#[derive(Debug)]
pub(crate) struct Tables {
    nsig: Box<[f32]>,
    nsig_slope: Box<[f32]>,
    nexp: Box<[f32]>,
    exp: Box<[f32]>,
    sigmoid: Box<[f32]>,
}

impl Tables {
    pub(crate) fn new() -> Self {
        let logistic = |x: f64| (1.0 / (1.0 + libm::exp(-x))) as f32;
        let nsig: Box<[f32]> = (0..=SIGMOID_STEPS)
            .map(|i| logistic(-12.0 + i as f64 / 16.0))
            .collect();
        let nsig_slope = nsig.windows(2).map(|pair| pair[1] - pair[0]).collect();
        let nexp = (0..NEXP_LEN)
            .map(|i| libm::exp(-(i as f64) / NEXP_STEPS as f64) as f32)
            .collect();
        let exp = (0..SIGMOID_STEPS + 2)
            .map(|i| libm::exp(-(i as f64) / 16.0) as f32)
            .collect();
        let sigmoid = (0..SIGMOID_STEPS + 2)
            .map(|i| logistic(i as f64 / 16.0 - 12.0))
            .collect();
        Self {
            nsig,
            nsig_slope,
            nexp,
            exp,
            sigmoid,
        }
    }

    /// Interpolated sigmoid of the encoder, clamped to 0 and 1 outside `[-12, 12)`.
    #[inline]
    pub(crate) fn nsig(&self, x: f32) -> f32 {
        let u = (x + 12.0) * 16.0;
        if u <= 0.0 {
            return 0.0;
        }
        if u >= SIGMOID_STEPS as f32 {
            return 1.0;
        }
        let i = u as usize;
        self.nsig[i] + (u - i as f32) * self.nsig_slope[i]
    }

    /// `exp(-index / 64)`, or 0 beyond the table.
    #[inline]
    pub(crate) fn nexp(&self, index: usize) -> f32 {
        self.nexp.get(index).copied().unwrap_or(0.0)
    }

    /// Interpolated `exp(-u)` for `u >= 0`.
    #[inline]
    pub(crate) fn fast_expneg(&self, u: f32) -> f32 {
        if u >= 23.9 {
            return 0.0;
        }
        let x = if u < 0.0 { 0.0 } else { u } * 16.0;
        let i = x as usize;
        let f = x - i as f32;
        self.exp[i] + f * (self.exp[i + 1] - self.exp[i])
    }

    /// Interpolated sigmoid of the language model.
    #[inline]
    pub(crate) fn fast_sigmoid(&self, x: f32) -> f32 {
        if x <= -11.99 {
            return self.sigmoid[0];
        }
        if x >= 11.99 {
            return self.sigmoid[SIGMOID_STEPS];
        }
        let u = (x + 12.0) * 16.0;
        let i = u as usize;
        let f = u - i as f32;
        self.sigmoid[i] + f * (self.sigmoid[i + 1] - self.sigmoid[i])
    }

    /// Hyperbolic tangent through [`Tables::fast_sigmoid`].
    #[inline]
    pub(crate) fn fast_tanh(&self, x: f32) -> f32 {
        2.0 * self.fast_sigmoid(2.0 * x) - 1.0
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Deterministic random inputs for the parity tests.
    pub(crate) struct XorShift32(u32);

    impl XorShift32 {
        pub(crate) fn new() -> Self {
            Self(0x2545_F491)
        }

        pub(crate) fn next_u32(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            self.0
        }

        /// A value uniformly drawn from `[low, high)`.
        pub(crate) fn uniform(&mut self, low: f32, high: f32) -> f32 {
            low + (self.next_u32() >> 8) as f32 / 16_777_216.0 * (high - low)
        }
    }

    #[track_caller]
    fn assert_bits(actual: f32, expected: f32, input: f32) {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "input {input}: {actual} != {expected}"
        );
    }

    #[test]
    fn rne_should_round_half_to_even() {
        for (input, expected) in [
            (0.5, 0),
            (1.5, 2),
            (2.5, 2),
            (-0.5, 0),
            (-1.5, -2),
            (126.6, 127),
            (-127.4, -127),
        ] {
            assert_eq!(rne(input), expected, "input {input}");
        }
    }

    #[test]
    fn expf_should_match_the_reference() {
        let mut rng = XorShift32::new();
        let edges = [0.0, -87.3, 88.7, f32::MIN_POSITIVE];
        let random = core::iter::repeat_with(|| rng.uniform(-90.0, 90.0)).take(100_000);
        for x in random.chain(edges) {
            assert_bits(expf(x), oido_reference::expf(x), x);
        }
    }

    #[test]
    fn logf_should_match_the_reference() {
        let mut rng = XorShift32::new();
        let edges = [0.0, -1.0, f32::MIN_POSITIVE, 1.0];
        let random =
            core::iter::repeat_with(|| libm::exp2f(rng.uniform(-126.0, 127.0))).take(100_000);
        for x in random.chain(edges) {
            assert_bits(logf(x), oido_reference::logf(x), x);
        }
    }

    #[test]
    fn tables_should_match_the_reference() {
        let tables = Tables::new();
        for i in 0..1280 {
            assert_bits(tables.nexp(i), oido_reference::nexp_tab(i), i as f32);
        }

        let mut rng = XorShift32::new();
        let edges = [-12.0, 12.0, 11.99, -11.99, 23.9, 24.0];
        let random = core::iter::repeat_with(|| rng.uniform(-30.0, 30.0)).take(50_000);
        for x in random.chain(edges) {
            assert_bits(tables.nsig(x), oido_reference::nsig(x), x);
            assert_bits(tables.fast_expneg(x), oido_reference::fast_expneg(x), x);
            assert_bits(tables.fast_sigmoid(x), oido_reference::fast_sigmoid(x), x);
            assert_bits(tables.fast_tanh(x), oido_reference::fast_tanh(x), x);
        }
    }
}
