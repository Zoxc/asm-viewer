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

`Handle::ended` is the first and `Handle::ending` the second, both over one non-blocking `look` that
does the reaping under the lock. What reconciles them is that a stop takes the process: **a process
no longer under the lock was taken by a stop, which waited for it there**, so a run's reaper reads
"taken" as `Ended::Stopped` and has nothing left to wait for. That replaced a separate "was it
stopped" flag the run used to keep. It also means a run whose reader thread could not be started is
reported as stopped rather than as an exit with no code, which is the truer of the two: the app
ended it. `ending` reads the same taken process as "not ended by itself", which is what the
handshake wants -- a program this app killed is not a program that would not start.

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
to find it again. A handle leaves the list when it is stopped or reaped, so the list is short and a
`stop_all` is a walk over what is really running.

`shutdown::before_exit` is the whole of the end of the process: the projects flushed, then
`stop_all`. Two calls and one list, so the sequence cannot be half-copied. The window's close hook
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

**A reader that will not start is a reader that has finished.** The run's count of pipes still open
has to reach zero however a thread ends, or the process is never reaped, the one `Ended` is never
said, and the pad reads "Running" for ever over a zombie. The pipe went with the closure that could
not be spawned, so nothing would read that stream either: the run is stopped rather than left
half-read, which also bounds the reap when the failing side is the last one. The server's reader is
the same shape: nobody will read it, so the conversation is over before it began and the closed
channel is what says so, rather than a wait with no end to it.

## What a run's output is cut into

`Stream`, `OutputLine`, `RunOutput`, `RunEvent` and `Ended` are the run-shaped reading of a pipe,
and they live here beside the reader rather than in the scratchpad, being about a program's output
and not about a scratchpad. **Two bounds, and each is a different failure.** `MAX_LINE` (4 KiB) cuts
a line with no newline in it, so a program writing megabytes in one line is still *delivered* rather
than accumulated. That cut falls **between characters**: a byte count lands wherever it lands, and a
multi-byte character straddling it would be a replacement character on each of the two rows with the
character itself on neither, so what is left of one is carried to the front of the next read. Only
an incomplete sequence at the end is carried -- bytes that are genuinely invalid go through lossily,
as what a program writes is not this app's to reject. `MAX_OUTPUT_LINES` (5000) is what is kept,
oldest first out, with `RunOutput::dropped` so the view can say the story is missing its beginning;
it is a line cap and not a byte cap, because the view is a list of rows and a byte budget would make
the row count depend on how long the lines happened to be.

## What is not tested

The two reaps are, against `/bin/sh`: a program that ends by itself is reaped with the status it
left and comes off the list, and one this app stopped reads as stopped. Nothing short of a real
program says whether a stop killed anything *else*, and building one means running cargo, which no
test here does, so the group and what a stop reaches are judged by hand. The Windows half is judged
by inspection: nothing in this repo runs there.
