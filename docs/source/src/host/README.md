The `host` module: registering plugins, activating them in plan order, verifying, releasing and destroying them, host services, C libraries as host services, their described definitions, and the plugin seam. The seam and the description binder's memory walk are the module's `unsafe`; every other file forbids it. What it guarantees is [the host doc](../../../host.md)'s.

<!-- generated: source-children -->
- [activation.rs](../../../../src/host/activation.rs) - activation in plan order, a settled activate verified or quarantined, and a verification asked for
- [declaration.rs](../../../../src/host/declaration.rs) - a C library's declaration as a value, and what of one is the caller's mistake before anything is opened
- [description/](description/) - described definitions: a C library's definition bound from its description, its layouts and plans built at the load
- [entry.rs](../../../../src/host/entry.rs) - a plugin's `Entry`, an activation's outcomes, and the host's options
- [libraries.rs](../../../../src/host/libraries.rs) - C libraries as host services: the load, each service registered under `WEIVEL_HOST`, the unload, and when a binary closes
- [mod.rs](../../../../src/host/mod.rs) - the module's public surface
- [release.rs](../../../../src/host/release.rs) - a release: the deactivate, the core's release, the forget and the record dropped
- [seam/](seam/) - the plugin seam: the C tables a host and a plugin built as a library of its own reach each other through
- [services.rs](../../../../src/host/services.rs) - a service of the host's own under `WEIVEL_HOST`, counted as provided by an activation plan
- [state.rs](../../../../src/host/state.rs) - the host itself: what it keeps of each plugin, its creation, registration, ledger and destroy
<!-- end generated -->
