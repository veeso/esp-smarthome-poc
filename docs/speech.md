# Speech budget for the 32 MiB controller

The ESP32-S3-DevKitC-1-N32R16V has **32 MiB flash and 16 MiB PSRAM**.
For free-form English input, the esp-agent example with Potion (7.82 MiB),
Oído v2B models (6.89 MiB) and US English PicoTTS (1.53 MiB) total about
**16.24 MiB**. That exceeds the old 16 MiB budget and leaves **15.76 MiB**
on the selected board for wake detection (model and runtime), the STT engine,
remaining firmware and partitions. Wake detection has yet to be sized.
This estimates one application image. The integrated image size, peak RAM
use and microphone-to-reply latency still need measurements on the board.

## What goes into flash

- **Agent and controller baseline, about 7.82 MiB.** Upstream reports an
  example firmware image of about 8.2 decimal MB. It already includes
  `potion-base-2M` (7.64 MiB for weights and compiled vocabulary). Count
  the example once. The lamp catalog, turn handler and audio drivers will
  change the final binary. [esp-agent model and firmware sizes][agent]
- **Free-form English STT, 6.89 MiB of models.** Oído v2B's acoustic and
  language models total 7,220,276 bytes. Pass the transcript to
  `Agent::turn`, which can dispatch actions supported by the lamp catalog.
  Oído's timed board benchmarks used audio fed from flash. Its engine uses
  GPLv3 and its weights carry noncommercial terms. [Oído][oido] ·
  [Model files][oido-files]
- **English synthesis, about 1.53 MiB.** US English PicoTTS voice files are
  1,428,064 bytes combined. The port reports roughly 175 KB of code and
  streams 16 kHz, 16-bit audio. Embed one language. Espressif's own TTS
  supports Chinese and its published flash figure is 2.2 MB.
  [PicoTTS port][pico] · [US voice assets][voices] ·
  [Espressif TTS][tts] · [Espressif benchmark][bench]

## Wake name and listening indicator

While idle, the microphone feeds a local wake-word detector and the green
indicator stays off. Detecting the wake name opens one command capture window
and turns the green LED on. End of speech, silence timeout or a capture error
closes the window and turns the indicator off. After a spoken reply, the
controller resumes wake detection. Only a successful lamp command changes the
red LED.

The wake name is undecided. “Tofu” is one candidate. Espressif's WakeNet is
an ESP32-S3 option to evaluate with a supplied wake phrase first. A custom
name needs a matching wake-word model. Measure its flash size, idle RAM use,
missed wakes and false activations on the board. [WakeNet][wake]

## Smaller speech options

For a fixed-command demo, English MultiNet 5 Q8 recognizes registered
phrases such as “turn on the lamp”. Its three model files total 2,177,064 bytes
(2.08 MiB). Using it in place of Oído brings the subtotal to about 11.4 MiB.
This configuration requires a predefined vocabulary. [Model files][mn5] ·
[Recognition API][multinet]

A smaller playback milestone can use prerecorded English replies. Ten seconds
total at 16,000 samples/s × 2 bytes costs 320,000 bytes (0.31 MiB) plus
headers. Clips cover a fixed script. PicoTTS can say a newly supplied name or
sentence without recording another clip.

For comparison, the quantized Whisper tiny.en Q5_1 weights alone are 31 MiB.
For unrestricted dictation with that model, a companion computer can return
text while the S3 still owns the catalog, dispatch and LED. [Whisper models][whisper]

## What to measure on the board

1. Build a single release app with one agent model, one English recognizer
   and one voice. Use serial flashing for the PoC. Two full OTA app slots
   require a separate budget. Compare the flashable app and packed speech
   assets with their actual partition sizes, including bootloader space,
   alignment and any data partitions. A debug ELF's file size includes
   information that never reaches flash. [Partition layout][partitions]
2. Verify that the bootloader and model loader can read an asset placed
   above the 16 MiB boundary in octal flash. The Oído firmware and PicoTTS
   port use ESP-IDF services. Prototype the C bindings, allocation, task and
   flash-mapping support required by bare-metal Rust. [Flash addressing][flash] ·
   [Oído firmware][oido] · [PicoTTS port][pico]
3. Enable the board's PSRAM and measure peak internal RAM and PSRAM
   separately during capture, recognition, dispatch and playback. Include
   Oído's working buffers and weight cache, microphone buffers, stacks and
   agent scratch. PicoTTS reports just over 1.1 MB of RAM while initialized.
   The starter firmware configures only a 73,744-byte reclaimed heap.
   [Oído memory use][oido] · [PicoTTS memory use][pico]
4. Say the wake name before each lamp command, then try silence and an
   unsupported phrase. Record wake detection to green indicator, end of
   command to red lamp change, and lamp change to first reply sample. Repeat after
   playback and verify the device state and spoken result agree.

[agent]: https://github.com/veeso/esp-agent/tree/8db97faf294d961e213babf9cf8dd5116cb968a5#embedding-models
[mn5]: https://github.com/espressif/esp-sr/tree/bb552f1a559396a17681299cb3e199a0f1a4520a/model/multinet_model/mn5q8_en
[multinet]: https://docs.espressif.com/projects/esp-sr/en/latest/esp32s3/speech_command_recognition/README.html
[pico]: https://github.com/DiUS/esp-picotts/tree/4ff043750e937587194a5cfc7c8b1bfe494183ae
[voices]: https://github.com/DiUS/esp-picotts/tree/4ff043750e937587194a5cfc7c8b1bfe494183ae/pico/lang
[tts]: https://docs.espressif.com/projects/esp-sr/en/latest/esp32s3/speech_synthesis/readme.html
[bench]: https://docs.espressif.com/projects/esp-sr/en/latest/esp32s3/benchmark/README.html
[wake]: https://docs.espressif.com/projects/esp-sr/en/latest/esp32s3/wake_word_engine/README.html
[oido]: https://github.com/lokutor-ai/oido
[oido-files]: https://huggingface.co/lokutor-ai/oido-v2-realtime-int4/tree/main
[whisper]: https://huggingface.co/ggerganov/whisper.cpp
[partitions]: https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-guides/partition-tables.html
[flash]: https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-reference/peripherals/spi_flash/spi_flash_optional_feature.html
