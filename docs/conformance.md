# `conformance`

The driver for the layer suite, as the core's `docs/contract/conformance.md` asks of a language's `conformance`
module. Its files are listed in [its source doc](source/src/conformance/README.md), and the suite in
[the tests' doc](source/tests/conformance/README.md).

`LayerDriver` reads the pinned kit's layer vectors through the standard's JSON reader. For each vector but a `host`
one it makes a host of its own on the real core, builds the vector's plugins as manifest values and entries, drives
them through the host's public surface, a `want` vector on a clock and scheduler of the suite's own that run a task
only when the suite moves time over it, a `library` vector through the core's `echo` test library, whose path the
caller hands `LayerDriver::with_echo_library`, a `description` vector through the standard's `described` test library
on a host given the standard's description check, both handed to `LayerDriver::with_described_library`, and answers a
`LayerWitness`: every field of the vector's `then` in its shape, so the witness equals the `then` exactly when the
run witnessed the vector, beside every log record and failure the run met. A vector, a step or a `then` field the
driver does not read fails the run rather than passing unread.

## The `host` vectors

A `host` vector runs the crate's own host program, [`program`](program.md), in process, against the kit's
`stub-host`, as the kit's `docs/vectors/layer/host.md` says; the caller hands `LayerDriver::with_host_program` a
`HostProgramFixtures` naming `stub-host`, with its declarations beside it, the core, the standard and the suite's test
plugins library. The driver lays the run out in a fresh directory: each plugin's manifest, unless the vector leaves it
unreadable, the vector's own words for each plugin under `plugins/<name>.json`, and `stub-host.json`, every record of
it in the order the vector writes it, since a plan's arguments reach `finished` as written. Every plugin's entry is
the test plugins library, loaded across the plugin seam by its symbol `host_vector_entry`, so the run exercises the
program's entry symbol beside its default. A plugin learns where its words lie from `run_pointer_file`, a file of the
process it runs in naming the standard and the run's directory, which is why the suite lays out one `host` run at a
time, as `stub-host` itself keeps one run at a time. The driver sends the stop request once the record holds
`stopAfter`, writes `stop-sent` after the program has taken it, holds every wait to the kit's 30-second bound, and
answers `record`, `exit` and `exitsNonZero` from the record and the exit code.

The test plugins realise every step, action, method and implementation the kit's host doc defines, through the
services the program offered them: `log` through `Log`, `logSetting` and `set` through `Config`, `absent` through a get
that answers nothing, `provide` writing `provided <id>`, or `refused contract-mismatch <operation>` for the refusal
the host's contract check answers.

## Which vectors the crate sits

Every one, 94 at the pinned kit, with no `layer-exempt` line: `consume` 2, `description` 6, `description/binding` 24,
`host` 32, `host/contract` 6, `library` 14, `lifecycle` 6, `want` 4. Each is one test named after its id, so the id
stands alone in the test sources as the kit's `layers` check asks, and a coverage test holds the suite's list to the
pinned kit's vector directory both ways. A `library` vector reads the `echo` library's definition through the address
the host registered, as `echo.h` lays it out, which is the one `unsafe` of the module, and reads the address each
definition symbol answers through a load of its own. A `description` vector calls only through the
`BoundDefinition` a holder resolved or an answer carried, its JSON translated into values by the kinds the
description gives each and every answer back. A process maps a library once however often it is opened, and the
`described` library counts its releases for the life of its mapping, so the suite runs one `library`, `description`
or `host` vector at a time.
