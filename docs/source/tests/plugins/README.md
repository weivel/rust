The test plugins the suite loads across the plugin seam, one `cdylib` that `scripts/native` builds and gathers into `build/native`, each plugin exported under a symbol of its own.

<!-- generated: source-children -->
- [host_vector.rs](../../../../tests/plugins/host_vector.rs) - a `host` vector's plugin: its steps, actions, methods and implementations realised from the vector's words
- [lib.rs](../../../../tests/plugins/lib.rs) - the test plugins, each exported under a symbol of its own: one providing, one consuming, one panicking
<!-- end generated -->
