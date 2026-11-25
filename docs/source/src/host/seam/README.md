The plugin seam: the size-prefixed C tables a host and a plugin built as a library of its own reach each other through, their host side and their plugin side. What it guarantees is [the host doc](../../../../host.md)'s.

<!-- generated: source-children -->
- [abi.rs](../../../../../src/host/seam/abi.rs) - the seam's size-prefixed C tables: the entry, the context, the completion, an event and an answer
- [host_side.rs](../../../../../src/host/seam/host_side.rs) - the host's side: a plugin's library loaded by its entry symbol, and the context table each activate is handed
- [mod.rs](../../../../../src/host/seam/mod.rs) - the seam's public surface and `export_entry!`, which exports a plugin's entry from its library
- [plugin_side.rs](../../../../../src/host/seam/plugin_side.rs) - the plugin's side: its entry behind the entry table, and the host's context table behind the same safe `Context`
<!-- end generated -->
