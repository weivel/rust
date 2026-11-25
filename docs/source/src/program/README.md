The `program` module: the host program, which carries out the standard's `weivel/host.h`: a run from `start` to `stop`, its plugins, their scopes, the stop request and the `weivel-host` command line. It is allowed `unsafe` in one file, `signals.rs`, which hooks the platform's terminate signals through functions declared by hand. What it guarantees is [the program doc](../../../program.md)'s.

<!-- generated: source-children -->
- [bin.rs](../../../../src/program/bin.rs) - the `weivel-host` binary, which runs the command line
- [command_line.rs](../../../../src/program/command_line.rs) - the command line: its words read into a run's options, the core and the standard beside the binary, the terminate signals hooked
- [mod.rs](../../../../src/program/mod.rs) - the module's public surface
- [options.rs](../../../../src/program/options.rs) - `RunHostOptions`, the exit code of a failed run, and how the program tells what went wrong
- [plugins.rs](../../../../src/program/plugins.rs) - the plan's plugins: each registered from its manifest, its scope opened, activated alone, its action or method found, and every one released
- [run.rs](../../../../src/program/run.rs) - one run from `start` to `stop`: the plan carried out, serving or the action, and `start_host`, `RunningHost` and `run_host`
- [scopes.rs](../../../../src/program/scopes.rs) - each plugin's scope, asked of the first library serving `weivel.scope` and offered to that plugin alone
- [signals.rs](../../../../src/program/signals.rs) - the platform's terminate signals, hooked through functions declared by hand, as a run's stop request
- [stop.rs](../../../../src/program/stop.rs) - `StopRequest`, and the clock the program reads
<!-- end generated -->
