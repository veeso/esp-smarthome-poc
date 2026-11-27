alias lint := clippy

# Format Markdown, Rust, TOML, and YAML files.
[group('code-check')]
fmt args="":
  dprint fmt {{ args }}

# Check formatting without changing files.
[group('code-check')]
fmt_check args="":
  dprint check {{ args }}

# Run Clippy for the ESP32-S3.
[group('code-check')]
clippy_s3 args="":
  cargo clippy --locked --workspace --all-features {{ args }}

# Run Clippy for every target on this computer.
[group('code-check')]
clippy_host args="":
  {{ host_cargo }} clippy --locked {{ host_packages }} --all-targets --all-features --target host-tuple {{ args }}

# Run Clippy for the ESP32-S3 and for this computer.
[group('code-check')]
clippy args="": (clippy_s3 args) (clippy_host args)

# Build the documentation for the ESP32-S3 and deny warnings.
[group('code-check')]
doc_s3 args="":
  RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --all-features --no-deps {{ args }}

# Build the library documentation for this computer and deny warnings.
[group('code-check')]
doc_host args="":
  RUSTDOCFLAGS="-D warnings" {{ host_cargo }} doc --locked {{ host_libraries }} --all-features --no-deps --target host-tuple {{ args }}

# Build the documentation for the ESP32-S3 and for this computer.
[group('code-check')]
doc args="": (doc_s3 args) (doc_host args)

# Check advisories, licenses, bans, and dependency sources.
[group('code-check')]
deny args="":
  cargo deny check {{ args }}

# Scan paths for verified or unknown secrets.
[group('code-check')]
scan_secrets *args=".":
  trufflehog filesystem {{ args }} --results=verified,unknown --fail --fail-on-scan-errors --no-update

# Point Git at the tracked hook directory.
[group('code-check')]
setup_githooks:
  git config core.hooksPath .githooks
  @echo "git hooks installed: core.hooksPath = .githooks"

# Run the complete local quality gate.
[group('code-check')]
check:
  just fmt_check
  just clippy "-- -D warnings"
  just doc
  just deny
  just test
