//! The one native call an operation makes and the call of a release, typed at the call from the kinds its parameters
//! hold, since Rust builds no function type at run time.

/// One parameter after the context, as the platform's C convention passes it: every integer and pointer in a 64-bit
/// word, a `double` as itself.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Slot {
    Word(u64),
    Float(f64),
}

/// The most parameters an operation may take, its context included: a description holding one that takes more is
/// refused at the load.
pub(crate) const MOST_PARAMETERS: usize = 7;

/// Calls `$function` with the types gathered so far, or takes the next slot's type and gathers on, so each of the
/// 2^n slot shapes of n parameters is one typed call.
macro_rules! call_typed {
    ($function:expr; [$($types:ty),*] [$($values:expr),*]; []) => {{
        // SAFETY: the caller of `call_operation` promised the function takes exactly these types.
        let typed = unsafe {
            std::mem::transmute::<usize, unsafe extern "C" fn($($types),*) -> i32>($function)
        };
        // SAFETY: as above; every pointer among the values lives across the call.
        unsafe { typed($($values),*) }
    }};
    ($function:expr; [$($types:ty),*] [$($values:expr),*]; [$head:expr $(, $rest:expr)*]) => {
        match $head {
            Slot::Word(word) => call_typed!($function; [$($types,)* u64] [$($values,)* word]; [$($rest),*]),
            Slot::Float(float) => call_typed!($function; [$($types,)* f64] [$($values,)* float]; [$($rest),*]),
        }
    };
}

/// Calls the operation at `function` with its context and `slots`, answering the `weivel_status` it returned.
///
/// # Safety
/// `function` is an operation of a definition whose binary stays mapped across the call, taking a `void *` context
/// then exactly one parameter per slot, each a 64-bit integer or pointer where the slot is a word and a `double` where
/// it is a float, and returning a `weivel_status`; every pointer a slot carries is valid for what the operation does
/// with it. `slots` holds at most [`MOST_PARAMETERS`] minus one.
pub(crate) unsafe fn call_operation(function: usize, context: usize, slots: &[Slot]) -> i32 {
    let context = context as u64;
    match *slots {
        [] => call_typed!(function; [u64] [context]; []),
        [first] => call_typed!(function; [u64] [context]; [first]),
        [first, second] => call_typed!(function; [u64] [context]; [first, second]),
        [first, second, third] => {
            call_typed!(function; [u64] [context]; [first, second, third])
        }
        [first, second, third, fourth] => {
            call_typed!(function; [u64] [context]; [first, second, third, fourth])
        }
        [first, second, third, fourth, fifth] => {
            call_typed!(function; [u64] [context]; [first, second, third, fourth, fifth])
        }
        [first, second, third, fourth, fifth, sixth] => {
            call_typed!(function; [u64] [context]; [first, second, third, fourth, fifth, sixth])
        }
        _ => unreachable!(
            "an operation of more than {MOST_PARAMETERS} parameters is refused at the load"
        ),
    }
}

/// Calls the release at `function` with its context and the one thing it frees.
///
/// # Safety
/// `function` is a release of a definition whose binary stays mapped across the call, `void f(void *, T *)`, and
/// `pointer` is what it frees, handed to it once.
pub(crate) unsafe fn call_release(function: usize, context: usize, pointer: usize) {
    // SAFETY: as the caller promised.
    let typed = unsafe {
        std::mem::transmute::<
            usize,
            unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void),
        >(function)
    };
    // SAFETY: as the caller promised.
    unsafe { typed(context as *mut _, pointer as *mut _) }
}

/// A release of one definition's struct, with that definition's context, or nothing where the provider left it NULL
/// or its struct ends before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReleaseCall {
    function: usize,
    context: usize,
}

impl ReleaseCall {
    /// The release at `function`, 0 for none, read from a definition's struct at the bind; only a definition's own
    /// release with its own context is ever made into one.
    pub(crate) fn new(function: usize, context: usize) -> ReleaseCall {
        ReleaseCall { function, context }
    }

    /// Frees `pointer`, which its caller hands over once while the binary stays mapped; a release the provider does
    /// not offer frees nothing.
    pub(crate) fn free(&self, pointer: usize) {
        if self.function == 0 {
            return;
        }
        // SAFETY: the function is the definition's release, read from its struct while its binary was mapped, and
        // its caller holds the binary mapped across the call and hands each answer, refusal or handle over once.
        unsafe { call_release(self.function, self.context, pointer) }
    }
}
