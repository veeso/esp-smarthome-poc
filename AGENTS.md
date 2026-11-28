# AGENTS.md

Guidance for coding agents working in this repository.

## Project context

This repository is a smart home proof of concept for the ESP32-S3, written in
bare-metal Rust. It is a virtual Cargo workspace: the hardware-independent
`no_std` library lives in `crates/esp-smarthome`, and the firmware lives in
`crates/esp-smarthome-poc`. The `no_std` Oído speech-to-text port lives in
`crates/oido`, its C test oracle in `crates/oido-reference`, and the “doufu”
microphone demo firmware in `crates/mic-stt-demo`. The setup follows
`veeso/esp-agent`.

The toolchain is the espup `esp` toolchain 1.97.0.0 with edition 2024. The
library also builds and tests on regular computers with the standard Rust
toolchain 1.97. Keep these synchronized:

- `rust-toolchain.toml`
- `workspace.package.rust-version` in `Cargo.toml`
- `host_toolchain` in `just/build.just`
- the `version:` input of `esp-rs/xtensa-toolchain` and every
  `RUSTUP_TOOLCHAIN` in `.github/workflows/`

## Working rules

- Read the relevant source, documentation, configuration, and tests before
  making changes.
- Keep changes scoped to the requested task and preserve unrelated user work.
- Use existing project patterns before adding new abstractions.
- Add or update focused tests before changing behavior.
- Update user documentation when commands, public APIs, configuration, or
  workflows change.
- Use `git`; never use `jj` or git worktrees.
- Never open a GitHub issue without prior user approval.
- Do not stage or commit local plans. Planning state belongs under the ignored
  `docs/superpowers/`, `.superpowers/`, or `.claude/plans/` directories.

## Source layout

- `crates/esp-smarthome/src/lib.rs` contains the library API and its focused
  unit tests. The library depends on no HAL, enables no chip features, and must
  build for both the ESP32-S3 and Linux x86_64.
- `crates/esp-smarthome-poc/src/main.rs` is the firmware entry point, and its
  `build.rs` configures the linker.
- `crates/oido/src/lib.rs` is the entry point of the `no_std` Rust port of the
  Oído engine: model parsing, the segmenter, and the transcriber. It must build
  for both the ESP32-S3 and the computer.
- `crates/oido-reference` builds the unmodified upstream C engine from
  `vendor/` through `cc`. It is a test-only oracle for the computer and is
  empty on the chip.
- `crates/mic-stt-demo/src/main.rs` is the microphone demo entry point.
  Hardware-independent logic lives in `state.rs`, `wake.rs`, and `pcm.rs`,
  and the chip code under `firmware/`. Its `build.rs` requires the Oído models.
- The Oído models and test clips are downloaded into the ignored `models/oido/`
  directory. Every chip build and the tests need them.
- `.cargo/config.toml` sets the default target, runner, and build flags.
  `.cargo/esp-config.toml` holds user and runtime settings such as `ESP_LOG`.
- Put logic that does not touch hardware in the library, so it can be tested.
- Use `module_name.rs`; never introduce `mod.rs`.

## ESP notes

- Load `$HOME/export-esp.sh` before building the firmware.
- Avoid `std`; use `core` and embedded-friendly crates.
- `alloc` is enabled. If allocations fail, adjust `HEAP_SIZE` in the firmware.
- `just run` flashes the board and opens the monitor.
- `just run_mic_demo <board>` flashes the microphone demo and opens the
  monitor; `<board>` is `n16r8` or `n32r16v`.
- Run `just download_oido_assets` once before building or checking.

## Rust conventions

- Follow `rustfmt.toml` and format Rust through `just fmt`.
- Clippy must pass with warnings denied.
- Public library modules and items require canonical rustdoc documentation.
- Avoid `unsafe` unless the task requires it and its safety invariants are
  documented and tested.
- Use named format placeholders instead of positional `{}` arguments.
- Prefer `#[expect]` with a reason over `#[allow]` for local lint overrides.
- Keep dependency entries and feature definitions alphabetically sorted.
- Use bare, minimal dependency versions in `Cargo.toml`.

## Command interface

Use [`just`](https://just.systems) recipes when one exists. Do not bypass a
recipe with an ad hoc `cargo` or tool command. If a recurring task has no
recipe, add a focused recipe before using it.

Run `just` to list every command. The primary recipes are:

```sh
just download_oido_assets
just build
just build_s3
just build_host
just release
just run
just run_mic_demo n16r8
just monitor
just test
just coverage
just fmt
just fmt_check
just lint "-- -D warnings"
just doc
just deny
just scan_secrets
just check
just setup_githooks
```

Recipes with the suffix `_s3` run for the chip, and recipes with the suffix
`_host` run for the computer. The unsuffixed recipe runs both.

`just check` is the required local quality gate. It runs formatting checks,
Clippy and rustdoc with warnings denied for both targets, cargo-deny, and the
test suite.

When invoking compilation or test commands from the CLI, never request
parallelism greater than eight. This is an invocation constraint; do not encode
the local cap in tracked project files.

## Required tools

The local recipes expect:

- espup with the `esp` toolchain 1.97.0.0, and rustup.
- The standard Rust toolchain 1.97 with `clippy` and `llvm-tools-preview`.
- espflash.
- just.
- A C compiler for the computer, used by `oido-reference` through `cc`.
- curl and shasum for `just download_oido_assets`.
- cargo-nextest for running tests.
- dprint 0.56.1 and nightly rustfmt.
- cargo-deny.
- TruffleHog.
- git-cliff.
- cargo-llvm-cov for coverage.
- zizmor for workflow changes.
- shellcheck for shell hook changes.

If a required tool is unavailable, report it explicitly. Do not claim its check
passed or silently replace the repository command with a weaker check.

## Documentation

- Write task-oriented documentation with runnable examples.
- Use ATX headings and fenced code blocks with language identifiers.
- Keep Markdown lines readable, tables aligned, and files terminated by one
  newline.
- Run `fmt-md-tables -i <file>` after editing a Markdown file that contains a
  table.

## GitHub Actions

- Run `zizmor .github/workflows` after every workflow change until it exits
  successfully with no findings.
- Pin every external action to the full commit SHA of its latest stable release
  and record the exact matching tag in a trailing comment.
- Declare explicit least-privilege permissions.
- Set `persist-credentials: false` on checkout steps unless later authenticated
  Git operations are explicitly required.
- Never interpolate attacker-controlled GitHub expressions directly into a
  shell script. Pass values through `env` and quote the shell variable.

## Git and releases

- Use Conventional Commits with an imperative, lower-case description.
- Do not add agent attribution, session links, or agent `Co-Authored-By` lines.
- Inspect the diff before staging or committing changes.
- Generate release notes with `just changelog_preview <version>` and
  `just changelog <version>`.
- Verify packages locally with `just publish "--dry-run --allow-dirty"`.
- Live publication uses crates.io trusted publishing through
  `.github/workflows/publish.yml`.
