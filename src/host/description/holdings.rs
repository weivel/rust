//! What outlives the call that answered it: a handle, an answer carrying a definition, and the scope a carried
//! definition lives in, each released once, by its holder, by the drop of its last part, or by its library's unload
//! while the binary is still mapped.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use super::invoke::ReleaseCall;
use super::memory::Block;
use crate::core::LibraryBinary;

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The binary a bound definition's functions lie in, or none for a definition its caller bound with no library
/// behind it, which its caller keeps mapped.
#[derive(Clone)]
pub(crate) enum Mapping {
    Library(Weak<LibraryBinary>),
    Unowned,
}

/// Keeps the binary mapped while it lives.
pub(crate) struct MappingHold {
    _binary: Option<Arc<LibraryBinary>>,
}

impl Mapping {
    /// Holds the binary mapped, or answers none once it has closed.
    pub(crate) fn hold(&self) -> Option<MappingHold> {
        match self {
            Mapping::Library(binary) => binary.upgrade().map(|binary| MappingHold {
                _binary: Some(binary),
            }),
            Mapping::Unowned => Some(MappingHold { _binary: None }),
        }
    }
}

/// Something that outlives its call and is released once.
pub(crate) trait Held: Send + Sync {
    fn release(&self);
}

static LAST_LIBRARY: AtomicU64 = AtomicU64::new(0);

/// Every handle and kept answer one library's definitions answered that nobody has released yet, so the unload
/// releases each of them while the binary is still mapped. What was got through a carried definition is held by a
/// child, which the end of that definition's answer releases; every child shares its library's identity, so a handle
/// any of them answered is taken back by every definition of the library.
pub(crate) struct Holdings {
    library: u64,
    live: Mutex<Vec<Weak<dyn Held>>>,
}

impl Holdings {
    /// The holdings of one library, of an identity of their own.
    pub(crate) fn of_a_library() -> Arc<Holdings> {
        Arc::new(Holdings {
            library: LAST_LIBRARY.fetch_add(1, Ordering::Relaxed) + 1,
            live: Mutex::new(Vec::new()),
        })
    }

    /// Holdings of the same library whose release releases only what they hold.
    pub(crate) fn child(&self) -> Arc<Holdings> {
        Arc::new(Holdings {
            library: self.library,
            live: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn library(&self) -> u64 {
        self.library
    }

    pub(crate) fn hold(&self, held: Weak<dyn Held>) {
        let mut live = locked(&self.live);
        live.retain(|kept| kept.strong_count() > 0);
        live.push(held);
    }

    /// Releases everything still held, in the order it was answered.
    pub(crate) fn release_all(&self) {
        let held = std::mem::take(&mut *locked(&self.live));
        for kept in held {
            if let Some(kept) = kept.upgrade() {
                kept.release();
            }
        }
    }
}

struct HandleInner {
    pointer: usize,
    type_name: String,
    library: u64,
    release: ReleaseCall,
    mapping: Mapping,
    live: AtomicBool,
}

impl Held for HandleInner {
    fn release(&self) {
        if !self.live.swap(false, Ordering::AcqRel) {
            return;
        }
        if let Some(_mapped) = self.mapping.hold() {
            self.release.free(self.pointer);
        }
    }
}

impl Drop for HandleInner {
    fn drop(&mut self) {
        Held::release(self);
    }
}

/// An opaque object a described operation answered, released once through the release its description names: by
/// [`DescribedHandle::release`], by the drop of its last clone, or by its library's unload, while the binary is still
/// mapped. Handed back as an input it must be live, of the same library and of the type the input takes.
#[derive(Clone)]
pub struct DescribedHandle {
    inner: Arc<HandleInner>,
}

impl fmt::Debug for DescribedHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "DescribedHandle({}, {})",
            self.inner.type_name,
            if self.is_live() { "live" } else { "released" }
        )
    }
}

impl PartialEq for DescribedHandle {
    fn eq(&self, other: &DescribedHandle) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl DescribedHandle {
    /// Wraps `pointer`, of the opaque `type_name`, which `release` frees, among `holdings`.
    pub(crate) fn hold(
        holdings: &Holdings,
        pointer: usize,
        type_name: &str,
        release: ReleaseCall,
        mapping: Mapping,
    ) -> DescribedHandle {
        let inner = Arc::new(HandleInner {
            pointer,
            type_name: type_name.to_owned(),
            library: holdings.library(),
            release,
            mapping,
            live: AtomicBool::new(true),
        });
        let held: Arc<dyn Held> = inner.clone();
        holdings.hold(Arc::downgrade(&held));
        DescribedHandle { inner }
    }

    /// Frees the handle; a later call, one after the unload, or the drop of a clone afterwards does nothing.
    pub fn release(&self) {
        Held::release(&*self.inner);
    }

    /// Whether it has not been released.
    pub fn is_live(&self) -> bool {
        self.inner.live.load(Ordering::Acquire)
    }

    /// The opaque type it is, as its description names it.
    pub fn type_name(&self) -> &str {
        &self.inner.type_name
    }

    /// The pointer it stands for, to hand an operation of `library` taking a `type_name`.
    pub(crate) fn pointer_for(&self, library: u64, type_name: &str) -> Result<usize, String> {
        if self.inner.library != library {
            return Err("it takes a handle this library answered".to_owned());
        }
        if self.inner.type_name != type_name {
            return Err(format!(
                "it takes a {type_name}, and the handle it was handed is a {}",
                self.inner.type_name
            ));
        }
        if !self.is_live() {
            return Err("it was handed a handle that has been released".to_owned());
        }
        Ok(self.inner.pointer)
    }
}

/// What a carried definition lives in: its calls counted so the end waits for them, and everything got through it
/// held, so its answer is released only once all of that has ended.
pub(crate) struct CarriedScope {
    state: Mutex<ScopeState>,
    pub(crate) holdings: Arc<Holdings>,
}

type Finish = Box<dyn FnOnce() + Send>;

struct ScopeState {
    live: bool,
    running: usize,
    when_idle: Option<Finish>,
}

/// One call through a carried definition, counted while it lives.
pub(crate) struct ScopeCall {
    scope: Arc<CarriedScope>,
}

impl Drop for ScopeCall {
    fn drop(&mut self) {
        let idle = {
            let mut state = locked(&self.scope.state);
            state.running -= 1;
            if state.running == 0 {
                state.when_idle.take()
            } else {
                None
            }
        };
        if let Some(finish) = idle {
            finish();
        }
    }
}

impl CarriedScope {
    pub(crate) fn within(parent: &Holdings) -> Arc<CarriedScope> {
        Arc::new(CarriedScope {
            state: Mutex::new(ScopeState {
                live: true,
                running: 0,
                when_idle: None,
            }),
            holdings: parent.child(),
        })
    }

    /// Counts one call, or answers none once the scope has ended.
    pub(crate) fn enter(self: &Arc<CarriedScope>) -> Option<ScopeCall> {
        let mut state = locked(&self.state);
        if !state.live {
            return None;
        }
        state.running += 1;
        Some(ScopeCall {
            scope: self.clone(),
        })
    }

    /// Refuses every new call at once; once every running call has returned, releases what was got through the
    /// definition, then runs `then`.
    pub(crate) fn end(&self, then: Finish) {
        let holdings = self.holdings.clone();
        let finish: Finish = Box::new(move || {
            holdings.release_all();
            then();
        });
        let now = {
            let mut state = locked(&self.state);
            if !state.live {
                return;
            }
            state.live = false;
            if state.running == 0 {
                Some(finish)
            } else {
                state.when_idle = Some(finish);
                None
            }
        };
        if let Some(finish) = now {
            finish();
        }
    }
}

/// What an answer carrying a definition frees at its end: its blocks, each through the release its description names.
pub(crate) struct KeptBlocks {
    pub(crate) blocks: Vec<Block>,
    pub(crate) releases: Vec<(usize, ReleaseCall)>,
    pub(crate) mapping: Mapping,
}

impl KeptBlocks {
    fn free(self) {
        if let Some(_mapped) = self.mapping.hold() {
            for (pointer, release) in &self.releases {
                release.free(*pointer);
            }
        }
        drop(self.blocks);
    }
}

/// An answer carrying a definition, kept until its release, the drop of its last part, or its library's unload.
pub(crate) struct KeptAnswer {
    live: AtomicBool,
    scopes: Vec<Arc<CarriedScope>>,
    blocks: Mutex<Option<KeptBlocks>>,
}

impl KeptAnswer {
    /// Keeps `blocks` until every scope of `scopes` has ended, held among `holdings`.
    pub(crate) fn keep(
        holdings: &Holdings,
        scopes: Vec<Arc<CarriedScope>>,
        blocks: KeptBlocks,
    ) -> Arc<KeptAnswer> {
        let kept = Arc::new(KeptAnswer {
            live: AtomicBool::new(true),
            scopes,
            blocks: Mutex::new(Some(blocks)),
        });
        let held: Arc<dyn Held> = kept.clone();
        holdings.hold(Arc::downgrade(&held));
        kept
    }

    /// Ends the answer, as [`Held::release`] does.
    pub(crate) fn release(&self) {
        Held::release(self);
    }
}

impl Held for KeptAnswer {
    fn release(&self) {
        if !self.live.swap(false, Ordering::AcqRel) {
            return;
        }
        let blocks = Arc::new(Mutex::new(locked(&self.blocks).take()));
        let remaining = Arc::new(AtomicUsize::new(self.scopes.len()));
        let free = move || {
            if let Some(blocks) = locked(&blocks).take() {
                blocks.free();
            }
        };
        if self.scopes.is_empty() {
            free();
            return;
        }
        let free = Arc::new(Mutex::new(Some(free)));
        for scope in &self.scopes {
            let remaining = remaining.clone();
            let free = free.clone();
            scope.end(Box::new(move || {
                if remaining.fetch_sub(1, Ordering::AcqRel) == 1
                    && let Some(free) = locked(&free).take()
                {
                    free();
                }
            }));
        }
    }
}

impl Drop for KeptAnswer {
    fn drop(&mut self) {
        Held::release(self);
    }
}
