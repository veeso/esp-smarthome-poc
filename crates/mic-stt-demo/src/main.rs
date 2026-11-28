//! ESP32-S3 demo that wakes on “doufu” and prints the next spoken command.
//!
//! The states:
//!
//! - `AwaitWake`: the green LED is off and the microphone listens for speech.
//! - `ConfirmWake`: the LED stays off while the first utterance is checked for
//!   “doufu”.
//! - `AwaitCommand`: the LED is on. A command must start within 10 s of the
//!   microphone listening and may last up to 30 s.
//! - `HandleCommand`: the command is transcribed and printed.
//!
//! Flash it with `just run_mic_demo <board>`.
//!
//! The firmware links the `oido` crate, so the flashed binary as distributed is
//! licensed under the GNU GPL version 3.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![expect(
    clippy::multiple_crate_versions,
    reason = "the ESP crates depend on several versions of shared crates"
)]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

#[cfg(target_os = "none")]
extern crate alloc;

#[cfg(target_os = "none")]
mod firmware;
#[cfg_attr(
    all(not(target_os = "none"), not(test)),
    expect(
        dead_code,
        reason = "runs on the chip; the computer build only tests it"
    )
)]
mod pcm;
#[cfg_attr(
    all(not(target_os = "none"), not(test)),
    expect(
        dead_code,
        reason = "runs on the chip; the computer build only tests it"
    )
)]
mod state;
#[cfg_attr(
    all(not(target_os = "none"), not(test)),
    expect(
        dead_code,
        reason = "runs on the chip; the computer build only tests it"
    )
)]
mod wake;

#[cfg(not(target_os = "none"))]
fn main() {
    eprintln!("mic-stt-demo runs on the ESP32-S3; flash it with just run_mic_demo <board>");
}
