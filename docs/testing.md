# `testing`

Helpers for the tests of a program that hosts plugins in Rust, as the core's `docs/contract/testing.md` asks of a
language's `testing` module. They name nothing of any one project, and no other module of the crate needs them. Its
files are listed in [its source doc](source/src/testing/README.md).

## Its surface

| item | what it is |
|---|---|
| `scratch_home(layout) -> Result<ScratchHome, TestingFailure>` | a fresh folder under the temporary directory, named by its real path, laid out as `layout`, a `ScratchLayout`, describes it: `environment(base)` set, each of `libraries()` (a source folder holding one binary and a `library.json`) copied to `<base>/libraries/<name>`, and `files(base, libraries)` written. A layout failing partway, or a library source holding no binary, puts the environment back and deletes the folder before the failure is answered, which names the source folder for a missing binary |
| `ScratchFile::Written`, `ScratchFile::text`, `ScratchFile::Copied` | a file a layout writes with the bytes or the text it gives, or copies byte for byte, its folders made first. A value meant as JSON is written as its text, since the crate has no JSON writer |
| `ScratchHome::remove()` | puts every variable the layout set back, removing one that was unset, and deletes the folder |
| `set_variables(variables) -> VariablesSet`, `VariablesSet::restore()` | sets each variable of the process in order, and puts each back, removing one that was unset; a name set twice in one call gets back what it held before the first. Sets are restored in the reverse order of their setting, since each restore puts back what it saw before its own set. Dropping a `VariablesSet` puts nothing back |
| `described_library(directory, options) -> Result<DescribedLibrary, TestingFailure>` | loads the C library in `directory`, its one binary beside its `library.json`, on a host of its own over the core and the standard `options` names, given the standard's description check, so a test calls any described library's operations with no binding written by hand. A declaration the standard refuses is a failure naming its path |
| `DescribedLibrary::described()`, `get(id)`, `close()` | each service declared with a description, bound as a plugin gets it, by id; `close`, as dropping it does, destroys that host, so a call through one of its bound services answers `Unloaded` |
| `free_port() -> Result<u16, TestingFailure>` | a port of the loopback address nothing listens on at that moment |

## The one `unsafe` operation

Rust 2024 makes setting a variable of the process `unsafe`: another thread reading the environment meanwhile, the C
code of a loaded library among them, is undefined behaviour on POSIX systems. The module sets and removes a variable
in one function, `put`, and every function that reaches it, `set_variables`, `VariablesSet::restore`, `scratch_home`
and `ScratchHome::remove`, is `unsafe`, its caller promising that no other thread reads or writes the environment
while it runs. A suite whose tests run on threads of one process keeps that promise by having each test take one
lock, as the crate's own tests of the module do. That is also why a `VariablesSet` restores nothing when dropped: a
drop cannot be `unsafe`, so it could not carry the promise.

## Where it differs from the other languages

TypeScript's `scratchHome` writes a value that is not a string as JSON; Rust's layout writes the text it gives,
because the crate carries a JSON reader only and no JSON crate. A described library needs the core's and the
standard's paths, which TypeScript defaults: the crate loads every binary from the path its caller names and has no
default location.
