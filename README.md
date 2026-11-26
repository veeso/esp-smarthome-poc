# ESP smart home PoC

[![CI](https://github.com/veeso/esp-smarthome-poc/actions/workflows/ci.yml/badge.svg)](https://github.com/veeso/esp-smarthome-poc/actions/workflows/ci.yml)
[![TruffleHog](https://github.com/veeso/esp-smarthome-poc/actions/workflows/trufflehog.yml/badge.svg)](https://github.com/veeso/esp-smarthome-poc/actions/workflows/trufflehog.yml)
[![zizmor](https://github.com/veeso/esp-smarthome-poc/actions/workflows/zizmor.yml/badge.svg)](https://github.com/veeso/esp-smarthome-poc/actions/workflows/zizmor.yml)
[![MIT license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Conventional Commits](https://img.shields.io/badge/Conventional%20Commits-1.0.0-%23FE5196?logo=conventionalcommits&logoColor=white)](https://www.conventionalcommits.org)

A voice-controlled lamp on the ESP32-S3, written in bare-metal Rust. The
first demo will let someone say “turn on the lamp”, see an LED light up and
hear the controller confirm the result. The current firmware logs a greeting
over USB. The voice and device modules are tracked below.

The workspace has two crates:

- `crates/esp-smarthome` is the hardware-independent `no_std` library. It
  builds for the ESP32-S3 and for Linux x86_64, where its tests run.
- `crates/esp-smarthome-poc` is the ESP32-S3 firmware.

## Build the first lamp

The controller targets the **Espressif ESP32-S3-DevKitC-1-N32R16V** with
32 MiB flash and 16 MiB PSRAM. The goal is to transcribe free-form English
on the board, let esp-agent handle supported lamp requests, and speak the
result. Start with the [parts list](docs/hardware.md), then follow the
[wiring diagram and pin map](docs/wiring.md). The
[32 MiB speech budget](docs/speech.md) covers the on-device models.

- [Microphone capture][mic] supplies audio to [speech recognition][stt].
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

## Flash the board

Connect an ESP32-S3 over USB, then build, flash, and open the serial monitor:

```sh
just run
```

The firmware logs a greeting and then idles. Reopen the monitor without
flashing with `just monitor`. The log level defaults to `info` and is set by
`ESP_LOG` in `.cargo/esp-config.toml`.

## Run common tasks

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

`just test` runs the library tests on your computer with cargo-nextest,
followed by documentation tests. Its first argument is passed to nextest; its
optional second argument is passed to the documentation test command:

```sh
just test "-E 'test(greeting)'"
just test "--release" "--release"
```

`just coverage` writes an LCOV report of the library tests to `lcov.info`.

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

- An ESP32-S3 job that builds, lints, and documents the firmware and library.
- A Linux x86_64 job that builds, tests, lints, and documents the library.
- Formatting and dependency-policy checks.
- TruffleHog secret scanning.
- zizmor workflow-security auditing.

CI does not run the firmware on hardware.

## License

This project is licensed under the [MIT License](LICENSE).

[mic]: https://github.com/veeso/esp-smarthome-poc/issues/2
[speaker]: https://github.com/veeso/esp-smarthome-poc/issues/3
[stt]: https://github.com/veeso/esp-smarthome-poc/issues/4
[tts]: https://github.com/veeso/esp-smarthome-poc/issues/5
[device]: https://github.com/veeso/esp-smarthome-poc/issues/6
[led]: https://github.com/veeso/esp-smarthome-poc/issues/7
[catalog]: https://github.com/veeso/esp-smarthome-poc/issues/8
[turn]: https://github.com/veeso/esp-smarthome-poc/issues/9
