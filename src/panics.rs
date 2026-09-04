//! A panic on any thread: written down, told about, and the app brought down after it.
//!
//! Nothing here is defensiveness about the app's own code. A panic is a bug, and the
//! reason it needs answering at all is that this is a windowed program: the default hook
//! writes a line to a stderr nobody is looking at, and the app's work is done on threads
//! of its own, so a panicking worker used to leave a pane waiting for an answer that was
//! never coming, with nothing on screen and nothing on disk to say why.
//!
//! Three things follow from a hook running on the panicking thread **before** the unwind:
//!
//! **It can tell a guarded panic from a real one.** `analysis::guard` marks the calls
//! whose panics are caught on purpose -- a demangler on a name out of a string table, a
//! debug format read by a dependency that does unchecked arithmetic. Those are written
//! down like any other and nothing else happens: nobody is told and nothing is shut down,
//! because nothing has gone wrong with the app.
//!
//! **It must not take a lock the panicking thread might hold.** The guard the panic is
//! unwinding out of is still alive while the hook runs, and a `std::sync::Mutex` is not
//! reentrant, so the shutdown -- which saves the projects, and takes that lock -- goes on
//! a thread of its own and only reaches the lock once the unwind has let it go.
//!
//! **It must not panic itself**, which aborts. Everything here is best-effort: a store
//! that cannot be written is one the reader is told about anyway.
//!
//! The hook is installed once the window is up (`crate::ui::app`) and not from `main`, so
//! that it is the outer one: freya installs its own inside `launch`, in a release build
//! only, which shows a "Fatal Error" box and exits (`notes/upstream/freya.md`). Ours takes
//! that one's place, so the app says the same thing in both builds -- and a guarded panic
//! stops being fatal in a release build, which it was.

use crate::{project, scratchpad};
use std::{
    backtrace::Backtrace,
    fs::{self, OpenOptions},
    io::Write,
    panic::{self, PanicHookInfo},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

/// Where a run's panics are written, under the directory everything is stored in.
const PANICS_DIR: &str = "panics";

/// The file this run appends to, made on the first panic and kept for the rest: one file
/// per launch, so a worker dying twenty thousand times over one bad file leaves one file
/// behind and the panics of one run read in order.
static FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// One panic, as much of it as a hook can be sure of.
struct Panic {
    /// The thread's name, which is why the app's workers have one.
    thread: String,
    /// `file:line:column`, or a note that the panic carried none.
    location: String,
    message: String,
    backtrace: String,
    /// Seconds since the epoch, for the stamp on the record and on the file.
    at: u64,
}

impl Panic {
    /// What the hook was handed, with the backtrace captured here and now:
    /// [`Backtrace::force_capture`], so it does not depend on `RUST_BACKTRACE` being set
    /// in whatever environment the app was launched from.
    fn of(info: &PanicHookInfo<'_>) -> Panic {
        let payload = info.payload();
        let message = match payload.downcast_ref::<&str>() {
            Some(message) => (*message).to_owned(),
            None => payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_else(|| "a panic carrying no message".to_owned()),
        };
        Panic {
            thread: std::thread::current()
                .name()
                .unwrap_or("an unnamed thread")
                .to_owned(),
            location: match info.location() {
                Some(at) => format!("{}:{}:{}", at.file(), at.line(), at.column()),
                None => "an unknown place".to_owned(),
            },
            message,
            backtrace: Backtrace::force_capture().to_string(),
            at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_secs()),
        }
    }

    /// The record as it is written down: a header line that reads on its own, then the
    /// backtrace indented under it so one panic is one block.
    fn record(&self) -> String {
        let mut record = format!(
            "{} {} panicked at {}\n  {}\n",
            stamp(self.at),
            self.thread,
            self.location,
            self.message
        );
        for line in self.backtrace.lines() {
            record.push_str("  ");
            record.push_str(line);
            record.push('\n');
        }
        record.push('\n');
        record
    }

    /// The two lines a reader is shown, the message under what panicked.
    fn told(&self) -> String {
        format!(
            "{} panicked at {}\n{}",
            self.thread, self.location, self.message
        )
    }
}

/// Install the hook. Called once the window is up, so that it takes the place of freya's
/// rather than sitting under it: `set_hook` replaces, and the hook replaced is not called
/// (freya's would put up a second box and exit before anything here could save).
pub(crate) fn install() {
    let base = project::base();
    panic::set_hook(Box::new(move |info| {
        let panic = Panic::of(info);
        // The line the default hook would have written, since a developer running the
        // app from a terminal is reading it and no other hook runs now.
        eprintln!("{}", panic.told());
        handle(
            &panic,
            analysis::guard::guarded(),
            &mut |panic| base.as_deref().and_then(|base| write_in(base, panic)),
            &mut tell,
            &mut shut_down,
        );
    }));
}

/// What a panic leads to, with the storing, the telling and the shutting down all handed
/// in so a test can have the rule without a disk or a window: every panic is written
/// down, and one the crate does not guard is told about and brings the app down after it.
fn handle(
    panic: &Panic,
    guarded: bool,
    store: &mut impl FnMut(&Panic) -> Option<PathBuf>,
    tell: &mut impl FnMut(&Panic, Option<&Path>),
    stop: &mut impl FnMut(),
) {
    let file = store(panic);
    if guarded {
        return;
    }
    tell(panic, file.as_deref());
    stop();
}

/// Append `panic`'s record to this run's file, making it and the directory over it on the
/// first panic, and answer where it went.
///
/// Appended and not written atomically: the file grows a record at a time and a reader
/// may be looking at it, where the app's other files are each replaced whole
/// (`project::write_atomically`).
fn write_in(base: &Path, panic: &Panic) -> Option<PathBuf> {
    write_to(&FILE, base, panic)
}

/// The same against a given cell, so a test has a run of its own: [`FILE`] is one static
/// and the tests share one process.
fn write_to(file: &Mutex<Option<PathBuf>>, base: &Path, panic: &Panic) -> Option<PathBuf> {
    let mut held = file.lock().unwrap_or_else(|held| held.into_inner());
    let path = match held.clone() {
        Some(path) => path,
        None => {
            let directory = base.join(PANICS_DIR);
            fs::create_dir_all(&directory).ok()?;
            let path = directory.join(format!("{}.txt", file_stamp(panic.at)));
            held.replace(path.clone());
            path
        }
    };
    let mut writing = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    writing.write_all(panic.record().as_bytes()).ok()?;
    Some(path)
}

/// Say what happened, in a box of the app's own: the same one whichever thread panicked,
/// since a panic on the UI thread leaves no frame to draw a window of the app's in, and
/// in a debug build as much as a release one. The backtrace is a button away rather than
/// in the box, being pages long and of no use to most readers.
fn tell(panic: &Panic, file: Option<&Path>) {
    const BACKTRACE: &str = "Show backtrace";
    let stored = match file {
        Some(path) => format!("The details were saved to {}.", path.display()),
        None => "The details could not be saved.".to_owned(),
    };
    let answer = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Assembly Viewer has stopped")
        .set_description(format!("{}\n\n{stored}", panic.told()))
        .set_buttons(rfd::MessageButtons::OkCancelCustom(
            BACKTRACE.to_owned(),
            "Close".to_owned(),
        ))
        .show();
    if answer == rfd::MessageDialogResult::Custom(BACKTRACE.to_owned()) {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Assembly Viewer has stopped")
            .set_description(&panic.backtrace)
            .show();
    }
}

/// Bring the app down the way closing the window does -- the projects saved, the
/// scratchpads' children stopped -- and leave.
///
/// On a thread of its own, and this is the whole reason: the panicking thread has not
/// unwound yet, so any lock it holds is still held, and `project::flush` takes one.
/// Started here and left to run; the thread that panicked returns into its unwind, which
/// is what lets the lock go.
fn shut_down() {
    let _ = std::thread::Builder::new()
        .name("shutdown".to_owned())
        .spawn(|| {
            project::flush();
            scratchpad::stop_all();
            std::process::exit(1);
        });
}

/// The date and time `seconds` after the epoch, UTC, as `2026-09-04 14:12:33`.
fn stamp(seconds: u64) -> String {
    let (year, month, day, hour, minute, second) = civil(seconds);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

/// The same as a file name: `2026-09-04-141233`, which sorts by when it was written.
fn file_stamp(seconds: u64) -> String {
    let (year, month, day, hour, minute, second) = civil(seconds);
    format!("{year:04}-{month:02}-{day:02}-{hour:02}{minute:02}{second:02}")
}

/// Seconds since the epoch as a UTC date and time.
///
/// Howard Hinnant's `civil_from_days`, whose trick is to count from an era beginning on
/// the 1st of March, so that a leap day is the last day of a year and every other month
/// keeps its length. No crate for this: it is the only date the app has ever needed, and
/// nothing here is a clock -- a stamp on a file is not read back.
fn civil(seconds: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (seconds / 86_400) as i64;
    let time = seconds % 86_400;
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let months = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * months + 2) / 5 + 1) as u32;
    let month = if months < 10 { months + 3 } else { months - 9 } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (
        year,
        month,
        day,
        (time / 3600) as u32,
        ((time / 60) % 60) as u32,
        (time % 60) as u32,
    )
}

#[cfg(test)]
mod tests;
