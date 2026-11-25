The crate's tests, one test target per module, each a folder Cargo builds from its `main.rs`, and the helpers every target includes. Every test runs against the binaries `scripts/native` gathers into `build/native` and against the pinned siblings under `siblings/`, as [building.md](../../building.md) says.

<!-- generated: source-children -->
- [core/](core/) - the `core` module's tests
- [support/](support/) - the helpers every test target includes
<!-- end generated -->
