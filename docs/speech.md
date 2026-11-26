# Can the voice controller fit in 16 MiB?

**A single-image English lamp controller looks feasible with a fixed command
list and synthesized replies.** MultiNet 5 Q8 and US English PicoTTS together
with the esp-agent example account for roughly 11.4 MiB. That leaves about
4.6 MiB for the speech runtime, controller changes, and partition overhead.
This is a sizing estimate. The current firmware only logs a greeting.

## What goes into flash

- **Agent and controller baseline, about 7.82 MiB.** Upstream reports an
  example firmware image of about 8.2 decimal MB. It already includes
  `potion-base-2M` (7.64 MiB for weights and compiled vocabulary). Count
  the example once. The lamp catalog, turn handler and audio drivers will
  change the final binary. [esp-agent model and firmware sizes][agent]
- **Command recognition, 2.08 MiB of assets.** The three files in
  `mn5q8_en` total 2,177,064 bytes. This excludes recognition code,
  model-container headers and any additional front-end models. MultiNet
  accepts 16 kHz, 16-bit mono audio from the front end. Register phrases
  such as “turn on the lamp” and “turn off the lamp”. Map a recognized ID
  to its canonical text for `Agent::turn`. A timeout produces no action.
  Vocabulary must be declared in advance. [Model files][mn5] ·
  [Recognition API][multinet]
- **English synthesis, about 1.53 MiB.** US English PicoTTS voice files are
  1,428,064 bytes combined. The port reports roughly 175 KB of code and
  streams 16 kHz, 16-bit audio. Embed one language. Espressif's own TTS
  supports Chinese and its published flash figure is 2.2 MB.
  [PicoTTS port][pico] · [US voice assets][voices] ·
  [Espressif TTS][tts] · [Espressif benchmark][bench]

A smaller playback milestone can use prerecorded English replies. Ten seconds
total at 16,000 samples/s × 2 bytes costs 320,000 bytes (0.31 MiB) plus
headers. Clips cover a fixed script. PicoTTS can say a newly supplied name or
sentence without recording another clip.

## If the user can say any sentence

**Oído v2B** is an ESP32-S3 open-vocabulary English recognizer. Its acoustic
and language models total 7,220,276 bytes (6.89 MiB). With the example agent
and English TTS, the subtotal already reaches about 16.24 MiB before the
recognizer and partitions. Upstream also supports greedy decoding without
the language model, a possible experiment with a smaller budget and different
accuracy. The project reports timed tests on audio fed from flash and identifies
live microphone integration as a separate concern. Its engine uses GPLv3 and
its weights carry noncommercial terms. [Oído][oido] · [Model files][oido-files]

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
2. Check linking and model loading first. ESP-SR and the PicoTTS port use
   ESP-IDF services, while this firmware uses bare-metal Rust. Prototype the
   required C bindings, allocation, task and flash-mapping support before
   committing to either engine. [ESP-SR component][sr] · [PicoTTS port][pico]
3. Enable the board's PSRAM and measure peak internal RAM and PSRAM
   separately during capture, recognition, dispatch and playback. Espressif
   reports 16 KB internal RAM and 2,310 KB PSRAM for MultiNet 5 Q8 alone.
   PicoTTS reports just over 1.1 MB of RAM while initialized. Add the audio
   front end, DMA buffers, stacks and agent scratch. The starter firmware
   configures only a 73,744-byte reclaimed heap. [Benchmarks][bench] ·
   [PicoTTS memory use][pico]
4. Speak both lamp commands, then try silence and an unsupported phrase.
   Record end-of-speech to LED and LED to first audio sample. Repeat after
   playback and verify the device state and spoken result agree.

[agent]: https://github.com/veeso/esp-agent/tree/8db97faf294d961e213babf9cf8dd5116cb968a5#embedding-models
[mn5]: https://github.com/espressif/esp-sr/tree/bb552f1a559396a17681299cb3e199a0f1a4520a/model/multinet_model/mn5q8_en
[multinet]: https://docs.espressif.com/projects/esp-sr/en/latest/esp32s3/speech_command_recognition/README.html
[pico]: https://github.com/DiUS/esp-picotts/tree/4ff043750e937587194a5cfc7c8b1bfe494183ae
[voices]: https://github.com/DiUS/esp-picotts/tree/4ff043750e937587194a5cfc7c8b1bfe494183ae/pico/lang
[tts]: https://docs.espressif.com/projects/esp-sr/en/latest/esp32s3/speech_synthesis/readme.html
[bench]: https://docs.espressif.com/projects/esp-sr/en/latest/esp32s3/benchmark/README.html
[oido]: https://github.com/lokutor-ai/oido
[oido-files]: https://huggingface.co/lokutor-ai/oido-v2-realtime-int4/tree/main
[whisper]: https://huggingface.co/ggerganov/whisper.cpp
[partitions]: https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-guides/partition-tables.html
[sr]: https://github.com/espressif/esp-sr
