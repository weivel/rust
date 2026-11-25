The `core` module's tests, against the real core's binary and the core's and the standard's `layouts` programs. What the wrapper guarantees is [the core doc](../../../core.md)'s.

<!-- generated: source-children -->
- [calls.rs](../../../../tests/core/calls.rs) - every call of the core made once through the wrapper, and the caller's mistakes it answers without the core
- [layouts.rs](../../../../tests/core/layouts.rs) - every hand-declared struct held to what the core's `layouts` program prints, and the standard's program read whole
- [load.rs](../../../../tests/core/load.rs) - the loader against the real core, a library that lacks a function, and paths that name no library
- [main.rs](../../../../tests/core/main.rs) - the test target, declaring each test file
- [table.rs](../../../../tests/core/table.rs) - the table held to the functions the pinned core's header declares, in order
- [wrapper.rs](../../../../tests/core/wrapper.rs) - the wrapper's rules: refusals, dispatch order, deferred steps, the sink, destroy, the deadline hooks and calls from several threads
<!-- end generated -->
