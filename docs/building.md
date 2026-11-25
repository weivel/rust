# Building and testing

`cargo run -p native` builds the native siblings the tests load, and `cargo test --workspace` then builds the crate and
runs every test, the benchmark gates briefly and judging no timing; `cargo test --release --test benchmarks` judges them,
as [benchmarks.md](benchmarks.md) says. `rust-toolchain.toml` pins one exact toolchain, which rustup acquires on the first `cargo` call, so
rustup, CMake 3.15 or later and a C compiler are the prerequisites and nothing else is installed by hand.

## The siblings

`repo.json` pins the core, the standard and the conformance kit by commit, and the build reads them from
`siblings/core`, `siblings/standard` and `siblings/conformance`: CI checks them out there, and locally each is a link
to, or a clone of, a checkout at the pinned commit. `scripts/native` is a binary crate of the workspace that builds the
core's shared library, its `echo` test library and its `layouts` program, the standard's shared library, its
`described` test library and its `layouts` program, and the kit's `stub-host`, configured as a project of its own
against the standard and the core siblings, with CMake in Release, then builds the test plugins under `tests/plugins`
with Cargo, and gathers all of them into `build/native`, `stub-host` with the kit's three declarations of it beside it
in `build/native/stub-host`, where the tests find them. A test
that finds one missing says to run `scripts/native` first. The layer suite reads the kit's vectors from `siblings/conformance/vectors/layer`.

## Where Cargo builds

The conformance kit's drift checks walk the whole tree and skip only `build`, `build-*`, `siblings`, `.git` and the
hosting service's directory at the root, so a `target` directory beside them is read and fails `purity` and `names`.
Point Cargo's build directory under `build/` for a local run, `CARGO_TARGET_DIR=build/cargo`. The setting cannot live
in the repository: a `.cargo/config.toml` is a file the `rust` role of `purity` does not allow.

## The drift checks

CI's Drift job runs the kit's `purity`, `names`, `parity` and `layers` checks on this repository at the kit commit it
pins, with the language `rust`, each stated in the kit's `docs/checks.md`. Locally, build the kit from
`siblings/conformance` with CMake and run each program on the repository root.

## The host program

`cargo build --bin weivel-host` builds the host program, `weivel-host [--core <path>] [--standard <path>] <host
library> [arguments...]`, which finds the core's and the standard's shared libraries beside itself unless `--core` and
`--standard` name others, as [program.md](program.md) says.

## What a release build should set

A library cannot set its consumer's profile. A program built on the crate is smallest with `opt-level = "s"`, fat LTO,
one codegen unit and `strip`; `panic = "abort"` would end the process on a listener's panic, which the crate otherwise
catches and tells its error sink.
