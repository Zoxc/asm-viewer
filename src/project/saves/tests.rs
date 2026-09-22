//! What is written when, against the two baselines.

use super::*;
use crate::cargo::Profile;
use crate::project::files::tests::*;
use crate::project::files::Cargo;
use crate::store::PROJECTS_DIR;

/// The writes landing, which is what the caller does once `write_or_warn` has answered:
/// the baselines move with the files and not before. Every test but the ones about a
/// write that fails goes through this.
fn landed(saves: &mut Saves, recorded: Option<Recorded>) -> Option<(Project, Option<Session>)> {
    let recorded = recorded?;
    saves.wrote_project(&recorded.project, recorded.binaries_changed);
    if let Some(session) = recorded.session.clone() {
        saves.wrote_session(session);
    }
    Some((recorded.project, recorded.session))
}

/// The same for the session a flush writes.
fn flushed(saves: &mut Saves) -> Option<Session> {
    let session = saves.take_owing()?;
    saves.wrote_session(session.clone());
    Some(session)
}

/// The same for the project file a change to the details owes.
fn owed(saves: &mut Saves) -> Option<Project> {
    let project = saves.take_owed_project()?;
    saves.wrote_project(&project, false);
    Some(project)
}

/// `record` with the details the project already has, so every test using this is asking
/// about a change to the binaries or the session and nothing else. The rename tests spell
/// theirs out.
fn recorded(
    saves: &mut Saves,
    binaries: Vec<PathBuf>,
    session: Session,
) -> Option<(Project, Option<Session>)> {
    let unchanged = saves.written.details.clone();
    let bookmarks = saves.written.bookmarks.clone();
    let decided = saves.record(&unchanged, &binaries, false, &bookmarks, session);
    landed(saves, decided)
}

fn written(
    saves: &mut Saves,
    binaries: &[&str],
    selection: Option<&str>,
) -> Option<(Project, Option<Session>)> {
    recorded(saves, paths(binaries), session_with(selection))
}

/// `record` from inside a load: the app holds `binaries` so far and the rest are still
/// being read.
fn mid_load(
    saves: &mut Saves,
    binaries: &[&str],
    session: Session,
) -> Option<(Project, Option<Session>)> {
    let unchanged = saves.written.details.clone();
    let bookmarks = saves.written.bookmarks.clone();
    let decided = saves.record(&unchanged, &paths(binaries), true, &bookmarks, session);
    landed(saves, decided)
}

#[test]
fn the_state_the_app_boots_into_is_never_written() {
    let mut saves = Saves::default();
    // The save observer's first run, before anything is restored. Nothing may come of
    // it: the files on disk are the good ones, and no project directory is allocated.
    assert_eq!(recorded(&mut saves, Vec::new(), Session::default()), None);
    assert_eq!(flushed(&mut saves), None);
}

#[test]
fn opening_a_binary_is_written_at_once() {
    let mut saves = Saves::default();

    let written = written(&mut saves, &["/tmp/lib.a"], None);
    assert_eq!(
        written,
        Some((
            Project {
                id: None,
                details: Details::default(),
                binaries: paths(&["/tmp/lib.a"]),
                bookmarks: Vec::new(),
            },
            Some(session_with(None)),
        ))
    );
    // And is not written a second time by the next flush.
    assert_eq!(flushed(&mut saves), None);
}

/// Closing one takes the same path opening one does: `binaries` is what `record` looks
/// at, and it does not care in which direction the list changed.
#[test]
fn closing_a_binary_is_written_at_once() {
    let mut saves = Saves::default();
    written(&mut saves, &["/tmp/lib.a", "/tmp/some.dll"], Some("a.o"));

    // The selection is still pending from the open above; closing writes the lot, so
    // `session.toml` never names a place inside a binary `project.toml` has let go of.
    let written = written(&mut saves, &["/tmp/lib.a"], Some("a.o"));
    assert_eq!(
        written.as_ref().map(|(project, _)| &project.binaries),
        Some(&paths(&["/tmp/lib.a"]))
    );
    assert_eq!(
        written.and_then(|(_, session)| session),
        Some(session_with(Some("a.o")))
    );
    assert_eq!(flushed(&mut saves), None);
}

/// The empty project is a project, and it has to reach the disk or the next run reopens
/// what was just closed.
#[test]
fn closing_the_only_binary_is_written_too() {
    let mut saves = Saves::default();
    written(&mut saves, &["/tmp/lib.a"], Some("a.o"));

    let written = recorded(&mut saves, Vec::new(), Session::default());
    assert_eq!(
        written,
        Some((Project::default(), Some(Session::default())))
    );
    assert_eq!(flushed(&mut saves), None);
}

#[test]
fn a_selection_change_waits_for_the_flush() {
    let mut saves = Saves::default();
    written(&mut saves, &["/tmp/lib.a"], None);

    assert_eq!(written(&mut saves, &["/tmp/lib.a"], Some("a.o")), None);
    assert_eq!(flushed(&mut saves), Some(session_with(Some("a.o"))));
    assert_eq!(flushed(&mut saves), None);
}

#[test]
fn recording_the_same_project_again_changes_nothing() {
    let mut saves = Saves::default();
    written(&mut saves, &["/tmp/lib.a"], None);

    // A pending change re-recorded unchanged, as the save observer does whenever
    // something it does not persist wakes it.
    written(&mut saves, &["/tmp/lib.a"], Some("a.o"));
    assert_eq!(written(&mut saves, &["/tmp/lib.a"], Some("a.o")), None);
    // Still pending, and still exactly one write.
    assert_eq!(flushed(&mut saves), Some(session_with(Some("a.o"))));
    assert_eq!(flushed(&mut saves), None);

    // And once written, re-recording it is not a second write either.
    assert_eq!(written(&mut saves, &["/tmp/lib.a"], Some("a.o")), None);
    assert_eq!(flushed(&mut saves), None);
}

#[test]
fn opening_a_binary_carries_the_pending_change_with_it() {
    let mut saves = Saves::default();
    written(&mut saves, &["/tmp/lib.a"], Some("a.o"));

    // The selection is pending; opening a second binary writes the lot.
    let written = written(&mut saves, &["/tmp/lib.a", "/tmp/some.dll"], Some("a.o"));
    assert_eq!(
        written.and_then(|(_, session)| session),
        Some(session_with(Some("a.o")))
    );
    assert_eq!(flushed(&mut saves), None);
}

/// A tab is pending and not an immediate write, and nothing in `record` says so: which
/// file a field lives in is what decides it, and a tab lives in the session.
#[test]
fn opening_a_tab_waits_for_the_flush() {
    let mut saves = Saves::default();
    written(&mut saves, &["/tmp/lib.a"], None);

    let mut session = session_with(Some("a.o"));
    session.tabs = vec![saved_tab("a.o", 0)];
    assert_eq!(
        recorded(&mut saves, paths(&["/tmp/lib.a"]), session.clone()),
        None
    );
    assert_eq!(flushed(&mut saves), Some(session));
    assert_eq!(flushed(&mut saves), None);
}

/// The directory survives a record that is not about it, the write carrying it rather than
/// the absence a derived project would have.
#[test]
fn a_record_keeps_the_directory_the_project_was_given() {
    let mut saves = Saves::default();
    let named = Project {
        id: None,
        details: Details {
            directory: Some(PathBuf::from("/src/kernel")),
            ..Details::default()
        },
        binaries: paths(&["/tmp/vmlinux"]),
        bookmarks: Vec::new(),
    };
    saves.opened(
        &Store::at("/state"),
        kept_at("kernel-1"),
        &named,
        &Session::default(),
    );

    let (project, _) = written(&mut saves, &["/tmp/lib.a"], None).expect("a write");
    assert_eq!(
        project.details.directory,
        Some(PathBuf::from("/src/kernel"))
    );
    // And the binaries are the ones the app is showing: that half *is* derived.
    assert_eq!(project.binaries, paths(&["/tmp/lib.a"]));
}

/// A reopen seeds what the user *said* and not the contents: the directory is restored
/// synchronously, while the binaries arrive from a worker thread — so a baseline holding
/// them would read the still-empty boot state as a change and write an empty project over a
/// good one.
#[test]
fn reopening_seeds_the_details_but_not_the_baseline() {
    let mut saves = Saves::default();
    let loaded = Project {
        id: None,
        details: Details {
            directory: Some(PathBuf::from("/src/kernel")),
            ..Details::default()
        },
        binaries: paths(&["/tmp/vmlinux"]),
        bookmarks: Vec::new(),
    };
    saves.opened(
        &Store::at("/state"),
        kept_at("kernel-1"),
        &loaded,
        &Session::default(),
    );

    // The boot state equals the baseline, so nothing is written.
    assert_eq!(recorded(&mut saves, Vec::new(), Session::default()), None);
    // And the restore that follows is an ordinary change, written at once.
    let (project, _) =
        recorded(&mut saves, paths(&["/tmp/vmlinux"]), Session::default()).expect("a write");
    assert_eq!(project, loaded);
}

/// The objects arrive one at a time, so a list still being read is not the app's list.
/// Writing it would put a project naming only what has landed on disk, and with it the
/// empty session the app holds until the restore has resolved its tabs.
#[test]
fn a_binary_landing_mid_load_is_not_written() {
    let mut saves = Saves::default();
    let loaded = Project {
        id: None,
        details: Details::default(),
        binaries: paths(&["/tmp/vmlinux", "/tmp/lib.a"]),
        bookmarks: Vec::new(),
    };
    saves.opened(
        &Store::at("/state"),
        kept_at("kernel-1"),
        &loaded,
        &Session::default(),
    );

    // The first of the two lands, and the app's session is still the empty one. Nothing
    // is written, and nothing is left pending for a flush to write either.
    assert_eq!(
        mid_load(&mut saves, &["/tmp/vmlinux"], Session::default()),
        None
    );
    assert_eq!(flushed(&mut saves), None);

    // The second lands while the load is still in flight, so the baseline stays behind
    // it: the record after the load has to see a change even where nothing more arrived.
    assert_eq!(
        mid_load(
            &mut saves,
            &["/tmp/vmlinux", "/tmp/lib.a"],
            Session::default()
        ),
        None
    );

    // The load ends, the restore resolves the session against everything it opened, and
    // that record is the one that writes: both files, and the whole list.
    let (project, session) = recorded(
        &mut saves,
        loaded.binaries.clone(),
        session_with(Some("a.o")),
    )
    .expect("a write");
    assert_eq!(project, loaded);
    assert_eq!(session, Some(session_with(Some("a.o"))));
}

/// The session is held back mid-load for the binaries' reason and one more: a session is
/// only ever marked pending, and whatever is pending is what the next flush writes. The
/// app holds no tabs until the restore has resolved them, so a close, a switch or the
/// timer landing inside the load would put that tabless session on disk over the good
/// file.
#[test]
fn a_session_recorded_mid_load_is_not_left_pending() {
    let mut saves = Saves::default();
    let loaded = Project {
        id: None,
        details: Details::default(),
        binaries: paths(&["/tmp/vmlinux"]),
        bookmarks: Vec::new(),
    };
    saves.opened(
        &Store::at("/state"),
        kept_at("kernel-1"),
        &loaded,
        &Session::default(),
    );

    // The save observer runs as the object lands. The app has the page it was on back,
    // but no tabs and no active document: those wait for the load to end.
    let half = Session {
        active_page: Some(String::from("settings")),
        ..Session::default()
    };
    assert_eq!(mid_load(&mut saves, &["/tmp/vmlinux"], half.clone()), None);
    assert_eq!(
        flushed(&mut saves),
        None,
        "a flush inside the load would write the tabless session over the good file"
    );

    // The restore resolves the tabs, and the record after the load is the one that sees
    // the session -- whole.
    let whole = Session {
        active: Some(saved_object("a.o")),
        ..half
    };
    let (project, session) =
        recorded(&mut saves, loaded.binaries.clone(), whole.clone()).expect("a write");
    assert_eq!(project, loaded);
    assert_eq!(session, Some(whole));
}

/// A write that does go out mid-load -- a directory typed in, a bookmark -- must not take
/// the half-read list for the baseline either, or the record after the load would see no
/// change and the file would never learn the rest of it.
#[test]
fn a_detail_changed_mid_load_leaves_the_binaries_baseline_behind() {
    let mut saves = Saves::default();
    written(&mut saves, &["/tmp/lib.a"], None);

    // A second binary is opened, and the reader points the project somewhere while it is
    // still being read.
    let named = Details {
        directory: Some(PathBuf::from("/src/kernel")),
        ..saves.written.details.clone()
    };
    let decided = saves.record(
        &named,
        &paths(&["/tmp/lib.a", "/tmp/some.dll"]),
        true,
        &[],
        Session::default(),
    );
    assert!(decided.is_none(), "owed to the flush");
    let project = owed(&mut saves).expect("a write");
    assert_eq!(
        project.details.directory,
        Some(PathBuf::from("/src/kernel"))
    );
    assert_eq!(
        project.binaries,
        paths(&["/tmp/lib.a"]),
        "the listed binaries, not the half-read list"
    );

    // The load ends, and this is the record that has to put the second binary on disk.
    let (project, _) = recorded(
        &mut saves,
        paths(&["/tmp/lib.a", "/tmp/some.dll"]),
        Session::default(),
    )
    .expect("a write");
    assert_eq!(project.binaries, paths(&["/tmp/lib.a", "/tmp/some.dll"]));
}

/// A change to what the user said is owed to the next flush rather than written at once,
/// since a box typed in makes one per keystroke. It is a project-file write and nothing
/// else: it lets go of no binary and so cannot leave the two files disagreeing.
#[test]
fn a_detail_is_owed_to_the_flush_and_leaves_the_session_pending() {
    let mut saves = Saves::default();
    written(&mut saves, &["/tmp/lib.a"], None);
    // A selection, pending as ever.
    written(&mut saves, &["/tmp/lib.a"], Some("a.o"));

    let named = Details {
        directory: Some(PathBuf::from("/src/kernel")),
        language_server: None,
        language_files: None,
        cargo: None,
    };
    let decided = saves.record(
        &named.clone(),
        &paths(&["/tmp/lib.a"]),
        false,
        &[],
        session_with(Some("a.o")),
    );
    assert!(decided.is_none(), "nothing written at once");
    assert_eq!(
        owed(&mut saves),
        Some(Project {
            id: None,
            details: named.clone(),
            binaries: paths(&["/tmp/lib.a"]),
            bookmarks: Vec::new(),
        })
    );
    // The session is still pending: the change did not take it along.
    assert_eq!(flushed(&mut saves), Some(session_with(Some("a.o"))));

    // And the same directory recorded again owes nothing.
    let decided = saves.record(
        &named,
        &paths(&["/tmp/lib.a"]),
        false,
        &[],
        session_with(Some("a.o")),
    );
    assert_eq!(landed(&mut saves, decided), None);
    assert_eq!(owed(&mut saves), None);
}

/// A detail typed and typed back before the flush owes nothing: the file already holds it.
#[test]
fn a_detail_changed_back_owes_nothing() {
    let mut saves = Saves::default();
    let named = Details {
        directory: Some(PathBuf::from("/src/kernel")),
        ..Details::default()
    };
    assert!(saves
        .record(&named, &[], false, &[], Session::default())
        .is_none());
    assert!(saves
        .record(&Details::default(), &[], false, &[], Session::default())
        .is_none());
    assert_eq!(owed(&mut saves), None);
}

/// A write that goes out at once carries the details the app holds, so it takes the owed
/// write with it: a flush after it must not write an older project over a newer one.
#[test]
fn a_write_at_once_takes_the_owed_details_along() {
    let mut saves = Saves::default();
    let named = Details {
        directory: Some(PathBuf::from("/src/kernel")),
        ..Details::default()
    };
    saves.record(&named, &[], false, &[], Session::default());

    let decided = saves.record(
        &named,
        &paths(&["/tmp/lib.a"]),
        false,
        &[],
        Session::default(),
    );
    let (project, _) = landed(&mut saves, decided).expect("a write");
    assert_eq!(project.details, named);
    assert_eq!(project.binaries, paths(&["/tmp/lib.a"]));
    assert_eq!(owed(&mut saves), None);
}

/// An owed write that failed is owed again, for the close hook's flush to find.
#[test]
fn an_owed_project_whose_write_failed_is_still_owed() {
    let mut saves = Saves::default();
    let named = Details {
        directory: Some(PathBuf::from("/src/kernel")),
        ..Details::default()
    };
    saves.record(&named, &[], false, &[], Session::default());

    let failed = saves.take_owed_project().expect("a project to write");
    saves.owes_project(failed);
    assert_eq!(owed(&mut saves).map(|project| project.details), Some(named));
    assert_eq!(owed(&mut saves), None);
}

/// Clearing a detail writes the key away rather than leaving the old one on disk.
#[test]
fn clearing_a_detail_is_a_change_too() {
    let mut saves = Saves::default();
    saves.opened(
        &Store::at("/state"),
        kept_at("kernel-1"),
        &Project {
            details: Details {
                directory: Some(PathBuf::from("/src/kernel")),
                ..Details::default()
            },
            ..Project::default()
        },
        &Session::default(),
    );

    let decided = saves.record(&Details::default(), &[], false, &[], Session::default());
    assert!(decided.is_none());
    let project = owed(&mut saves).expect("a write");
    assert_eq!(project.details.directory, None);
}

/// A detail changed while the binaries are still being parsed writes back the list the file
/// already holds: the app holds none in that window, and writing its own empty list would
/// forget them through a change that had nothing to do with them.
#[test]
fn a_detail_changed_before_the_binaries_have_loaded_does_not_forget_them() {
    let mut saves = Saves::default();
    let loaded = Project {
        id: None,
        details: Details::default(),
        binaries: paths(&["/tmp/vmlinux", "/tmp/lib.a"]),
        bookmarks: Vec::new(),
    };
    saves.opened(
        &Store::at("/state"),
        kept_at("kernel-1"),
        &loaded,
        &Session::default(),
    );

    let named = Details {
        directory: Some(PathBuf::from("/src/kernel")),
        language_server: None,
        language_files: None,
        cargo: None,
    };
    saves.record(&named, &[], true, &[], Session::default());
    let project = owed(&mut saves).expect("a write");
    assert_eq!(
        project.details.directory,
        Some(PathBuf::from("/src/kernel"))
    );
    assert_eq!(project.binaries, loaded.binaries);

    // Once the parse lands the write *is* about the binaries, which is the one kind that
    // may replace the list.
    let decided = saves.record(
        &saves.written.details.clone(),
        &paths(&["/tmp/vmlinux"]),
        false,
        &[],
        Session::default(),
    );
    let written = landed(&mut saves, decided).expect("a write");
    assert_eq!(written.0.binaries, paths(&["/tmp/vmlinux"]));
    // Closing the last one is still a real change and still empties the file.
    let written = recorded(&mut saves, Vec::new(), Session::default()).expect("a write");
    assert_eq!(written.0.binaries, Vec::<PathBuf>::new());
}

/// A write that did not land leaves the change for the next record to see: a baseline is
/// what the file holds, so it moves with the file and not with the decision to write it.
/// Otherwise a full disk costs the reader their binaries and bookmarks until the next
/// binaries change, with one warning in a log a windowed app never shows.
#[test]
fn a_write_that_failed_is_recorded_again() {
    let mut saves = Saves::default();

    // A binaries change, written at once -- and neither file reaches the disk, so
    // nothing is noted as written and the session is owed.
    let decided = saves.record(
        &saves.written.details.clone(),
        &paths(&["/tmp/lib.a"]),
        false,
        &[],
        session_with(Some("a.o")),
    );
    let failed = decided.expect("a write");
    assert_eq!(failed.project.binaries, paths(&["/tmp/lib.a"]));
    saves.owes_session(failed.session.expect("the session went with it"));

    // The next record is the same change over again, both halves of it.
    let (project, session) = recorded(
        &mut saves,
        paths(&["/tmp/lib.a"]),
        session_with(Some("a.o")),
    )
    .expect("the write again");
    assert_eq!(project.binaries, paths(&["/tmp/lib.a"]));
    assert_eq!(session, Some(session_with(Some("a.o"))));
    // And now that it has landed, it is not written a third time.
    assert_eq!(flushed(&mut saves), None);
}

/// **The id is stamped once**, and by the policy: every half `Saves::record` hands back
/// already carries the open project's id, and so does the session a flush takes out. The
/// writes used to stamp both again for a project whose file was claimed by the first
/// write -- a mechanism that is gone, ids now being minted in `start_new` and `put_in`.
#[test]
fn the_project_id_is_stamped_once_and_both_halves_come_back_with_it() {
    let mut saves = Saves::default();
    let open = a_project();
    saves.opened(
        &Store::at("/state"),
        kept_at("kernel-1"),
        &open,
        &Session::default(),
    );

    // A binaries change: both halves at once, and both stamped.
    let decided = saves
        .record(
            &open.details.clone(),
            &paths(&["/tmp/lib.a"]),
            false,
            &[],
            session_with(Some("a.o")),
        )
        .expect("a write");
    assert_eq!(decided.project.id, open.id);
    let carried = decided.session.clone().expect("the session went with it");
    assert_eq!(carried.id, open.id);
    landed(&mut saves, Some(decided));

    // And a session-only change, which waits for a flush: stamped before it is compared
    // against the baseline, so it is stamped by the time it is taken out again.
    let decided = saves.record(
        &open.details.clone(),
        &paths(&["/tmp/lib.a"]),
        false,
        &[],
        session_with(Some("b.o")),
    );
    assert!(decided.is_none(), "the session alone waits");
    assert_eq!(saves.take_owing().map(|owed| owed.id), Some(open.id));
}

/// The same for the session a flush writes. The tick that fails is the ordinary case, and
/// the close hook's flush is the one that must not then find nothing to do.
#[test]
fn a_session_whose_write_failed_is_still_owed() {
    let mut saves = Saves::default();
    written(&mut saves, &["/tmp/lib.a"], None);
    // A selection, pending as ever.
    written(&mut saves, &["/tmp/lib.a"], Some("a.o"));

    // The 30 s tick, and the write fails: taken out and handed straight back.
    let owed = saves.take_owing().expect("a session to write");
    assert_eq!(owed, session_with(Some("a.o")));
    saves.owes_session(owed.clone());

    // The window is closed, and the hook's flush still has it.
    assert_eq!(flushed(&mut saves), Some(owed));
    assert_eq!(flushed(&mut saves), None);
}

/// The user-given half of a project is compared **field by field**, so a change to any
/// one of the four owes a write of `project.toml` on its own and the same details again
/// owe nothing. A field left out of the comparison would read as "nothing changed"
/// and be lost until something else was written.
#[test]
fn a_change_to_any_one_detail_is_owed_and_the_same_one_again_is_not() {
    for change in [
        Details {
            directory: Some(PathBuf::from("/src/kernel")),
            ..Details::default()
        },
        Details {
            language_server: Some("ra-multiplex".into()),
            ..Details::default()
        },
        Details {
            language_files: Some("c h".into()),
            ..Details::default()
        },
        Details {
            cargo: Some(Cargo {
                profile: Profile::Debug,
            }),
            ..Details::default()
        },
    ] {
        let mut saves = Saves::default();
        let decided = saves.record(&change, &[], false, &[], Session::default());
        assert!(decided.is_none(), "the project file alone: {change:?}");
        let project = owed(&mut saves).expect("a write");
        assert_eq!(project.details, change, "{change:?}");

        saves.record(&change, &[], false, &[], Session::default());
        assert_eq!(owed(&mut saves), None, "written twice: {change:?}");
    }
}

/// Entering another project empties every baseline, the app being about to be emptied: a
/// baseline still describing the old binaries would write that emptying into the new
/// project.
#[test]
fn entering_a_project_empties_every_baseline() {
    let mut saves = Saves::default();
    written(&mut saves, &["/tmp/lib.a"], Some("a.o"));

    let entered = Project {
        details: Details {
            directory: Some(PathBuf::from("/src/other")),
            ..Details::default()
        },
        ..Project::default()
    };
    saves.opened(
        &Store::at("/state"),
        kept_at("other-2"),
        &entered,
        &Session::default(),
    );

    // The state a switch leaves the app in: nothing open, nothing selected, and the
    // directory of the project just entered — every one of them the baseline.
    let decided = saves.record(
        &Details {
            directory: entered.details.directory.clone(),
            language_server: None,
            language_files: None,
            cargo: None,
        },
        &[],
        false,
        &[],
        Session::default(),
    );
    assert_eq!(landed(&mut saves, decided), None);
    // Nor is the old project's pending session waiting to be written into the new one.
    assert_eq!(flushed(&mut saves), None);
}

/// The claim is the `create_new`, so two claims in the same directory cannot land on the
/// same name.
#[test]
fn unsaved_projects_do_not_collide() {
    let directory = directory();

    let store = Store::at(directory.join(PROJECTS_DIR));
    let first = unsaved_project(&store).expect("a file");
    let second = unsaved_project(&store).expect("a second file");
    assert_ne!(first, second);
    assert!(first.is_file());
    assert!(second.is_file());

    // A file that is already there is stepped over rather than opened, whether this app
    // made it or not.
    let squatter = directory.join(format!("3.{PROJECT_EXTENSION}"));
    fs::write(&squatter, b"someone else's").expect("a squatter");
    let third = unsaved_project(&store).expect("a third file");
    assert_ne!(third, squatter);
    assert_eq!(
        fs::read(&squatter).expect("the squatter reads"),
        b"someone else's"
    );
}

/// A bookmarks change is written at once and to `project.toml` alone, like a rename: it
/// lets go of no binary, so it cannot leave the two files disagreeing, and it writes back
/// the binaries the file already lists rather than the app's own.
#[test]
fn a_bookmarks_change_writes_the_project_file_alone() {
    let mut saves = Saves::default();
    let reopened = Project {
        bookmarks: vec![Bookmark {
            name: Some("caller".into()),
            document: saved_symbol("a.o", "caller", 0),
        }],
        ..a_project()
    };
    saves.opened(
        &Store::at("/state"),
        kept_at("1"),
        &reopened,
        &Session::default(),
    );

    // Seeded: the same bookmarks are no change, while the parse has yet to land.
    let unchanged = saves.record(
        &saves.written.details.clone(),
        &[],
        false,
        &reopened.bookmarks.clone(),
        Session::default(),
    );
    assert!(unchanged.is_none());

    let mut added = reopened.bookmarks.clone();
    added.push(Bookmark {
        name: Some("target".into()),
        document: saved_symbol("a.o", "target", 6),
    });
    let decided = saves.record(
        &saves.written.details.clone(),
        &[],
        false,
        &added.clone(),
        Session::default(),
    );
    let (project, session) = landed(&mut saves, decided).expect("a write");
    assert!(session.is_none(), "the session went with it");
    assert_eq!(project.bookmarks, added);
    assert_eq!(
        project.binaries, reopened.binaries,
        "the listed binaries, not the app's"
    );

    // Removing them all is a change too, written as an absent key.
    let decided = saves.record(
        &saves.written.details.clone(),
        &[],
        false,
        &[],
        Session::default(),
    );
    let (project, _) = landed(&mut saves, decided).expect("a write");
    assert!(project.bookmarks.is_empty());
}
