## What this is

The Rust language implementation of weivel, for a host written in Rust that runs plugins written in Rust. It loads the
core's binary from the path its caller gives and carries out the core's answers, held to the core's contract and to
the conformance kit's vectors. It is one Cargo package with no dependency at all. A host its caller gives no
scheduler arms the crate's own timer, on a thread of its own.

## What it is not

It is not the core and decides nothing itself: compatibility, order, reach, waiting and revocation are the core's
answers. It links no core and carries no C: the core is loaded at run time, never linked. It forces no path, file
name, variable or format on its caller.

## Where to read on

Start at [docs/README.md](docs/README.md), the implementation's architecture and the full statement of what it is and
is not, and descend: one doc per module, how to build and test the crate, and one doc per source folder.
