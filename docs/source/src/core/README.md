The `core` module: the only place the crate names a function of the core or declares one of its structs. `table.rs` resolves the functions from the core's binary, `abi.rs` declares the structs by hand, and `wrapper.rs` is the one wrapper every method of `calls.rs` goes through, so one build of the crate runs against whichever core its caller loads. What the wrapper guarantees is [the core doc](../../../core.md)'s.

<!-- generated: source-children -->
- [abi.rs](../../../../src/core/abi.rs) - every struct of the core's header as a `#[repr(C)]` struct written by hand, and the statuses and kinds the crate reads
- [answer.rs](../../../../src/core/answer.rs) - the `Answer` a call gives when it does not do what it was asked, and how the core declined it
- [binary.rs](../../../../src/core/binary.rs) - a shared library opened by path, the platform's loader chosen at compile time
- [binary_posix.rs](../../../../src/core/binary_posix.rs) - the library calls on `dlopen`, `dlsym` and `dlclose`, declared by hand
- [binary_windows.rs](../../../../src/core/binary_windows.rs) - the library calls on `LoadLibraryW`, `GetProcAddress` and `FreeLibrary`, declared by hand, with the error dialog held off
- [calls.rs](../../../../src/core/calls.rs) - every call of the core as one method of `Core`, laying out what it hands in and reading back what the core answers
- [codes.rs](../../../../src/core/codes.rs) - the core's enumerations, each a newtype over the number that travels, so a value a newer core adds is carried as it came
- [declarations.rs](../../../../src/core/declarations.rs) - the layout of every hand-declared struct and the name of every function, for the tests that hold them to the core
- [event.rs](../../../../src/core/event.rs) - an event in the crate's own values, copied from the core's before it is released
- [library.rs](../../../../src/core/library.rs) - a C library loaded as host services: its binary by path, a bytes call answering the library's bytes in place, a definition and a description answered once
- [marshal.rs](../../../../src/core/marshal.rs) - laying out a call's inputs as the core's structs in memory that lives until the call returns, refusing a text holding a NUL
- [mod.rs](../../../../src/core/mod.rs) - the module's public surface
- [read.rs](../../../../src/core/read.rs) - reading texts, lists and arrays out of the core's memory, stepping a growing array at the stride its first element declares
- [table.rs](../../../../src/core/table.rs) - every function of the core resolved by name into one table, and `CoreBinary`, which owns the loaded library
- [values.rs](../../../../src/core/values.rs) - the values the core's calls take and answer, as owned Rust values
- [wrapper.rs](../../../../src/core/wrapper.rs) - the wrapper: the lock, the drain, the answers, the dispatch queue and deferred steps, the sink and the deadline hooks
- [wrapper_tests.rs](../../../../src/core/wrapper_tests.rs) - the unit tests of what the wrapper keeps to the crate: deferred steps, destroy, the deadline hooks, and hooks dropped outside the lock
<!-- end generated -->
