How many heap allocations one described call and one library call make, counted on the test's own thread and pinned, so a call that slows by allocating fails. Why it is pinned is [the descriptions doc](../../../descriptions.md)'s.

<!-- generated: source-children -->
- [main.rs](../../../../tests/allocations/main.rs) - an allocator counting on the test's thread, and the pinned counts of `reflect`, `negate` and a bytes call
<!-- end generated -->
