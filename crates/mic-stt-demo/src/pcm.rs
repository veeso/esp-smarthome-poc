//! Converts the I²S frames of the microphone to 16-bit PCM.

/// Bytes of one stereo frame: two 32-bit slots, left then right.
pub const FRAME_BYTES: usize = 8;

/// Writes the top 16 bits of each left word of `raw` into `out`.
///
/// The words are little-endian. Returns the samples written, which is the
/// number of whole frames of `raw` or the length of `out`, whichever is less.
pub fn left_slot_to_pcm16(raw: &[u8], out: &mut [i16]) -> usize {
    let mut written = 0;
    for (sample, frame) in out.iter_mut().zip(raw.chunks_exact(FRAME_BYTES)) {
        // The upper half of the little-endian left word.
        *sample = i16::from_le_bytes([frame[2], frame[3]]);
        written += 1;
    }
    written
}

/// A stream of bytes that is handed out in contiguous parts, like a DMA buffer.
pub trait Chunks {
    /// The next contiguous part, empty when there is no data.
    fn peek(&mut self) -> &[u8];

    /// Drops the first `bytes` bytes.
    fn consume(&mut self, bytes: usize);
}

/// Fills `out` from `source`, which may split a frame between two parts.
///
/// Returns `false` when `source` runs dry.
pub fn read_frames(source: &mut impl Chunks, out: &mut [i16]) -> bool {
    let mut filled = 0;
    while filled < out.len() {
        let whole = left_slot_to_pcm16(source.peek(), &mut out[filled..]);
        if whole > 0 {
            source.consume(whole * FRAME_BYTES);
            filled += whole;
            continue;
        }
        // The part ends inside a frame: gather the bytes of that frame.
        let mut frame = [0; FRAME_BYTES];
        let mut have = 0;
        while have < FRAME_BYTES {
            let part = source.peek();
            let take = part.len().min(FRAME_BYTES - have);
            if take == 0 {
                return false;
            }
            frame[have..have + take].copy_from_slice(&part[..take]);
            source.consume(take);
            have += take;
        }
        filled += left_slot_to_pcm16(&frame, &mut out[filled..]);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(left: u32, right: u32) -> [u8; 8] {
        let mut f = [0u8; 8];
        f[..4].copy_from_slice(&left.to_le_bytes());
        f[4..].copy_from_slice(&right.to_le_bytes());
        f
    }

    #[test]
    fn should_take_the_top_16_bits_of_the_left_slot() {
        let raw: Vec<u8> = [
            frame(0x7FFF_FF00, 0x1234_5678),
            frame(0x8000_0000, 0),
            frame(0x0001_0000, 0),
            frame(0xFFFF_0000, 0x7FFF_0000),
            frame(0x0000_FF00, 0),
        ]
        .concat();
        let mut out = [0i16; 5];
        assert_eq!(left_slot_to_pcm16(&raw, &mut out), 5);
        assert_eq!(out, [32_767, -32_768, 1, -1, 0]);
    }

    #[test]
    fn should_ignore_a_trailing_partial_frame() {
        let mut raw = frame(0x0002_0000, 0).to_vec();
        raw.extend_from_slice(&[0xFF; 7]);
        let mut out = [0i16; 4];
        assert_eq!(left_slot_to_pcm16(&raw, &mut out), 1);
        assert_eq!(out[0], 2);
    }

    #[test]
    fn should_stop_when_the_output_is_full() {
        let raw = [
            frame(0x0001_0000, 0),
            frame(0x0002_0000, 0),
            frame(0x0003_0000, 0),
        ]
        .concat();
        let mut out = [0i16; 2];
        assert_eq!(left_slot_to_pcm16(&raw, &mut out), 2);
        assert_eq!(out, [1, 2]);
    }

    #[test]
    fn should_write_nothing_for_empty_input() {
        let mut out = [7i16; 2];
        assert_eq!(left_slot_to_pcm16(&[], &mut out), 0);
        assert_eq!(out, [7, 7]);
    }

    /// Hands out `bytes` in parts of `lengths`, like DMA descriptors that close
    /// before they are full.
    struct Parts<'a> {
        bytes: &'a [u8],
        lengths: Vec<usize>,
    }

    impl Chunks for Parts<'_> {
        fn peek(&mut self) -> &[u8] {
            &self.bytes[..self.lengths.first().copied().unwrap_or(0)]
        }

        fn consume(&mut self, bytes: usize) {
            self.bytes = &self.bytes[bytes..];
            self.lengths[0] -= bytes;
            if self.lengths[0] == 0 {
                self.lengths.remove(0);
            }
        }
    }

    #[test]
    fn should_read_frames_that_are_split_between_parts() {
        let raw: Vec<u8> = (1..=6u32)
            .flat_map(|n| frame(n << 16, 0xAAAA_AAAA))
            .collect();
        // The second frame spans three parts and the fifth spans two.
        let mut source = Parts {
            bytes: &raw,
            lengths: vec![12, 2, 2, 17, 15],
        };
        let mut out = [0i16; 6];
        assert!(read_frames(&mut source, &mut out));
        assert_eq!(out, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn should_report_a_source_that_runs_dry() {
        let raw = frame(0x0001_0000, 0);
        let mut source = Parts {
            bytes: &raw,
            lengths: vec![8],
        };
        assert!(!read_frames(&mut source, &mut [0i16; 2]));
    }
}
