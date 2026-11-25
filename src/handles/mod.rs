//! The table from the core's handles to the crate's own objects, as the core's `docs/contract/handles.md` asks.
//!
//! [`Handles`] keeps an entry per handle the core answered, a plugin, a service registration, a capability
//! implementation, a watch or a want, made in room a [`Reservation`] took before the call, and ended by its caller
//! when what the handle stands for ends; and the holdings, what each plugin was handed for another's registration or
//! implementation, kept by the pair of the two handles and dropped by the revocation that names them. The table
//! decides nothing and calls nothing, and its one lock is always the innermost.

mod table;

pub use table::{Entry, HandleKind, Handles, Reservation};
