//! FNV-1a hashes of intermediate tensors, like `ndbg` of the C engine.

// Ported from esp32/components/tinyasr/tasr_nemo.c:257-262 at commit
// c8d60325ba8bffd4bad65a9ac63c713ee64d1aae.

use alloc::string::String;
use alloc::vec::Vec;

const OFFSET_BASIS: u32 = 2_166_136_261;
const PRIME: u32 = 16_777_619;

/// Hashes a float tensor, one 32-bit word at a time.
pub(crate) fn fnv_f32(x: &[f32]) -> u32 {
    x.iter()
        .fold(OFFSET_BASIS, |h, v| (h ^ v.to_bits()).wrapping_mul(PRIME))
}

/// The stage hashes of one greedy transcription by the C engine.
pub(crate) fn reference_hashes(pcm: &[i16]) -> Vec<(String, i32, u32)> {
    let model = oido_reference::RefModel::load(oido_reference::model_blob());
    oido_reference::transcribe(&model, None, 0, 0, pcm).stage_hashes
}

/// The hash of stage `name` of layer `layer`.
#[track_caller]
pub(crate) fn stage(hashes: &[(String, i32, u32)], name: &str, layer: usize) -> u32 {
    hashes
        .iter()
        .find(|(stage, at, _)| stage == name && usize::try_from(*at) == Ok(layer))
        .unwrap_or_else(|| panic!("no hash of stage {name} for layer {layer}"))
        .2
}

/// A 440 Hz sine at amplitude 8000, for inputs shorter than the clips.
pub(crate) fn tone(len: u32) -> Vec<i16> {
    (0..len)
        .map(|i| {
            let phase = core::f64::consts::TAU * 440.0 * f64::from(i) / 16_000.0;
            (8_000.0 * libm::sin(phase)) as i16
        })
        .collect()
}
