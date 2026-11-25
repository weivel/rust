# `context`

What a plugin sees of the core, as the core's `docs/contract/context.md` asks of a language's `context` module. Its
files are listed in [its source doc](source/src/context/README.md).

`Context` is what a plugin sees of the core, and the only way it reaches any of it: it gets, wants, watches, lists
and registers services, provides, lists and watches capabilities, listens to what the core reports about it (a
revocation, a refusal of reach, a degraded settlement), writes to the host's log under its own name, and reads its
manifest. A resolved service or implementation is the provider's own `Object`, an `Arc` compared by `Arc::ptr_eq`. A
refusal of the core is answered as the core gave it, never as an empty answer, and a get that finds nothing answers
`Absent`. A registration may offer a `CapacityAnswerer`, and one that offers none answers a constrained want's
questions with no live attributes and that it serves everyone. What a plugin's listener, watcher or answerer panics
with goes to the error sink and every other still hears each change.

A plugin reads the enums of a definition a description bound through its context: `enums_of` answers the
`DescribedEnums` of a `BoundDefinition`, each enum by the name its header spells, `weivel_status` beside them, and none
for any other object, and `require_enums` answers exactly the enums and entries a plugin names, the caller's mistake for
the first one the description lacks, as [descriptions.md](descriptions.md) states.

A want answers a `Pending`, a result that comes later on `std` alone: it is a `Future`, it can be waited for with a
blocking `wait`, and it takes a callback, so any runtime and plain threads all use it. A want settles once, from a
dispatch step and never inside the call that made it, with the provider's object or the core's refusal; a want that
needs the time on a host with no clock settles as the caller's mistake.

Every call that reaches the core is made in one hold of the layer lock with the record it needs: the room is taken
before, the record made between the core's answer and the dispatch of its events. Whether the plugin is forgotten is
decided under the same lock, so a call through a forgotten plugin's context answers `Detached` without reaching the
core.

`Contexts` is the surface a host builds them through: `make` builds a plugin's context no later than its first open,
`open` opens it with its token as payload, `verify` verifies it and, when the core refuses, quarantines it in the
same hold, `release` and `quarantine` make that call and queue the end of its entries behind the call's own events,
after which it hears nothing until it is opened again, and `forget` forgets it and detaches its context in the same
hold. One listener on the core routes every event to the plugin it concerns, by the handle it names, and the core's
own log events to the host's log.

A registration made with `register_awaiting_releases` keeps each holding past its unregistration until the holder
lets go: the holder hears `PluginEvent::Revoked` whose `Revocation` awaits its release, and hands it to
`release_holding` once it no longer uses the object, and the owner then hears `PluginEvent::HoldingsReleased`, as it
hears any event about it: one its own release or quarantine queued still reaches it, one queued after those reaches no
one. `Contexts` registers the host's own services under `WEIVEL_HOST` through a context of the host's, and a C
library's definitions awaiting releases, whose last release lets go of the library's binary. A watch takes the room of
each holding it hears as it hears it, and a watch that cannot is told to the error sink as an unknown object, where a
want takes its room when it is made.

A context of a plugin across the plugin seam is the same `Context` over the host's context table rather than over the
core: what the table carries is in [the host doc](host.md), and every other call answers the caller's mistake.
