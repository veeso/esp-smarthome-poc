//! Relative positional encoding.

// Ported from esp32/components/tinyasr/tasr_nemo.c:1000-1027 at commit
// c8d60325ba8bffd4bad65a9ac63c713ee64d1aae.

use alloc::vec::Vec;

use crate::quant::quant_rows;
use crate::transcriber::{TranscribeError, zeroed};

/// Rows between two exact evaluations of the rotation recurrence.
const ANCHOR_ROWS: usize = 64;

/// Rows quantized to int8, zero-padded to `kp`, with one scale each.
#[derive(Debug)]
pub(crate) struct QuantRows {
    pub rows: usize,
    pub kp: usize,
    pub data: Vec<i8>,
    pub scales: Vec<f32>,
}

/// The `2 * frames - 1` encodings of width `d`, as floats and quantized.
///
/// Row `r` holds position `frames - 1 - r`.
pub(crate) fn positions(frames: usize, d: usize) -> Result<(Vec<f32>, QuantRows), TranscribeError> {
    let rows = (2 * frames).saturating_sub(1);
    let mut pe = zeroed::<f32>(rows * d)?;
    for i in 0..d / 2 {
        let frequency = libm::exp((2.0 * i as f64) * -(libm::log(10_000.0) / d as f64));
        let (cos_step, sin_step) = (libm::cos(frequency) as f32, libm::sin(frequency) as f32);
        let (mut sin, mut cos) = (0.0f32, 1.0f32);
        for (r, row) in pe.chunks_exact_mut(d).enumerate() {
            if r % ANCHOR_ROWS == 0 {
                let position = frames as f64 - 1.0 - r as f64;
                let angle = libm::fmod(position * frequency, 2.0 * core::f64::consts::PI);
                sin = libm::sin(angle) as f32;
                cos = libm::cos(angle) as f32;
            }
            row[2 * i] = sin;
            row[2 * i + 1] = cos;
            // Rotate by one step towards the next lower position.
            (sin, cos) = (
                sin * cos_step - cos * sin_step,
                cos * cos_step + sin * sin_step,
            );
        }
    }

    let kp = d.next_multiple_of(16);
    let mut data = zeroed::<i8>(rows * kp)?;
    let mut scales = zeroed::<f32>(rows)?;
    quant_rows(&pe, rows, d, d, &mut data, kp, kp, &mut scales);
    Ok((
        pe,
        QuantRows {
            rows,
            kp,
            data,
            scales,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_produce_a_single_row_for_one_frame() {
        let (pe, quantized) = positions(1, 176).unwrap();
        assert_eq!(pe.len(), 176);
        for pair in pe.chunks_exact(2) {
            assert_eq!(
                (pair[0].to_bits(), pair[1].to_bits()),
                (0, 1.0f32.to_bits())
            );
        }
        assert_eq!((quantized.rows, quantized.kp), (1, 176));
    }
}
