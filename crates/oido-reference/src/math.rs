//! Math helpers and lookup tables of the C engine.

use std::sync::Once;

use crate::ffi;

/// Builds the lazily initialized tables of the C engine once.
pub(crate) fn init_tables() {
    static INIT: Once = Once::new();
    // SAFETY: `Once` makes this the only writer of the tables.
    INIT.call_once(|| unsafe { ffi::ref_init_tables() });
}

/// `tasr_expf`.
#[must_use]
pub fn expf(x: f32) -> f32 {
    // SAFETY: a pure function of its argument.
    unsafe { ffi::ref_expf(x) }
}

/// `tasr_logf`.
#[must_use]
pub fn logf(x: f32) -> f32 {
    // SAFETY: a pure function of its argument.
    unsafe { ffi::ref_logf(x) }
}

/// The sigmoid table of the utterance engine.
#[must_use]
pub fn nsig(x: f32) -> f32 {
    init_tables();
    // SAFETY: the tables are built.
    unsafe { ffi::ref_nsig(x) }
}

/// Entry `index` of the `exp(-index / 64)` table.
///
/// # Panics
///
/// Panics when `index` is not below 1280.
#[must_use]
pub fn nexp_tab(index: usize) -> f32 {
    assert!(index < 1280, "the table has 1280 entries");
    init_tables();
    // SAFETY: the tables are built and the index is in range.
    unsafe { ffi::ref_nexp_tab(crate::to_c_int(index)) }
}

/// `tasr_fast_expneg`.
#[must_use]
pub fn fast_expneg(u: f32) -> f32 {
    init_tables();
    // SAFETY: the tables are built.
    unsafe { ffi::tasr_fast_expneg(u) }
}

/// `tasr_fast_sigmoid`.
#[must_use]
pub fn fast_sigmoid(x: f32) -> f32 {
    init_tables();
    // SAFETY: the tables are built.
    unsafe { ffi::tasr_fast_sigmoid(x) }
}

/// `tasr_fast_tanh`.
#[must_use]
pub fn fast_tanh(x: f32) -> f32 {
    init_tables();
    // SAFETY: the tables are built.
    unsafe { ffi::tasr_fast_tanh(x) }
}
