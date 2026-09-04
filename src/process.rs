//! The process group a child is started in, so that killing it reaches what it started.
//!
//! Two things here start programs the app must be able to end outright: a scratchpad's run
//! (`src/scratchpad.rs`) and the language server (`src/lsp.rs`). Without a group, a stop
//! kills the process this app has a handle for and leaves everything it forked running with
//! nothing that could ever find it again -- the grandchild's pid was never anywhere but
//! inside the program that is now gone. The two platforms have the same shape and nothing
//! else in common: [`Group::arrange`] runs before the spawn, [`Group::of`] takes hold of
//! what was spawned, and [`Group::kill`] ends the lot.

use std::process::{Child, Command};
#[cfg(windows)]
use std::sync::Mutex;

#[cfg(unix)]
pub(crate) struct Group(i32);

#[cfg(unix)]
impl Group {
    /// `process_group(0)` is "a new group whose id is the child's own pid", set between the
    /// fork and the exec by the standard library. It is std's, not `libc`'s: only the kill
    /// needs a crate.
    pub(crate) fn arrange(command: &mut Command) {
        use std::os::unix::process::CommandExt;

        command.process_group(0);
    }

    /// Which group that turned out to be. Asked of the `Child` rather than assumed, so the
    /// number a stop signals is one the kernel handed back.
    pub(crate) fn of(child: &Child) -> Self {
        Group(child.id() as i32)
    }

    /// `kill(-pgid)` is the whole group. Guarded because the negative of a small number is
    /// not a group at all: `-1` is every process this user may signal and `0` is *this*
    /// app's own group, and neither can come of a real child, so neither may be reached by
    /// a pid that somehow arrived as one.
    pub(crate) fn kill(&self) {
        if self.0 > 1 {
            // SAFETY: a signal number and a pid, both plain values; `kill` reads no memory.
            unsafe { libc::kill(-self.0, libc::SIGKILL) };
        }
    }
}

/// The Windows half of [`Group`] — a job object with kill-on-close, which the child and
/// everything it starts are inside. Closing the last handle to it is the kill, so a run the
/// app somehow drops without stopping dies with the [`Process`] rather than outliving it.
#[cfg(windows)]
pub(crate) struct Group(Mutex<Option<std::os::windows::io::OwnedHandle>>);

#[cfg(windows)]
impl Group {
    /// Nothing: there is no pre-spawn half here, the job is joined after the fact.
    pub(crate) fn arrange(_command: &mut Command) {}

    /// Assign the spawned process to a fresh kill-on-close job.
    ///
    /// The sliver between the spawn and this call is real and accepted: a program that
    /// forks in its first microseconds forks outside the job. Closing it would mean
    /// `CREATE_SUSPENDED` and a `ResumeThread`, which is a raw thread handle and a spawn
    /// this module no longer shares with `std`, for a window a scratchpad's program does
    /// not use. `None` where the system refused — a job it may not create, or a job it may
    /// not nest — and then the stop is `Child::kill` alone, exactly what it was before.
    pub(crate) fn of(child: &Child) -> Self {
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
            return Group(Mutex::new(None));
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

        Group(Mutex::new(assigned.then_some(job)))
    }

    /// Close the handle, which is what kills: this app holds the only one, the child never
    /// having been given it to inherit. Taken rather than borrowed, so the second stop of a
    /// run is the no-op the first made it.
    pub(crate) fn kill(&self) {
        drop(
            self.0
                .lock()
                .unwrap_or_else(|held| held.into_inner())
                .take(),
        );
    }
}

/// Neither Unix nor Windows: there is no group, and a stop is the child alone.
#[cfg(not(any(unix, windows)))]
pub(crate) struct Group;

#[cfg(not(any(unix, windows)))]
impl Group {
    pub(crate) fn arrange(_command: &mut Command) {}

    pub(crate) fn of(_child: &Child) -> Self {
        Group
    }

    pub(crate) fn kill(&self) {}
}
