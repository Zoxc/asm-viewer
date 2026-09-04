use super::*;

/// A directory of this test's own under the system temporary directory, named after the
/// line that asked for it, so a failing test leaves something identifiable behind.
fn directory(line: u32) -> PathBuf {
    std::env::temp_dir().join(format!(
        "assembly-viewer-scratchpad-test-{}-{line}",
        std::process::id()
    ))
}

fn scratchpad() -> Scratchpad {
    Scratchpad::new("sketch").expect("an id")
}

fn dependency(name: impl Into<String>, version: impl Into<String>) -> Dependency {
    Dependency {
        name: name.into(),
        version: version.into(),
    }
}

/// The whole generated manifest, asserted as text rather than as a value: the field order
/// rule is a property of the *serializer*, and a round trip through a struct would not see
/// it. `[workspace]` being emitted at all is here for the same reason.
#[test]
fn a_package_is_a_manifest_and_a_main() {
    let mut scratchpad = scratchpad();
    // What the reader calls it is under `[package.metadata]`, the one place cargo lets a
    // tool of its own keep anything -- and it is under no obligation to be a crate name,
    // where `[package] name`, which is the id, is. Untrimmed on purpose, like the rows.
    scratchpad.name = " a name with spaces ".to_owned();
    scratchpad.dependencies = vec![
        dependency("rand", "0.8"),
        // Out of order and untrimmed on purpose: the manifest sorts and trims, the list
        // does not.
        dependency(" anyhow ", " 1.0.86 "),
    ];

    assert_eq!(
        scratchpad.manifest().expect("a manifest"),
        "\
[package]
name = \"sketch\"
version = \"0.1.0\"
edition = \"2021\"

[package.metadata.scratchpad]
name = \"a name with spaces\"

[dependencies]
anyhow = \"1.0.86\"
rand = \"0.8\"

[workspace]
"
    );
}

/// The empty case is the one that actually ships: no `[dependencies]` header at all rather
/// than an empty one.
#[test]
fn a_scratchpad_with_no_crates_has_no_dependencies_table() {
    let manifest = scratchpad().manifest().expect("a manifest");
    assert!(!manifest.contains("[dependencies]"), "{manifest}");
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

/// A table cannot hold a key twice, so the second row would silently win.
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

/// The package is the storage, so this is the whole of the persistence test.
#[test]
fn writes_and_reads_back() {
    let directory = directory(line!());
    let mut scratchpad = scratchpad();
    scratchpad.source = "fn main() { /* edited */ }\n".to_owned();
    scratchpad.dependencies = vec![dependency("anyhow", "1.0.86")];
    // A name nothing could file a pad under: it is a value in the package and not the
    // directory, so it may hold spaces, punctuation and any alphabet at all.
    scratchpad.name = "Sam's ✎ notes".to_owned();

    scratchpad.write_to(&directory).expect("writing");
    assert_eq!(Scratchpad::load_from(&directory), Some(scratchpad.clone()));

    // The temporaries were renamed, not left behind.
    assert!(!directory.join("Cargo.toml.tmp").exists());
    assert!(!directory.join("src").join("main.rs.tmp").exists());

    // Writing again replaces rather than merges -- the name included, a rename being an
    // ordinary edit now that nothing is filed under it.
    scratchpad.source = "fn main() {}\n".to_owned();
    scratchpad.name = "renamed".to_owned();
    scratchpad.dependencies = vec![dependency("rand", "0.8")];
    scratchpad.write_to(&directory).expect("writing again");
    assert_eq!(Scratchpad::load_from(&directory), Some(scratchpad));

    // A directory with nothing in it is not a scratchpad, and neither is one with a
    // manifest and no source.
    assert_eq!(Scratchpad::load_from(&directory.join("src")), None);
    fs::remove_file(directory.join("src").join("main.rs")).expect("removing the source");
    assert_eq!(Scratchpad::load_from(&directory), None);

    let _ = fs::remove_dir_all(&directory);
}

/// An id is the directory a pad lives in, so it is read back through the same check the app
/// generated it through. A file naming something that is not one is refused rather than
/// interpolated into a path.
#[test]
fn an_id_out_of_a_file_goes_through_the_same_check_a_generated_one_does() {
    assert_eq!(
        PadId::new("sketch").map(|id| id.as_str().to_owned()),
        Some("sketch".to_owned())
    );
    // Not trimmed, unlike a name: nobody types an id, so there is no stray space to
    // forgive, and a path component with a space at either end is a different directory.
    assert_eq!(PadId::new("  sketch  "), None);
    assert_eq!(PadId::new(""), None);
    assert_eq!(PadId::new("9lives"), None);
    assert_eq!(PadId::new("a/b"), None);
    assert_eq!(PadId::new(".."), None);

    // And the same through serde, which is the path a hand-edited file takes.
    let read = |text: &str| toml::from_str::<BTreeMap<String, PadId>>(text);
    assert_eq!(
        read("name = \"sketch\"").expect("an id")["name"].as_str(),
        "sketch"
    );
    assert!(read("name = \"../evil\"").is_err());
}

/// A directory whose manifest names something that could not be a directory is not a
/// scratchpad. That is the same sentence the pad listing is built on: a directory
/// [`Scratchpad::load_from`] answers for is a pad, anything else is not — and the crate's
/// name is the id, so this is where a hand-edited one is caught.
#[test]
fn a_manifest_naming_a_path_is_not_a_scratchpad() {
    let directory = directory(line!());
    let source = directory.join("src");
    fs::create_dir_all(&source).expect("the directory");
    fs::write(
        directory.join("Cargo.toml"),
        "[package]\nname = \"../evil\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("the manifest");
    fs::write(source.join("main.rs"), "fn main() {}\n").expect("the source");

    assert_eq!(Scratchpad::load_from(&directory), None);

    let _ = fs::remove_dir_all(&directory);
}

/// The reason [`Scratchpad::default`] may hand out an id without an `Option`: it is an id
/// this module would generate, and a package it would agree to write. It carries no name at
/// all, a pad nobody has named having an empty one and what stands in for it on screen
/// being the UI's business.
#[test]
fn the_default_scratchpad_is_one_this_module_would_write() {
    let scratchpad = Scratchpad::default();

    assert_eq!(scratchpad.id().as_str(), DEFAULT_ID);
    assert_eq!(scratchpad.name(), "");
    assert_eq!(Scratchpad::new(DEFAULT_ID), Some(scratchpad.clone()));
    assert!(scratchpad.problems().is_empty());
    assert!(scratchpad.manifest().is_ok());
}

/// Reopening: what is on disk wins over what the caller was holding, except for the name,
/// which is the directory the next write goes back to.
#[test]
fn a_scratchpad_opens_as_its_directory_has_it() {
    let directory = directory(line!());

    // Nothing there yet: what the caller was holding, unchanged.
    let fresh = Scratchpad::default().opened_in(&directory);
    assert_eq!(fresh, Scratchpad::default());

    let mut written = scratchpad();
    written.source = "fn main() { /* saved */ }\n".to_owned();
    written.dependencies = vec![dependency("anyhow", "1.0.86")];
    written.write_to(&directory).expect("writing");

    let opened = Scratchpad::default().opened_in(&directory);
    assert_eq!(opened.source, written.source);
    assert_eq!(opened.dependencies, written.dependencies);
    // The manifest's crate name says `sketch` and the caller asked for `scratch`: the
    // caller wins, because the id is where the next write goes. The *name* comes off the
    // disk like everything else, being a value and not a place.
    assert_eq!(opened.id().as_str(), DEFAULT_ID);
    assert_eq!(opened.name(), written.name());

    let _ = fs::remove_dir_all(&directory);
}

/// An id for the tests below, which all deal in ids this module would generate.
fn id(text: &str) -> PadId {
    PadId::new(text).expect("an id")
}

/// What the listing says a pad is, for asserting against.
fn row(id_text: &str, name: &str) -> PadListing {
    PadListing {
        id: id(id_text),
        name: name.to_owned(),
    }
}

/// The order is an *order* and not an index of what exists, so `touch` answers whether
/// anything moved — which is the whole of why a startup that reopens the pad already at the
/// front writes no file.
#[test]
fn the_order_answers_whether_anything_moved() {
    let mut order = PadOrder::default();

    assert!(order.touch(&id("one")));
    assert!(order.touch(&id("two")));
    assert_eq!(order.first(), Some(&id("two")));
    // Already at the front: nothing moved, so nothing is written.
    assert!(!order.touch(&id("two")));
    // Behind the front: it moves, and is not repeated.
    assert!(order.touch(&id("one")));
    assert_eq!(order.ids(), [id("one"), id("two")]);
}

/// Every pad is reachable, which is the difference from the recent-projects list: that one
/// is the projects a reader has *opened*, where this is the scratchpads there are. So an id
/// the order has kept whose directory is not a package is dropped, and a package the order
/// has never heard of is listed anyway. Each row carries the name out of that pad's own
/// package, which is what lets the panel draw a pad nothing has opened.
#[test]
fn the_listing_drops_what_is_not_a_pad_and_keeps_what_the_order_forgot() {
    let base = directory(line!());
    let pads = base.join("scratchpads");
    fs::create_dir_all(&pads).expect("the directory");

    for (pad, name) in [("kept", "Kept one"), ("stray", "")] {
        let mut scratchpad = Scratchpad::of(id(pad));
        scratchpad.name = name.to_owned();
        scratchpad.write_to(&pads.join(pad)).expect("writing");
    }
    // A directory with nothing in it, which `load_from` does not answer for.
    fs::create_dir(pads.join("empty")).expect("the directory");

    let mut order = PadOrder::default();
    order.touch(&id("empty"));
    order.touch(&id("gone"));
    order.touch(&id("kept"));
    write_toml(&pads.join("recents.toml"), &order).expect("the order");

    // `kept` from the order, then the pad the order never named. `gone` has no directory
    // and `empty` is not a package, so neither is a row. A pad the reader never named
    // comes back with an empty name rather than with its id.
    assert_eq!(pads_in(&base), [row("kept", "Kept one"), row("stray", "")]);

    let _ = fs::remove_dir_all(&base);
}

/// A new pad claims its directory with the `create_dir` that fails rather than opens, so an
/// id another copy of the app is already using is stepped over rather than taken. The
/// package goes in at once: a claimed directory with nothing in it is not a pad, and the
/// listing above would repair it away.
#[test]
fn a_new_pad_steps_over_what_is_already_claimed() {
    let base = directory(line!());
    let pads = base.join("scratchpads");
    fs::create_dir_all(pads.join("pad-1")).expect("the squatter");

    let made = new_pad_in(&base).expect("a pad");
    assert_eq!(made.id().as_str(), "pad-2");
    // And no name: naming it is the reader's, and until they do the pane calls it
    // `<pad-2>` without anything having been written down.
    assert_eq!(made.name(), "");
    assert_eq!(
        Scratchpad::load_from(&pads.join("pad-2")),
        Some(made.clone())
    );
    // And it is at the front of the order, so it is what a restart would open.
    assert_eq!(
        PadOrder::load_from(&pads.join("recents.toml")).first(),
        Some(made.id())
    );

    let _ = fs::remove_dir_all(&base);
}

/// A delete takes the pad's whole directory, cargo's leavings included — and reaches
/// nothing else. The path is the id's, and an id is a checked crate name, so the only way
/// left to aim a `remove_dir_all` at something that is not a pad is for the directory to
/// have stopped being one, which is what the load answers. The pad beside it is untouched,
/// which is the assertion that would fail if the path were ever built from anything but the
/// id.
#[test]
fn a_delete_takes_the_package_and_only_the_package() {
    let base = directory(line!());
    let pads = base.join("scratchpads");
    fs::create_dir_all(&pads).expect("the directory");

    for pad in ["going", "staying"] {
        Scratchpad::of(id(pad))
            .write_to(&pads.join(pad))
            .expect("writing");
    }
    // What cargo leaves behind, which goes with the pad rather than being left orphaned.
    fs::create_dir_all(pads.join("going").join("target")).expect("the directory");

    assert_eq!(delete_pad_in(&base, &id("going")), Ok(()));
    assert!(!pads.join("going").exists());
    assert!(Scratchpad::load_from(&pads.join("staying")).is_some());

    // Gone already is not a failure: the pad a first run holds has no directory until
    // something is typed into it.
    assert_eq!(delete_pad_in(&base, &id("going")), Ok(()));

    // A directory that is not a package is refused rather than removed, whatever the order
    // beside it says about it.
    let stranger = pads.join("stranger");
    fs::create_dir(&stranger).expect("the directory");
    fs::write(stranger.join("notes.txt"), "someone's own").expect("the file");
    assert!(matches!(
        delete_pad_in(&base, &id("stranger")),
        Err(Failure::Delete(_))
    ));
    assert!(stranger.join("notes.txt").exists());

    // A link where a pad's directory should be is refused as well: `symlink_metadata` does
    // not follow one, so a delete reaches the directory itself or nothing.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(pads.join("staying"), pads.join("linked")).expect("a link");
        assert!(matches!(
            delete_pad_in(&base, &id("linked")),
            Err(Failure::Delete(_))
        ));
        assert!(Scratchpad::load_from(&pads.join("staying")).is_some());
    }

    let _ = fs::remove_dir_all(&base);
}

/// The line cap: a program writing megabytes with no newline in it must still be
/// *delivered*, in pieces, rather than kept in one growing string nobody ever sees.
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

/// The ordinary case, including the two things a naive `read_line` gets wrong: a Windows
/// line ending left in the text, and a last line with no terminator being dropped.
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

/// The other bound: the *oldest* goes, and the view can say how much of the story it is
/// missing.
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

/// Build a scratchpad whose source is `source` and hand back what to run. Hermetic and
/// needs no network: no dependencies means no registry, so it is one rustc invocation in a
/// temporary directory. The path is the one cargo named.
fn program(directory: &Path, source: &str) -> PathBuf {
    let mut scratchpad = scratchpad();
    scratchpad.source = source.to_owned();

    let build = scratchpad.build_in(directory);
    let Build::Built { executable, .. } = &build else {
        panic!("a build, got {build:?}");
    };
    executable.clone()
}

/// Whether [`stop_all`] would still reach this run. Not called here -- other tests have
/// programs of their own running in parallel threads of this same binary.
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

/// A program that prints and exits: both streams arrive, the end comes last, and the exit
/// status is the program's own.
///
/// Asserted without an order *between* the streams, which is not a promise this module
/// makes: two pipes read by two threads deliver in whatever order the kernel woke them.
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

/// The hazard this is all about: a program that does not exit. What it printed **before**
/// it stopped exiting is on screen, and asking it to stop really ends it — `Ended::Stopped`
/// is emitted only after the process has been reaped.
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
    // On the list the window's close hook walks.
    assert!(listed(&running), "nothing would have stopped it at exit");

    running.stop();
    assert_eq!(until_ended(&arrived), vec![RunEvent::Ended(Ended::Stopped)]);
    assert!(running.finished());
    // And off it again, so a long session of runs is not a list of dead handles.
    assert!(!listed(&running), "it stayed on the list after it ended");

    let _ = fs::remove_dir_all(&directory);
}

/// The group is real, which is the half of "a stop kills the grandchildren too" that can be
/// asserted: a run is started in a process group of its own, so the pgid the kernel reports
/// for the child is the child's own pid and not this test binary's. That is what makes
/// `kill(-pgid)` the program and everything it forked, rather than everything this process
/// belongs to. What is inside the group once the program starts forking is the same fact
/// one step on, and is judged by hand.
#[cfg(unix)]
#[test]
fn a_run_is_a_process_group_of_its_own() {
    let directory = directory(line!());
    let executable = program(
        &directory,
        "fn main() {\n\
             \x20   loop { std::thread::sleep(std::time::Duration::from_millis(50)); }\n\
             }\n",
    );

    let (events, arrived) = std::sync::mpsc::channel();
    let running = run_in(&executable, &directory, move |event| {
        let _ = events.send(event);
    })
    .expect("it started");

    // Asked of the kernel and not of the `Command`: the group is set between the fork and
    // the exec, so nothing this side of the spawn has seen it.
    let pid = {
        let child = running.0.child.lock().expect("the child");
        child.id() as i32
    };
    let group = unsafe { libc::getpgid(pid) };
    assert_eq!(group, pid, "the run did not lead a group of its own");
    // And it is not the one the test binary is in, which is what it would have inherited.
    assert_ne!(group, unsafe { libc::getpgid(0) });

    // Reaped before the test ends, so nothing is left behind for the next one to find.
    running.stop();
    assert_eq!(until_ended(&arrived), vec![RunEvent::Ended(Ended::Stopped)]);

    let _ = fs::remove_dir_all(&directory);
}

/// Nothing to run is an answer and not a panic — the executable a build named can be gone
/// by the time the reader presses the button.
#[test]
fn a_program_that_is_not_there_says_so() {
    let directory = directory(line!());
    let failure = run_in(&directory.join("not-a-program"), &directory, |_| {})
        .err()
        .expect("a refusal");

    assert!(matches!(failure, Failure::NoProgram(_)), "{failure:?}");
}
