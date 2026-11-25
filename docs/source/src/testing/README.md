The `testing` module, for the tests of a program that hosts plugins in Rust; it names nothing of any one project and no other module needs it. What each helper does, and why the ones that set variables are `unsafe`, is [the testing doc](../../../testing.md)'s.

<!-- generated: source-children -->
- [described_library.rs](../../../../src/testing/described_library.rs) - `described_library`: a described C library bound from its folder on a host of its own
- [failure.rs](../../../../src/testing/failure.rs) - `TestingFailure`: why a helper could not do what it was asked
- [free_port.rs](../../../../src/testing/free_port.rs) - `free_port`: a port nothing listens on at that moment
- [mod.rs](../../../../src/testing/mod.rs) - the module's public surface
- [scratch_home.rs](../../../../src/testing/scratch_home.rs) - `scratch_home`: a fresh folder laid out with libraries, files and variables, and its removal
- [variables.rs](../../../../src/testing/variables.rs) - `set_variables` and its restore, around `put`, the module's one setting of a variable
<!-- end generated -->
