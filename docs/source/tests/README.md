The crate's tests, one test target per module, each a folder Cargo builds from its `main.rs`, and the helpers every target includes. Every test runs against the binaries `scripts/native` gathers into `build/native` and against the pinned siblings under `siblings/`, as [building.md](../../building.md) says.

<!-- generated: source-children -->
- [allocations/](allocations/) - the allocations one described call and one library call make, pinned
- [benchmarks/](benchmarks/) - the benchmark gates, a target of their own run without the standard harness
- [conformance/](conformance/) - the layer suite and its coverage test
- [context/](context/) - the `context` module's tests, among them four threads calling while a fifth dispatches
- [core/](core/) - the `core` module's tests
- [handles/](handles/) - the `handles` module's tests
- [host/](host/) - the `host` module's tests: the plugin seam, C libraries, and the races of a release, a settling and the destroy
- [plugins/](plugins/) - the test plugins the suite loads across the plugin seam, one `cdylib`
- [program/](program/) - the `program` module's tests: the `weivel-host` command line and a running host
- [standard/](standard/) - the `standard` module's tests: manifests, declarations, service ids and the typed services against `stub-host`
- [support/](support/) - the helpers every test target includes
- [testing/](testing/) - the `testing` module's tests
- [timer/](timer/) - the `timer` module's tests
<!-- end generated -->
