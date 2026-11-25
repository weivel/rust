# `host`

Registers plugins written in Rust, activates, verifies and releases them, and destroys them with the core, as the
core's `docs/contract/host.md` asks of a language's `host` module. Its files are listed in
[its source doc](source/src/host/README.md). Host services, C libraries and the plugin seam are not here yet.

A plugin's entry is an `Entry`: an `activate` taking the plugin's context and answering a `Pending` that settles once
the activation is done, and optionally a `deactivate` answering one the same way. A panic or an error is a failed
activation, which the host quarantines with a reason of its own; a failed deactivate goes to the error sink and the
release goes on.

`Host::activate` asks the core for the plan and starts each plugin in its order: builds its context, opens it and
calls its activate, never waiting for one before starting the next, and returns the plan and each plugin's outcome to
come. Where no dispatch runs, the activating thread starts the plan as the dispatcher, so what an activate's own calls
queue is dispatched once every activate of the plan has been called. An activate that settles before it returns is
verified, or quarantined when the core refuses, before the next plugin is started. An outcome is `Active`,
`Quarantined`, `Refused`, `Released` or `Failed`, and carries the core's refusal that decided it.

`Host::release` calls the deactivate while the plugin is still whole, releases it in the core, lets every event that
release queued reach it, ends its entries, forgets it and drops its record, the last steps put on the dispatch queue
behind the release's events. `Host::destroy` releases every plugin it opened in the reverse of the order it opened
them, one whose activate is in flight included, then disarms the timer and destroys the core; every call after it
answers `Destroyed` quietly. `Host::ledger` answers the core's ledger, and the options' timer observer sees each move
of the timer.
