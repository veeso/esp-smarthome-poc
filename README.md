# ESP smart home PoC

[![CI](https://github.com/veeso/esp-smarthome-poc/actions/workflows/ci.yml/badge.svg)](https://github.com/veeso/esp-smarthome-poc/actions/workflows/ci.yml)
[![TruffleHog](https://github.com/veeso/esp-smarthome-poc/actions/workflows/trufflehog.yml/badge.svg)](https://github.com/veeso/esp-smarthome-poc/actions/workflows/trufflehog.yml)
[![zizmor](https://github.com/veeso/esp-smarthome-poc/actions/workflows/zizmor.yml/badge.svg)](https://github.com/veeso/esp-smarthome-poc/actions/workflows/zizmor.yml)
[![MIT license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Conventional Commits](https://img.shields.io/badge/Conventional%20Commits-1.0.0-%23FE5196?logo=conventionalcommits&logoColor=white)](https://www.conventionalcommits.org)

A voice-controlled lamp on the ESP32-S3, written in bare-metal Rust. In the
planned demo, you say the controller's wake name. A green LED comes on while
it listens for your command. Say “turn on the lamp” and the red lamp lights
up, followed by a spoken confirmation. The wake name is undecided.
The current firmware logs a greeting over USB. A separate
[microphone demo](#run-the-microphone-demo) transcribes speech on the board.

The workspace has these crates:

- `crates/esp-smarthome` is the hardware-independent `no_std` library. It
  builds for the ESP32-S3 and for Linux x86_64, where its tests run.
- `crates/esp-smarthome-poc` is the ESP32-S3 firmware.
- `crates/oido` is a `no_std` Rust port of the Oído speech-to-text engine,
  licensed GPL-3.0-only.
- `crates/mic-stt-demo` is the ESP32-S3 firmware of the microphone demo.

`crates/oido-reference` is a test-only C oracle. It builds the upstream engine
on your computer, and the tests of `oido` compare their results with it.

## Build the first lamp

The controller targets the **Espressif ESP32-S3-DevKitC-1-N32R16V** with
32 MiB flash and 16 MiB PSRAM. The goal is to transcribe free-form English
on the board, let esp-agent handle supported lamp requests, and speak the
result. Start with the [parts list](docs/hardware.md), then follow the
[wiring diagram and pin map](docs/wiring.md). The
[32 MiB speech budget](docs/speech.md) covers the on-device models.

- [Microphone capture][mic] feeds the wake-word detector while idle. After
  activation it supplies command audio to [speech recognition][stt].
- The [esp-agent catalog][catalog] names the lamp. A [conversation turn][turn]
  resolves the user's command and calls the [device interface][device].
- The [LED driver][led] switches the lamp. [Speech synthesis][tts] turns the
  outcome into a reply for [speaker playback][speaker].

See [Contributing](CONTRIBUTING.md) to work on a module.

## Install the tools

Install the Espressif toolchain, the flashing tool, and the standard Rust
toolchain used on regular computers:

```sh
cargo install espup espflash --locked
espup install --toolchain-version 1.97.0.0
rustup toolchain install 1.97 --component clippy,llvm-tools-preview
rustup toolchain install nightly --profile minimal --component rustfmt
```

On Unix, load the Espressif environment in every shell that builds the
firmware:

```sh
. $HOME/export-esp.sh
```

Local recipes also use these tools:

- [just](https://just.systems) for task execution.
- [cargo-nextest](https://nexte.st) for running tests.
- [dprint](https://dprint.dev) 0.56.1 for formatting.
- [cargo-deny](https://embarkstudios.github.io/cargo-deny/) for dependency
  policy.
- [TruffleHog](https://github.com/trufflesecurity/trufflehog) for secret
  scanning.
- [git-cliff](https://git-cliff.org) for changelog generation.
- [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) for coverage.
- [zizmor](https://docs.zizmor.sh) for GitHub Actions auditing.
- [shellcheck](https://www.shellcheck.net) for hook validation.
- A C compiler for this computer, such as `cc`, to build the
  `oido-reference` test crate.
- curl and shasum to download the speech models.

## Flash the board

Connect an ESP32-S3 over USB, then build, flash, and open the serial monitor:

```sh
just run
```

The firmware logs a greeting and then idles. Reopen the monitor without
flashing with `just monitor`. The log level defaults to `info` and is set by
`ESP_LOG` in `.cargo/esp-config.toml`.

## Run the microphone demo

The demo waits for the name “doufu” (like 豆腐), then transcribes your next
command on the board and prints it over USB. It runs on the
ESP32-S3-WROOM-1-N16R8 (16 MiB flash) and on the ESP32-S3-DevKitC-1-N32R16V
(32 MiB flash).

It uses only the microphone and the green LED of the
[wiring diagram](docs/wiring.md). Leave GPIO7, GPIO8 and the amplifier
unconnected.

| DevKit pin | Connect to                        |
| ---------- | --------------------------------- |
| 3V3        | INMP441 VDD                       |
| G (GND)    | INMP441 GND and L/R, LED cathode  |
| GPIO4      | INMP441 SCK                       |
| GPIO5      | INMP441 WS                        |
| GPIO6      | INMP441 SD                        |
| GPIO11     | 330 Ω resistor to green LED anode |

Download the speech models once, then flash the board you have and open the
serial monitor:

```sh
. $HOME/export-esp.sh
just download_oido_assets
just run_mic_demo n16r8
just run_mic_demo n32r16v
```

Then talk to it:

1. Wait for `state: AwaitWake`, and say “doufu”.
2. Wait for the green LED to come on, or for `listening for a command (10 s)`.
   The LED lights once the board has recognized “doufu” and the microphone
   records. Words said earlier are lost, because the demo does not record while
   it checks the wake word.
3. Say your command. It has to start within 10 s and may last up to 30 s.
4. The LED goes out when you stop speaking, and the transcript follows.

The board transcribes slowly, most likely several seconds of work for each
second of audio. Expect a wait after “doufu”, and a long one after a long
command. The `RTF` value in the log is the ratio of processing time to audio
time.

A session looks like this, with the sizes and timings left out:

```text
INFO - PSRAM: <bytes> bytes
INFO - mic peak over 1 s: <peak>
INFO - speech engine ready
INFO - transcribing 1.0 s of audio
INFO - transcribed 1.0 s of audio in <ms> ms (RTF <rtf>)
INFO - self-test transcript: "<text>"
INFO - mic-stt-demo ready
INFO - state: AwaitWake
INFO - state: ConfirmWake
INFO - transcribing <s> s of audio
INFO - transcribed <s> s of audio in <ms> ms (RTF <rtf>)
INFO - wake transcript: "do fo"
INFO - state: AwaitCommand
INFO - listening for a command (10 s)
INFO - state: HandleCommand
INFO - transcribing <s> s of audio
INFO - transcribed <s> s of audio in <ms> ms (RTF <rtf>)
INFO - command: turn on the lamp
INFO - state: AwaitWake
```

The speech model knows English only, so it spells “doufu” as English-like
words. With ten synthetic voices it most often wrote “do fo”, “do f”, “dof”,
“darf” or “dolf”, and the demo accepts these, “tofu”, and spellings within
one letter of “doufu”. It also often heard only “do” or “da”, which the demo
rejects so that ordinary speech does not wake it: if the LED stays off, say
the word again, more slowly.

Licensing differs from the rest of the repository:

- The demo links `oido`, which is GPL-3.0-only. The demo firmware as you
  flash or distribute it is therefore licensed under the GPL version 3.
- The speech models are licensed under CC BY-NC-SA 4.0 with Lokutor's terms
  of use, which exclude commercial use without a written license.

## Run common tasks

Every build and check for the chip embeds the speech models, and the tests
read them, so download them once before running any recipe below:

```sh
just download_oido_assets
```

Run `just` to list every recipe.

```sh
just build
just build_s3
just build_host
just release
just test
just coverage
just fmt
just fmt_check
just lint "-- -D warnings"
just doc
just deny
just scan_secrets
just check
```

Recipes with the suffix `_s3` run for the chip, and recipes with the suffix
`_host` run for your computer. `build`, `release`, `lint`, and `doc` run both.

`just check` is the local quality gate. It verifies formatting, runs Clippy
and builds documentation with warnings denied for both targets, checks the
dependency policy, and runs the tests.

`just test` runs the tests on your computer with cargo-nextest, followed by
the documentation tests of the libraries. The tests read the speech models and
audio clips, so run `just download_oido_assets` first. The first argument of
`just test` is passed to nextest; the optional second one is passed to the
documentation test command:

```sh
just test "-E 'test(greeting)'"
just test "--release" "--release"
```

`just coverage` writes an LCOV report of the tests to `lcov.info`.

## Enable the Git hooks

Install the tracked pre-commit hook with:

```sh
just setup_githooks
```

The hook scans staged files for secrets, checks formatting, runs Clippy, and
checks dependencies with cargo-deny.

## Update the changelog

Commits follow the
[Conventional Commits](https://www.conventionalcommits.org) specification.
Preview or generate release notes with:

```sh
just changelog_preview 0.1.0
just changelog 0.1.0
```

Review and commit `CHANGELOG.md` before publishing.

## Run continuous integration

GitHub Actions provides:

- An ESP32-S3 job that builds, lints, and documents the firmware and the
  libraries.
- A Linux x86_64 job that builds, tests, and lints the computer targets, and
  documents the libraries.
- Formatting and dependency-policy checks.
- TruffleHog secret scanning.
- zizmor workflow-security auditing.

CI does not run the firmware on hardware.

## License

This project is licensed under the [MIT License](LICENSE), except for
`crates/oido` and `crates/oido-reference`, which are licensed under the
[GNU GPL version 3](crates/oido/LICENSE).

[mic]: https://github.com/veeso/esp-smarthome-poc/issues/2
[speaker]: https://github.com/veeso/esp-smarthome-poc/issues/3
[stt]: https://github.com/veeso/esp-smarthome-poc/issues/4
[tts]: https://github.com/veeso/esp-smarthome-poc/issues/5
[device]: https://github.com/veeso/esp-smarthome-poc/issues/6
[led]: https://github.com/veeso/esp-smarthome-poc/issues/7
[catalog]: https://github.com/veeso/esp-smarthome-poc/issues/8
[turn]: https://github.com/veeso/esp-smarthome-poc/issues/9
