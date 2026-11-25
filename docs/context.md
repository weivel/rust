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
own log events to the host's log. Awaiting releases, which C libraries need, is not here yet.
