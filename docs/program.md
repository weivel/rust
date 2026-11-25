# `program`

The host program, as the core's `docs/contract/program.md` asks of a language's `program` module: it carries out the
standard's `weivel/host.h` and nothing more, every decision the provider's. It holds no interface of its own to draw;
a program with one embeds `start_host` and draws while the run goes on. Its files are listed in
[its source doc](source/src/program/README.md).

## Its surface

| item | what it is |
|---|---|
| `RunHostOptions::new(host_library, core_library, standard_library)` | a run of the provider library at `host_library`, its declaration the `library.json` beside it, over the core and the standard at those paths; `arguments` are handed to `start`, `stop` is the run's `StopRequest`, `log` receives what went wrong and every log record of the host, `sink` the failures of plugins' callbacks, and `entry_symbol` names the symbol each plugin's library exports its entry under, `weivel_rust_entry` when `None` |
| `start_host(options) -> RunningHost` | starts the run on a thread of its own and answers once `started` has been told every plugin's outcome, or once the run ended without getting that far |
| `RunningHost::service(plugin, id)` | the object the plugin whose manifest's `importName` is `plugin` itself registered under `id`, and still has; `None` for a registration of another plugin or of the host, and once the run has ended |
| `RunningHost::stop()`, `RunningHost::ended()` | the stop request, and the exit code the run ends with, each a `Pending<u32>` a caller waits for, awaits or hands a callback |
| `run_host(options) -> u32` | the run from start to end, answering its exit code |
| `main(command_line) -> u32` | the `weivel-host` binary's command line, `weivel-host [--core <path>] [--standard <path>] <host library> [arguments...]`, the core's and the standard's shared libraries found beside the binary unless named, each terminate signal of the platform a stop request |

## One run

The program loads the standard and the core by path and makes a host whose description check is the standard's. It
loads the library it was started with from the declaration the standard reads beside it, withholding `weivel.host`
and `weivel.scope`, binds `weivel.host` from its description as a `HostProvider`, and calls `start` with its arguments.
A `start` that does not answer OK ends the run with exit code 1 and nothing else called. Otherwise, whatever happens
next, it calls `stop` last, once, and exits with the code `stop` answered, or 1 when `stop` does not answer OK.

A plan whose mode is neither SERVE nor ACTION, or an ACTION plan naming `action_service` without `action_method`, the
other way round, or either one empty, is malformed and ends the run before any library is loaded. The plan's libraries
are loaded in order, each withholding the same two ids, and the first that will not load ends the run. Scopes are
asked of the first loaded library serving a described `weivel.scope`, the one the program was started with first; with
none, every plugin's scope opens empty.

Each plugin of the plan is then taken in turn. Its manifest is read and validated by the standard and registered with
an entry on disk, the library its plan names, before anything else of it, and one that cannot be read or is refused is
told as REFUSED and gets no scope. Its scope is opened immediately before its activation, each definition the scope
answers offered to that plugin alone under the standard's id for it; a scope refused, or one that cannot be offered, as
when a loaded library declares one of the ids, is released again and the plugin is told as REFUSED, not activated.
Its activation is then waited out before the next plugin's scope is opened: activated is ACTIVATED, refused is
REFUSED, and an activation that failed is FAILED, its detail the reason the core's ledger keeps. `started` is told
every outcome in the plan's order.

A stop requested before `started` returned takes effect then, with no tick and no action. In SERVE mode the program
ticks at once, then at each time a tick answered, reading its clock again before each tick and waiting again while it
still reads before that time, until a stop is requested; a tick answering no next time is followed by a wait for the
stop request alone, and a `has_next` of neither 0 nor 1 ends the run. In ACTION mode it never ticks: with
`action_service` and `action_method` both set it calls the method of that name of the service object the plugin
itself registered, which is a `Methods`, and otherwise the action `action_name` among its entry's `actions`; a plugin
not activated, a service it does not provide, or a method or action it does not offer is told to `finished` as a
failure, and so is one that failed. A stop requested while the action runs takes effect once `finished` returned.

Before `stop` the program releases every plugin it registered, the last first, and then that plugin's scope, each
whatever became of the one before; after `stop` it destroys the host, which unloads every library. A text the program
tells the provider that holds a NUL is written with each NUL as `\0`, so a C text carries it whole.

## Stop requests and signals

A run's stop request is a `StopRequest`, shared by the run and whoever started it, and once requested it stays
requested; `RunningHost::stop` and the options' `stop` are the same request. The command line hooks SIGINT and SIGTERM
on POSIX systems with `signal`, and Ctrl+C and Ctrl+Break, SIGINT and SIGBREAK, on Windows with
`SetConsoleCtrlHandler`, each declared by hand, a handler only setting a flag, which a thread of the program's reads and
turns into the stop request; that file is the module's only `unsafe`.
