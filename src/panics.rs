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
//! that cannot be written is one the reader is told about anyway, and the line put on
//! stderr goes through `echo` rather than `eprintln!`, which panics when that write
//! fails.
//!
//! The hook is installed once the window is up (`crate::ui::app`) and not from `main`, so
//! that it is the outer one: freya installs its own inside `launch`, in a release build
//! only, which shows a "Fatal Error" box and exits (`notes/upstream/freya.md`). Ours takes
//! that one's place, so the app says the same thing in both builds -- and a guarded panic
//! stops being fatal in a release build, which it was.

use crate::{project, reveal, scratchpad};
use std::{
    backtrace::Backtrace,
    fs::{self, OpenOptions},
    io::Write,
    panic::{self, PanicHookInfo},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

/// Where a run's panics are written, under the directory everything is stored in.
const PANICS_DIR: &str = "panics";

/// The file this run appends to, made on the first panic and kept for the rest: one file
/// per launch, so a worker dying twenty thousand times over one bad file leaves one file
/// behind and the panics of one run read in order.
static FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Whether the app is already on its way down.
///
/// **The shutdown is asynchronous and the panic is not the end of the thread.** The hook
/// runs before the unwind, so after it returns the panicking thread goes on unwinding --
/// back into freya's render loop, on the UI thread -- while the shutdown thread is still
/// saving. A render that panicked once panics again on the next pass, and the reader who
/// pressed Close on the first box is handed a second. So the first unguarded panic is the
/// one that is told about and the one that stops the app, and every panic after it is
/// written down and nothing more, exactly as a guarded one is.
static STOPPING: AtomicBool = AtomicBool::new(false);

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

    /// The two lines a reader is shown, the message under what panicked. Whole, for the
    /// file and for stderr: both of them scroll.
    fn told(&self) -> String {
        format!(
            "{} panicked at {}\n{}",
            self.thread, self.location, self.message
        )
    }

    /// The same for the box, which does not scroll: the path cut down as a frame's is, and
    /// the message capped.
    ///
    /// **A panic message is the panicking code's to write and can be an essay.** freya's
    /// hook-order error is 51 lines of prose and example code, of which the first two say
    /// what happened; put in whole it made the trimmed backtrace under it pointless.
    fn shown(&self) -> String {
        format!(
            "{} panicked at {}\n{}",
            self.thread,
            trim_path(&self.location),
            first_lines(&self.message, MAX_MESSAGE_LINES)
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
        echo(std::io::stderr(), &panic);
        handle(
            &panic,
            analysis::guard::guarded(),
            &STOPPING,
            &mut |panic| base.as_deref().and_then(|base| write_in(base, panic)),
            &mut tell,
            &mut shut_down,
        );
    }));
}

/// Write the line the default hook would have written, since a developer running the app
/// from a terminal is reading it and no other hook runs now.
///
/// Best-effort, which is why it is not `eprintln!`: that panics when the write fails, and
/// a stderr that is a pipe whose reader has gone answers `EPIPE`. A panic raised inside
/// the hook aborts the process on the spot, before the record is written or the reader is
/// told.
fn echo(mut out: impl Write, panic: &Panic) {
    let _ = writeln!(out, "{}", panic.told());
}

/// What a panic leads to, with the storing, the telling and the shutting down all handed
/// in so a test can have the rule without a disk or a window: **every** panic is written
/// down, and the **first** one the crate does not guard is told about and brings the app
/// down after it.
///
/// `stopping` is [`STOPPING`], passed in for [`FILE`]'s reason: it is a static and the
/// tests share one process.
fn handle(
    panic: &Panic,
    guarded: bool,
    stopping: &AtomicBool,
    store: &mut impl FnMut(&Panic) -> Option<PathBuf>,
    tell: &mut impl FnMut(&Panic, Option<&Path>),
    stop: &mut impl FnMut(),
) {
    let file = store(panic);
    if guarded {
        return;
    }
    // Claimed here and not after the telling: the box is a blocking call, and a second
    // panic arrives while the reader is still looking at the first.
    if stopping.swap(true, Ordering::SeqCst) {
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
/// in a debug build as much as a release one.
///
/// **One box, with the top of the backtrace in it.** The box is the desktop's own -- a
/// `zenity` child process on Linux, `TaskDialogIndirect` on Windows, `NSAlert` on macOS --
/// which is exactly why the panic path can use it at all, and also why it will not scroll
/// and why its text cannot be selected. So it holds the few frames that say where the
/// panic was and nothing more; everything else is in the file, and the button beside Close
/// goes there.
///
/// **The file is shown on this thread**, through [`reveal::reveal_now`] rather than
/// [`reveal::reveal`]: the shutdown after this would kill a thread of its own before it
/// had spawned anything. It was a loop back to the box first, to keep the app alive long
/// enough, and that was worse than the problem -- a crash box that comes back reads as a
/// second crash, and was reported as one.
fn tell(panic: &Panic, file: Option<&Path>) {
    const REVEAL: &str = "Show file";
    const CLOSE: &str = "Close";

    let said = match file {
        Some(path) => format!(
            "{}\n\n{}\nThe whole of it was saved to {}.",
            panic.shown(),
            short(&panic.backtrace, MAX_FRAMES),
            path.display()
        ),
        None => format!(
            "{}\n\n{}\nNothing could be saved.",
            panic.shown(),
            short(&panic.backtrace, MAX_FRAMES)
        ),
    };
    // Two buttons where there is a file to show and one where there is not: a button that
    // would do nothing is left out, as the app's menus leave one out.
    let buttons = match file {
        Some(_) => rfd::MessageButtons::OkCancelCustom(REVEAL.to_owned(), CLOSE.to_owned()),
        None => rfd::MessageButtons::OkCustom(CLOSE.to_owned()),
    };

    let answer = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Assembly Viewer has stopped")
        .set_description(&said)
        .set_buttons(buttons)
        .show();

    if answer == rfd::MessageDialogResult::Custom(REVEAL.to_owned()) {
        if let Some(path) = file {
            if !reveal::reveal_now(path) {
                // Said in the log and not in a second box: the reader has just read the
                // path in the box above and can go there themselves.
                log::warn!("no file manager showed {}", path.display());
            }
        }
    }
}

/// How many frames of a backtrace the box is given. A number rather than a scroll bar:
/// the box is the desktop's own and has none, and one grown past the screen is one whose
/// buttons cannot be reached. Six is twelve lines, each carrying the file it is in, under
/// three of the panic itself -- enough to say where it was and short enough that the path
/// to the rest is still on the screen.
const MAX_FRAMES: usize = 6;

/// The frames of `backtrace` worth reading, at most `most` of them.
///
/// A capture taken **inside a panic hook** opens with the hook itself and the whole of the
/// runtime that called it -- this module, `Box<dyn Fn>`, `std::panicking`,
/// `core::panicking`, and the `expect` or `unwrap` that raised it. That is a dozen frames
/// saying nothing about the panic, they are always the same dozen, and the first frame
/// that does say something is the one after them.
///
/// **The run at the top and not every such frame.** A stack also *ends* in the runtime --
/// `lang_start`, the `catch_unwind` around `main` -- and cutting at the last one anywhere
/// in the capture would throw away everything between, which is the whole of what was
/// asked for. So the names below are consulted only while the opening run lasts, and a
/// frame further down that happens to carry one is left where it is.
///
/// The numbers are the capture's own, kept rather than renumbered, so a frame in the box
/// and the same frame in the file are the same frame. Nothing is cut where the whole
/// capture is runtime, or where none of it is: a backtrace this does not recognise is
/// shown as it came.
fn short(backtrace: &str, most: usize) -> String {
    let frames = frames(backtrace);
    let start = frames.iter().take_while(|frame| is_runtime(frame)).count();
    let wanted = &frames[start.min(frames.len())..];
    let shown = wanted.len().min(most);

    let mut text = String::new();
    for frame in &wanted[..shown] {
        text.push_str(&drawn(frame));
        text.push('\n');
    }
    if wanted.len() > shown {
        text.push_str(&format!(
            "\n... and {} more frames, in the file.",
            wanted.len() - shown
        ));
    }
    match text.is_empty() {
        true => backtrace.to_owned(),
        false => text,
    }
}

/// How wide a frame's name may be drawn. A monomorphised name in a UI framework runs to
/// several hundred characters of turbofish, all of it wrapped into a wall by a box that
/// takes no width: what the reader is looking for is at the front.
const MAX_WIDTH: usize = 110;

/// How many lines of the panic's own message the box is given. Three is the rule and its
/// first item for the error above, which is the whole of what that one says.
const MAX_MESSAGE_LINES: usize = 3;

/// The first `most` lines of `text`, with a note where any were left behind.
///
/// Blank lines at the cut go with what was cut. A message is often a heading, a blank and
/// then the detail, so the cap lands on the blank as often as not, and a box ending in an
/// empty line above the note reads as though something failed to draw.
fn first_lines(text: &str, most: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= most {
        return text.to_owned();
    }
    let kept = {
        let mut kept = &lines[..most];
        while kept.last().is_some_and(|line| line.trim().is_empty()) {
            kept = &kept[..kept.len() - 1];
        }
        kept
    };
    format!(
        "{}\n... and {} more lines, in the file.",
        kept.join("\n"),
        lines.len() - kept.len()
    )
}

/// One frame as the box draws it: the name cut to [`MAX_WIDTH`], and the `at` line under
/// it with the part of the path nobody needs taken off the front. A file out of the
/// registry is named by its crate and one out of the standard library by the library,
/// which is the whole of what a `/home/…/.cargo/registry/src/index.crates.io-1949cf…/`
/// says that its next segment does not.
fn drawn(frame: &str) -> String {
    frame
        .lines()
        .map(|line| match line.trim_start().starts_with("at ") {
            true => shorten_path(line),
            false => cut(line, MAX_WIDTH),
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// `line` cut to `width` characters, with an ellipsis where anything was taken. Counted in
/// `char`s, a name being text and not bytes.
fn cut(line: &str, width: usize) -> String {
    match line.chars().count() > width {
        true => line.chars().take(width).collect::<String>() + "\u{2026}",
        false => line.to_owned(),
    }
}

/// An `at` line with its path cut down. A line this does not recognise is left as it is.
fn shorten_path(line: &str) -> String {
    match line.trim_start().strip_prefix("at ") {
        Some(path) => format!("             at {}", trim_path(path)),
        None => line.to_owned(),
    }
}

/// A path with the prefix that says nothing removed: everything up to the crate's own
/// directory for one out of the registry, and up to `library/` for one in the standard
/// library. A path this does not recognise -- the app's own `./src/…` -- is left as it is,
/// being short already.
///
/// Whatever follows the path is kept, so a `file.rs:87:53` keeps its line and column.
fn trim_path(path: &str) -> &str {
    const REGISTRY: &str = "/registry/src/";
    const LIBRARY: &str = "/library/";

    if let Some(at) = path.rfind(LIBRARY) {
        return &path[at + 1..];
    }
    // Past the index directory as well as the marker: it is a hash and names nothing.
    if let Some(at) = path.find(REGISTRY) {
        let rest = &path[at + REGISTRY.len()..];
        if let Some(slash) = rest.find('/') {
            return &rest[slash + 1..];
        }
    }
    path
}

/// A capture cut into frames: a line beginning `<number>:` starts one and the lines under
/// it -- the `at file:line` the capture puts there -- belong to it. Anything before the
/// first numbered line is dropped, there being no frame for it to be part of.
fn frames(backtrace: &str) -> Vec<String> {
    let mut frames: Vec<String> = Vec::new();
    for line in backtrace.lines() {
        let numbered = line
            .trim_start()
            .split_once(':')
            .is_some_and(|(number, _)| {
                !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
            });
        match numbered {
            true => frames.push(line.to_owned()),
            false => {
                if let Some(frame) = frames.last_mut() {
                    frame.push('\n');
                    frame.push_str(line);
                }
            }
        }
    }
    frames
}

/// Whether `frame` is one of the ones between the panic and the hook capturing it, rather
/// than anything the app did. This module is on the list, being the innermost frame of
/// every capture taken here, and so is the `Box<dyn Fn>` the hook is called through.
///
/// `Option` and `Result` are named whole rather than by their failure helpers: what raises
/// the panic is `expect_failed`, but the frame under it is the `expect` itself and the
/// caller of *that* is the code worth reading. Only ever asked about the opening run, so a
/// name matching one of these deeper in a stack is not affected ([`short`]).
fn is_runtime(frame: &str) -> bool {
    const RUNTIME: [&str; 10] = [
        "rust_begin_unwind",
        "core::panicking",
        "std::panicking",
        "std::sys::backtrace",
        "core::option::Option",
        "core::option::expect_failed",
        "core::result::Result",
        "core::result::unwrap_failed",
        "core::ops::function::Fn",
        "viewer::panics",
    ];
    RUNTIME.iter().any(|name| frame.contains(name))
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
