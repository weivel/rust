The library, one folder per module of the contract, under the contract's names, and `lib.rs`, the one file its compiler starts the library from. What the crate's modules share with one another and with no caller is `pub(crate)`, which hides it only because the crate is one crate.

<!-- generated: source-children -->
- [core/](core/) - the `core` module: the loader, the hand declarations, the wrapper and the core's calls
- [lib.rs](../../../src/lib.rs) - the crate root, declaring each module
<!-- end generated -->
