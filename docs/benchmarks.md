# `benchmarks`

The Rust half of the benchmarks, as the core's `docs/contract/benchmarks.md` asks of a language's `benchmarks` module.
Its files are listed in [its source doc](source/src/benchmarks/README.md) and its gates in
[the gates' source doc](source/tests/benchmarks/README.md). Every gate is judged as the conformance kit's
`docs/benchmarks.md` defines it and bounded by the pinned kit's `benchmarks/rust.txt`, which is where each bound and
how it was measured is stated; this repository sets none.

The same-language gate is the identity check: the object a consumer's get resolves is the provider's own allocation,
compared with `Arc::ptr_eq`, or the benchmark fails before it times anything. The timing beside it is three paths
running the very same compiled call, the provider's `add` called on the provider's object twice and on what the
consumer resolved, and it is judged only against the platform's noise. The library call's definition row is judged the
same way: the echo definition's `echo` called through the address a consumer resolved, over the same operation called
through the address `echo_service_definition` answered to a load of the library's own, after a check that the two
addresses are one. Each path is handed the address itself and reads the operations out of the struct once, before its
timed loop.

`control-call` is `weivel_get_service` through the wrapper, on a host whose timer is armed on the system scheduler as a
host arms it, over the same function of the core's table called with the very core the wrapper calls, the same plugin
and id, nothing around it. `library-call` is the echo library's `echo_echo` through the host's `LibraryService`, 16
bytes in and the same 16 back, over the same function reached through the platform's loader and called directly, its
answer released through `echo_release` as the host releases it. `described-call` is the standard's described mirror's
`reflect`, a sample holding a text and a counted array of three texts in and its reflection out, called on the
`BoundDefinition` a consumer resolved, over the same operation through a binding of the fixture's structs written by
hand in the module alone, on a load of the library of its own. Both copy every member the operation answers out into
Rust values and release the reflection and the refusal after every call; the hand binding keeps one answer and one
refusal block across calls, as a binding written for one operation keeps them, where the generic binding makes its
blocks for each call. The echo library's bytes call through the host is sampled in the same rounds, and its time
printed beside the pair's. The crate runs a blocking operation on the calling thread, so it carries no
`blocking-call` row.

Paths are sampled side by side after a discarded warm-up, one batch of each a round with the first path moving on each
round, and judged on the median of the per-round ratios: the same-language pair in 101 rounds of a million calls, the
definition row in 101 rounds of 20 000, the control and library calls in 201 rounds of 1 000, and the described call
in 201 rounds of 200. A slowed path, spun for a set time before each of its calls, is how each gate is shown red.

A debug build runs every benchmark briefly, its identity checks included, and judges no timing, since a timing
verdict from an unoptimised build means nothing. `cargo test --release --test benchmarks` judges: a gate the kit's
file gives no line on the platform it runs on fails, and such a run measures every benchmark ten times and prints
each sample, which is what a line is set from. `cargo test --release --test benchmarks -- --measure <repetitions>`
measures that many times and judges nothing. Each sample is printed as a `weivel-benchmark-sample` line: a ratio
gate's per-round median ratio, and a pair of identical cost's noise, the deviation from 1 of its second direct path
over its first, beside its deviation, that of the path through weivel over the first.
