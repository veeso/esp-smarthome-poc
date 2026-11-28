//! The utterance segmenter of the C engine.

use std::ptr;

use crate::{ffi, to_c_int};

/// A `tasr_seg_t` with its utterance buffer.
#[derive(Debug)]
pub struct RefSegmenter {
    state: ffi::Segment,
    buffer: Vec<i16>,
}

impl RefSegmenter {
    /// `tasr_seg_init` over a buffer of `capacity` samples.
    ///
    /// # Panics
    ///
    /// Panics when `capacity` is shorter than the pre-roll of 3200 samples.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        assert!(
            capacity >= 3_200,
            "the C segmenter needs room for its pre-roll"
        );
        let mut segmenter = Self {
            state: ffi::Segment {
                gain: 0.0,
                noise_floor: 0.0,
                buf: ptr::null_mut(),
                cap: 0,
                n: 0,
                speech: 0,
                silence: 0,
                voiced: 0,
                init: 0,
                hang: 0,
            },
            buffer: vec![0; capacity],
        };
        segmenter.init();
        segmenter
    }

    /// `tasr_seg_init` again: a full reset.
    pub fn init(&mut self) {
        let capacity = to_c_int(self.buffer.len());
        // SAFETY: the buffer holds `capacity` samples and lives as long as the state.
        unsafe { ffi::tasr_seg_init(&raw mut self.state, self.buffer.as_mut_ptr(), capacity) }
    }

    /// `tasr_seg_feed`: the length of a complete utterance, or 0.
    ///
    /// # Panics
    ///
    /// Panics when the engine returns a negative length.
    pub fn feed(&mut self, block: &[i16]) -> usize {
        // SAFETY: the state is initialized and `block` holds `block.len()` samples.
        let len = unsafe {
            ffi::tasr_seg_feed(&raw mut self.state, block.as_ptr(), to_c_int(block.len()))
        };
        usize::try_from(len).expect("the length is not negative")
    }

    /// The samples that the buffer holds.
    ///
    /// # Panics
    ///
    /// Panics when the engine holds a negative length.
    #[must_use]
    pub fn samples(&self) -> &[i16] {
        &self.buffer[..usize::try_from(self.state.n).expect("the length is not negative")]
    }
}
