//! ESP32-S3 firmware of the smart home proof of concept.

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::main;
use log::info;

/// Bytes of the second-stage bootloader RAM that are handed to the heap.
const HEAP_SIZE: usize = 73_744;

/// Pause of the idle loop.
const IDLE_INTERVAL_MS: u32 = 1_000;

// The default app descriptor required by the ESP-IDF bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let _peripherals = esp_hal::init(config);
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);

    info!("{greeting}", greeting = esp_smarthome::greeting());

    let delay = Delay::new();
    loop {
        delay.delay_millis(IDLE_INTERVAL_MS);
    }
}
