# `host`

Registers plugins written in Rust, activates, verifies and releases them, offers host services, loads C libraries as
host services, loads plugins built as libraries of their own across the plugin seam, and destroys them with the core,
as the core's `docs/contract/host.md` asks of a language's `host` module. Its files are listed in
[its source doc](source/src/host/README.md).

## Plugins and their lifecycle

A plugin's entry is an `Entry`: an `activate` taking the plugin's context and answering a `Pending` that settles once
the activation is done, and optionally a `deactivate` answering one the same way, and the `actions` it offers by
name. A panic or an error is a failed activation, which the host quarantines with a reason of its own; a failed
deactivate goes to the error sink and the release goes on. `Host::entry_on_disk` is the entry of a plugin built as a
library of its own at a path: its first activate loads the library across the plugin seam, and fails, saying why,
for a path that is not a regular file reached through no link or a library that does not load.

## Methods by name

Rust reflects no method of an object, so whatever is called by a name read at run time is a `Methods`: named
functions, each taking a value tree, or none for a value of kind NONE, and answering one, none, or why it failed, a
panic answered as a failure saying so. An entry's `actions` are a `Methods`, a service that offers methods by name
registers a `Methods` as its object, and `methods_of` answers the methods of such an object, or of a `ForeignObject` a
plugin across the seam registered with them. A host program's plan names an action or a method this way
([program.md](program.md)).

`Host::activate` asks the core for the plan, every service of the host's own counted as provided by the host, and
starts each plugin in its order: builds its context, opens it and calls its activate, never waiting for one before
starting the next, and returns the plan and each plugin's outcome to come. Where no dispatch runs, the activating
thread starts the plan as the dispatcher, so what an activate's own calls queue is dispatched once every activate of
the plan has been called; an activate therefore must not wait for a result such a dispatch would settle, and
`Pending::wait` for one of the crate's own results on the thread dispatching the core that settles it panics rather
than hangs, as does a wait inside a plugin's activate for that plugin's own release. An activate that settles before it returns is
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
mistake for an id a loaded C library declares or the host offers plugins one by one. `Host::offer_to_plugin` offers
an object to one registered plugin alone: every plugin offered an id shares one registration of the host's, which an
activation plan counts as provided by the host, and each plugin that resolves it is answered the object offered to
it, a plugin offered nothing under the id nothing, as the get of an unregistered id answers; the last `PluginOffer`
of an id withdrawn unregisters the registration. `Host::service_registered_by` answers the object one plugin itself
registered under an id, never a registration of another plugin or of the host. `Host::load_library` loads a C library from a `LibraryDeclaration`,
which the caller builds, and the library's path. A service of bytes functions is registered as a `LibraryService`,
whose `call` lends the input for the call and answers the library's bytes in place as `LibraryBytes`, released through
the library's release symbol when dropped and keeping the binary open until then, so there is no copy. A definition
service declaring no description is registered awaiting releases as a `ForeignObject` carrying the very address its symbol answered, called
once at the load; a definition answering NULL, or a struct whose `size` is below `2 * sizeof(void *)`, refuses the
load as the caller's mistake. A service that declares a `description` loads only on a host given a description check
in `HostOptions`, which `standard::StandardDescriptions` provides: `weivel_standard_description_check`, then
`weivel_standard_description_answers` with the definition, each refusal the caller's mistake. A described definition is
registered awaiting releases as the `BoundDefinition` bound from its description, standing for the very address its
symbol answered, as [descriptions.md](descriptions.md) states. `Host::load_library_withholding` loads a library the
same way but registers none of the ids it names, so only its caller reaches them, through `LoadedLibrary::described`
or `LoadedLibrary::services`, as a host program binds `weivel.host` and `weivel.scope` for itself. An id declared
twice, or registered under the host already by a plugin, another live library or the host itself, refuses the load.

Each contract a declaration publishes is checked by the host's description check and published to the plugins'
contexts until the library's unload, and a declaration of contracts alone registers nothing. A plugin's provide of a
capability whose contract is published is refused before the core is asked, as `Answer::ContractMismatch`, code
`contract-mismatch`, naming the first operation the contract's description lists that the implementation does not
offer, a release, an operation answering nothing, never being one to offer. The operations an implementation offers
are a `Methods`'s names or a `BoundDefinition`'s operations; one offering more is accepted, and a provide of a
capability no live library publishes a contract for is never checked. A contract published for a capability another
live library publishes one for refuses the load.

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
reached through size-prefixed C tables of `extern "C"` functions, never through Rust types. The plugin exports its entry
with `weivel_rust::export_entry!`, under `weivel_rust_entry` or a symbol of its choosing, and `Host::load_entry(path,
symbol)` answers an `Entry` the host registers as any other. Each activate is handed the host's context table of its
plugin and its whole manifest, which the plugin's side wraps in the same safe `Context`: every call of a context
crosses, gets, wants and their withdrawal, watches of services and of capabilities, the listing of ids and of
implementations, registrations awaiting releases or not and with a capacity answerer or not, unregistrations, releases
of a holding and capabilities provided, the log and the error sink with every kind of failure, and every event about the
plugin. A service object crosses as its provider's address, so identity is the address: a consumer in the host's binary
receives a `ForeignObject` or a `BoundDefinition`, a plugin across the seam receives the address of the host's own
object, or its own object back for one it registered, and a typed view of one is a `#[repr(C)]` struct both sides
compile. A described definition crosses with the description its library answered, so the plugin's side binds it into a
`BoundDefinition` of its own, once per definition, whose enums it reads through its context and whose operations it
calls as an in-process plugin does. It crosses with the host's id of its library too, and that copy holds the library
mapped through the host's context table for each call and release, as the host's own binding holds its library, so once
the binary has closed a call through it answers unloaded rather than reaching unmapped code. Before an unload closes a
library to calls, the host tells every plugin across the seam, which releases what its own bindings of that library got
while it is still mapped. A definition an answer carried, as a scope's are, crosses with the nested description it was
bound from and a mapping of its own, which a hold also enters the answer's scope through, so once the answer has ended,
as when the scope is released, a call through the plugin's binding is refused as the caller's mistake, never reaching
the provider's freed memory, exactly as a call through the host's own binding is. A hold is let go of by its mapping id
alone, so one made while the plugin's context lived is let go of after the host let go of that context too.

A plugin's object that is a `Methods` crosses with its names, told to the host through `offer_methods` before it is
registered or provided, so the host's `ForeignObject` for it offers each, a call back across the seam to the plugin's
object by its address through its context; the entry's actions cross the same way, the entry table answering their
names and calling one. A method's arguments and its answer cross as a value tree in the seam's own encoding of bytes,
never JSON, and a contract's refusal crosses whole, its capability and operation kept.

Each table is led by its `size`. A side refuses a table below its first shape, the fields it was first published with,
reads a later field only where the other side's `size` reaches past it, and never reads past that `size`, so either side
may append fields without breaking the other. Neither side keeps a pointer into the other's memory past a call: each
table it receives is copied, every text and list is lent for one call, of any length, and a call's answer is handed to
the caller's own receiver before the call returns. A context and a completion cross as the host's id of them, found in
the host's registry, so a call or a finish after the host let go of either answers destroyed or does nothing. When the
host forgets a plugin it lets go of what it kept of it and tells the plugin's side to close and detach that context.
When the host drops an entry, the seam's own thread tells the plugin's side to free its table and closes the library
once no other entry of it is loaded, after every call any plugin made into the host has returned, so neither ever runs
inside a call that returns into the library's code. A plugin's `free` returns only after every thread it started has
ended, and the host frees it on its retiring thread only after no plugin call into the host runs, so its library never
closes under its own code; the library is never pinned, so a plugin stays replaceable while the host runs. The plugin's
side keeps the rule for threads started through `plugin_thread` while the seam calls into the entry, or from such a
thread: the entry's `free` joins each, as a thread that finishes a deactivate and runs on after the finish needs.
Everything the host keeps of the seam is the host's own and goes with it, whether or not it was destroyed first.

Every function on both sides catches a panic: a panic in a plugin's activate or deactivate is told to the host's error
sink and finishes it as failed, so the host quarantines that plugin and goes on. A want made across the seam knows
whether the calling thread dispatches the host's core, so a plugin waiting for it there panics rather than hangs, as
an in-process one does.
