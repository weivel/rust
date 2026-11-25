The `standard` module: the standard's binary reached by path, its manifest loader, its library declaration reader, its JSON reader and its description checks, typed access to its services, and the layouts of the structs the crate declares of the standard's headers. It is one of the modules allowed `unsafe`, since it declares the standard's functions by hand. What it guarantees is [the standard doc](../../../standard.md)'s.

<!-- generated: source-children -->
- [binding.rs](../../../../src/standard/binding.rs) - the standard's manifest loader, library declaration reader and service ids, declared by hand
- [declarations.rs](../../../../src/standard/declarations.rs) - the layouts of every struct of the standard's headers the crate declares by hand, for the layout test
- [description.rs](../../../../src/standard/description.rs) - the standard's two description checks, declared by hand, run as the host's description check
- [json.rs](../../../../src/standard/json.rs) - the standard's JSON reader, declared by hand, and the `Json` values it reads
- [mod.rs](../../../../src/standard/mod.rs) - the module's public surface
- [services.rs](../../../../src/standard/services.rs) - typed access to config, secrets, paths, log, scopes and the host provider, each over the definition its description bound
<!-- end generated -->
