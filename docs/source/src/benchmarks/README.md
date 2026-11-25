The `benchmarks` module: the calls each gate times, the calls it times them against made past the layer, and the verdicts against the pinned kit's bounds. What each gate measures is [the benchmarks doc](../../../benchmarks.md)'s.

<!-- generated: source-children -->
- [bare.rs](../../../../src/benchmarks/bare.rs) - the calls past the layer: the core's own `weivel_get_service`, and the echo library's functions and definition reached through the platform's loader
- [budget.rs](../../../../src/benchmarks/budget.rs) - the kit's bounds read from its `benchmarks/rust.txt`, and the noise arithmetic
- [gates.rs](../../../../src/benchmarks/gates.rs) - each gate's verdict on a sample, no bound never a pass
- [hand.rs](../../../../src/benchmarks/hand.rs) - the described mirror's `reflect` bound by hand, the control of the `described-call` gate
- [measure.rs](../../../../src/benchmarks/measure.rs) - paths sampled side by side in rotating rounds, and their summaries
- [mod.rs](../../../../src/benchmarks/mod.rs) - the module's public surface
- [pairs.rs](../../../../src/benchmarks/pairs.rs) - each benchmark: the call through weivel, the call it is measured against, and the identity checks before the timing
- [world.rs](../../../../src/benchmarks/world.rs) - the host a benchmark runs in, with a provider and a consumer activated and the libraries it loads
<!-- end generated -->
