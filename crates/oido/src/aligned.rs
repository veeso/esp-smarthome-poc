//! Storage for model files that the parsers read in place.

/// A value stored at a 16-byte boundary.
///
/// The model parsers read `f32` tables straight from the file bytes, so the
/// bytes must be aligned. [`include_aligned_bytes!`](crate::include_aligned_bytes)
/// builds an aligned copy of a file at compile time.
///
/// # Examples
///
/// ```
/// let aligned = oido::Aligned([0u8; 4]);
/// assert_eq!(aligned.0.as_ptr().addr() % 16, 0);
/// ```
#[derive(Debug)]
#[repr(C, align(16))]
pub struct Aligned<B: ?Sized>(pub B);

/// Embeds a file in the program at a 16-byte boundary.
///
/// Expands to a `&'static Aligned<[u8]>`. The path is resolved like the path of
/// [`include_bytes!`].
#[macro_export]
macro_rules! include_aligned_bytes {
    ($path:expr $(,)?) => {{
        const ALIGNED: &$crate::Aligned<[u8]> = &$crate::Aligned(*::core::include_bytes!($path));
        ALIGNED
    }};
}

#[cfg(test)]
mod tests {
    #[test]
    fn should_embed_a_file_at_a_16_byte_boundary() {
        let embedded = crate::include_aligned_bytes!("aligned.rs");
        assert_eq!(embedded.0.as_ptr().addr() % 16, 0);
        assert_eq!(&embedded.0, include_bytes!("aligned.rs"));
    }
}
