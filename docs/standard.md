# `standard`

The standard's defaults, reached through the standard's own binary, which its caller names by path, as the core's
`docs/contract/standard.md` asks of a language's `standard` module. Its files are listed in
[its source doc](source/src/standard/README.md). It holds the standard's manifest loader, its library declaration
reader, its service ids, its JSON reader and its description checks, typed access to the six services the standard
defines, and the list of the standard's structs the crate declares by hand.

`Standard::load` opens the standard's shared binary with the same loader the core's binary is opened with and
resolves every function the crate calls of it, each declared by hand. `Standard::load_manifest` validates a JSON
manifest by the standard's schema and rules and reads it only when it is valid, as `weivel_standard_load_manifest`
does, into the core's `Manifest`; `Standard::read_library` reads a C library's declaration, its services, the
description each declares and the contracts it publishes, into the host's `LibraryDeclaration`. Either answers a
`StandardRefused` naming the standard's code, its detail and fix, and the field's path. `Standard::service_ids` are
the ids the standard's binary answers, `weivel.config` to `weivel.host`, never ids the crate writes itself.

`StandardJson` reads one document through the standard's JSON reader into a `Json` value of the crate's own, whose
objects are keyed by member name, or with `parse_in_order` into a value tree whose records keep the order the
document writes them in, so the crate reads JSON without a parser of its own and with no dependency. A text the
reader refuses is answered as a `JsonRefusal` carrying where and why. Nothing of the host or the contexts reads JSON:
a document reaches them only as values the standard read.

`StandardDescriptions` is the host's `DescriptionCheck`: a description a C library's service declares is checked
whole with `weivel_standard_description_check`, then held to the definition its service's symbol answered with
`weivel_standard_description_answers`, and a contract's description, which describes a definition the library serves
none of, is checked whole alone. A refusal of either is the caller's mistake naming the check and the path it refused
at.

## Typed access to the standard's services

Each is a wrapper over the `BoundDefinition` the service's description bound, the one C definition every language
translates, so nothing of a service is declared twice. A call the provider declines answers the binding's
`DescribedCallError`, its `detail` and `fix` the provider's own, and an operation appended after a definition's
first shape that the provider does not offer is the caller's mistake, calling nothing.

| type | definition | what it offers |
|---|---|---|
| `Config` | `weivel_standard_config` | `get` and `set` of a `Setting` (text, number, boolean), `get_tree` of a whole setting as a value tree |
| `Secrets` | `weivel_standard_secrets` | `get`, `set` and `remove` of bytes |
| `Paths` | `weivel_standard_paths` | `cache`, `config` and `plugin`, each an absolute directory |
| `Log` | `weivel_standard_log` | `write` of an entry at a `LogLevel`, with fields as a record |
| `Scopes` | `weivel_standard_scopes` | `open` of one plugin's `Scope`: its config, secrets, paths and log, each the plugin's alone until `Scope::release` |
| `HostProvider` | `weivel_standard_host` | `start`, answering a `HostPlan`; `started`, told each `HostOutcome`; `tick`, answering a `NextTick`; `finished`, told an `ActionEnding`; `stop`, answering the exit code |

`Config`, `Secrets`, `Paths` and `Log` are made from the object a plugin's context resolved, `Scopes` and
`HostProvider` from the definition a loaded library withheld for its caller. A plan's text that is NULL, and a member
past the size its provider answered, is `None`, and so is a value of kind NONE; `ActionEnding::Answered(None)` is
handed to `finished` as a value of kind NONE, never as NULL.

`declarations::standard_struct_layouts` lists the refusal, the validation, the library declaration and its elements,
and every struct of `description.h` and `value.h` the description binder declares, each field as this compiler lays
it out, and a test holds every one to what the standard's `layouts` program prints, the way the core's structs are
held to the core's.
