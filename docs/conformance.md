# `conformance`

The driver for the layer suite, as the core's `docs/contract/conformance.md` asks of a language's `conformance`
module. Its files are listed in [its source doc](source/src/conformance/README.md), and the suite in
[the tests' doc](source/tests/conformance/README.md).

`LayerDriver` reads the pinned kit's layer vectors through the standard's JSON reader and writes no file. For each
vector it makes a host of its own on the real core, builds the vector's plugins as manifest values and entries,
drives them through the host's public surface, a `want` vector on a clock and scheduler of the suite's own that run a
task only when the suite moves time over it, a `library` vector through the core's `echo` test library, whose path the
caller hands `LayerDriver::with_echo_library`, a `description` vector through the standard's `described` test library
on a host given the standard's description check, both handed to `LayerDriver::with_described_library`, and answers a `LayerWitness`: every field of the vector's `then` in its
shape, so the witness equals the `then` exactly when the run witnessed the vector, beside every log record and
failure the run met. A vector, a step or a `then` field the driver does not read fails the run rather than passing
unread.

## Which vectors the crate sits

So far the `consume`, `description` (its `binding` directory included), `library`, `lifecycle` and `want`
directories, 56 vectors, each one test named after its id, so the id stands alone in the test sources as the kit's
`layers` check asks. A coverage test holds the suite's list to the
pinned kit's vectors in those directories both ways. A `library` vector reads the `echo` library's definition through
the address the host registered, as `echo.h` lays it out, which is the one `unsafe` of the module, and reads the
address each definition symbol answers through a load of its own. A `description` vector calls only through the `BoundDefinition` a holder resolved
or an answer carried, its JSON translated into values by the kinds the description gives each and every answer back.
A process maps a library once however often it is opened, and the `described` library counts its releases for the
life of its mapping, so the suite runs one `library` or `description` vector at a time. The kit's `layers` drift check, which asks for every directory,
is red until the rest are sat.
