# See https://just.systems/man/en/ for docs

set dotenv-load  # Loads .env

bt := '0'

export RUST_BACKTRACE := bt

log := "warn"

# List available commands
default:
    just --list

# Installs any and all cargo utilities used for the development of this app
setup:
  # Cargo binstall downloads pre-built binaries for cargo extensions
  # This saves minutes on each `cargo binstall` invocation, relative
  # to what `cargo install` would have done
  @cargo install cargo-binstall

  # cargo-udeps checks for unused dependencies
  @cargo binstall cargo-udeps --locked --no-confirm
  @rustup toolchain install nightly

  # cargo-deny checks dependency licenses, to make sure we
  # dont accidentally use any copy-left licensed packages.
  # See deny.toml for configuration.
  @cargo binstall cargo-deny --locked --no-confirm

  # cargo-audit checks for security vulnerabilities
  @cargo binstall cargo-audit --locked --no-confirm

  # sccache does caching of Rust dependencies really well
  @cargo binstall sccache --locked --no-confirm

  # coverage
  cargo binstall cargo-llvm-cov --locked --no-confirm

  # helps identify unused dependency features
  cargo binstall cargo-unused-features --locked --no-confirm

  # pre-commit is used to run checks on-commit
  @pip install pre-commit && pre-commit install
  @export RUSTC_WRAPPER=$(which sccache)
  @echo "Run \`echo 'export RUSTC_WRAPPER=\$(which sccache)' >> ~/.bashrc\` to use sccache for caching"

# Run the binary "for real" for the container-retention-policy package
run:
    RUST_LOG=container_retention_policy=debug cargo r -- \
        --account snok \
        --token $DELETE_PACKAGES_CLASSIC_TOKEN \
        --tag-selection both \
        --image-names "container-retention-policy"  \
        --image-tags "!latest !test-1* !v*" \
        --shas-to-skip "" \
        --keep-n-most-recent 5 \
        --timestamp-to-use "updated_at" \
        --cut-off 1d \
        --dry-run false

fuzz time:
    cargo build
    cargo +nightly fuzz run --sanitizer address fuzz_cli -- -max_total_time={{time}}
