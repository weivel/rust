# Building and testing

`cargo run -p native` builds the native siblings the tests load, and `cargo test --workspace` then builds the crate and
runs every test. `rust-toolchain.toml` pins one exact toolchain, which rustup acquires on the first `cargo` call, so
rustup, CMake 3.15 or later and a C compiler are the prerequisites and nothing else is installed by hand.

## The siblings

`repo.json` pins the core, the standard and the conformance kit by commit, and the build reads them from
`siblings/core`, `siblings/standard` and `siblings/conformance`: CI checks them out there, and locally each is a link
to, or a clone of, a checkout at the pinned commit. `scripts/native` is a binary crate of the workspace that builds the
core's shared library, its `echo` test library and its `layouts` program, and the standard's `layouts` program, with
CMake in Release, and gathers them into `build/native`, where the tests find them. A test that finds one missing says
to run `scripts/native` first.

## Where Cargo builds

The conformance kit's drift checks walk the whole tree and skip only `build`, `build-*`, `siblings`, `.git` and the
hosting service's directory at the root, so a `target` directory beside them is read and fails `purity` and `names`.
Point Cargo's build directory under `build/` for a local run, `CARGO_TARGET_DIR=build/cargo`. The setting cannot live
in the repository: a `.cargo/config.toml` is a file the `rust` role of `purity` does not allow.

## The drift checks

CI's Drift job runs the kit's `purity`, `names`, `parity` and `layers` checks on this repository at the kit commit it
pins, with the language `rust`, each stated in the kit's `docs/checks.md`. Locally, build the kit from
`siblings/conformance` with CMake and run each program on the repository root.

## What a release build should set

A library cannot set its consumer's profile. A program built on the crate is smallest with `opt-level = "s"`, fat LTO,
one codegen unit and `strip`; `panic = "abort"` would end the process on a listener's panic, which the crate otherwise
catches and tells its error sink.
