//! The internal and external heaps.

use esp_alloc::{HeapRegion, MemoryCapability};
use esp_hal::peripherals::PSRAM;
use esp_hal::psram::{Psram, PsramConfig, PsramMode};

/// Bytes of the second-stage bootloader RAM that are handed to the heap.
const HEAP_SIZE: usize = 73_744;

/// Sets up the internal heap and adds the PSRAM to it.
///
/// Returns the bytes of PSRAM, which is 0 when none was found. Must be called
/// once.
pub fn init(psram: PSRAM<'static>) -> usize {
    // The internal region comes first, so that it is preferred.
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);

    // Both supported boards have octal PSRAM.
    let config = PsramConfig {
        mode: PsramMode::OctalSpi,
        ..PsramConfig::default()
    };
    let (start, size) = Psram::new(psram, config).raw_parts();
    if size > 0 {
        let region = MemoryCapability::External.into();
        // SAFETY: the range stays mapped for the rest of the program, it is not
        // empty, and nothing but the heap uses it.
        unsafe { esp_alloc::HEAP.add_region(HeapRegion::new(start, size, region)) };
    }
    size
}
