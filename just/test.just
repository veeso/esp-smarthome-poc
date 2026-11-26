# Run the library unit, integration, and documentation tests on this computer.
[group('test')]
test args="" doc_args="":
  {{ host_cargo }} nextest run --locked --package {{ library }} --all-targets --target host-tuple {{ args }}
  {{ host_cargo }} test --locked --package {{ library }} --doc --target host-tuple {{ doc_args }}

# Generate an LCOV coverage report of the library tests.
# cargo-llvm-cov does not resolve the `host-tuple` alias, so it gets the real triple.
[doc("Generate an LCOV coverage report of the library tests.")]
[group('test')]
coverage output="lcov.info":
  {{ host_cargo }} llvm-cov nextest --locked --package {{ library }} --all-targets --target "$(rustc +{{ host_toolchain }} --print host-tuple)" --lcov --output-path {{ output }}
