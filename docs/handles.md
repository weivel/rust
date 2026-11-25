# `handles`

The table from the core's handles to the crate's own records, as the core's `docs/contract/handles.md` asks of a
language's `handles` module. Its files are listed in [its source doc](source/src/handles/README.md).

`Handles` keeps an entry per handle the core answered: a plugin, a service registration, a capability
implementation, a watch or a want. The entry is made in room a `Reservation` took before the call, so making it
cannot fail once the core has answered, and it is made in the same hold of the layer lock as the call, before any
event of that call is dispatched. Its caller ends it when what the handle stands for ends: an unregistration, a
stopped watch, a settled or withdrawn want, a released or forgotten plugin. The holdings, what each plugin was handed
for another's registration or implementation, are kept by the pair of the two handles and dropped by the revocation
that names them, even for a plugin already forgotten.

Each entry carries a token never reissued, which a plugin is opened with as its payload and which leads back to the
entry without a search. A handle the table does not hold answers nothing, and an ended token is never matched to a
newer entry. The table is generic over what it keeps, so it decides nothing and calls nothing; its one lock is held
only inside its own methods and is always the innermost.
