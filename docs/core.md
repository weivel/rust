# `core`

How the Rust crate loads the core and makes every call into it. The rules the module keeps in any language are the
core's `docs/contract/core.md`. The crate names no function of the core but here, which is why one build of it runs
against whichever core binary its caller loads.

`CoreBinary::load` opens the core's shared binary with the platform's own loader, `dlopen` or `LoadLibraryW`, both
declared by hand, and resolves every function the core exports into one table. A path that is no library and a
library that lacks a function are each answered as a `LoadFailure`, the second naming the first function missing. The
library is closed when the last core made through it and the last clone of the binary are gone.

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
  is released and never heard;
- a call answers its result, or an `Answer`: `Absent`, `Refused` with the refusal the core queued for it, read from the
  call's own drain and never invented, `Destroyed`, `Defect`, which also goes to the error sink, `Mistake`, the
  caller's own error found before anything reached the core, such as a text holding a NUL character, and `Detached`,
  answered only through a context. A malformed answer to a call that carries something the caller built, a manifest,
  an id or options, is the caller's `Refused` with status `Malformed`; to a call the crate built whole it is a
  `Defect`;
- events are dispatched to the listeners first in, first out, outside the lock, by one thread at a time; a call made
  while a dispatch runs, from a listener or from another thread, joins the end of the queue, and `Core::defer` puts a
  step there;
- an event whose texts are not UTF-8 is still heard, marked `incomplete`, every handle, code and number exact and a
  fixed sentence in place of each such text;
- what a listener, a step or a deadline hook panics with goes to the `ErrorSink` given at creation, which writes a
  line to standard error when none is given, and a sink that panics is reported to standard error;
- nothing of the caller's runs under the lock, and what a destroy drops is dropped after the lock is let go, since a
  dropped closure may itself call in;
- the wrapper records each deadline announcement as it drains it. `Core::install_deadline_hooks` gives it a clock, read
  before a call only while a deadline is announced so a due deadline reaches the core first, and a `follow` hook told
  the latest deadline after the lock is let go, by one thread at a time; no event of a call that drained an
  announcement is dispatched before `follow` has returned with it.
