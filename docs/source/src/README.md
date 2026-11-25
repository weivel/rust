The library, one folder per module of the contract, under the contract's names, and `lib.rs`, the one file its compiler starts the library from. What the crate's modules share with one another and with no caller is `pub(crate)`, which hides it only because the crate is one crate.

<!-- generated: source-children -->
- [benchmarks/](benchmarks/) - the `benchmarks` module: the calls the gates time, the calls past the layer they are timed against, and the verdicts against the kit's bounds
- [conformance/](conformance/) - the `conformance` module: the driver for the layer suite
- [context/](context/) - the `context` module: a plugin's context, its calls, the event routing and `Pending`
- [core/](core/) - the `core` module: the loader, the hand declarations, the wrapper and the core's calls
- [handles/](handles/) - the `handles` module: the table from the core's handles to the crate's records
- [host/](host/) - the `host` module: registration, activation, verification, release and destroy, host services, C libraries and the plugin seam
- [lib.rs](../../../src/lib.rs) - the crate root, declaring each module
- [program/](program/) - the `program` module: the host program, `start_host`, `run_host` and the `weivel-host` command line
- [standard/](standard/) - the `standard` module: the standard's binary by path, its manifests, declarations, JSON reader and description checks, and typed access to its services
- [testing/](testing/) - the `testing` module: a scratch home, variables set and put back, a described library bound from its folder, and a free port
- [timer/](timer/) - the `timer` module: the scheduler, the system scheduler and the core's timer
<!-- end generated -->
