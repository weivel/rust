//! The core's enumerations, each a number that travels as `uint32_t`. Each is a newtype rather than a Rust `enum`, so a
//! value a newer core adds is carried as it came instead of being refused.

macro_rules! numbered {
    ($(#[$meta:meta])* $name:ident { $($(#[$value_meta:meta])* $value:ident = $number:literal,)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
        pub struct $name(pub u32);

        impl $name {
            $($(#[$value_meta])* pub const $value: $name = $name($number);)*
        }
    };
}

numbered! {
    /// Why the core refused something; each code says what its refusal's subject names.
    RefusalCode {
        /// No refusal.
        NONE = 0,
        /// A call the core could not read.
        MALFORMED_CALL = 1,
        /// A handle whose object is gone.
        STALE_HANDLE = 2,
        /// A manifest the core could not accept as written.
        MALFORMED_MANIFEST = 3,
        /// A plugin this host cannot run.
        INCOMPATIBLE = 4,
        /// An import name already registered.
        NAME_TAKEN = 5,
        /// A need nothing meets.
        UNMET_NEED = 6,
        /// A plugin named twice.
        REPEATED_PLUGIN = 7,
        /// A plugin in the wrong state for the call.
        PLUGIN_STATE = 8,
        /// An identity already taken.
        IDENTITY_TAKEN = 9,
        /// An id outside the plugin's namespace.
        OUTSIDE_NAMESPACE = 10,
        /// A service id already registered.
        SERVICE_TAKEN = 11,
        /// Something the plugin did not declare.
        UNDECLARED = 12,
        /// A provider other than the one named.
        WRONG_PROVIDER = 13,
        /// A plugin no longer running.
        STOPPED = 14,
        /// A want that ran out of time.
        TIMED_OUT = 15,
        /// A constraint no provider met.
        UNMET_CONSTRAINT = 16,
        /// A service nobody serves.
        NOT_SERVED = 17,
        /// An activation that does not match its declarations.
        ACTIVATION_MISMATCH = 18,
        /// A capability provided twice.
        PROVIDED_TWICE = 19,
    }
}

numbered! {
    /// How much a log event matters.
    LogLevel {
        /// The core's default.
        DEFAULT = 0,
        /// Detail for debugging.
        DEBUG = 1,
        /// Information.
        INFO = 2,
        /// Something worth a look.
        WARNING = 3,
        /// Something failed.
        ERROR = 4,
    }
}

numbered! {
    /// What a service watch heard.
    WatchChange {
        /// The service was registered.
        REGISTERED = 1,
        /// The service was unregistered.
        UNREGISTERED = 2,
    }
}

numbered! {
    /// What a plugin reached for.
    ReachKind {
        /// A service.
        SERVICE = 1,
        /// A capability.
        CAPABILITY = 2,
    }
}

numbered! {
    /// What a capacity question asks of a provider's object.
    CapacityQuestion {
        /// Its live attributes.
        ATTRIBUTES = 1,
        /// Whether it serves the consumer under the constraints.
        SERVE = 2,
    }
}

numbered! {
    /// What a capability watch heard.
    CapabilityChange {
        /// An implementation joined.
        PROVIDED = 1,
        /// An implementation left.
        WITHDRAWN = 2,
    }
}

numbered! {
    /// How a constraint compares against its attribute.
    Bound {
        /// The attribute must reach the constraint.
        LOWER = 1,
        /// The attribute must not pass the constraint.
        UPPER = 2,
    }
}

numbered! {
    /// Which axis of compatibility a support answer refused on.
    Axis {
        /// None: supported.
        NONE = 0,
        /// The applications the host hosts.
        APPLICATION = 1,
        /// The api level.
        API = 2,
    }
}

numbered! {
    /// Which declarations an activation plan draws its edges from.
    EdgeSource {
        /// Capabilities.
        CAPABILITIES = 1,
        /// Services.
        SERVICES = 2,
    }
}

numbered! {
    /// The condition of whatever a handle names.
    HandleCondition {
        /// A want still waiting.
        WAITING = 1,
        /// A want that settled.
        SETTLED = 2,
        /// A want withdrawn.
        WITHDRAWN = 3,
        /// A watch still watching.
        WATCHING = 4,
        /// A watch whose plugin stopped.
        STOPPED = 5,
    }
}

numbered! {
    /// Where a plugin stands in its life.
    PluginStatus {
        /// Registered, not yet active.
        REGISTERED = 1,
        /// Active.
        ACTIVE = 2,
        /// Quarantined.
        QUARANTINED = 3,
        /// Released.
        RELEASED = 4,
    }
}
