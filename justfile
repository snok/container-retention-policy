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
        --tag-selection untagged \
        --image-names "container-retention-policy"  \
        --image-tags "!latest !test-1* !v*" \
        --shas-to-skip "" \
        --keep-n-most-recent 5 \
        --timestamp-to-use "updated_at" \
        --cut-off 1h \
        --dry-run false

fetch-package-digests tag="v3.0.0":
    curl -L \
          -X GET \
            -H "Accept: application/vnd.oci.image.index.v1+json" \
            -H "Authorization: Bearer $(echo $DELETE_PACKAGES_CLASSIC_TOKEN | base64)" \
            -H "X-GitHub-Api-Version: 2022-11-28" \
          https://ghcr.io/v2/snok%2Fcontainer-retention-policy/manifests/{{ tag }}

test:
    curl -L \
          -X GET \
            -H "Accept: application/vnd.oci.image.index.v1+json" \
            -H "Authorization: Bearer $(echo $DELETE_PACKAGES_CLASSIC_TOKEN | base64)" \
            -H "X-GitHub-Api-Version: 2022-11-28" \
          https://ghcr.io/v2/snok%2Fcontainer-retention-policy/manifests/test-5-3
