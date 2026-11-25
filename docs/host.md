# `host`

Registers plugins written in Rust, activates, verifies and releases them, offers host services, loads C libraries as
host services, loads plugins built as libraries of their own across the plugin seam, and destroys them with the core,
as the core's `docs/contract/host.md` asks of a language's `host` module. Its files are listed in
[its source doc](source/src/host/README.md).

## Plugins and their lifecycle

A plugin's entry is an `Entry`: an `activate` taking the plugin's context and answering a `Pending` that settles once
the activation is done, and optionally a `deactivate` answering one the same way. A panic or an error is a failed
activation, which the host quarantines with a reason of its own; a failed deactivate goes to the error sink and the
release goes on.

`Host::activate` asks the core for the plan, every service of the host's own counted as provided by the host, and
starts each plugin in its order: builds its context, opens it and calls its activate, never waiting for one before
starting the next, and returns the plan and each plugin's outcome to come. Where no dispatch runs, the activating
thread starts the plan as the dispatcher, so what an activate's own calls queue is dispatched once every activate of
the plan has been called; an activate therefore must not wait for a result such a dispatch would settle, and
`Pending::wait` on a dispatching thread panics rather than hangs. An activate that settles before it returns is
verified, or quarantined when the core refuses, before the next plugin is started. An outcome is `Active`,
`Quarantined`, `Refused`, `Released` or `Failed`, and carries the core's refusal that decided it; an open the core
answers destroyed is `Released`.

`Host::release` calls the deactivate while the plugin is still whole, releases it in the core, lets every event that
release queued reach it, ends its entries, forgets it and drops its record, the last steps put on the dispatch queue
behind the release's events. A release asked for from another thread while the plugin's activate call has not yet
returned, or while a settled activate is being verified or quarantined, waits for that to end, so no deactivate runs
before its activate and an outcome already settling is never answered `Released`. `Host::destroy` releases every
plugin it opened in the reverse of the order it opened them, one whose activate is in flight included, waits for
every open, settling and release under way, closes every C library to calls, then disarms the timer, destroys the
core and joins the system scheduler's thread when the host made one; every call after it answers `Destroyed` quietly.
`Host::ledger` answers the core's ledger, and the options' timer observer sees each move of the timer.

## Host services and C libraries

`Host::register_host_service` registers an object of the host's own under `WEIVEL_HOST`, refused as the caller's
mistake for an id a loaded C library declares. `Host::load_library` loads a C library from a `LibraryDeclaration`,
which the caller builds, and the library's path. A service of bytes functions is registered as a `LibraryService`,
whose `call` lends the input for the call and answers the library's bytes in place as `LibraryBytes`, released through
the library's release symbol when dropped and keeping the binary open until then, so there is no copy. A definition
service declaring no description is registered awaiting releases as a `ForeignObject` carrying the very address its symbol answered, called
once at the load; a definition answering NULL, or a struct whose `size` is below `2 * sizeof(void *)`, refuses the
load as the caller's mistake. A service that declares a `description` loads only on a host given a description check
in `HostOptions`, which `standard::StandardDescriptions` provides: `weivel_standard_description_check`, then
`weivel_standard_description_answers` with the definition, each refusal the caller's mistake. A described definition is
registered awaiting releases as the `BoundDefinition` bound from its description, standing for the very address its
symbol answered, as [descriptions.md](descriptions.md) states. A declaration of contracts alone
loads and registers nothing; its contracts stay on the declaration, and no capability a plugin provides is checked
against them yet. An id declared twice, or registered under the host already by a plugin, another live library or the
host itself, refuses the load.

`Host::unload_library` releases every handle its bound definitions answered and every answer carrying a definition
that nobody released, while the binary is still mapped, closes the library to calls at once, so a later call through its bytes service answers
`Unloaded`, and unregisters every service, which revokes each definition to its holders. A holder hears the
revocation awaiting its release and lets go with `Context::release_holding`; one released, quarantined or forgotten
has let go with it. The binary closes once the core has told the host that nothing holds any of its definitions, no
call runs in it and no bytes it answered are held, or once the destroy has destroyed the core; a close the platform
refuses goes to the error sink. A process maps a library once however often it is opened, so two hosts loading the
same binary keep it open for each other.

## The plugin seam

Rust has no stable ABI, so a plugin built as a library of its own, a `cdylib` with its own copy of this crate, is
reached through size-prefixed C tables of `extern "C"` functions, never through Rust types. The plugin exports its
entry with `weivel_rust::export_entry!`, under `weivel_rust_entry` or a symbol of its choosing, and
`Host::load_entry(path, symbol)` answers an `Entry` the host registers as any other. Each activate is handed the host's
context table of its plugin, which the plugin's side wraps in the same safe `Context`. A service object crosses as its
provider's address, so identity is the address: a consumer in the host's binary receives a `ForeignObject` or, for a
described definition, its `BoundDefinition`, a plugin across the seam receives the address of the host's own object, and a typed view of one is a `#[repr(C)]` struct both
sides compile. Every function on both sides catches a panic: a panic in a plugin's activate or deactivate is told to
the host's error sink and finishes it as failed, so the host quarantines that plugin and goes on. The library stays
open until the host is freed, since a plugin's code may still run after its release.

The first tables carry a get, a registration, awaiting releases or not, an unregistration, a release of a holding, a
capability provided, the log and the error sink, and the events of a revocation and of holdings released. A context
across the seam answers a want, a watch, a listing of ids or of implementations, and a capacity answerer as the
caller's mistake, since its table carries no function for them, and its manifest carries the plugin's import name
alone.
