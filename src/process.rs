//! Every program this app starts and must be able to end outright: the group each is
//! started in, the handle that stops it, the one list a shutdown walks, the pipes read on
//! threads of their own, and a run's output cut into rows.
//!
//! Two things start programs -- a scratchpad's run (`src/scratchpad.rs`) and the language
//! server (`src/lsp.rs`) -- and they ask a process the same things, so neither has a copy
//! of any of this. [`start`] is the one spawn, [`Handle`] the one thing that ends what it
//! made, [`stop_all`] what the shutdown calls, and [`read_on_thread`] the one place a
//! child's pipe is put on a thread.
//!
//! **A stop is a kill, and it reaches the whole group.** Without a group, a stop kills the
//! process this app has a handle for and leaves everything it forked running with nothing
//! that could ever find it again -- the grandchild's pid was never anywhere but inside the
//! program that is now gone. The two platforms have the same shape and nothing else in
//! common: [`Group::arrange`] runs before the spawn, [`Group::of`] takes hold of what was
//! spawned, and [`Group::kill`] ends the lot.
//!
//! `agents/Process.md` is the reasoning.

use std::collections::VecDeque;
use std::io::{self, BufRead, Read};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
struct Group(i32);

#[cfg(unix)]
impl Group {
    /// `process_group(0)` is "a new group whose id is the child's own pid", set between the
    /// fork and the exec by the standard library. It is std's, not `libc`'s: only the kill
    /// needs a crate.
    fn arrange(command: &mut Command) {
        use std::os::unix::process::CommandExt;

        command.process_group(0);
    }

    /// Which group that turned out to be. Asked of the `Child` rather than assumed, so the
    /// number a stop signals is one the kernel handed back.
    fn of(child: &Child) -> Self {
        Group(child.id() as i32)
    }

    /// `kill(-pgid)` is the whole group. Guarded because the negative of a small number is
    /// not a group at all: `-1` is every process this user may signal and `0` is *this*
    /// app's own group, and neither can come of a real child, so neither may be reached by
    /// a pid that somehow arrived as one.
    ///
    /// By value: the group a stop takes out from under the lock is one nothing else can
    /// reach, so there is no second kill to make harmless.
    fn kill(self) {
        if self.0 > 1 {
            // SAFETY: a signal number and a pid, both plain values; `kill` reads no memory.
            unsafe { libc::kill(-self.0, libc::SIGKILL) };
        }
    }
}

/// The Windows half of [`Group`] — a job object with kill-on-close, which the child and
/// everything it starts are inside. Closing the last handle to it is the kill, so a
/// program the app somehow drops without stopping dies with the [`Process`] rather than
/// outliving it.
#[cfg(windows)]
struct Group(Option<std::os::windows::io::OwnedHandle>);

#[cfg(windows)]
impl Group {
    /// Nothing: there is no pre-spawn half here, the job is joined after the fact.
    fn arrange(_command: &mut Command) {}

    /// Assign the spawned process to a fresh kill-on-close job.
    ///
    /// The sliver between the spawn and this call is real and accepted: a program that
    /// forks in its first microseconds forks outside the job. Closing it would mean
    /// `CREATE_SUSPENDED` and a `ResumeThread`, which is a raw thread handle and a spawn
    /// this module no longer shares with `std`, for a window a scratchpad's program does
    /// not use. `None` where the system refused — a job it may not create, or a job it may
    /// not nest — and then the stop is `Child::kill` alone, exactly what it was before.
    fn of(child: &Child) -> Self {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        // SAFETY: an unnamed job with default security, owned from the moment it exists —
        // `OwnedHandle` is what closes it, on every path out of here and out of `kill`.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Group(None);
        }
        let job = unsafe { OwnedHandle::from_raw_handle(job) };

        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: the structure the information class names, and its own size.
        let set = unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        // SAFETY: two handles this call only reads; the child is alive, since `Child` holds
        // it and nothing has waited on it yet.
        let assigned = set != 0
            && unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) } != 0;

        Group(assigned.then_some(job))
    }

    /// Close the handle, which is what kills: this app holds the only one, the child never
    /// having been given it to inherit. By value, and the close *is* the drop: the group a
    /// stop takes out from under the lock is one nothing else can reach, so there is no
    /// second kill to make harmless.
    fn kill(self) {
        drop(self.0);
    }
}

/// Neither Unix nor Windows: there is no group, and a stop is the child alone.
#[cfg(not(any(unix, windows)))]
struct Group;

#[cfg(not(any(unix, windows)))]
impl Group {
    fn arrange(_command: &mut Command) {}

    fn of(_child: &Child) -> Self {
        Group
    }

    fn kill(self) {}
}

/// How often a process is asked again whether it has ended. Polled rather than waited on:
/// a blocking `wait` needs the `Child`, and holding it is exactly what would make a stop
/// wait for the process it is trying to kill.
const POLL: Duration = Duration::from_millis(20);

/// The process behind a [`Handle`], shared by the handle, whatever is reading its pipes,
/// and the list [`stop_all`] walks.
struct Process {
    /// The process and the group it was started in -- what a stop kills, and what makes it
    /// reach further than [`Child::kill`] would. Behind a `Mutex` because two stops, and a
    /// stop and whoever is waiting for the end, race by construction; **taken** by the
    /// stop that kills it, so the second stop is the no-op the first made it and a killed
    /// process is not waited for twice.
    child: Mutex<Option<(Child, Group)>>,
    /// Whether nothing more is to be done to it: stopped, or found to have ended by
    /// itself. Set under `child`'s lock and read under it, which is the whole of the
    /// guard below.
    over: AtomicBool,
}

/// A started program, as anything holding one holds it: enough to end it, and nothing to
/// talk to it with. Cloneable and cheap — the app holds one in a state it clones on every
/// render, and [`stop_all`] holds another.
#[derive(Clone)]
pub struct Handle(Arc<Process>);

/// Two handles are the same handle when they are the same process, pointer identity being
/// the only identity a process has here.
impl PartialEq for Handle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// What has become of a started program, asked without waiting.
enum State {
    /// Still going.
    Going,
    /// Gone by itself, with the status it left.
    Exited(ExitStatus),
    /// A stop took it, and that stop waited for it.
    Stopped,
    /// It could not be waited for.
    Failed(String),
}

impl Handle {
    /// Kill the program and everything it started, and wait for it to be gone.
    ///
    /// Dropping the handle would do nothing: `Child`'s own `Drop` neither waits nor kills,
    /// so a program abandoned rather than stopped goes on running with nothing left that
    /// could find it, and a grandchild is worse still -- nothing but the group ever knew
    /// its pid. It is also how a thread parked in a read is let go: the pipes close with
    /// the process, so the read ends instead of waiting for an answer that is not coming.
    ///
    /// **A program that is already over is not signalled.** `over` is read under the lock
    /// it is set under: a stop that read it first and then waited for the lock would go on
    /// to signal a group whose last member has since been reaped, and the system is free
    /// to have handed that pid on -- to a group leader of its own, which every other
    /// program started here is. The second stop is a no-op for the same reason, the first
    /// having taken the process out from under the lock.
    pub fn stop(&self) {
        let mut held = self.0.child.lock().unwrap_or_else(|held| held.into_inner());
        let over = self.0.over.swap(true, Ordering::SeqCst);
        let Some((mut child, group)) = held.take() else {
            return;
        };
        if over {
            return;
        }

        group.kill();
        // The child's own kill after the group's: it is what a platform with no group, or
        // a job object the system refused, still gets.
        let _ = child.kill();
        // It has been killed, so this returns at once, and it is what keeps a stopped
        // program from sitting in the process table until the app ends.
        let _ = child.wait();
    }

    /// Whether nothing more is to be done to it: stopped, or seen to have ended by itself.
    pub fn finished(&self) -> bool {
        self.0.over.load(Ordering::SeqCst)
    }

    /// How it ended, if it has, waiting `within` for it to finish doing so. `None` while it
    /// is still going, and for one this app stopped -- a program the app killed is not a
    /// program that ended on its own.
    ///
    /// The language server's: asked only of a conversation that has already failed, so the
    /// wait is the price of telling a program that would not start from a server that
    /// stopped answering.
    pub fn ending(&self, within: Duration) -> Option<String> {
        let until = Instant::now() + within;
        loop {
            match self.look() {
                State::Exited(status) => return Some(status.to_string()),
                State::Failed(error) => return Some(error),
                State::Stopped => return None,
                State::Going => {}
            }
            if Instant::now() >= until {
                return None;
            }
            thread::sleep(POLL);
        }
    }

    /// Wait for it to be gone however long that takes, say how it went, and take it off
    /// the list [`stop_all`] walks.
    ///
    /// A run's: the pipes have reached their end and what is left is the reap. A process
    /// no longer under the lock was taken by a stop, which waited for it, so "taken" is
    /// how a stop is read from here.
    pub fn ended(&self) -> Ended {
        let ended = loop {
            match self.look() {
                State::Exited(status) => break Ended::Exited(status.code()),
                State::Stopped => break Ended::Stopped,
                State::Failed(error) => break Ended::Failed(error),
                State::Going => {}
            }
            thread::sleep(POLL);
        };

        let mut list = STARTED.lock().unwrap_or_else(|held| held.into_inner());
        list.retain(|other| other != self);
        ended
    }

    /// What has become of it, asked without waiting.
    ///
    /// A process found to be gone is marked over **under the lock**, since `try_wait` is
    /// what reaps it: after this the pid is the system's to hand on, and [`Handle::stop`]
    /// reads the flag under this same lock so that it cannot signal a group that is no
    /// longer this one.
    fn look(&self) -> State {
        let mut held = self.0.child.lock().unwrap_or_else(|held| held.into_inner());
        let Some((child, _)) = held.as_mut() else {
            return State::Stopped;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                self.0.over.store(true, Ordering::SeqCst);
                State::Exited(status)
            }
            Ok(None) => State::Going,
            Err(error) => {
                self.0.over.store(true, Ordering::SeqCst);
                State::Failed(error.to_string())
            }
        }
    }
}

/// What a started program is talked to and read through, for whoever asked for them.
/// Whichever of the three [`Command`] was given a pipe for, and `None` for the rest.
pub struct Pipes {
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    pub stderr: Option<ChildStderr>,
}

/// Start `command` in a group of its own, register the handle, and hand back the pipes for
/// the caller to read. The one spawn.
pub fn start(command: &mut Command) -> io::Result<(Handle, Pipes)> {
    Group::arrange(command);
    let mut child = command.spawn()?;
    // The group is claimed here and never again: everything the program forks from now on
    // is born into it, so a stop reaches the whole tree and not only the process this app
    // has a handle for.
    let group = Group::of(&child);

    // Taken before the child goes behind the mutex, since whoever reads a pipe owns it
    // outright and must never need the lock a stop is waiting on.
    let pipes = Pipes {
        stdin: child.stdin.take(),
        stdout: child.stdout.take(),
        stderr: child.stderr.take(),
    };

    let handle = Handle(Arc::new(Process {
        child: Mutex::new(Some((child, group))),
        over: AtomicBool::new(false),
    }));
    {
        let mut list = STARTED.lock().unwrap_or_else(|held| held.into_inner());
        list.retain(|other| !other.finished());
        list.push(handle.clone());
    }

    Ok((handle, pipes))
}

/// Read a pipe on a thread of its own, named so that a panic on one says which thread died
/// (`crate::panics`). The one place a child's pipe is read.
///
/// `read` is handed the pipe and returns when it has read all it means to; the thread ends
/// with it. What a reader does with what it reads is its own -- lines for a run, capped
/// bytes for a program's last words, whole messages for a conversation -- and only the
/// owning of the pipe is shared.
///
/// The thread is handed back, since when it has reached the end of the pipe is what says a
/// program's last words are all in. `Err` is a thread that would not start: the pipe went
/// with the closure, so nothing will read that stream and the caller has to go on without
/// it.
pub fn read_on_thread<P: Read + Send + 'static>(
    name: &'static str,
    pipe: P,
    read: impl FnOnce(P) + Send + 'static,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || read(pipe))
}

/// Every program started in this run of the app that has not been stopped or ended. A
/// `static` because the window's close hook can be handed nothing.
static STARTED: Mutex<Vec<Handle>> = Mutex::new(Vec::new());

/// Stop every program the app started and that has not ended by itself.
///
/// For the window's close hook, which is a `Send` callback that can read no `State` --
/// `project.rs`'s `flush` is there for the same reason. A child outliving the app holds a
/// terminal, a port or a file the next run will want, with nothing able to find it again.
pub fn stop_all() {
    let started = {
        let mut list = STARTED.lock().unwrap_or_else(|held| held.into_inner());
        std::mem::take(&mut *list)
    };
    for handle in started {
        handle.stop();
    }
}

// The run-shaped reading of a pipe: what a program writes, cut into the rows a list draws.

/// How much of one line of a program's output is kept before it is cut and continued on
/// the next: a program writing megabytes with no newline in them is still *delivered*
/// rather than accumulated into a string nobody ever sees.
const MAX_LINE: u64 = 4096;

/// How many lines of a program's output are kept, oldest first out. A line cap and not a
/// byte cap, because the view is a list of rows; [`RunOutput::dropped`] is what lets it
/// say the story is missing its beginning.
const MAX_OUTPUT_LINES: usize = 5000;

/// Which of a program's two output streams a line came from. `stderr` is not an error, it
/// is the other stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Out,
    Err,
}

/// One line a running program wrote. The text is an `Arc<str>` because the app keeps
/// thousands of these in a value it clones whenever a line is added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLine {
    pub stream: Stream,
    pub text: Arc<str>,
}

/// What a running program has written, bounded by [`MAX_OUTPUT_LINES`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunOutput {
    lines: VecDeque<OutputLine>,
    dropped: usize,
}

impl RunOutput {
    /// Keep one more line, letting the oldest go if that is what it costs.
    pub fn push(&mut self, line: OutputLine) {
        if self.lines.len() >= MAX_OUTPUT_LINES {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.lines.push_back(line);
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// The line at `index`, counting from the oldest one still kept.
    pub fn line(&self, index: usize) -> Option<&OutputLine> {
        self.lines.get(index)
    }

    /// How many lines were let go to make room, so the view can say the story is missing
    /// its beginning.
    pub fn dropped(&self) -> usize {
        self.dropped
    }
}

/// What a run says as it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunEvent {
    Wrote(OutputLine),
    /// The last thing any run says, and it is said exactly once.
    Ended(Ended),
}

/// How a run finished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    /// The program returned by itself. `None` where the system ended it without a code.
    Exited(Option<i32>),
    /// [`Handle::stop`] was asked for.
    Stopped,
    /// It could not be waited for.
    Failed(String),
}

/// Split what a program writes into lines and hand each one over as it arrives, cut at
/// [`MAX_LINE`]. Invalid UTF-8 is taken lossily: what a program writes is not this app's
/// to reject.
///
/// **The cut falls between characters**, not between bytes. `take` stops after a byte
/// count wherever that lands, and a multi-byte character straddling it would arrive as a
/// replacement character on each of the two rows with the character itself on neither, so
/// what is left of one is carried to the front of the next read.
pub fn stream_lines(mut reader: impl BufRead, stream: Stream, mut emit: impl FnMut(OutputLine)) {
    let mut carry = Vec::new();
    loop {
        let mut buffer = std::mem::take(&mut carry);
        let room = MAX_LINE - buffer.len() as u64;
        match reader.by_ref().take(room).read_until(b'\n', &mut buffer) {
            // The end of the pipe, or a pipe that will not read: what the last cut fell
            // inside of is the last thing there is to say, and it is said lossily, the
            // rest of that character never having been written.
            Ok(0) | Err(_) => {
                if !buffer.is_empty() {
                    emit(output_line(stream, &buffer));
                }
                return;
            }
            Ok(_) => {}
        }

        // `error_len() == None` is exactly "an incomplete sequence at the end", so bytes
        // that are genuinely invalid still go through lossily below.
        carry = match std::str::from_utf8(&buffer) {
            Err(error) if error.error_len().is_none() => buffer.split_off(error.valid_up_to()),
            _ => Vec::new(),
        };
        // The read was that character's first bytes and nothing else: no row yet.
        if buffer.is_empty() {
            continue;
        }

        // The terminator, and a `\r` in front of it: the rows are drawn one line each, so
        // a carriage return left in would be a control character in the middle of a label.
        while matches!(buffer.last(), Some(b'\n' | b'\r')) {
            buffer.pop();
        }

        emit(output_line(stream, &buffer));
    }
}

/// One row out of the bytes it was read as.
fn output_line(stream: Stream, text: &[u8]) -> OutputLine {
    OutputLine {
        stream,
        text: Arc::from(String::from_utf8_lossy(text).as_ref()),
    }
}

#[cfg(test)]
mod tests;
