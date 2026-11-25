# `timer`

Arms the core's next deadline on the caller's scheduler, as the core's `docs/contract/timer.md` asks of a language's
`timer` module: the core owns no clock and takes time only as a number its caller hands in. Its files are listed in
[its source doc](source/src/timer/README.md).

`CoreTimer` installs itself as the core's deadline hooks and follows the latest deadline announcement the wrapper
drained and nothing else: one with a deadline schedules one task for it, cancelling the one before, and one of none
cancels it. When the task runs it reads the clock and hands the core that time through the wrapper; on a scheduler
that cannot wait as long as the deadline lies away it waits as long as it can and arms again, never advancing the core
early. It also gives the wrapper the clock for its check before each call, so a deadline that has fallen due reaches
the core first whether the task ran or not. Every task carries a ticket, so a callback that runs after its task was
replaced does nothing. One core is followed by one timer: a second is the caller's mistake, as is a scheduler that can
wait no time, and a timer's destroy takes off its own hooks and nothing else.

`Scheduler` is the caller's clock and scheduler. `SystemScheduler` is the default, one thread of its own waiting on a
condition variable for the earliest task, its clock the milliseconds since it was made, read from `Instant`; its
thread runs the listeners of the core a timer hands the time. A timer made with no scheduler arms nothing and reads no
clock, and a want that needs the time is then the caller's mistake. A host its caller gives no scheduler runs on a
`SystemScheduler` it starts and closes with its destroy.
