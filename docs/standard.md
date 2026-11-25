# `standard`

The standard's defaults, reached through the standard's own binary, which its caller names by path. Its files are
listed in [its source doc](source/src/standard/README.md). So far it holds the standard's JSON reader, its
description checks and the list of the standard's structs the crate declares by hand.

`StandardJson::load` opens the standard's shared binary with the same loader the core's binary is opened with and
resolves the functions of `json.h`, each declared by hand. `StandardJson::parse` reads one document through the
standard's reader into a `Json` value of the crate's own, whose objects are keyed by member name, so the crate reads
JSON without a parser of its own and with no dependency. A text the reader refuses is answered as a `JsonRefusal`
carrying where and why. Nothing of the host or the contexts reads JSON: a document reaches them only as values the
standard read.

`StandardDescriptions::load` resolves `weivel_standard_description_check`, `weivel_standard_description_answers` and
the refusal's release from the same binary, and is the host's `DescriptionCheck`: a description a C library's service
declares is checked whole, then held to the definition its service's symbol answered, and a refusal of either is the
caller's mistake naming the check and the path it refused at. The `weivel_standard_refusal` it hands in is declared by
hand.

`declarations::standard_struct_layouts` lists that refusal and every struct of `description.h` and `value.h` the
description binder declares, each field as this compiler lays it out, and a test holds every one to what the
standard's `layouts` program prints, the way the core's structs are held to the core's.
