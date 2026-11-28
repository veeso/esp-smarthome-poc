//! Declarations of the C functions that the oracle calls.

use std::ffi::{c_char, c_float, c_int};
use std::marker::{PhantomData, PhantomPinned};

/// Opaque `tasr_nemo_t`.
#[repr(C)]
pub(crate) struct Model {
    _data: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

/// Opaque `tasr_lm_t`.
#[repr(C)]
pub(crate) struct Lm {
    _data: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

/// `tasr_qlin_t`.
#[repr(C)]
pub(crate) struct QLinear<'a> {
    pub(crate) n: c_int,
    pub(crate) k: c_int,
    pub(crate) kp: c_int,
    pub(crate) bits: c_int,
    pub(crate) blocked: c_int,
    pub(crate) w: *const i8,
    pub(crate) s: *const c_float,
    pub(crate) b: *const c_float,
    pub(crate) _borrow: PhantomData<&'a [u8]>,
}

/// `tasr_seg_t`.
#[derive(Debug)]
#[repr(C)]
pub(crate) struct Segment {
    pub(crate) gain: c_float,
    pub(crate) noise_floor: c_float,
    pub(crate) buf: *mut i16,
    pub(crate) cap: c_int,
    pub(crate) n: c_int,
    pub(crate) speech: c_int,
    pub(crate) silence: c_int,
    pub(crate) voiced: c_int,
    pub(crate) init: c_int,
    pub(crate) hang: c_int,
}

/// Opaque `tasr_decoder_t`.
#[repr(C)]
pub(crate) struct Decoder {
    _data: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

// The signatures match the vendored headers and the shims.
unsafe extern "C" {
    pub(crate) fn ref_ctc_log_probs(row: *const c_float, v1: c_int, out: *mut c_float);
    pub(crate) fn ref_dbg_take(out: *mut c_char, cap: usize);
    pub(crate) fn ref_expf(x: c_float) -> c_float;
    pub(crate) fn ref_features(
        model: *const Model,
        pcm: *const i16,
        n: c_int,
        out: *mut c_float,
        cap_frames: c_int,
    ) -> c_int;
    pub(crate) fn ref_init_tables();
    pub(crate) fn ref_lm_hidden_size(lm: *const Lm) -> c_int;
    pub(crate) fn ref_lm_steps(
        lm: *const Lm,
        tokens: *const c_int,
        h_prev: *const c_float,
        n: c_int,
        h: *mut c_float,
        logp: *mut c_float,
    );
    pub(crate) fn ref_logf(x: c_float) -> c_float;
    pub(crate) fn ref_nexp_tab(i: c_int) -> c_float;
    pub(crate) fn ref_nsig(x: c_float) -> c_float;
    pub(crate) fn ref_qvec(x: *const c_float, n: c_int, q: *mut i8, np: c_int) -> c_float;
    pub(crate) fn ref_token(model: *const Model, i: c_int, out: *mut c_char, cap: c_int) -> c_int;
    pub(crate) fn tasr_decoder_create(
        lm: *const Lm,
        v1: c_int,
        beam: c_int,
        topk: c_int,
        lm_weight: c_float,
        token_bonus: c_float,
    ) -> *mut Decoder;
    pub(crate) fn tasr_decoder_free(decoder: *mut Decoder);
    pub(crate) fn tasr_fast_expneg(u: c_float) -> c_float;
    pub(crate) fn tasr_fast_sigmoid(x: c_float) -> c_float;
    pub(crate) fn tasr_fast_tanh(x: c_float) -> c_float;
    pub(crate) fn tasr_lm_bonus(lm: *const Lm, fallback: c_float) -> c_float;
    pub(crate) fn tasr_lm_free(lm: *mut Lm);
    pub(crate) fn tasr_lm_load(blob: *const u8, size: usize) -> *mut Lm;
    pub(crate) fn tasr_lm_weight(lm: *const Lm, fallback: c_float) -> c_float;
    pub(crate) fn tasr_nemo_free(model: *mut Model);
    pub(crate) fn tasr_nemo_load(blob: *const u8, size: usize) -> *mut Model;
    pub(crate) fn tasr_nemo_transcribe(
        model: *const Model,
        pcm: *const i16,
        n: c_int,
        decoder: *mut Decoder,
        text: *mut c_char,
        maxlen: c_int,
        logit_sink: *mut c_float,
        max_frames: c_int,
        n_frames: *mut c_int,
    ) -> c_int;
    pub(crate) fn tasr_qlin_range(
        linear: *const QLinear<'_>,
        xq: *const i8,
        xs: *const c_float,
        rows: c_int,
        ldq: c_int,
        y: *mut c_float,
        ldy: c_int,
        wtmp: *mut i8,
        acc: *mut i32,
        n0: c_int,
        n1: c_int,
    );
    pub(crate) fn tasr_seg_feed(segment: *mut Segment, input: *const i16, k: c_int) -> c_int;
    pub(crate) fn tasr_seg_init(segment: *mut Segment, buf: *mut i16, cap: c_int);
    pub(crate) fn tasr_quant_rows(
        x: *const c_float,
        rows: c_int,
        k: c_int,
        ldx: c_int,
        xq: *mut i8,
        ldq: c_int,
        kp: c_int,
        xs: *mut c_float,
    );
}
