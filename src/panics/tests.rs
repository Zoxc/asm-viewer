use super::*;

/// A directory of this test's own under the system temporary directory, named after the
/// line that asked for it, standing in for the one everything is stored in.
fn base(line: u32) -> PathBuf {
    std::env::temp_dir().join(format!(
        "assembly-viewer-panics-test-{}-{line}",
        std::process::id()
    ))
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

    let _ = fs::remove_dir_all(&base);
}

/// A panic the crate guards -- a demangler on a name out of a file -- is written down and
/// nothing more: nothing has gone wrong with the app, so nobody is told and nothing is
/// brought down. Any other panic is told about and stops the app.
#[test]
fn only_a_panic_the_crate_does_not_guard_is_told_about() {
    for guarded in [true, false] {
        let (mut stored, mut told, mut stopped) = (0, 0, 0);
        handle(
            &panic_at(1_757_000_000, "a dependency's bug"),
            guarded,
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
