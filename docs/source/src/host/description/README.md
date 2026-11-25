The described definitions of the `host` module: a C library's definition bound from the description it answers, its layouts and plans built at the load, and each call made from them. The memory walk, the native call and the reading of a description are the module's `unsafe`; every other file here forbids it. What it guarantees is [the descriptions doc](../../../../descriptions.md)'s.

<!-- generated: source-children -->
- [abi.rs](../../../../../src/host/description/abi.rs) - `description.h` and `value.h`, declared by hand, and the kinds, roles and value kinds they number
- [definition.rs](../../../../../src/host/description/definition.rs) - a description bound once at the load, and `BoundDefinition`: a service's definition, a carried one, or one bound with no library
- [described.rs](../../../../../src/host/description/described.rs) - a description as Rust values, read once at the load
- [enums.rs](../../../../../src/host/description/enums.rs) - a description's enums by name, `weivel_status` beside them and refused when it is not `core.h`'s
- [holdings.rs](../../../../../src/host/description/holdings.rs) - what outlives its call: a handle, an answer carrying a definition, the scope a carried definition lives in, and the holdings an unload releases
- [invoke.rs](../../../../../src/host/description/invoke.rs) - the one native call an operation makes, typed at the call from its parameters' kinds, and the call of a release
- [layouts.rs](../../../../../src/host/description/layouts.rs) - every described struct laid out by this platform's C rules and held against the description
- [members.rs](../../../../../src/host/description/members.rs) - `Members`: a struct's members by name in one allocation, each name shared with the description
- [memory.rs](../../../../../src/host/description/memory.rs) - the blocks a call's inputs and answers live in, and the reads and writes every walk is built from
- [mod.rs](../../../../../src/host/description/mod.rs) - the module's surface
- [operations.rs](../../../../../src/host/description/operations.rs) - one operation planned at the bind, and each call made from its plan: inputs written, the call, answers read and released
- [read.rs](../../../../../src/host/description/read.rs) - every answer read back from what its provider filled, on a stack of its own rather than by recursing
- [reader.rs](../../../../../src/host/description/reader.rs) - reads a checked description into Rust values, a field past its answered `size` read as none
- [stack.rs](../../../../../src/host/description/stack.rs) - the stack the walks keep their first items in place on, allocating only past them
- [value.rs](../../../../../src/host/description/value.rs) - `Value`, `Tree`, what a call answers, and `DescribedCallError`
- [write.rs](../../../../../src/host/description/write.rs) - a call's inputs written into memory of its own, on a stack of its own rather than by recursing
<!-- end generated -->
