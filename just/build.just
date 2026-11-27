# Standard Rust toolchain for regular computers; keep it equal to `workspace.package.rust-version`.
host_toolchain := "1.97"

# Cargo toolchain override for regular computers, which bypasses the ESP toolchain in rust-toolchain.toml.
host_cargo := "cargo +" + host_toolchain

# Firmware package, which builds only for the ESP32-S3.
firmware := "esp-smarthome-poc"

# Packages that build and test on regular computers.
host_packages := "--package esp-smarthome --package mic-stt-demo --package oido --package oido-reference"

# Microphone speech-to-text demo firmware.
mic_demo := "mic-stt-demo"

# Library packages, whose documentation and doctests run on regular computers.
host_libraries := "--package esp-smarthome --package oido"

# Folder of the downloaded Oído models and test clips.
oido_assets_dir := "models/oido"

# Pinned Hugging Face revision of the Oído v2B models.
oido_model_url := "https://huggingface.co/lokutor-ai/oido-v2-realtime-int4/resolve/7cd4c5a5e942e691a572000b1952386c9fad5c80"

# Pinned upstream commit of the Oído test clips.
oido_clip_url := "https://raw.githubusercontent.com/lokutor-ai/oido/c8d60325ba8bffd4bad65a9ac63c713ee64d1aae/docs/demo/examples"

# Download the Oído models and test clips that the tests and the demo use.
[group('build')]
download_oido_assets:
  #!/usr/bin/env bash
  set -euo pipefail
  mkdir -p "{{ oido_assets_dir }}/clips"
  for entry in \
    "{{ oido_model_url }} oido_v2b.tnm 9e8b40913f532fb44b716a1f882f38b3b21e220713e417899aba32ace8c35066 ." \
    "{{ oido_model_url }} oido_v2.tlm 8329a4c94992653f6d1592be1d7747f2d3009473767d0d25c81c17a72af91283 ." \
    "{{ oido_clip_url }} b05.wav 66c6582b36f92d2e62e2103dccb999a55c01eb237c6e5aa60fa0ed3548233854 clips" \
    "{{ oido_clip_url }} b14.wav d32a966cb9aeb13f4c33da901a05b4fb07cc76434e42246c3800f98766804851 clips" \
    "{{ oido_clip_url }} b16.wav 2b138df7e2bdf5c54c99c7a6b9878efffdd112eb3591ac4cb21c42950eb91ed8 clips"
  do
    read -r url name checksum folder <<< "$entry"
    path="{{ oido_assets_dir }}/$folder/$name"
    if [ -f "$path" ] && echo "$checksum  $path" | shasum -a 256 -c --status; then
      continue
    fi
    echo "downloading $name"
    curl --fail --location --silent --show-error --output "$path.part" "$url/$name"
    if ! echo "$checksum  $path.part" | shasum -a 256 -c --status; then
      rm -f "$path.part"
      echo "checksum mismatch for $name from $url" >&2
      exit 1
    fi
    mv "$path.part" "$path"
  done

# Build the firmwares and the libraries for the ESP32-S3.
[group('build')]
build_s3 args="":
  cargo build --locked --workspace {{ args }}

# Build every target for this computer.
[group('build')]
build_host args="":
  {{ host_cargo }} build --locked {{ host_packages }} --all-targets --target host-tuple {{ args }}

# Build for the ESP32-S3 and for this computer in debug mode.
[group('build')]
build args="": (build_s3 args) (build_host args)

# Build for the ESP32-S3 and for this computer in release mode.
[group('build')]
release args="": (build_s3 ("--release " + args)) (build_host ("--release " + args))

# Flash the firmware to the ESP32-S3 in release mode and open the serial monitor.
[group('build')]
run args="":
  cargo run --locked --release --package {{ firmware }} {{ args }}

# Flash the microphone demo in release mode and open the serial monitor; board is n16r8 or n32r16v.
[group('build')]
run_mic_demo board args="":
  cargo run --locked --release --package {{ mic_demo }} --config 'target.xtensa-esp32s3-none-elf.runner="espflash flash --monitor --chip esp32s3 --flash-size {{ if board == "n16r8" { "16mb" } else if board == "n32r16v" { "32mb" } else { error("board must be n16r8 or n32r16v") } }}"' {{ args }}

# Open the serial monitor without flashing.
[group('build')]
monitor args="":
  espflash monitor {{ args }}

# Remove Cargo build artifacts.
[group('build')]
[confirm("Are you sure you want to clean the build artifacts?")]
clean:
  cargo clean
