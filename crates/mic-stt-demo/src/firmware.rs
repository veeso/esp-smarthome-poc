//! The demo on the chip.

// The entry point accepts no lint attributes of its own.
#![expect(
    clippy::large_stack_frames,
    reason = "the entry point sets up and owns the drivers, and the main stack holds \
    most of the internal RAM"
)]
#![expect(
    clippy::used_underscore_binding,
    reason = "the expansion of esp_rtos::main forwards the unused spawner"
)]

mod heap;
mod mic;
mod stt;

use alloc::boxed::Box;
use alloc::vec;

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer, with_timeout};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::timer::timg::TimerGroup;
use log::{info, warn};
use oido::{BLOCK_SAMPLES, PREROLL_SAMPLES, SegmentEvent, Segmenter};

use self::mic::{Mic, MicError};
use self::stt::Stt;
use crate::state::{COMMAND_MAX_MS, COMMAND_WAIT_MS, Effect, Event, Machine, State, WAKE_MAX_MS};

/// Blocks of the boot check of the microphone (1 s).
const CHECK_BLOCKS: usize = 50;

/// Samples per millisecond of 16 kHz audio.
const SAMPLES_PER_MS: usize = 16;

/// Samples of the longest wake clip, with the audio before the onset.
const WAKE_CAPACITY: usize = utterance_capacity(WAKE_MAX_MS);

/// Samples of the longest command, with the audio before the onset.
const COMMAND_CAPACITY: usize = utterance_capacity(COMMAND_MAX_MS);

/// Samples of the pre-roll and of `millis` of speech.
#[expect(
    clippy::cast_possible_truncation,
    reason = "both clip lengths are far below 2^32 milliseconds"
)]
const fn utterance_capacity(millis: u64) -> usize {
    PREROLL_SAMPLES + SAMPLES_PER_MS * millis as usize
}

/// Longest wait for a block, which arrives every 20 ms.
const READ_TIMEOUT_MS: u64 = 200;

/// Longest wait for the microphone to start, which takes about 300 ms.
const START_TIMEOUT_MS: u64 = 1_000;

/// Pause before the capture is retried after a failed start.
const RETRY_DELAY_MS: u64 = 500;

// The default app descriptor required by the ESP-IDF bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    let psram_bytes = heap::init(peripherals.PSRAM);
    info!("PSRAM: {psram_bytes} bytes");
    let timers = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timers.timer0, peripherals.FROM_CPU_INTR0);

    // The green LED, active high.
    let led = Output::new(peripherals.GPIO11, Level::Low, OutputConfig::default());
    let mut mic = Mic::new(
        peripherals.I2S0,
        peripherals.DMA_CH0,
        peripherals.GPIO4,
        peripherals.GPIO5,
        peripherals.GPIO6,
    )
    .expect("the I2S configuration should be valid");

    let mut clip = vec![0; CHECK_BLOCKS * BLOCK_SAMPLES];
    match record(&mut mic, &mut clip).await {
        Ok(()) => {
            let peak = clip.iter().map(|sample| sample.unsigned_abs()).max();
            info!("mic peak over 1 s: {peak}", peak = peak.unwrap_or(0));
        }
        Err(error) => warn!("microphone check failed: {error:?}"),
    }

    let mut stt = Stt::load();
    let text = stt.transcribe(&clip);
    info!("self-test transcript: {text:?}");
    drop(clip);

    // Large enough for a command, so it lands in PSRAM.
    let mut utterance = vec![0; COMMAND_CAPACITY];
    info!("mic-stt-demo ready");
    let demo = Demo {
        machine: Machine::new(now_ms()),
        segmenter: Segmenter::new(&mut utterance),
        mic,
        led,
        stt,
    };
    demo.run().await
}

/// The state machine and the hardware that it drives.
struct Demo<'b> {
    machine: Machine,
    segmenter: Segmenter<'b>,
    mic: Mic,
    led: Output<'static>,
    stt: Stt,
}

impl Demo<'_> {
    /// Feeds the machine with what the microphone hears and with the time.
    async fn run(mut self) -> ! {
        let state = self.machine.state();
        self.perform(Some(Effect::Enter(state))).await;

        let mut block: Box<[i16; BLOCK_SAMPLES]> = vec![0; BLOCK_SAMPLES]
            .into_boxed_slice()
            .try_into()
            .expect("the vector has the length of a block");
        loop {
            let read = with_timeout(
                Duration::from_millis(READ_TIMEOUT_MS),
                self.mic.read_block(&mut block),
            )
            .await;
            let event = match read {
                Ok(Ok(())) => match self.segmenter.feed(&block[..]) {
                    SegmentEvent::Idle => None,
                    SegmentEvent::SpeechStarted => Some(Event::SpeechStarted),
                    SegmentEvent::SpeechDropped => Some(Event::SpeechDropped),
                    SegmentEvent::UtteranceReady => Some(Event::UtteranceReady),
                },
                Ok(Err(error)) => {
                    warn!("capture failed: {error:?}");
                    Some(Event::CaptureFailed)
                }
                Err(_timeout) => {
                    warn!("capture timed out");
                    Some(Event::CaptureFailed)
                }
            };
            if let Some(event) = event {
                self.feed(event).await;
            }
            self.feed(Event::Tick).await;
        }
    }

    async fn feed(&mut self, event: Event<'_>) {
        let effect = self.machine.on_event(event, now_ms());
        self.perform(effect).await;
    }

    /// Carries out `effect` and every effect that follows from it.
    async fn perform(&mut self, mut effect: Option<Effect>) {
        while let Some(next) = effect {
            effect = match next {
                Effect::Enter(state) => self.enter(state).await,
                Effect::Transcribe => {
                    let text = self.transcribe_utterance();
                    info!("wake transcript: {text:?}");
                    self.machine.on_event(Event::Transcript(&text), now_ms())
                }
            };
        }
    }

    /// Sets the LED and runs the entry action of `state`.
    async fn enter(&mut self, state: State) -> Option<Effect> {
        // The LED of `AwaitCommand` lights only once the microphone records.
        if state != State::AwaitCommand {
            self.led.set_level(state.indicator_on().into());
        }
        info!("state: {state:?}");
        match state {
            State::AwaitWake => {
                self.segmenter.restart(WAKE_CAPACITY);
                self.start_capture().await
            }
            // The capture goes on with the wake clip.
            State::ConfirmWake => None,
            State::AwaitCommand => {
                self.segmenter.restart(COMMAND_CAPACITY);
                let failure = self.start_capture().await;
                if failure.is_some() {
                    return failure;
                }
                // The wait starts, and is said, once the microphone listens,
                // so that no word is lost.
                self.led.set_level(state.indicator_on().into());
                info!(
                    "listening for a command ({seconds} s)",
                    seconds = COMMAND_WAIT_MS / 1_000
                );
                self.machine.on_event(Event::CaptureStarted, now_ms())
            }
            State::HandleCommand => {
                let text = self.transcribe_utterance();
                if text.trim().is_empty() {
                    info!("command: no words recognized");
                } else {
                    info!("command: {text}");
                }
                self.machine.on_event(Event::Transcript(&text), now_ms())
            }
        }
    }

    /// Stops the capture and transcribes what it recorded.
    fn transcribe_utterance(&mut self) -> alloc::string::String {
        self.mic.stop();
        self.stt.transcribe(self.segmenter.utterance())
    }

    /// Starts a fresh capture; a failure is reported to the machine.
    async fn start_capture(&mut self) -> Option<Effect> {
        match with_timeout(Duration::from_millis(START_TIMEOUT_MS), self.mic.start()).await {
            Ok(Ok(())) => return None,
            Ok(Err(error)) => warn!("capture did not start: {error:?}"),
            Err(_timeout) => warn!("capture start timed out"),
        }
        Timer::after(Duration::from_millis(RETRY_DELAY_MS)).await;
        self.machine.on_event(Event::CaptureFailed, now_ms())
    }
}

/// Milliseconds since boot.
fn now_ms() -> u64 {
    Instant::now().as_millis()
}

/// Fills `clip`, a whole number of blocks long, with microphone audio.
async fn record(mic: &mut Mic, clip: &mut [i16]) -> Result<(), MicError> {
    mic.start().await?;
    let (blocks, _) = clip.as_chunks_mut::<BLOCK_SAMPLES>();
    for block in blocks {
        mic.read_block(block).await?;
    }
    mic.stop();
    Ok(())
}
