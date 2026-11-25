//! Variables of the process set for a test and put back: the module's one site of `unsafe` code.

use std::ffi::{OsStr, OsString};

/// The variables one [`set_variables`] set, each with what it held before. Dropping it puts nothing back, so a
/// variable stays as set until [`VariablesSet::restore`].
#[must_use = "only restore puts the variables back"]
#[derive(Debug)]
pub struct VariablesSet {
    saved: Vec<(OsString, Option<OsString>)>,
}

/// Sets or removes one variable of the process.
///
/// # Safety
///
/// No other thread may read or write the environment while it runs.
unsafe fn put(name: &OsStr, value: Option<&OsStr>) {
    // SAFETY: the caller promises that no other thread touches the environment meanwhile.
    unsafe {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}

/// Sets each variable of the process, in order, and answers what each held before.
///
/// Restore sets in the reverse order of the setting: each restore puts back what it saw before its own set, so
/// restoring an earlier set before a later one leaves the earlier set's values behind.
///
/// # Safety
///
/// No other thread may read or write the environment while it runs, the C code of a library loaded in the process
/// included, since setting a variable then is undefined behaviour on POSIX systems.
pub unsafe fn set_variables<Name: AsRef<OsStr>, Value: AsRef<OsStr>>(
    variables: &[(Name, Value)],
) -> VariablesSet {
    let mut saved = Vec::with_capacity(variables.len());
    for (name, value) in variables {
        let name = name.as_ref();
        saved.push((name.to_owned(), std::env::var_os(name)));
        // SAFETY: the caller's promise is this function's own.
        unsafe { put(name, Some(value.as_ref())) };
    }
    VariablesSet { saved }
}

impl VariablesSet {
    /// Puts every variable back as it was before the set, removing one that was unset; a variable named twice in one
    /// set gets the value it had before the first.
    ///
    /// # Safety
    ///
    /// As for [`set_variables`]: no other thread may read or write the environment while it runs.
    pub unsafe fn restore(self) {
        for (name, value) in self.saved.iter().rev() {
            // SAFETY: the caller's promise is this function's own.
            unsafe { put(name, value.as_deref()) };
        }
    }
}
