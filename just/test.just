# Run the unit, integration, and documentation tests on this computer.
[group('test')]
test args="" doc_args="":
  {{ host_cargo }} nextest run --locked {{ host_packages }} --all-targets --target host-tuple {{ args }}
  {{ host_cargo }} test --locked {{ host_libraries }} --doc --target host-tuple {{ doc_args }}

# Generate an LCOV coverage report of the tests.
# cargo-llvm-cov does not resolve the `host-tuple` alias, so it gets the real triple.
[doc("Generate an LCOV coverage report of the tests.")]
[group('test')]
coverage output="lcov.info":
  {{ host_cargo }} llvm-cov nextest --locked {{ host_packages }} --all-targets --target "$(rustc +{{ host_toolchain }} --print host-tuple)" --lcov --output-path {{ output }}
