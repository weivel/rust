The described definitions of the `host` module: a C library's definition bound from the description it answers, its layouts and plans built at the load, and each call made from them. The memory walk, the native call and the reading of a description are the module's `unsafe`; every other file here forbids it. What it guarantees is [the descriptions doc](../../../../descriptions.md)'s.

<!-- generated: source-children -->
- [abi.rs](../../../../../src/host/description/abi.rs) - `description.h` and `value.h`, declared by hand, and the kinds, roles and value kinds they number
- [definition.rs](../../../../../src/host/description/definition.rs) - a description bound once at the load, and `BoundDefinition`: a service's definition, a carried one, or one bound with no library
- [described.rs](../../../../../src/host/description/described.rs) - a description as Rust values, read once at the load
- [enums.rs](../../../../../src/host/description/enums.rs) - a description's enums by name, `weivel_status` beside them and refused when it is not `core.h`'s
- [holdings.rs](../../../../../src/host/description/holdings.rs) - what outlives its call: a handle, an answer carrying a definition, the scope a carried definition lives in, and the holdings an unload releases
- [invoke.rs](../../../../../src/host/description/invoke.rs) - the one native call an operation makes, typed at the call from its parameters' kinds, and the call of a release
- [layouts.rs](../../../../../src/host/description/layouts.rs) - every described struct laid out by this platform's C rules and held against the description
- [memory.rs](../../../../../src/host/description/memory.rs) - the memory walk: a call's inputs written into memory of its own, and every answer read back from what its provider filled
- [mod.rs](../../../../../src/host/description/mod.rs) - the module's surface
- [operations.rs](../../../../../src/host/description/operations.rs) - one operation planned at the bind, and each call made from its plan: inputs written, the call, answers read and released
- [reader.rs](../../../../../src/host/description/reader.rs) - reads a checked description into Rust values, a field past its answered `size` read as none
- [value.rs](../../../../../src/host/description/value.rs) - `Value`, `Tree`, what a call answers, and `DescribedCallError`
<!-- end generated -->
