# `core`

How the Rust crate loads the core and makes every call into it. The rules the module keeps in any language are the
core's `docs/contract/core.md`. The crate names no function of the core but here, which is why one build of it runs
against whichever core binary its caller loads.

`CoreBinary::load` opens the core's shared binary with the platform's own loader, `dlopen` or `LoadLibraryW`, both
declared by hand, and resolves every function the core exports into one table. A path that is no library and a
library that lacks a function are each answered as a `LoadFailure`, the second naming the first function missing. The
library is closed when the last core made through it and the last clone of the binary are gone.

The same loader opens a C library the host loads as host services, in `library.rs`: each declared symbol resolved by
name, a bytes function called with its input lent and its answer read in place as `LibraryBytes`, which release
themselves through the library's release symbol, and a definition and a description answered once. Every holder of the
binary holds a reference, so it closes with the last one.

## The hand declarations

Every struct of the core's header is a `#[repr(C)]` struct written by hand, and every function a field of the table
typed by hand; nothing is generated, so building the crate needs no libclang. `core::declarations` describes them: the
layout of each struct, read from the declarations themselves with `offset_of!`, and the name of each function. A test
compares every field's offset and size, in declaration order, and every struct's size with what the core's `layouts`
program prints, and fails when the core prints a struct the crate does not declare; another holds the table to the
functions the pinned header declares, in order. A function's type has no printer, so a prototype written wrong is
caught only by the tests that call it.

## The wrapper

`Core::create` makes a core through a loaded binary, and every call of the core is a method of `Core` that goes
through one wrapper:

- a call is made under one lock and its events are drained under the same hold, each copied into an `Event` of the
  crate's own values and released in the core before the next is drained; an event of a kind the crate does not know
  is released and never heard, though it takes its place in the sequence, so the sequences heard may skip one;
- a call answers its result, or an `Answer`: `Absent`, `Refused` with the refusal the core queued for it, read from the
  call's own drain and never invented, `Destroyed`, `Defect`, which also goes to the error sink, `Mistake`, the
  caller's own error found before anything reached the core, such as a text holding a NUL character, and `Detached`,
  answered only through a context. A malformed answer to a call that carries something the caller built, a manifest,
  an id or options, is the caller's `Refused` with status `Malformed`; to a call the crate built whole it is a
  `Defect`;
- events are dispatched to the listeners first in, first out, outside the lock, by one thread at a time; a call made
  while a dispatch runs, from a listener or from another thread, joins the end of the queue, and a deferred step of
  the crate's own, which the timer and the host put there, runs in its turn;
- an event whose texts are not UTF-8 is still heard, marked `incomplete`, every handle, code and number exact and a
  fixed sentence in place of each such text;
- what a listener, a step or a deadline hook panics with goes to the `ErrorSink` given at creation, which writes a
  line to standard error when none is given, and a sink that panics is reported to standard error; a write to a
  closed standard error is dropped rather than panicking, so a dispatch never stops halfway;
- nothing of the caller's runs under the lock, and nothing of the caller's is dropped under it, since a dropped
  closure may itself call in: what a destroy, a removal or a deadline turn lets go of is dropped after the lock is;
- the wrapper records each deadline announcement as it drains it. The deadline hooks give it a clock, read before a
  call only while a deadline is announced so a due deadline reaches the core first, and a `follow` hook told the
  latest deadline after the lock is let go, by one thread at a time; no event of a call that drained an announcement
  is dispatched before `follow` has returned with it.

A call of the crate's own modules may make several calls of the core in one hold of the lock, each drained as it
returns, and do its own work between them: the contexts make the record of a handle the core answered there, before
any event naming it is dispatched, decide there whether a plugin is forgotten, and verify and quarantine in one hold.
The host may also start a plan as the dispatcher, so what its activates queue is dispatched once the last has been
called.

The holds, the deferred steps and the deadline hooks are the crate's own, `pub(crate)`, for the timer, the contexts
and the host to build on:
the host hands its caller nothing of the wrapper's dispatch, clock or hooks. They are tested by the module's unit tests
in `src/core/wrapper_tests.rs`, the rest by `tests/core`. `core::declarations` is public only so those tests can hold
the hand declarations to the core, and is hidden from the crate's documentation.
