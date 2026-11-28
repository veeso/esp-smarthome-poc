//! The INMP441 microphone over I²S.

use esp_hal::dma::{DmaRxStreamBuf, DmaRxStreamBufView};
use esp_hal::i2s::master::{Channels, DataFormat, I2s, I2sRx, I2sRxDmaTransfer, TdmConfig};
use esp_hal::peripherals::{DMA_CH0, GPIO4, GPIO5, GPIO6, I2S0};
use esp_hal::time::Rate;
use esp_hal::{Async, dma_rx_stream_buffer};
use oido::BLOCK_SAMPLES;

use crate::pcm::{Chunks, FRAME_BYTES, read_frames};

/// Bytes of the DMA buffer, which holds 250 ms of audio.
const DMA_BYTES: usize = 32_000;

/// Bytes that one DMA descriptor holds at most.
///
/// The I²S also closes descriptors early, in the middle of a frame.
const DMA_CHUNK_BYTES: usize = 4_000;

/// Bytes of one block of frames (20 ms).
const BLOCK_BYTES: usize = BLOCK_SAMPLES * FRAME_BYTES;

/// Blocks dropped after a start.
///
/// The INMP441 sends zeros for 2^18 bit clocks (256 ms) after its clock starts.
/// Passed on, they would set the noise floor of the segmenter to silence and
/// make the room noise look like speech.
const SETTLE_BLOCKS: usize = 15;

/// Why the microphone delivers no audio.
#[derive(Debug)]
pub enum MicError {
    /// The I²S peripheral rejected its configuration.
    Config,
    /// The DMA transfer failed or is not running.
    Dma,
    /// The audio was not read in time and the DMA stopped.
    Overrun,
}

/// A running capture.
type Transfer = I2sRxDmaTransfer<'static, Async, DmaRxStreamBuf>;

impl Chunks for DmaRxStreamBufView {
    fn peek(&mut self) -> &[u8] {
        DmaRxStreamBufView::peek(self)
    }

    fn consume(&mut self, bytes: usize) {
        DmaRxStreamBufView::consume(self, bytes);
    }
}

/// The receiver, idle or capturing.
enum Capture {
    Stopped(I2sRx<'static, Async>, DmaRxStreamBuf),
    Running(Transfer),
}

/// The left-slot microphone on GPIO4 (BCLK), GPIO5 (WS), and GPIO6 (SD).
pub struct Mic {
    /// Empty only while the driver changes hands.
    capture: Option<Capture>,
}

impl core::fmt::Debug for Mic {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mic").finish_non_exhaustive()
    }
}

impl Mic {
    /// Sets up 16 kHz reception of two 32-bit Philips slots.
    ///
    /// Must be called once, because the DMA buffer is static.
    pub fn new(
        i2s: I2S0<'static>,
        dma: DMA_CH0<'static>,
        bclk: GPIO4<'static>,
        ws: GPIO5<'static>,
        din: GPIO6<'static>,
    ) -> Result<Self, MicError> {
        let config = TdmConfig::new_tdm_philips()
            .with_sample_rate(Rate::from_hz(oido::SAMPLE_RATE_HZ))
            .with_data_format(DataFormat::Data32Channel32)
            .with_channels(Channels::STEREO);
        let i2s = I2s::new(i2s, dma, config)
            .map_err(|_error| MicError::Config)?
            .into_async();
        let rx = i2s.i2s_rx.with_bclk(bclk).with_ws(ws).with_din(din).build();
        let buffer = dma_rx_stream_buffer!(DMA_BYTES, DMA_CHUNK_BYTES);
        Ok(Self {
            capture: Some(Capture::Stopped(rx, buffer)),
        })
    }

    /// Restarts the capture, which discards the audio that was buffered.
    ///
    /// Returns once the microphone has started up, after about 300 ms.
    pub async fn start(&mut self) -> Result<(), MicError> {
        self.stop();
        let Some(Capture::Stopped(rx, buffer)) = self.capture.take() else {
            return Err(MicError::Dma);
        };
        match rx.read(buffer) {
            Ok(transfer) => self.capture = Some(Capture::Running(transfer)),
            Err((_error, rx, buffer)) => {
                self.capture = Some(Capture::Stopped(rx, buffer));
                return Err(MicError::Dma);
            }
        }
        for _ in 0..SETTLE_BLOCKS {
            self.wait_for_block().await?.consume(BLOCK_BYTES);
        }
        Ok(())
    }

    /// Stops the capture, if it runs.
    pub fn stop(&mut self) {
        self.capture = self.capture.take().map(|capture| match capture {
            Capture::Running(transfer) => {
                let (rx, buffer) = transfer.stop();
                Capture::Stopped(rx, buffer)
            }
            stopped @ Capture::Stopped(..) => stopped,
        });
    }

    /// Waits for the next 20 ms of audio.
    pub async fn read_block(&mut self, out: &mut [i16; BLOCK_SAMPLES]) -> Result<(), MicError> {
        let transfer = self.wait_for_block().await?;
        if read_frames(&mut **transfer, out) {
            Ok(())
        } else {
            Err(MicError::Dma)
        }
    }

    /// Waits until the running transfer holds a block of frames.
    async fn wait_for_block(&mut self) -> Result<&mut Transfer, MicError> {
        let Some(Capture::Running(transfer)) = &mut self.capture else {
            return Err(MicError::Dma);
        };
        loop {
            let available = transfer.available_bytes();
            // With data in every descriptor, the DMA has nowhere left to write.
            if available > DMA_BYTES - DMA_CHUNK_BYTES {
                return Err(MicError::Overrun);
            }
            if available >= BLOCK_BYTES {
                return Ok(transfer);
            }
            if transfer.wait_for_available_async().await.is_err() {
                // The driver reports every failure alike.
                let full = transfer.available_bytes() > DMA_BYTES - DMA_CHUNK_BYTES;
                return Err(if full {
                    MicError::Overrun
                } else {
                    MicError::Dma
                });
            }
        }
    }
}
