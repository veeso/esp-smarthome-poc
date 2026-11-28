//! Compiles the upstream C engine and its shims for computer builds.

fn main() {
    println!("cargo:rerun-if-changed=shim");
    println!("cargo:rerun-if-changed=vendor");
    // The oracle only serves tests, which run on regular computers.
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "none") {
        return;
    }

    // `tasr_nemo.c` and `tinyasr_lm.c` are compiled through the shims that include them.
    cc::Build::new()
        .files([
            "shim/shim_lm.c",
            "shim/shim_math.c",
            "shim/shim_nemo.c",
            "vendor/kernels.c",
            "vendor/tasr_seg.c",
            "vendor/tinyasr.c",
        ])
        .include("vendor")
        .include("vendor/include")
        .opt_level(2)
        .std("c11")
        .define("_DEFAULT_SOURCE", None)
        .flag("-ffp-contract=off")
        .flag("-Wno-unused-function")
        .compile("oido_reference");
    println!("cargo:rustc-link-lib=m");
}
