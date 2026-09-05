use super::*;
use crate::temporary::Temporary;

/// A directory of this test's own under the system temporary directory, named after the
/// line that asked for it, standing in for the one everything is stored in. Gone when the
/// test ends.
fn base(line: u32) -> Temporary {
    Temporary::at(std::env::temp_dir().join(format!(
        "assembly-viewer-panics-test-{}-{line}",
        std::process::id()
    )))
}

fn panic_at(at: u64, message: &str) -> Panic {
    Panic {
        thread: "the analysis worker".to_owned(),
        location: "src/ui/analyzed.rs:214:9".to_owned(),
        message: message.to_owned(),
        backtrace: "0: one\n1: two".to_owned(),
        at,
    }
}

/// Every panic of one run is appended to one file, named for when the run's first was:
/// the directory is made on the way, and a second panic adds a record rather than a file.
#[test]
fn a_run_s_panics_are_appended_to_one_file() {
    let base = base(line!());
    let _ = fs::remove_dir_all(&base);
    let file = Mutex::new(None);

    let first = write_to(&file, &base, &panic_at(1_757_000_000, "the first")).expect("written");
    assert_eq!(
        first,
        base.join(PANICS_DIR).join("2025-09-04-153320.txt"),
        "the file is named for the run's first panic"
    );

    // A second panic, later and on the other side of a minute: the same file.
    let second = write_to(&file, &base, &panic_at(1_757_000_100, "the second")).expect("written");
    assert_eq!(second, first);
    let directory: Vec<PathBuf> = fs::read_dir(base.join(PANICS_DIR))
        .expect("the directory was made")
        .filter_map(|entry| Some(entry.ok()?.path()))
        .collect();
    assert_eq!(directory, [first.clone()], "a second file was written");

    let written = fs::read_to_string(&first).expect("the file reads");
    assert!(
        written.starts_with("2025-09-04 15:33:20 the analysis worker panicked at src/ui/analyzed.rs:214:9\n  the first\n"),
        "{written}"
    );
    assert!(written.contains("\n  0: one\n  1: two\n"), "{written}");
    assert!(
        written.contains("2025-09-04 15:35:00 ") && written.contains("  the second\n"),
        "the second panic is not in the file: {written}"
    );
}

/// A panic the crate guards -- a demangler on a name out of a file -- is written down and
/// nothing more: nothing has gone wrong with the app, so nobody is told and nothing is
/// brought down. Any other panic is told about and stops the app.
#[test]
fn only_a_panic_the_crate_does_not_guard_is_told_about() {
    for guarded in [true, false] {
        let (mut stored, mut told, mut stopped) = (0, 0, 0);
        let stopping = AtomicBool::new(false);
        handle(
            &panic_at(1_757_000_000, "a dependency's bug"),
            guarded,
            &stopping,
            &mut |_| {
                stored += 1;
                Some(PathBuf::from("panics/one.txt"))
            },
            &mut |_, file| {
                assert_eq!(file, Some(Path::new("panics/one.txt")));
                told += 1;
            },
            &mut || stopped += 1,
        );
        assert_eq!(stored, 1, "guarded: {guarded}");
        assert_eq!(told, usize::from(!guarded), "guarded: {guarded}");
        assert_eq!(stopped, usize::from(!guarded), "guarded: {guarded}");
    }
}

/// **One box, however many panics follow.** The hook runs before the unwind, so the thread
/// that panicked goes on running -- back into the render loop it panicked in -- while the
/// shutdown is still saving. A second panic there used to hand the reader a second box on
/// top of the one they had just closed. It is written down like any other and stops there.
#[test]
fn only_the_first_panic_is_told_about_and_the_rest_are_written_down() {
    let stopping = AtomicBool::new(false);
    let (mut stored, mut told, mut stopped) = (0, 0, 0);
    let mut again = |message: &str| {
        handle(
            &panic_at(1_757_000_000, message),
            false,
            &stopping,
            &mut |_| {
                stored += 1;
                None
            },
            &mut |_, _| told += 1,
            &mut || stopped += 1,
        );
    };

    again("the render that broke");
    again("the same render, one pass later");
    again("and again");

    assert_eq!(stored, 3, "a panic after the first was not written down");
    assert_eq!(told, 1, "the reader was handed more than one box");
    assert_eq!(stopped, 1, "the app was brought down more than once");
}

/// A stderr whose reader has gone answers `EPIPE`, and the hook has to survive it: a
/// panic raised inside the hook aborts before anything is written down. `eprintln!` is
/// exactly that panic, which is why the line goes through `echo`.
#[test]
fn a_stderr_that_will_not_take_the_line_is_not_a_second_panic() {
    struct Broken;

    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }
    }

    let panic = panic_at(1_757_000_000, "a dependency's bug");
    echo(Broken, &panic);

    // And the line itself, so the writer that does take it is pinned too.
    let mut taken = Vec::new();
    echo(&mut taken, &panic);
    assert_eq!(
        String::from_utf8(taken).expect("the line is text"),
        format!("{}\n", panic.told())
    );
}

/// The stamp on a record and on a file: UTC, and right over a leap day and a year's end,
/// which is the whole of what the arithmetic can get wrong.
#[test]
fn a_stamp_is_the_utc_date_and_time() {
    assert_eq!(stamp(0), "1970-01-01 00:00:00");
    assert_eq!(stamp(1), "1970-01-01 00:00:01");
    assert_eq!(stamp(951_782_400), "2000-02-29 00:00:00");
    assert_eq!(stamp(1_757_000_000), "2025-09-04 15:33:20");
    assert_eq!(stamp(4_102_444_799), "2099-12-31 23:59:59");
    assert_eq!(file_stamp(1_757_000_000), "2025-09-04-153320");
}

/// A capture taken in the hook, cut down to the frames that say anything: the runtime
/// between the panic and the hook goes, and the app is what is left. The text is a real
/// capture with the middle taken out.
#[test]
fn a_backtrace_opens_at_the_first_frame_that_is_not_the_runtime() {
    let capture = "\
   0: <viewer::panics::Panic>::of
             at ./src/panics.rs:87:24
   1: viewer::panics::install::{closure#0}
             at ./src/panics.rs:128:21
   2: <alloc::boxed::Box<dyn core::ops::function::Fn>>::call
   3: std::panicking::panic_with_hook
   4: __rustc::rust_begin_unwind
   5: core::panicking::panic_fmt
   6: core::option::expect_failed
   7: <core::option::Option<()>>::expect
   8: viewer::ui::project_view::use_restore_on_startup
             at ./src/ui/project_view.rs:828:5
   9: viewer::ui::app
             at ./src/ui.rs:419:5
";
    let short = short(capture, 24);

    // The whole opening run goes, `expect_failed` and the `expect` under it included: what
    // is worth reading is the caller of the `expect`, not the `expect`.
    assert!(
        short.starts_with("   8: viewer::ui::project_view"),
        "{short}"
    );
    // The `at` line under a frame goes with it, and the numbers are the capture's own, so
    // a frame in the box is that frame in the file.
    assert!(
        short.contains("   9: viewer::ui::app\n             at ./src/ui.rs:419:5\n"),
        "{short}"
    );
    assert!(!short.contains("   0:"), "{short}");
}

/// **The opening run and not every runtime frame anywhere.** A stack also *ends* in the
/// runtime -- `lang_start` and the `catch_unwind` around `main` -- so a rule that cut at
/// the last such frame threw away everything between and left the box holding libc.
#[test]
fn the_runtime_under_main_is_not_what_the_backtrace_is_cut_at() {
    let capture = "\
   0: core::panicking::panic_fmt
   1: viewer::ui::app
   2: std::panicking::try::<(), ()>
   3: std::rt::lang_start_internal
   4: main
";
    let short = short(capture, 24);

    assert!(short.starts_with("   1: viewer::ui::app"), "{short}");
    assert!(
        short.contains("   4: main"),
        "the frames below the app went: {short}"
    );
}

/// The cap, and what says a frame was left out. A box the desktop will not scroll is one
/// whose buttons go off the screen when the text is long enough.
#[test]
fn a_long_backtrace_is_capped_and_says_so() {
    let mut capture = String::from("   0: __rustc::rust_begin_unwind\n");
    for frame in 1..=10 {
        capture.push_str(&format!("  {frame:2}: viewer::deep::frame_{frame}\n"));
    }

    let short = short(&capture, 4);
    assert!(
        short.starts_with("   1: viewer::deep::frame_1\n"),
        "{short}"
    );
    assert!(short.contains("   4: viewer::deep::frame_4\n"), "{short}");
    assert!(!short.contains("frame_5"), "{short}");
    assert!(
        short.ends_with("\n... and 6 more frames, in the file."),
        "{short}"
    );
}

/// What each frame costs the box: a monomorphised name cut to a width, and a path with
/// the part that names nothing taken off the front. Between them these are most of the
/// bytes -- the capture behind this one is 12,885 of them and 1,776 after.
#[test]
fn a_frame_is_drawn_without_the_bytes_that_say_nothing() {
    let registry = "             at /home/j/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/freya-core-0.4.3/src/lifecycle/base.rs:87:53";
    assert_eq!(
        shorten_path(registry),
        "             at freya-core-0.4.3/src/lifecycle/base.rs:87:53"
    );
    // Both spellings of a standard library path, the compiler's and rustup's.
    assert_eq!(
        shorten_path("             at /rustc/17fd5b8a/library/core/src/option.rs:2262:5"),
        "             at library/core/src/option.rs:2262:5"
    );
    assert_eq!(
        shorten_path("             at /home/j/.rustup/toolchains/nightly/lib/rustlib/src/rust/library/std/src/thread/local.rs:463:12"),
        "             at library/std/src/thread/local.rs:463:12"
    );
    // The app's own, which is short already and is left exactly as it is.
    assert_eq!(
        shorten_path("             at ./src/ui.rs:419:5"),
        "             at ./src/ui.rs:419:5"
    );

    assert_eq!(cut("abcdef", 4), "abcd\u{2026}");
    assert_eq!(cut("abcd", 4), "abcd");
    // Characters and not bytes: a name is text.
    assert_eq!(cut("\u{e9}\u{e9}\u{e9}", 2), "\u{e9}\u{e9}\u{2026}");
}

/// A capture with nothing this recognises in it is shown as it came: the trimming is a
/// convenience and never the only way to see what happened.
#[test]
fn a_backtrace_with_no_runtime_frames_is_left_alone() {
    assert_eq!(short("0: one\n1: two\n", 24), "0: one\n1: two\n");
    assert_eq!(short("", 24), "");
    // Every frame is the runtime: nothing is left, so the whole is shown rather than a
    // box with nothing in it.
    let all = "   0: core::panicking::panic_fmt\n   1: std::panicking::panic_with_hook\n";
    assert_eq!(short(all, 24), all);
}

/// The box does not scroll and its text cannot be selected, so what goes in it is capped
/// on both sides: the message the panicking code wrote, and the frames under it. The whole
/// of each is in the file, which is what the button beside Close opens.
#[test]
fn the_box_caps_the_message_as_well_as_the_backtrace() {
    // freya's hook-order error, whose first two lines are what it actually says.
    let mut message = String::from(
        "Hook functions must follow these rules:\n1. You cannot call them conditionally\n\n",
    );
    for line in 0..48 {
        message.push_str(&format!("example line {line}\n"));
    }
    let panic = Panic {
        message,
        location:
            "/home/j/.cargo/registry/src/index.crates.io-1949cf/freya-core-0.4.3/src/lifecycle/base.rs:87:53"
                .to_owned(),
        ..panic_at(1_757_000_000, "")
    };

    let shown = panic.shown();
    assert!(
        shown.starts_with(
            "the analysis worker panicked at freya-core-0.4.3/src/lifecycle/base.rs:87:53\n"
        ),
        "the location was not cut down: {shown}"
    );
    assert!(
        shown.contains("1. You cannot call them conditionally\n"),
        "{shown}"
    );
    assert!(
        shown.ends_with("... and 49 more lines, in the file."),
        "{shown}"
    );
    assert!(!shown.contains("example line"), "{shown}");

    // And the record keeps every word of it, the file being the thing that scrolls.
    assert!(panic.record().contains("example line 47"));
    assert!(panic.record().contains(&panic.location));
}

/// The blank line a message so often has under its heading: cut there, the box would end
/// on an empty line above the note, which reads as something that failed to draw.
#[test]
fn a_message_cut_at_a_blank_line_does_not_end_on_one() {
    assert_eq!(
        first_lines("one\n\nthree\nfour\n", 2),
        "one\n... and 3 more lines, in the file."
    );
    // Nothing to say where nothing was left behind.
    assert_eq!(first_lines("one\ntwo\n", 2), "one\ntwo\n");
    assert_eq!(first_lines("one\ntwo\n", 9), "one\ntwo\n");
}
