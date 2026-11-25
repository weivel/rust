The `conformance` module: the driver for the layer suite, reading the kit's vectors through the standard's JSON reader and running each through a host of its own. What it guarantees is [the conformance doc](../../../conformance.md)'s.

<!-- generated: source-children -->
- [consume.rs](../../../../src/conformance/consume.rs) - a `consume` vector driven through contexts and the host
- [description.rs](../../../../src/conformance/description.rs) - a `description` vector: the `described` library loaded on the standard's check, holders opened in flight, and every call through a bound definition, its values translated by the description's kinds
- [driver.rs](../../../../src/conformance/driver.rs) - reading the kit's layer vectors and running one through a host of its own
- [echo.rs](../../../../src/conformance/echo.rs) - the suite's own reading of the core's `echo` test library: its definition laid out by hand, called through the address the host registered
- [library.rs](../../../../src/conformance/library.rs) - a `library` vector: the `echo` library loaded through the host, holders and owners opened in flight, and each step on the suite's thread
- [lifecycle.rs](../../../../src/conformance/lifecycle.rs) - a `lifecycle` vector: entries that run and trace their actions, and the host's steps
- [mod.rs](../../../../src/conformance/mod.rs) - the module's public surface
- [observations.rs](../../../../src/conformance/observations.rs) - a run answered in the shape of its vector's `then`, and a refusal as the kit writes it
- [shapes.rs](../../../../src/conformance/shapes.rs) - reading the parts of a vector, each checked for its shape
- [want.rs](../../../../src/conformance/want.rs) - a `want` vector on the suite's own clock and scheduler
- [world.rs](../../../../src/conformance/world.rs) - the suite's clock and scheduler, and a host whose log, sink and timer moves are kept
<!-- end generated -->
