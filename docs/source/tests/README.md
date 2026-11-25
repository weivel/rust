The crate's tests, one test target per module, each a folder Cargo builds from its `main.rs`, and the helpers every target includes. Every test runs against the binaries `scripts/native` gathers into `build/native` and against the pinned siblings under `siblings/`, as [building.md](../../building.md) says.

<!-- generated: source-children -->
- [conformance/](conformance/) - the layer suite and its coverage test
- [context/](context/) - the `context` module's tests, among them four threads calling while a fifth dispatches
- [core/](core/) - the `core` module's tests
- [handles/](handles/) - the `handles` module's tests
- [support/](support/) - the helpers every test target includes
- [timer/](timer/) - the `timer` module's tests
<!-- end generated -->
