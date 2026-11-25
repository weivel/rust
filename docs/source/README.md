The repository root: one Cargo workspace, the package `weivel-rust` (`Cargo.toml` and its `Cargo.lock`), the binary crate `scripts/native` beside it, `rust-toolchain.toml`, which pins the one toolchain every build uses, and `repo.json`, which pins the commits of the core, the standard and the conformance kit the build reads from `siblings/`. No code lies at the root. How to build the crate and run its tests is [building.md](../building.md).

<!-- generated: source-children -->
- [scripts/](scripts/) - the workspace's helper programs, which the tests and CI run before the suite
- [src/](src/) - the library: one folder per module and the crate root
- [tests/](tests/) - the crate's tests, one target per module, and the helpers they share
<!-- end generated -->
