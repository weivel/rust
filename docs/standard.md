# `standard`

The standard's defaults, reached through the standard's own binary, which its caller names by path. Its files are
listed in [its source doc](source/src/standard/README.md). So far it holds the standard's JSON reader alone.

`StandardJson::load` opens the standard's shared binary with the same loader the core's binary is opened with and
resolves the functions of `json.h`, each declared by hand. `StandardJson::parse` reads one document through the
standard's reader into a `Json` value of the crate's own, whose objects are keyed by member name, so the crate reads
JSON without a parser of its own and with no dependency. A text the reader refuses is answered as a `JsonRefusal`
carrying where and why. Nothing of the host or the contexts reads JSON: a document reaches them only as values the
standard read.
