The `program` module's tests beside the `host` vectors: the `weivel-host` command line run as a child, a terminate signal ending its run, and a running host answering the service its plugin registered, and the host program's size and startup measured.

<!-- generated: source-children -->
- [main.rs](../../../../tests/program/main.rs) - the command line run as a child, a terminate signal ending its run, and a running host answering its plugin's service
- [size.rs](../../../../tests/program/size.rs) - the size and the startup of `weivel-host` built with the `smallest` profile, with each panic strategy, beside an empty program built the same way; a target without the standard harness, so its lines reach the log
<!-- end generated -->
