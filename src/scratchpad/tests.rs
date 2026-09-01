use super::*;

/// A directory of this test's own under the system temporary directory, named after
/// the line that asked for it — `project.rs`'s and `settings.rs`'s file tests do the
/// same, so a failing test leaves something identifiable behind.
fn directory(line: u32) -> PathBuf {
    std::env::temp_dir().join(format!(
        "assembly-viewer-scratchpad-test-{}-{line}",
        std::process::id()
    ))
}

fn scratchpad() -> Scratchpad {
    Scratchpad::new("sketch").expect("a name")
}

/// One row, as a reader would have left the two boxes. A test helper and not a
/// constructor on [`Dependency`]: the editor builds its rows a field at a time out of
/// two text boxes, so nothing outside these tests ever has both halves at once.
fn dependency(name: impl Into<String>, version: impl Into<String>) -> Dependency {
    Dependency {
        name: name.into(),
        version: version.into(),
    }
}

#[test]
fn a_new_scratchpad_is_a_name_and_something_to_look_at() {
    let scratchpad = scratchpad();
    assert_eq!(scratchpad.name(), "sketch");
    assert_eq!(scratchpad.source, DEFAULT_SOURCE);
    assert!(scratchpad.dependencies.is_empty());
    assert!(scratchpad.problems().is_empty());

    // The name is a path component as well as a crate name, and the crate-name rules
    // are what keeps it one.
    assert_eq!(Scratchpad::new("../escape"), Err(Problem::NameStart));
    assert_eq!(Scratchpad::new("a/b"), Err(Problem::NameCharacter('/')));
    assert_eq!(Scratchpad::new(""), Err(Problem::NoName));
    // Trimmed, because it comes from a text box.
    assert_eq!(
        Scratchpad::new("  sketch  ").map(|s| s.name),
        Ok("sketch".into())
    );
}

/// The whole generated manifest, asserted as text rather than as a value: the field
/// order rule this codebase keeps hitting is a property of the *serializer*, and a
/// round trip through a struct would not see it. `[workspace]` being emitted at all
/// is here for the same reason — an empty table is the one thing a serializer might
/// reasonably drop.
#[test]
fn a_package_is_a_manifest_and_a_main() {
    let mut scratchpad = scratchpad();
    scratchpad.dependencies = vec![
        dependency("rand", "0.8"),
        // Out of order and untrimmed on purpose: the manifest sorts and trims, the
        // list does not.
        dependency(" anyhow ", " 1.0.86 "),
    ];

    assert_eq!(
        scratchpad.manifest().expect("a manifest"),
        "\
[package]
name = \"sketch\"
version = \"0.1.0\"
edition = \"2021\"

[dependencies]
anyhow = \"1.0.86\"
rand = \"0.8\"

[workspace]
"
    );
}

/// The empty case is the one that actually ships, so it is asserted whole too: no
/// `[dependencies]` header at all rather than an empty one.
#[test]
fn a_scratchpad_with_no_crates_has_no_dependencies_table() {
    let manifest = scratchpad().manifest().expect("a manifest");
    assert!(!manifest.contains("[dependencies]"), "{manifest}");

    let package = manifest.find("[package]").expect("the package table");
    let workspace = manifest.find("[workspace]").expect("the workspace table");
    assert!(package < workspace, "{manifest}");
}

#[test]
fn a_row_that_is_not_a_crate_name_says_which_row() {
    let mut scratchpad = scratchpad();
    scratchpad.dependencies = vec![
        dependency("serde", "1"),
        dependency("", "1"),
        dependency("1password", "1"),
        dependency("hello world", "1"),
        dependency("a".repeat(MAX_NAME + 1), "1"),
    ];

    assert_eq!(
        scratchpad.problems(),
        vec![
            (1, Problem::NoName),
            (2, Problem::NameStart),
            (3, Problem::NameCharacter(' ')),
            (4, Problem::NameTooLong),
        ]
    );
}

#[test]
fn a_version_that_is_not_a_version_says_so() {
    for good in [
        "1",
        "1.2",
        "1.2.3",
        "^1.2.3",
        "~1.2",
        "=1.2.3",
        ">=1.2, <2.0",
        "1.0.0-rc.1",
        "1.0.0-alpha+build.5",
        " 1.0 ",
    ] {
        assert_eq!(dependency("serde", good).check(), Ok(()), "{good}");
    }

    for (bad, problem) in [
        ("", Problem::NoVersion),
        ("   ", Problem::NoVersion),
        // The whole point of the requirement, and its own answer.
        ("*", Problem::Wildcard),
        ("1.*", Problem::Wildcard),
        (">=1, <2.*", Problem::Wildcard),
        ("latest", Problem::NotAVersion),
        ("v1.2", Problem::NotAVersion),
        ("1.2.3.4", Problem::NotAVersion),
        ("1..2", Problem::NotAVersion),
        ("1.2-", Problem::NotAVersion),
        ("1.2-rc/1", Problem::NotAVersion),
        (">=1,", Problem::NotAVersion),
    ] {
        assert_eq!(dependency("serde", bad).check(), Err(problem), "{bad:?}");
    }
}

/// A table cannot hold a key twice, so the second row would silently win. That is
/// exactly the "silently different build" this module exists to refuse.
#[test]
fn the_same_crate_twice_is_a_row_that_says_so() {
    let mut scratchpad = scratchpad();
    scratchpad.dependencies = vec![
        dependency("serde", "1"),
        dependency(" serde ", "2"),
        // A second empty row is empty, not a duplicate: it has nothing to duplicate.
        dependency("", ""),
        dependency("", ""),
    ];

    assert_eq!(
        scratchpad.problems(),
        vec![
            (1, Problem::Repeated),
            (2, Problem::NoName),
            (3, Problem::NoName),
        ]
    );
}

#[test]
fn a_scratchpad_with_a_bad_row_will_not_write() {
    let directory = directory(line!());
    let mut scratchpad = scratchpad();
    scratchpad.dependencies = vec![dependency("rand", "")];

    let failure = scratchpad.write_to(&directory).expect_err("a refusal");
    assert_eq!(
        failure,
        Failure::Dependencies(vec![(0, Problem::NoVersion)])
    );
    // And nothing was written on the way to refusing.
    assert!(!directory.exists());

    // A build refuses in the same terms rather than in cargo's.
    assert_eq!(scratchpad.build_in(&directory), Build::Unavailable(failure));
    assert!(!directory.exists());
}

/// The package is the storage, so this is the whole of the persistence test: what
/// was written comes back, dependencies and all, with no second file involved.
#[test]
fn writes_and_reads_back() {
    let directory = directory(line!());
    let mut scratchpad = scratchpad();
    scratchpad.source = "fn main() { /* edited */ }\n".to_owned();
    scratchpad.dependencies = vec![dependency("anyhow", "1.0.86")];

    scratchpad.write_to(&directory).expect("writing");
    assert_eq!(Scratchpad::load_from(&directory), Some(scratchpad));

    // The temporaries were renamed, not left behind.
    assert!(!directory.join("Cargo.toml.tmp").exists());
    assert!(!directory.join("src").join("main.rs.tmp").exists());

    // A directory with nothing in it is not a scratchpad, and neither is one with a
    // manifest and no source.
    assert_eq!(Scratchpad::load_from(&directory.join("src")), None);
    fs::remove_file(directory.join("src").join("main.rs")).expect("removing the source");
    assert_eq!(Scratchpad::load_from(&directory), None);

    let _ = fs::remove_dir_all(&directory);
}

/// What the app opens with, and the reason [`Scratchpad::default`] may hand out a
/// name without a `Result`: it is a name this module would accept if it were typed,
/// and it is a package it would agree to write.
#[test]
fn the_default_scratchpad_is_one_this_module_would_write() {
    let scratchpad = Scratchpad::default();

    assert_eq!(scratchpad.name(), DEFAULT_NAME);
    assert_eq!(Scratchpad::new(DEFAULT_NAME), Ok(scratchpad.clone()));
    assert!(scratchpad.problems().is_empty());
    assert!(scratchpad.manifest().is_ok());
}

/// Reopening: what is on disk wins over what the caller was holding, except for the
/// name -- which is the directory the next write goes back to, and so cannot be
/// something a hand-edited manifest gets to choose.
#[test]
fn a_scratchpad_opens_as_its_directory_has_it() {
    let directory = directory(line!());

    // Nothing there yet: what the caller was holding, unchanged, so a first run opens
    // on the default source rather than on nothing.
    let fresh = Scratchpad::default().opened_in(&directory);
    assert_eq!(fresh, Scratchpad::default());

    let mut written = scratchpad();
    written.source = "fn main() { /* saved */ }\n".to_owned();
    written.dependencies = vec![dependency("anyhow", "1.0.86")];
    written.write_to(&directory).expect("writing");

    let opened = Scratchpad::default().opened_in(&directory);
    assert_eq!(opened.source, written.source);
    assert_eq!(opened.dependencies, written.dependencies);
    // The manifest says `sketch` and the caller asked for `scratch`. The caller wins:
    // the name is where the next write lands.
    assert_eq!(opened.name(), DEFAULT_NAME);

    let _ = fs::remove_dir_all(&directory);
}

/// Which box a row's problem belongs against. The editor marks one of the two, so
/// this is the model's answer rather than the view guessing from the wording.
#[test]
fn a_problem_is_about_one_half_of_its_row() {
    for problem in [
        Problem::NoName,
        Problem::NameStart,
        Problem::NameCharacter('/'),
        Problem::NameTooLong,
        // A repeat is a name collision: `[dependencies]` is keyed by the name, and
        // the version of the second row is what would silently go missing.
        Problem::Repeated,
    ] {
        assert_eq!(problem.half(), Half::Name, "{problem:?}");
    }

    for problem in [Problem::NoVersion, Problem::Wildcard, Problem::NotAVersion] {
        assert_eq!(problem.half(), Half::Version, "{problem:?}");
    }
}

/// Writing twice is what the editor does on every build, so the second write has to
/// be the new source and not a merge of the two.
#[test]
fn writing_again_replaces_what_was_there() {
    let directory = directory(line!());
    let mut scratchpad = scratchpad();
    scratchpad.write_to(&directory).expect("writing");
    scratchpad.source = "fn main() {}\n".to_owned();
    scratchpad.dependencies = vec![dependency("rand", "0.8")];
    scratchpad.write_to(&directory).expect("writing again");

    assert_eq!(Scratchpad::load_from(&directory), Some(scratchpad));

    let _ = fs::remove_dir_all(&directory);
}

/// What a failed build reports, over a canned cargo stream — which is why `outcome`
/// is a function of its own. Nothing here shells out.
#[test]
fn a_failed_build_reports_the_compilers_diagnostics() {
    let stdout = concat!(
        r#"{"reason":"compiler-artifact","executable":null}"#,
        "\n",
        r#"{"reason":"compiler-message","package_id":"sketch","message":{"#,
        r#""level":"error","message":"cannot find value `x` in this scope","#,
        r#""rendered":"error[E0425]: cannot find value `x`\n --> src/main.rs:2:5\n","#,
        r#""spans":[{"file_name":"src/main.rs","line_start":2,"column_start":5,"is_primary":true}]}}"#,
        "\n",
        r#"{"reason":"build-finished","success":false}"#,
        "\n",
    );
    let stderr = "   Compiling sketch v0.1.0\nerror: could not compile `sketch`\n";

    let Build::Rejected {
        diagnostics,
        message,
    } = outcome(stdout, stderr, false)
    else {
        panic!("a rejection");
    };

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].level, Level::Error);
    assert_eq!(
        diagnostics[0].message,
        "cannot find value `x` in this scope"
    );
    assert!(diagnostics[0].rendered.contains("E0425"));
    assert_eq!(
        diagnostics[0].span,
        Some(Span {
            file: "src/main.rs".into(),
            line: 2,
            column: 5,
        })
    );
    // cargo's own stderr is kept whole: some failures are said there and nowhere
    // else.
    assert!(message.contains("could not compile"));
}

/// The failure with no diagnostics behind it at all — a dependency row that names a
/// crate nothing has heard of. cargo says it on stderr and emits no compiler
/// message, so a build result that only carried diagnostics would report nothing.
#[test]
fn a_dependency_that_does_not_resolve_is_cargos_own_words() {
    let stderr = "error: no matching package named `not-a-real-crate` found\n\
                      location searched: crates.io index\n";

    assert_eq!(
        outcome("", stderr, false),
        Build::Rejected {
            diagnostics: Vec::new(),
            message: stderr.trim().to_owned(),
        }
    );
}

#[test]
fn a_successful_build_reports_the_artifact_and_its_warnings() {
    let stdout = concat!(
        r#"{"reason":"compiler-artifact","executable":null}"#,
        "\n",
        r#"{"reason":"build-script-executed","package_id":"libc"}"#,
        "\n",
        r#"{"reason":"compiler-message","message":{"level":"warning","#,
        r#""message":"unused variable: `y`","rendered":"warning: unused variable","#,
        r#""spans":[]}}"#,
        "\n",
        r#"{"reason":"compiler-artifact","executable":"/tmp/sketch/target/debug/sketch"}"#,
        "\n",
        r#"{"reason":"build-finished","success":true}"#,
        "\n",
        "warning: some future cargo says something new here\n",
    );

    let Build::Built {
        executable,
        diagnostics,
    } = outcome(stdout, "", true)
    else {
        panic!("a build");
    };

    assert_eq!(executable, PathBuf::from("/tmp/sketch/target/debug/sketch"));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].level, Level::Warning);
    assert_eq!(diagnostics[0].span, None);
}

/// A cargo that succeeded and named nothing is a third answer, not an `unwrap`.
#[test]
fn a_build_that_names_no_artifact_says_so() {
    assert_eq!(
        outcome(r#"{"reason":"build-finished","success":true}"#, "", true),
        Build::Unavailable(Failure::NoArtifact)
    );
}

/// The one test that shells out. It is hermetic and needs no network: a scratchpad
/// with no dependencies never touches the registry, so this is one rustc invocation
/// in a temporary directory. `$CARGO` is set for anything cargo launches, this test
/// included, so the cargo running the suite is the cargo that runs here.
#[test]
fn an_empty_scratchpad_really_builds() {
    let directory = directory(line!());
    let mut scratchpad = scratchpad();
    scratchpad.source = "fn main() {}\n".to_owned();

    let build = scratchpad.build_in(&directory);
    let Build::Built { executable, .. } = &build else {
        panic!("a build, got {build:?}");
    };
    // The path cargo named, and not one derived from the crate name: this is the
    // whole argument for `--message-format=json` in one assertion.
    assert!(executable.is_file(), "{}", executable.display());

    let _ = fs::remove_dir_all(&directory);
}

/// The line cap, over a reader that never says anything: a program writing megabytes
/// with no newline in it must still be *delivered*, in pieces, rather than kept in one
/// growing string nobody ever sees.
#[test]
fn a_line_with_no_end_to_it_is_cut_rather_than_kept() {
    let written = "x".repeat(MAX_LINE as usize * 2 + 7);
    let mut lines = Vec::new();
    stream_lines(io::Cursor::new(written), Stream::Out, |line| {
        lines.push(line)
    });

    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].text.len(), MAX_LINE as usize);
    assert_eq!(lines[1].text.len(), MAX_LINE as usize);
    assert_eq!(lines[2].text.len(), 7);
    assert!(lines.iter().all(|line| line.stream == Stream::Out));
}

/// And the ordinary case, including the two things a naive `read_line` gets wrong: a
/// Windows line ending left in the text, and a last line with no terminator at all
/// being dropped instead of delivered.
#[test]
fn lines_arrive_without_their_terminators() {
    let mut lines = Vec::new();
    stream_lines(
        io::Cursor::new("first\r\nsecond\n\nlast"),
        Stream::Err,
        |line| lines.push(line),
    );

    let text: Vec<&str> = lines.iter().map(|line| &*line.text).collect();
    assert_eq!(text, ["first", "second", "", "last"]);
    assert!(lines.iter().all(|line| line.stream == Stream::Err));
}

/// The other bound. A program printing in a tight loop is not an edge case in a
/// scratchpad, so what has to be true is that the *oldest* goes and that the view can
/// say how much of the story it is missing.
#[test]
fn output_keeps_the_newest_and_counts_what_it_dropped() {
    let mut output = RunOutput::default();
    assert_eq!(output.len(), 0);

    for line in 0..MAX_OUTPUT_LINES + 12 {
        output.push(OutputLine {
            stream: Stream::Out,
            text: Arc::from(line.to_string().as_str()),
        });
    }

    assert_eq!(output.len(), MAX_OUTPUT_LINES);
    assert_eq!(output.dropped(), 12);
    // The oldest kept is the twelfth written, and the newest is the last.
    assert_eq!(&*output.line(0).expect("a line").text, "12");
    assert_eq!(
        &*output.line(MAX_OUTPUT_LINES - 1).expect("a line").text,
        (MAX_OUTPUT_LINES + 11).to_string()
    );
    assert_eq!(output.line(MAX_OUTPUT_LINES), None);
}

/// Build a scratchpad whose source is `source` and hand back what to run. Hermetic
/// and needs no network for `an_empty_scratchpad_really_builds`'s reason: no
/// dependencies means no registry, so it is one rustc invocation in a temporary
/// directory.
fn program(directory: &Path, source: &str) -> PathBuf {
    let mut scratchpad = scratchpad();
    scratchpad.source = source.to_owned();

    let build = scratchpad.build_in(directory);
    let Build::Built { executable, .. } = &build else {
        panic!("a build, got {build:?}");
    };
    executable.clone()
}

/// Whether [`stop_all`] would still reach this run. Not called here -- other tests
/// have programs of their own running in parallel threads of this same binary, and
/// stopping *all* of them is exactly what it does.
fn listed(running: &Running) -> bool {
    RUNNING
        .lock()
        .expect("the list")
        .iter()
        .any(|other| Arc::ptr_eq(&other.0, &running.0))
}

/// Collect a run's events until it ends, or give up saying so.
fn until_ended(events: &std::sync::mpsc::Receiver<RunEvent>) -> Vec<RunEvent> {
    let mut collected = Vec::new();
    loop {
        let event = events
            .recv_timeout(Duration::from_secs(30))
            .expect("the run never ended");
        let ended = matches!(event, RunEvent::Ended(_));
        collected.push(event);
        if ended {
            return collected;
        }
    }
}

/// A program that prints and exits: both streams arrive, the end comes last, and the
/// exit status is the program's own.
///
/// Asserted without an order *between* the streams, which is not a promise this module
/// makes and could not keep: two pipes read by two threads deliver in whatever order
/// the kernel woke them. Within a stream the order is the program's own, which is what
/// the other run tests rest on.
#[test]
fn a_program_that_prints_and_exits_is_streamed_and_reported() {
    let directory = directory(line!());
    let executable = program(
        &directory,
        "fn main() {\n\
             \x20   println!(\"to stdout\");\n\
             \x20   eprintln!(\"to stderr\");\n\
             \x20   std::process::exit(3);\n\
             }\n",
    );

    let (events, arrived) = std::sync::mpsc::channel();
    let running = run_in(&executable, &directory, move |event| {
        let _ = events.send(event);
    })
    .expect("it started");

    let collected = until_ended(&arrived);
    let (ended, written) = collected.split_last().expect("something happened");
    // The program's own status, not a zero for having run at all, and said last.
    assert_eq!(ended, &RunEvent::Ended(Ended::Exited(Some(3))));

    let mut written: Vec<(Stream, String)> = written
        .iter()
        .map(|event| match event {
            RunEvent::Wrote(line) => (line.stream, line.text.to_string()),
            RunEvent::Ended(_) => panic!("it ended twice"),
        })
        .collect();
    written.sort_by(|left, right| left.1.cmp(&right.1));
    assert_eq!(
        written,
        vec![
            (Stream::Err, "to stderr".to_owned()),
            (Stream::Out, "to stdout".to_owned()),
        ]
    );
    assert!(running.finished());

    let _ = fs::remove_dir_all(&directory);
}

/// The hazard this whole sub-step is about: a program that does not exit.
///
/// Two things have to be true and only a real process can say either. What it printed
/// **before** it stopped exiting is on screen — which is the difference between
/// streaming and collecting an output at exit, since this one has no exit — and asking
/// it to stop really ends it. `Ended::Stopped` arriving is itself the proof of the
/// second: it is emitted only after the process has been *reaped*, so a run that
/// reports it is a run the system no longer has.
#[test]
fn a_program_that_never_exits_still_says_something_and_can_be_killed() {
    let directory = directory(line!());
    let executable = program(
        &directory,
        "fn main() {\n\
             \x20   println!(\"before the loop\");\n\
             \x20   loop { std::thread::sleep(std::time::Duration::from_millis(50)); }\n\
             }\n",
    );

    let (events, arrived) = std::sync::mpsc::channel();
    let running = run_in(&executable, &directory, move |event| {
        let _ = events.send(event);
    })
    .expect("it started");

    // Said while it is still going, which is the whole point.
    assert_eq!(
        arrived.recv_timeout(Duration::from_secs(30)),
        Ok(RunEvent::Wrote(OutputLine {
            stream: Stream::Out,
            text: Arc::from("before the loop"),
        }))
    );
    assert!(!running.finished(), "it exited on its own");
    // On the list the window's close hook walks, which is the only way a program
    // still going when the app goes away can be reached at all.
    assert!(listed(&running), "nothing would have stopped it at exit");

    running.stop();
    assert_eq!(until_ended(&arrived), vec![RunEvent::Ended(Ended::Stopped)]);
    assert!(running.finished());
    // And off it again, so a long session of runs is not a list of dead handles.
    assert!(!listed(&running), "it stayed on the list after it ended");

    let _ = fs::remove_dir_all(&directory);
}

/// Nothing to run is an answer and not a panic — the executable a build named can be
/// gone by the time the reader presses the button.
#[test]
fn a_program_that_is_not_there_says_so() {
    let directory = directory(line!());
    let failure = run_in(&directory.join("not-a-program"), &directory, |_| {})
        .err()
        .expect("a refusal");

    assert!(matches!(failure, Failure::NoProgram(_)), "{failure:?}");
}

/// And the same directory built again with source that does not compile: what a
/// failed build reports, end to end, once.
#[test]
fn a_scratchpad_that_does_not_compile_reports_it() {
    let directory = directory(line!());
    let mut scratchpad = scratchpad();
    scratchpad.source = "fn main() { let _: u32 = \"not a number\"; }\n".to_owned();

    let build = scratchpad.build_in(&directory);
    let Build::Rejected { diagnostics, .. } = &build else {
        panic!("a rejection, got {build:?}");
    };
    let error = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.level == Level::Error)
        .expect("an error");
    assert_eq!(
        error.span.as_ref().map(|span| span.file.as_str()),
        Some("src/main.rs")
    );

    let _ = fs::remove_dir_all(&directory);
}
