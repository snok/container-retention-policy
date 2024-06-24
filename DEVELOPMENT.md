# Creating a release

To create a release we need to:

1. Increment the version in the [Cargo.toml](./Cargo.toml)
2. Manually trigger the [`release`](.github/workflows/release.yaml) workflow to build a new image
3. Update the image tag in the [action.yaml](action.yaml) and commit that to the main branch
4. Create a GitHub release post for the repo, with the version tag (e.g., `v3.0.0`)

# Running lints and tests

Install [pre-commit](https://pre-commit.com/) (e.g., using `pip install pre-commit`),
then run `pre-commit run --all-files` before submitting a PR.

All cargo-components run can be installed by calling `cargo install just && just setup`.
This will install [just](https://github.com/casey/just) and run the `setup` script
in the local [justfile](./justfile).

If you prefer not to install any of these components, that's fine. Just submit a PR,
then fix errors as they're caught in CI.

# Integration testing

Since the action fundamentally depends on the Github container registry,
the only real way to test (that I've thought of at least) is to simply
upload real images and running the binary with dry-run on and off.

To upload images, see the [live_test workflow](./.github/workflows/live_test.yaml)
where we do the same thing.

To run the binary, see the `run` command in the [justfile](./justfile). If you run this,
you'll need an `.env` file containing the token you want to pass.

# Pruning unused features

You might notice that there's a lot of disabled features in the [Cargo.toml](./Cargo.toml).
This might be redundant, but is a measure for trying to minimize the binary size. We've
used [cargo-unused-features](https://crates.io/crates/cargo-unused-features) and the
`unused-features analyze` command to aid in identifying redundant features.
