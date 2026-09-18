# Starting and ending a program

Two things here start a program the app must be able to end outright: a scratchpad's run
(`src/scratchpad.rs`) and the language server (`src/lsp.rs`). Everything above the spawn used to be
written twice, once each, and the two copies drifted -- most visibly into two `stop_all`s, of which
one exit path called one. It is one runner now, `src/process.rs`, and neither consumer holds a
process of its own: `start` is the one spawn, `Handle` the one thing that ends what it made,
`stop_all` what the shutdown calls, and `read_on_thread` the one place a child's pipe is put on a
thread.

## A stop kills, and it takes the process

`Handle` is what `start` hands back, cloneable and cheap, and the only thing that ends the program.
The process and the group it was started in sit together in one `Mutex<Option<(Child, Group)>>`,
and the stop **takes** them out. So the second stop of a program is the no-op the first made it, a
killed program is waited for exactly once -- that wait is what keeps it out of the process table
until the app ends -- and no stop can name a pid the system has since handed on. Dropping a handle
does nothing at all, `Child`'s own `Drop` neither waiting nor killing, so a program abandoned rather
than stopped goes on running with nothing left that could find it.

A stop is also how a thread parked in a read is let go: the pipes close with the process, so the
read ends instead of waiting for an answer that is never coming. That is why the language server is
killed on the UI thread and the worker only told afterwards, and why the handle reaches the app at
the spawn rather than at the handshake -- the handshake being one of the reads a worker can be
parked in.

**Whether it is already over is read under the lock it is set under.** The other way a process
stops being there is `try_wait` reaping it, and that is what the flag records, under the same lock.
A stop that read the flag first and then waited for the lock would go on to signal a group whose
last member has just been reaped, and the system is free to have handed that pid on -- to a group
leader of its own, which every other program started here is. Stop pressed as a program exits by
itself is the ordinary way into that window.

## The two reaps, which had to become one

The two consumers ask different questions of the same process. A run **polls and reports**: its
pipes reach their end, the last reader reaps, and the one `Ended` says how it went -- exited with a
status, stopped, or could not be waited for. The server is **waited for after a kill**, and asked
how it ended only when a handshake has already failed, to tell a program that would not start from
a server that stopped answering.

`Handle::ended` is the first and `Handle::ending` the second, and they are **one wait** apart:
`Handle::wait` polls the non-blocking `look`, which does the reaping under the lock, and gives up at
a deadline or never. `ended` is that with no bound and `ending` with one, so a change to the poll --
a backoff, a wait on the child after a kill -- is one edit. `look` answers `Ended` itself, the one
enum for the three outcomes; it used to answer a private `State` that existed only to be translated
twice, once per caller, and the meaning of `Stopped` with it.

What reconciles the two callers is that a stop takes the process: **a process no longer under the
lock was taken by a stop, which waited for it there**, so a run's reaper reads "taken" as
`Ended::Stopped` and has nothing left to wait for. That replaced a separate "was it stopped" flag
the run used to keep. It also means a run whose reader thread could not be started is reported as
stopped rather than as an exit with no code, which is the truer of the two: the app ended it. The
handshake reads that same `Ended::Stopped` as "not ended by itself" -- a program this app killed is
not a program that would not start -- and words the other two for the failure it is there to carry
(`ended_by_itself`, `src/lsp.rs`). What that loses is `ExitStatus`'s own `Display`, which names a
signal on Unix. `Ended::Exited` carries the exit code alone because a test has to be able to write
one, and `ExitStatus` has no portable constructor.

## The group

**A stop reaches the grandchildren too.** A scratchpad is a buffer someone is experimenting in and
`Command::new` is an ordinary thing to experiment with; rust-analyzer forks `cargo`, `rustc` and a
proc-macro server of its own. A stop that killed only the process this app holds a handle for would
leave the rest running with nothing that could ever find them: the grandchild's pid was never
anywhere but inside the program that is now gone.

`Group` is that one idea with two implementations and the same three moments: something before the
spawn, something taking hold of what was spawned, and a kill. On Unix it is
`Command::process_group(0)`, std's own, so only the kill needs a crate, and `libc::kill(-pgid,
SIGKILL)`, the group being the child's own pid and the negative guarded, since `-1` is every process
this user may signal. On Windows it is a **kill-on-close job object**, created and assigned right
after the spawn, and closing the app's only handle to it is the kill. The sliver between the spawn
and the assignment is accepted rather than bought back with `CREATE_SUSPENDED` and a `ResumeThread`,
for a window a scratchpad's program does not use, and a job the system refuses leaves the stop
exactly what it was. The child's own kill stays, under the same lock and after the group's, as what
a refused job or a third platform still gets. The kill is **by value**: the group a stop took out
from under the lock is one nothing else can reach, so there is no second kill to make harmless, and
the Windows half no longer carries a mutex of its own to make it so.

## The one list, and the one way down

Every handle `start` makes goes on a `static` list, because the window's close hook is a `Send`
callback that can read no UI state -- `project.rs`'s `flush` is there for the same reason -- and a
child outliving the app holds a terminal, a port or a file the next run will want with nothing able
to find it again. **A handle leaves the list the moment it is known to be gone**, stopped or reaped,
and by that one rule (`Handle::forget`): the list is short and a `stop_all` is a walk over what is
really running. It used to be two rules -- the reap took its own handle off, and the next `start`
pruned every finished one -- so a stopped handle sat there until something else was started, and a
language server stopped and never started again was still on the list at the shutdown, where
`stop_all` signalled a pid the system was free to have handed on.

`shutdown::before_exit` is the whole of the end of the process: the project and the settings
flushed, then `stop_all`. One list, so the sequence cannot be half-copied. The window's close hook
and the panic hook's shutdown thread are the two ways the app comes down and both call it; the
30-second autosave in the Project view calls `flush` alone, a switch not being an exit.

## The pipes

Three threads own a child's pipe: a run's two readers, the server's stderr, and the server's own
answers. Two of them used to be bare `std::thread::spawn`, so a panic in one reached `crate::panics`
as an anonymous thread, which is the one thing the naming rule exists to prevent -- and `spawn`
itself panics where a thread will not start. `read_on_thread` is the one place, named, and it hands
back an `io::Result` so a thread that would not start is an answer. What a reader does with what it
reads is its own: lines for a run, the first few kilobytes for a program's last words, whole
messages for a conversation.

**A run's two readers are made here**, by `run`: the spawn, both pipes on threads of their own,
the lock the two of them share so the streams interleave in the order the program wrote them, and
the count that says when the run is over. Only the `Command` is the caller's -- what the program
is, where it runs, what it is given on stdin -- and `run` sets the two output pipes itself so a
caller cannot forget one. It lived in `src/scratchpad.rs`, which meant the invariant this file
states -- `Ended` said exactly once -- was kept in a file about cargo packages. The language server
does its own end-of-pipe accounting, and a third program the app started would have been a second
copy of the run's.

**A reader that will not start is a reader that has finished.** The run's count of pipes still open
has to reach zero however a thread ends, or the process is never reaped, the one `Ended` is never
said, and the caller reads "running" for ever over a zombie. The pipe went with the closure that
could not be spawned, so nothing would read that stream either: the run is stopped rather than left
half-read, which also bounds the reap when the failing side is the last one. The server's reader is
the same shape: nobody will read it, so the conversation is over before it began and the closed
channel is what says so, rather than a wait with no end to it.

## What a run's output is cut into

`Stream`, `OutputLine`, `RunOutput`, `RunEvent` and `Ended` are the run-shaped reading of a pipe,
and they live here beside `run` and the reader it starts, being about a program's output and not
about whatever asked for the program. **Two bounds, and each is a different failure.** `MAX_LINE`
(4 KiB) cuts a line with no newline in it, so a program writing megabytes in one line is still
*delivered* rather than accumulated. That cut falls **between characters**: a byte count lands
wherever it lands, and a multi-byte character straddling it would be a replacement character on
each of the two rows with the character itself on neither, so what is left of one is carried to the
front of the next read. Only
an incomplete sequence at the end is carried -- bytes that are genuinely invalid go through lossily,
as what a program writes is not this app's to reject. `MAX_OUTPUT_LINES` (5000) is what is kept,
oldest first out, with `RunOutput::dropped` so the view can say the story is missing its beginning;
it is a line cap and not a byte cap, because the view is a list of rows and a byte budget would make
the row count depend on how long the lines happened to be. **It is cheap to copy**, because the
pane holds it and so every batch of lines copies it before adding to it: the lines are kept in full
blocks of 256, each behind an `Arc` and never written again, and a tail shorter than one. A copy is
a pointer per block and the tail rather than all 5000 lines.

## What is not tested

The two reaps are, against `/bin/sh`: a program that ends by itself is reaped with the status it
left, and one this app stopped reads as stopped and comes off the list. Both go through `run`, so
what those tests pin is the count as well -- `Ended` said exactly once and last, with nothing to
say on either pipe as much as with both of them written to. The list's one rule is pinned on its
own, either way in: a stop takes its handle off at once, and so does a bounded wait that found the
program gone. Nothing short of a real program says
whether a stop killed anything *else*, and building one means running cargo, which no test here
does, so the group and what a stop reaches are judged by hand. The Windows half is judged
by inspection: nothing in this repo runs there.
