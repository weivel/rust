The `host` module: registering plugins, activating them in plan order, verifying, releasing and destroying them. What it guarantees is [the host doc](../../../host.md)'s.

<!-- generated: source-children -->
- [activation.rs](../../../../src/host/activation.rs) - activation in plan order, a settled activate verified or quarantined, and a verification asked for
- [entry.rs](../../../../src/host/entry.rs) - a plugin's `Entry`, an activation's outcomes, and the host's options
- [mod.rs](../../../../src/host/mod.rs) - the module's public surface
- [release.rs](../../../../src/host/release.rs) - a release: the deactivate, the core's release, the forget and the record dropped
- [state.rs](../../../../src/host/state.rs) - the host itself: what it keeps of each plugin, its creation, registration, ledger and destroy
<!-- end generated -->
