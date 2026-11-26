//! Hardware-independent logic of the ESP32 smart home proof of concept.
//!
//! The crate is `no_std` and depends on no HAL, so the same code runs in the
//! ESP32-S3 firmware and in tests on a regular computer. Chip setup, linker
//! scripts, and the heap belong to the firmware crate.

#![no_std]

/// Returns the greeting the firmware logs at startup.
///
/// # Examples
///
/// ```
/// assert_eq!(esp_smarthome::greeting(), "Hello from esp-smarthome-poc!");
/// ```
#[must_use]
pub const fn greeting() -> &'static str {
    "Hello from esp-smarthome-poc!"
}

#[cfg(test)]
mod tests {
    #[test]
    fn greeting_identifies_project() {
        assert_eq!(super::greeting(), "Hello from esp-smarthome-poc!");
    }
}
