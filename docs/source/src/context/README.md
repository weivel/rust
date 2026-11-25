The `context` module: a plugin's context and every call it makes, the records the handles table keeps for them, the one listener that routes each event to its plugin, and `Pending`, the result that comes later. What it guarantees is [the context doc](../../../context.md)'s.

<!-- generated: source-children -->
- [capabilities.rs](../../../../src/context/capabilities.rs) - a context's calls about capabilities: provide, list and watch
- [contexts.rs](../../../../src/context/contexts.rs) - the surface a host builds, opens, verifies, releases, quarantines and forgets contexts through
- [enums.rs](../../../../src/context/enums.rs) - `DescribedEnums`: a description's enums by name, and the entries a plugin requires of them
- [link.rs](../../../../src/context/link.rs) - what a context reaches the core through: the host's own contexts, or the host's context table across the plugin seam
- [log.rs](../../../../src/context/log.rs) - the host's log and a plugin's log
- [mod.rs](../../../../src/context/mod.rs) - the module's public surface
- [pending.rs](../../../../src/context/pending.rs) - `Pending`, a result that comes later: a future, a blocking wait or a callback
- [plugin.rs](../../../../src/context/plugin.rs) - a plugin's context: its handle, manifest, listeners, token and whether it is detached or silenced
- [records.rs](../../../../src/context/records.rs) - what a plugin hands the crate and hears from it, and the records the handles table keeps
- [routing.rs](../../../../src/context/routing.rs) - the one listener routing each event to the plugin it concerns, and capacity answering
- [services.rs](../../../../src/context/services.rs) - a context's calls about services: get, list, want, watch and register
- [state.rs](../../../../src/context/state.rs) - what every context of a host shares, and the call that answers detached under the layer lock
<!-- end generated -->
