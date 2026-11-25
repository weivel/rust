The test plugins the suite loads across the plugin seam, one `cdylib` that `scripts/native` builds and gathers into `build/native`, each plugin exported under a symbol of its own.

<!-- generated: source-children -->
- [lib.rs](../../../../tests/plugins/lib.rs) - the test plugins, each exported under a symbol of its own: one providing, one consuming, one panicking
<!-- end generated -->
