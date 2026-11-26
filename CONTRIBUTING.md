# Contributing

Start with the [toolchain setup](README.md#install-the-tools) and read the
[AI policy](AI_POLICY.md) before submitting a contribution.

## Work on a module

Keep device capabilities, catalog lookup and turn handling in the
`esp-smarthome` library so they can be exercised on a computer. Keep GPIO,
I²S, memory setup and board wiring in the `esp-smarthome-poc` firmware.

Add a focused host test for a behavior change, document public Rust items
with rustdoc, and run the library tests from the workspace root.

```sh
just test
```

## Verify a change

Load the ESP environment before running the full quality gate.

```sh
. "$HOME/export-esp.sh"
just fmt
just check
```

For a hardware change, include the board variant, wiring and steps used to
test it. Use the [wiring diagram](docs/wiring.md) as the reference pin map.
Use `just run` with the board connected to flash the release firmware and
open the serial monitor.

## Submit a pull request

Use a Conventional Commit title, describe the user-visible result, link the
relevant issue and include the checks you ran. Fill in the AI disclosure in
the pull request template.
