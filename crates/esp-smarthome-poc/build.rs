//! Configures the linker of the ESP32-S3 firmware.

fn main() {
    explain_linker_errors();
    check_linker_available();

    let script = std::env::current_exe().expect("a build script knows its own path");
    println!(
        "cargo:rustc-link-arg=-Wl,--error-handling-script={script}",
        script = script.display()
    );
    // `linkall.x` is provided by esp-hal and must be the last linker script.
    println!("cargo:rustc-link-arg=-Tlinkall.x");
}

/// Stops the build with instructions when the Xtensa linker is not in `PATH`.
fn check_linker_available() {
    const LINKER: &str = "xtensa-esp32s3-elf-gcc";

    println!("cargo:rerun-if-env-changed=PATH");
    if let Err(error) = std::process::Command::new(LINKER).arg("--version").output() {
        panic!(
            "Xtensa linker `{LINKER}` could not be executed: {error}.\n\n\
             Load the espup environment before building:\n\n    . $HOME/export-esp.sh\n\n\
             Run `espup install --toolchain-version 1.97.0.0` first if that file is missing."
        );
    }
}

/// Prints a hint for common linker errors.
///
/// The linker runs this build script again with the error kind and the symbol as
/// arguments. Cargo runs it without arguments, and then this function returns.
fn explain_linker_errors() {
    let mut args = std::env::args().skip(1);
    let Some(kind) = args.next() else {
        return;
    };
    if kind != "undefined-symbol" {
        std::process::exit(1);
    }

    let hint = match args.next().as_deref() {
        Some("_stack_start") => Some("is the linker script `linkall.x` missing?"),
        Some("free" | "malloc" | "calloc" | "realloc") => {
            Some("is the `esp-alloc` dependency or its `compat` feature missing?")
        }
        _ => None,
    };
    if let Some(hint) = hint {
        eprintln!("\nhint: {hint}\n");
    }
    std::process::exit(0);
}
