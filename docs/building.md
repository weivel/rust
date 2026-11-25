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
one codegen unit and `strip`, which `Cargo.toml` names the `smallest` profile, `cargo build --profile smallest --bin
weivel-host`. It is a profile of its own rather than `release`, so the benchmark gates keep timing the build they were
bounded on. It keeps `panic = "unwind"`: under `panic = "abort"` a listener's panic, which the crate otherwise catches
and tells its error sink as the contract asks, ends the process. A consumer may still choose abort, for the size
measured below, at that cost.

## The host program's size and startup

`cargo test --test size` builds `weivel-host` with the `smallest` profile, once with each panic strategy, and an empty
program, `fn main() {}`, with the same profile, all under `build/size`, then times 21 runs of the host over a plan
whose one plugin has no manifest, so a run loads the core, the standard and the kit's `stub-host`, starts, refuses the
plugin, fails the action, stops and destroys the host: every step of a run but a plugin's own. It prints one line per
panic strategy and judges only that each build succeeds and each run ends with the code the plan names; the Test job
runs it on every system, so its lines are in that job's log. Measured on 2026-11-25 with Rust 1.100.0:

| system | panic | `weivel-host` stripped | empty program | what the crate adds | empty run, median of 21 | least |
|---|---|---|---|---|---|---|
| Linux, x86_64, the CI's hosted `ubuntu-latest` runner | unwind | 1 067 008 bytes (1042 KiB) | 280 560 bytes | 786 448 bytes (768 KiB) | 1.4 ms | 1.4 ms |
| Linux, x86_64, the CI's hosted `ubuntu-latest` runner | abort | 916 800 bytes (895 KiB) | 278 776 bytes | 638 024 bytes (623 KiB) | 1.4 ms | 1.4 ms |
| macOS, aarch64, the CI's hosted `macos-latest` runner | unwind | 999 904 bytes (976 KiB) | 286 064 bytes | 713 840 bytes (697 KiB) | 5.6 ms | 4.9 ms |
| macOS, aarch64, the CI's hosted `macos-latest` runner | abort | 768 192 bytes (750 KiB) | 285 904 bytes | 482 288 bytes (471 KiB) | 8.1 ms | 5.3 ms |
| Windows, x86_64, MSVC, the CI's hosted `windows-latest` runner | unwind | 1 111 552 bytes (1085 KiB) | 106 496 bytes | 1 005 056 bytes (982 KiB) | 7.2 ms | 7.0 ms |
| Windows, x86_64, MSVC, the CI's hosted `windows-latest` runner | abort | 689 664 bytes (674 KiB) | 103 936 bytes | 585 728 bytes (572 KiB) | 7.0 ms | 6.9 ms |
| Windows 11, x86_64, MSVC, a workstation | unwind | 1 111 552 bytes (1085 KiB) | 107 008 bytes | 1 004 544 bytes (981 KiB) | 22.1 ms | 18.5 ms |
| Windows 11, x86_64, MSVC, a workstation | abort | 690 176 bytes (674 KiB) | 103 936 bytes | 586 240 bytes (573 KiB) | 20.5 ms | 19.9 ms |

The crate adds about 0.7 to 1.0 MB to a program with `panic = "unwind"`, and abort saves 150 KB on Linux, 230 KB on
macOS and 420 KB on Windows, where unwinding costs most. An empty run, from the process starting to its exit, takes
1.4 ms on Linux, 5 to 8 ms on macOS and 7 ms on Windows; the workstation's 20 ms is its own, the same binary taking 7
ms on the hosted runner. The CI rows are of the commit the measurement was added in, and the Test job prints them
again on every push.

A program with an interface adds its framework on top. Measured on Windows with `opt-level = "z"`, fat LTO, one
codegen unit, `strip` and `panic = "abort"`, an empty window of ratatui is about 280 KB and one of iced about 2.3 MB,
each with std included. That profile differs from this one twice, so the numbers compare only roughly: `"z"` is
smaller than `"s"`, and abort is the saving measured above. On that footing a program of ratatui and the crate is
about 0.9 MB with abort and 1.3 MB with unwind, and one of iced about 2.9 MB and 3.3 MB.
