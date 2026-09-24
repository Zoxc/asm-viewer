use std::io::Cursor;

use super::*;

impl Handle {
    /// Whether nothing more is to be done to it: stopped, or seen to have ended by
    /// itself. Nothing in the app asks -- a handle that is over has left the list a
    /// shutdown walks, which is the only reason anything wanted to know -- so the
    /// question is the tests' alone.
    pub fn finished(&self) -> bool {
        self.0.over.load(Ordering::SeqCst)
    }

    /// A handle with no process behind it, for the tests: everything a handle is asked
    /// about a program it has stopped is bookkeeping, and only the killing needs one.
    pub fn to_nothing() -> Handle {
        Handle(Arc::new(Process {
            child: Mutex::new(None),
            over: AtomicBool::new(false),
            list: &STARTED,
        }))
    }
}

/// The line cap: a program writing megabytes with no newline in it must still be
/// *delivered*, in pieces, rather than kept in one growing string nobody ever sees.
#[test]
fn a_line_with_no_end_to_it_is_cut_rather_than_kept() {
    let written = "x".repeat(MAX_LINE as usize * 2 + 7);
    let mut lines = Vec::new();
    stream_lines(Cursor::new(written), Stream::Out, |line| lines.push(line));

    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].text.len(), MAX_LINE as usize);
    assert_eq!(lines[1].text.len(), MAX_LINE as usize);
    assert_eq!(lines[2].text.len(), 7);
    assert!(lines.iter().all(|line| line.stream == Stream::Out));
}

/// The cut falls between characters and not between bytes. A program printing a long line
/// of box-drawing glyphs -- a table, a progress bar -- would otherwise show a replacement
/// character at the seam of every 4 KiB, with the character that was there on neither of
/// the two rows.
#[test]
fn a_character_on_the_cut_is_not_split_between_two_rows() {
    // The cut lands one byte into the `é`, so its two bytes are on either side of it.
    let written = format!("{}é{}", "x".repeat(MAX_LINE as usize - 1), "y".repeat(10));
    let mut lines = Vec::new();
    stream_lines(Cursor::new(written.clone()), Stream::Out, |line| {
        lines.push(line)
    });

    assert_eq!(lines.len(), 2);
    // One byte short of the cap: what the character needed is on the next row instead.
    assert_eq!(lines[0].text.len(), MAX_LINE as usize - 1);
    assert_eq!(&*lines[1].text, format!("é{}", "y".repeat(10)));
    // Nothing added and nothing lost: the rows are the line, in pieces.
    assert_eq!(
        lines.iter().map(|line| &*line.text).collect::<String>(),
        written
    );
}

/// A line the cut falls exactly at the end of is one row, not that row and an empty one
/// made of its terminator. Either terminator, and a line that is two cuts long too; an
/// empty line the program did write after one is still a row.
#[test]
fn a_line_as_long_as_the_cut_is_one_row() {
    let full = "x".repeat(MAX_LINE as usize);
    let written = format!("{full}\n{full}\r\n{full}{full}\n\nlast");
    let mut lines = Vec::new();
    stream_lines(Cursor::new(written), Stream::Out, |line| lines.push(line));

    let lengths: Vec<usize> = lines.iter().map(|line| line.text.len()).collect();
    let cut = MAX_LINE as usize;
    // The `\r` of the second line is past the cut, so it is read with its `\n`.
    assert_eq!(lengths, [cut, cut, cut, cut, 0, 4]);
}

/// What is not a character is still delivered, lossily, as it always was. The carry is for
/// a cut this module made, and `error_len() == None` is what tells that from output that is
/// simply not UTF-8: bytes that are genuinely invalid must not be held back for a
/// continuation nobody is going to write.
#[test]
fn what_is_not_a_character_is_still_delivered() {
    let mut written = b"a\xffb\n".to_vec();
    // The first two bytes of a box-drawing character, and then the program stops.
    written.extend_from_slice(&[0xe2, 0x94]);

    let mut lines = Vec::new();
    stream_lines(Cursor::new(written), Stream::Err, |line| lines.push(line));

    let text: Vec<&str> = lines.iter().map(|line| &*line.text).collect();
    assert_eq!(text, ["a\u{fffd}b", "\u{fffd}"]);
}

/// The ordinary case, including the two things a naive `read_line` gets wrong: a Windows
/// line ending left in the text, and a last line with no terminator being dropped.
#[test]
fn lines_arrive_without_their_terminators() {
    let mut lines = Vec::new();
    stream_lines(
        Cursor::new("first\r\nsecond\n\nlast"),
        Stream::Err,
        |line| lines.push(line),
    );

    let text: Vec<&str> = lines.iter().map(|line| &*line.text).collect();
    assert_eq!(text, ["first", "second", "", "last"]);
    assert!(lines.iter().all(|line| line.stream == Stream::Err));
}

/// The other bound: the *oldest* goes, and the view can say how much of the story it is
/// missing. Past a whole block, so the block that was let go is gone and every line after
/// it is still where the index says.
#[test]
fn output_keeps_the_newest_and_counts_what_it_dropped() {
    let mut output = RunOutput::default();
    assert_eq!(output.len(), 0);

    let over = CHUNK + 12;
    for line in 0..MAX_OUTPUT_LINES + over {
        output.push(OutputLine {
            stream: Stream::Out,
            text: Arc::from(line.to_string().as_str()),
        });
    }

    assert_eq!(output.len(), MAX_OUTPUT_LINES);
    assert_eq!(output.dropped(), over);
    // The oldest kept is the first not dropped, and the newest is the last.
    for index in 0..MAX_OUTPUT_LINES {
        assert_eq!(
            &*output.line(index).expect("a line").text,
            (index + over).to_string()
        );
    }
    assert_eq!(output.line(MAX_OUTPUT_LINES), None);
}

/// **A line added to a copy copies no full block.** The pane holds the output the app
/// pushes into, so every batch copies it first, and a copy of every line per batch was the
/// cost.
#[test]
fn a_copy_of_the_output_shares_its_full_blocks() {
    let line = OutputLine {
        stream: Stream::Out,
        text: Arc::from("line"),
    };
    let mut output = RunOutput::default();
    for _ in 0..3 * CHUNK + 5 {
        output.push(line.clone());
    }

    let copy = output.clone();
    output.push(line);
    assert_eq!(output.sealed.len(), 3);
    assert!(output
        .sealed
        .iter()
        .zip(&copy.sealed)
        .all(|(mine, theirs)| Arc::ptr_eq(mine, theirs)));
    assert_eq!(output.len(), copy.len() + 1);
}

/// Every event a run said, in order, once both its pipes are at their end, with `handle`
/// what the caller does to the running program. The `Ended` is the last thing any run
/// says, so it is what the wait is on.
#[cfg(unix)]
fn events_of(command: &mut Command, handle: impl FnOnce(&Handle)) -> (Handle, Vec<RunEvent>) {
    let said = Arc::new(Mutex::new(Vec::new()));
    let running = run("a test run's output reader", command, {
        let said = said.clone();
        move |event| {
            said.lock()
                .unwrap_or_else(|held| held.into_inner())
                .push(event)
        }
    })
    .expect("/bin/sh started");
    handle(&running);

    // Polled rather than joined: the two reader threads are the run's own and nothing
    // hands them back. Half a minute is a build machine under load, not a program that is
    // slow to print.
    for _ in 0..30_000 {
        let held = said.lock().unwrap_or_else(|held| held.into_inner());
        if held.iter().any(|event| matches!(event, RunEvent::Ended(_))) {
            return (running.clone(), held.clone());
        }
        drop(held);
        thread::sleep(Duration::from_millis(1));
    }
    running.stop();
    panic!("the run never ended");
}

/// **`Ended` is said exactly once**, after both pipes have reached their end. Two reader
/// threads count one process down, and either of them saying it on its own -- or a count
/// no pipe ever reaches -- is a pad that reads "Running" for ever, or one whose verdict is
/// written twice.
#[cfg(unix)]
#[test]
fn a_run_says_it_ended_once_and_last() {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg("echo out; echo err 1>&2; exit 3")
        .stdin(Stdio::null());
    let (_handle, said) = events_of(&mut command, |_| {});

    let endings: Vec<&Ended> = said
        .iter()
        .filter_map(|event| match event {
            RunEvent::Ended(ended) => Some(ended),
            RunEvent::Wrote(_) => None,
        })
        .collect();
    assert_eq!(endings.len(), 1, "what the run said: {said:?}");
    assert_eq!(endings[0], &Ended::Exited(Some(3)));
    assert!(
        matches!(said.last(), Some(RunEvent::Ended(_))),
        "a line arrived after the end: {said:?}"
    );

    // Both pipes were read, and each line arrived before the end.
    let lines: Vec<(Stream, &str)> = said
        .iter()
        .filter_map(|event| match event {
            RunEvent::Wrote(line) => Some((line.stream, &*line.text)),
            RunEvent::Ended(_) => None,
        })
        .collect();
    assert!(lines.contains(&(Stream::Out, "out")), "{lines:?}");
    assert!(lines.contains(&(Stream::Err, "err")), "{lines:?}");
}

/// A program with nothing to say on either pipe still ends, once. It is the same count,
/// reached without a line going through it.
#[cfg(unix)]
#[test]
fn a_run_that_writes_nothing_still_says_it_ended() {
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg("exit 0").stdin(Stdio::null());
    let (_handle, said) = events_of(&mut command, |_| {});

    assert_eq!(said, vec![RunEvent::Ended(Ended::Exited(Some(0)))]);
}

/// **A run this app stopped ends once too, and comes off the list a shutdown walks.** A
/// stop is no special path through the count: the pipes close with the process, both
/// readers finish, and the last of them reaps what the stop already waited for. A run left
/// on the list is a `stop_all` signalling a pid the system is free to have handed on.
#[cfg(unix)]
#[test]
fn a_stopped_run_says_it_ended_once_and_leaves_the_list() {
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg("sleep 30").stdin(Stdio::null());
    let (handle, said) = events_of(&mut command, |running| running.stop());

    assert_eq!(said, vec![RunEvent::Ended(Ended::Stopped)]);
    let list = STARTED.lock().unwrap_or_else(|held| held.into_inner());
    assert!(
        !list.handles.contains(&handle),
        "the reaped run is still on the list a shutdown walks"
    );
}

/// **A program started after the shutdown walked the list is stopped, not listed.** The
/// workers run on while the shutdown does, and a run started just after `stop_all` took
/// the list used to go on a list nothing would walk again, and outlive the app.
#[cfg(unix)]
#[test]
fn a_start_after_the_shutdown_is_stopped_at_once() {
    let list: &'static Mutex<Started> = Box::leak(Box::new(Mutex::new(Started::new())));
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg("sleep 30").stdin(Stdio::null());
    let (before, _pipes) = start_in(list, &mut command).expect("/bin/sh started");

    stop_all_in(list);
    assert!(
        before.finished(),
        "the shutdown did not stop what was listed"
    );

    let error = start_in(list, &mut command)
        .err()
        .expect("a start after the shutdown");
    assert_eq!(error.to_string(), "the app is closing");
    let listed = list.lock().unwrap_or_else(|held| held.into_inner());
    assert!(listed.handles.is_empty(), "a program went on a walked list");
}

/// Whether `handle` is still on the list a shutdown walks.
#[cfg(unix)]
fn listed(handle: &Handle) -> bool {
    let list = STARTED.lock().unwrap_or_else(|held| held.into_inner());
    list.handles.iter().any(|other| other == handle)
}

/// **A handle leaves the list the moment it is known to be gone**, and by that one rule.
/// A stop used to leave one on it: the list was pruned of the finished by the next
/// `start`, so a language server stopped and never started again sat there until the
/// shutdown's `stop_all` signalled a pid the system was free to have handed on.
#[cfg(unix)]
#[test]
fn a_stopped_program_leaves_the_list_at_once() {
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg("sleep 30").stdin(Stdio::null());
    let (handle, _pipes) = start(&mut command).expect("/bin/sh started");
    assert!(listed(&handle), "a started program is not on the list");

    handle.stop();
    assert!(
        !listed(&handle),
        "a stopped program is still on the list a shutdown walks"
    );
}

/// The other end of the same rule: a program waited for and found gone comes off the list
/// too, and the wait says how it went whether it is bounded or not.
#[cfg(unix)]
#[test]
fn a_program_waited_for_and_found_gone_leaves_the_list_too() {
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg("exit 7").stdin(Stdio::null());
    let (handle, _pipes) = start(&mut command).expect("/bin/sh started");

    assert_eq!(
        handle.ending(Duration::from_secs(30)),
        Some(Ended::Exited(Some(7)))
    );
    assert!(!listed(&handle), "the reaped program is still on the list");
}

/// What `output` hands back is both pipes whole and how the program ended, as
/// `Command::output` would say it.
#[cfg(unix)]
#[test]
fn an_output_is_both_pipes_and_the_exit() {
    let list: &'static Mutex<Started> = Box::leak(Box::new(Mutex::new(Started::new())));
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg("echo out; echo err >&2; exit 3")
        .stdin(Stdio::null());
    let output = output_in(list, "a test's stderr", &mut command).expect("/bin/sh started");

    assert_eq!(output.ended, Ended::Exited(Some(3)));
    assert_eq!(output.stdout, b"out\n");
    assert_eq!(output.stderr, b"err\n");
    let listed = list.lock().unwrap_or_else(|held| held.into_inner());
    assert!(
        listed.handles.is_empty(),
        "the ended program is still listed"
    );
}

/// **A program whose output is wanted at its end is on the list a shutdown walks** while it
/// runs. A cargo build went through `Command::output`, which no shutdown could reach, and
/// went on building after the app had closed.
#[cfg(unix)]
#[test]
fn an_output_still_running_is_stopped_by_the_shutdown() {
    let list: &'static Mutex<Started> = Box::leak(Box::new(Mutex::new(Started::new())));
    let running = thread::spawn(move || {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg("sleep 30").stdin(Stdio::null());
        output_in(list, "a test's stderr", &mut command).expect("/bin/sh started")
    });
    let until = Instant::now() + Duration::from_secs(30);
    while list
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .handles
        .is_empty()
    {
        assert!(Instant::now() < until, "the program never went on the list");
        thread::sleep(Duration::from_millis(5));
    }

    stop_all_in(list);
    let output = running.join().expect("the wait did not panic");
    assert_eq!(output.ended, Ended::Stopped);
}
