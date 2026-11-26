# Standard Rust toolchain for regular computers; keep it equal to `workspace.package.rust-version`.
host_toolchain := "1.97"

# Cargo toolchain override for regular computers, which bypasses the ESP toolchain in rust-toolchain.toml.
host_cargo := "cargo +" + host_toolchain

# Firmware package, which builds only for the ESP32-S3.
firmware := "esp-smarthome-poc"

# Library package, which builds for the ESP32-S3 and for regular computers.
library := "esp-smarthome"

# Build the firmware and the library for the ESP32-S3.
[group('build')]
build_s3 args="":
  cargo build --locked --workspace {{ args }}

# Build every library target for this computer.
[group('build')]
build_host args="":
  {{ host_cargo }} build --locked --package {{ library }} --all-targets --target host-tuple {{ args }}

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

# Open the serial monitor without flashing.
[group('build')]
monitor args="":
  espflash monitor {{ args }}

# Remove Cargo build artifacts.
[group('build')]
[confirm("Are you sure you want to clean the build artifacts?")]
clean:
  cargo clean
