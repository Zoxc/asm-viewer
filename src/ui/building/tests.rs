//! What [`Builds`] does with a finished build.

use super::*;
use crate::temporary::Temporary;

fn built(paths: &[&str]) -> cargo::Run {
    cargo::Run::Built {
        artifacts: paths
            .iter()
            .map(|path| cargo::Artifact {
                path: PathBuf::from(path),
                target: "viewer".to_owned(),
                kind: "bin".to_owned(),
                fresh: false,
            })
            .collect(),
        diagnostics: Vec::new(),
    }
}

fn paths(paths: &[&str]) -> Vec<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

fn produced(paths: &[&str]) -> Order<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

#[test]
fn only_what_a_build_produced_that_is_open_is_reopened() {
    let mut state = Builds {
        building: true,
        produced: produced(&["target/debug/viewer"]),
        ..Builds::default()
    };
    // One of the two is open; the other file the reader has never opened, and a third is
    // theirs and was not built.
    let open = paths(&["target/debug/viewer", "theirs.o"]);
    let reopening = state.finished(
        built(&["target/debug/viewer", "target/debug/other"]),
        HashMap::new(),
        &open,
    );

    assert_eq!(reopening, paths(&["target/debug/viewer"]));
    assert!(!state.building);
    assert_eq!(
        state.produced.entries(),
        paths(&["target/debug/viewer", "target/debug/other"]),
        "what this build made is what the next one replaces"
    );
}

/// **What is replaced is what an earlier build produced, not what this one did.** A file
/// no build before produced was opened some other way, so it is the reader's even where
/// this build has just written it; and one a build before produced that this one did not
/// was not rewritten.
#[test]
fn a_build_replaces_only_what_a_build_before_produced() {
    let mut state = Builds {
        building: true,
        produced: produced(&["target/debug/viewer", "target/debug/gone"]),
        ..Builds::default()
    };
    let open = paths(&[
        "target/debug/viewer",
        "target/debug/gone",
        "target/debug/theirs",
    ]);
    let reopening = state.finished(
        built(&["target/debug/viewer", "target/debug/theirs"]),
        HashMap::new(),
        &open,
    );

    assert_eq!(reopening, paths(&["target/debug/viewer"]));
    assert_eq!(
        state.produced.entries(),
        paths(&[
            "target/debug/viewer",
            "target/debug/theirs",
            "target/debug/gone"
        ]),
    );
}

/// **A build replaces what any build before it produced, not only the last one.** A build
/// of another profile, or of another member, writes elsewhere: the binary the first build
/// produced is still open, and the next build of that profile rewrites it.
#[test]
fn a_build_of_another_profile_in_between_still_leaves_the_binary_replaced() {
    let mut state = Builds::default();
    let debug = PathBuf::from("target/debug/app");
    state.building = true;
    state.finished(built(&["target/debug/app"]), HashMap::new(), &[]);
    let open = [debug.clone()];
    state.building = true;
    state.finished(built(&["target/release/app"]), HashMap::new(), &open);
    state.building = true;
    let reopening = state.finished(built(&["target/debug/app"]), HashMap::new(), &open);
    assert_eq!(
        reopening,
        vec![debug],
        "rebuilt debug binary should be reopened"
    );
}

/// **What is remembered stops at [`MAX_PRODUCED`] paths**, the oldest dropped first.
#[test]
fn what_builds_produced_is_capped() {
    let mut state = Builds::default();
    for build in 0..=MAX_PRODUCED {
        state.building = true;
        let path = format!("target/debug/app{build}");
        state.finished(built(&[&path]), HashMap::new(), &[]);
    }
    assert_eq!(state.produced.len(), MAX_PRODUCED);
    assert_eq!(
        state.produced.first(),
        Some(&PathBuf::from(format!("target/debug/app{MAX_PRODUCED}")))
    );
    assert!(state
        .produced
        .position(&PathBuf::from("target/debug/app0"))
        .is_none());
}

/// **An artifact cargo found up to date is not reopened.** cargo lists it all the same,
/// but did not write it, so a close would take every tab into it for the same bytes. It is
/// still in the list the next build replaces.
#[test]
fn an_artifact_cargo_did_not_write_is_not_reopened() {
    let open = paths(&["target/debug/viewer", "target/debug/libanalysis.rlib"]);
    let mut state = Builds {
        building: true,
        produced: open.iter().cloned().collect(),
        ..Builds::default()
    };
    let mut run = built(&["target/debug/viewer", "target/debug/libanalysis.rlib"]);
    if let cargo::Run::Built { artifacts, .. } = &mut run {
        artifacts[1].fresh = true;
    }
    let reopening = state.finished(run, HashMap::new(), &open);

    assert_eq!(reopening, paths(&["target/debug/viewer"]));
    assert_eq!(state.produced.entries(), open);
}

#[test]
fn a_failed_build_reopens_nothing_and_keeps_what_was_produced() {
    let open = paths(&["target/debug/viewer"]);
    let mut state = Builds {
        building: true,
        produced: open.iter().cloned().collect(),
        ..Builds::default()
    };
    let run = cargo::Run::Rejected {
        artifacts: Vec::new(),
        diagnostics: Vec::new(),
        message: "no".to_owned(),
    };
    let reopening = state.finished(run, HashMap::new(), &open);

    assert!(reopening.is_empty(), "a failed build wrote over nothing");
    assert_eq!(state.produced.entries(), open);
    assert!(!state.building);
}

/// **A failed build replaces what cargo wrote before it stopped.** In a workspace the
/// members that compiled are written all the same, and the objects open for them would
/// show code that is no longer on disk. What it wrote joins the list; nothing leaves it.
#[test]
fn a_failed_build_reopens_what_it_wrote() {
    let open = paths(&["target/debug/app", "target/debug/libcore.rlib"]);
    let mut state = Builds {
        building: true,
        produced: open.iter().cloned().collect(),
        ..Builds::default()
    };
    let cargo::Run::Built { artifacts, .. } = built(&["target/debug/libcore.rlib"]) else {
        unreachable!()
    };
    let run = cargo::Run::Rejected {
        artifacts,
        diagnostics: Vec::new(),
        message: "could not compile `app`".to_owned(),
    };
    let reopening = state.finished(run, HashMap::new(), &open);

    assert_eq!(reopening, paths(&["target/debug/libcore.rlib"]));
    assert_eq!(
        state.produced.entries(),
        paths(&["target/debug/libcore.rlib", "target/debug/app"])
    );
}

/// **A clone of the state shares the build rather than copying it.** The cargo section
/// clones the whole of `Builds` to draw it, and a keystroke in any of the pane's boxes is a
/// redraw: a build that said two hundred things would copy every diagnostic and the text
/// the compiler rendered for it, hundreds of kilobytes, per character typed. Taking a
/// finished build clones it too.
///
/// The sharing itself is what is asserted: the diagnostics a clone reads are the same
/// allocation, and so is the set of files it may open.
#[test]
fn a_clone_of_the_state_shares_the_build_rather_than_copying_it() {
    let said = |line: usize| Diagnostic {
        level: Level::Warning,
        message: "unused variable".to_owned(),
        rendered: "warning: unused variable".to_owned(),
        span: Some(cargo::Span {
            file: "src/main.rs".to_owned(),
            line,
            column: 1,
        }),
    };
    let run = cargo::Run::Built {
        artifacts: Vec::new(),
        diagnostics: (1..=3).map(said).collect(),
    };

    let mut state = Builds::default();
    state.finished(
        run,
        HashMap::from([("src/main.rs".to_owned(), PathBuf::from("src/main.rs"))]),
        &[],
    );
    let copy = state.clone();

    assert!(
        std::ptr::eq(state.diagnostics().as_ptr(), copy.diagnostics().as_ptr()),
        "the clone copied every diagnostic"
    );
    // Typed, so what is compared is the sets and not the fields holding them.
    let one_set =
        |held: &HashMap<String, PathBuf>, also: &HashMap<String, PathBuf>| std::ptr::eq(held, also);
    assert!(
        one_set(&state.sources, &copy.sources),
        "the clone copied the set of files it may open"
    );
    // Not vacuous: there is something there to have been copied.
    assert_eq!(state.diagnostics().len(), 3);
}

#[test]
fn a_second_build_is_not_started_over_the_first() {
    let mut state = Builds::default();
    assert!(state.start());
    assert!(!state.start(), "a second would compile changed bytes");
}

/// **What a diagnostic's place may open is decided beside the build**, and not by the row
/// that draws it: a build says two hundred things as readily as two, and asking the
/// filesystem per row costs a `stat` a row a frame.
///
/// Two questions, both answered here: the file is under the directory cargo ran in, and
/// the source cache would read it. A file outside is a dependency's, and a file that is
/// not there is a target that would do nothing when pressed.
#[test]
fn only_the_diagnostic_files_under_the_directory_that_read_are_named() {
    // A real directory, this being one of the few things the filesystem itself is the
    // question (`AGENTS.md`).
    let directory = Temporary::fresh_directory("openable");
    std::fs::create_dir_all(directory.join("src")).expect("the directory");
    std::fs::write(directory.join("src/main.rs"), "fn main() {}\n").expect("the file");

    let span = |file: &str| cargo::Span {
        file: file.to_owned(),
        line: 1,
        column: 1,
    };
    let said = |file: &str| Diagnostic {
        level: Level::Warning,
        message: "unused".to_owned(),
        rendered: "warning: unused".to_owned(),
        span: Some(span(file)),
    };

    let named = openable(
        &directory,
        &[
            // The file that is there, said twice: one entry, and one `stat`.
            said("src/main.rs"),
            said("src/main.rs"),
            // Under the directory and not on disk.
            said("src/gone.rs"),
            // A dependency's, which `join` leaves absolute.
            said("/home/reader/.cargo/registry/src/index/serde-1.0/src/lib.rs"),
            // A diagnostic with no place at all.
            Diagnostic {
                level: Level::Note,
                message: "somewhere".to_owned(),
                rendered: "note: somewhere".to_owned(),
                span: None,
            },
        ],
    );

    assert_eq!(
        named,
        HashMap::from([("src/main.rs".to_owned(), directory.join("src/main.rs"))]),
        "the set is the files this pane may open, and nothing else"
    );
}

/// **A member's diagnostics are spelled from the workspace root.** cargo runs the compiler
/// there, so a file of the member is `crates/app/src/lib.rs` and not `src/lib.rs`: joined
/// to the member's own directory it names nothing, and no place of the build can be opened.
/// A sibling member's file is under the root but not under the project's directory, so it
/// stays out.
#[test]
fn a_members_diagnostics_are_read_from_the_workspace_root() {
    let root = Temporary::fresh_directory("openable-member");
    let member = root.join("crates/app");
    let sibling = root.join("crates/other");
    for package in [&member, &sibling] {
        std::fs::create_dir_all(package.join("src")).expect("the directory");
        std::fs::write(package.join("src/lib.rs"), "\n").expect("the file");
        std::fs::write(
            package.join(cargo::MANIFEST),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        )
        .expect("the member manifest");
    }
    std::fs::write(
        root.join(cargo::MANIFEST),
        "[workspace]\nmembers = [\"crates/*\"]\n",
    )
    .expect("the root manifest");

    let said = |file: &str| Diagnostic {
        level: Level::Error,
        message: "mismatched types".to_owned(),
        rendered: "error: mismatched types".to_owned(),
        span: Some(cargo::Span {
            file: file.to_owned(),
            line: 1,
            column: 1,
        }),
    };
    let named = openable(
        &member,
        &[
            said("crates/app/src/lib.rs"),
            said("crates/other/src/lib.rs"),
        ],
    );

    assert_eq!(
        named,
        HashMap::from([(
            "crates/app/src/lib.rs".to_owned(),
            member.join("src/lib.rs")
        )]),
    );
}

/// From a symlink to a member, the root is found where cargo finds it, and a file under
/// the member is offered under the directory as the reader spelled it.
#[cfg(unix)]
#[test]
fn a_member_reached_through_a_symlink_has_its_diagnostics_opened() {
    let root = Temporary::fresh_directory("openable-link");
    let member = root.join("ws/app");
    std::fs::create_dir_all(member.join("src")).expect("the directory");
    std::fs::write(member.join("src/lib.rs"), "\n").expect("the file");
    std::fs::write(
        member.join(cargo::MANIFEST),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .expect("the member manifest");
    std::fs::write(
        root.join("ws").join(cargo::MANIFEST),
        "[workspace]\nmembers = [\"app\"]\n",
    )
    .expect("the root manifest");
    let link = root.join("link");
    std::os::unix::fs::symlink(&member, &link).expect("a symlink");

    let named = openable(
        &link,
        &[Diagnostic {
            level: Level::Error,
            message: "mismatched types".to_owned(),
            rendered: "error: mismatched types".to_owned(),
            span: Some(cargo::Span {
                file: "app/src/lib.rs".to_owned(),
                line: 1,
                column: 1,
            }),
        }],
    );

    assert_eq!(
        named,
        HashMap::from([("app/src/lib.rs".to_owned(), link.join("src/lib.rs"))]),
    );
}

/// The two panes that draw a build say the same words about the same one. Both ask
/// `cargo::Run` (`agents/Scratchpad.md`), which is the whole of why: a summary written
/// twice drifts, and these two are meant to be read as the same line in two places.
#[test]
fn both_build_panes_say_the_same_line_about_the_same_build() {
    /// The pad holding one build, `PadState`'s own fields not all being this module's.
    fn pad_holding(build: Build) -> PadState {
        let mut pad = PadState::default();
        pad.built = Some(Ok(build));
        pad
    }

    // The executable beside each run is what `build_in` would have named for it: a pad
    // whose build made nothing has a line of its own and is not this comparison.
    for (run, executable) in [
        (
            built(&["target/debug/viewer"]),
            Some(PathBuf::from("target/debug/viewer")),
        ),
        (
            cargo::Run::Rejected {
                artifacts: Vec::new(),
                diagnostics: Vec::new(),
                message: "no matching package".to_owned(),
            },
            None,
        ),
    ] {
        let project = Builds {
            built: Some(Arc::new(run.clone())),
            ..Builds::default()
        };
        let pad = pad_holding(Build { run, executable });
        assert_eq!(project.verdict(), pad.verdict());
        assert_eq!(project.refusal(), pad.refusal());
    }

    // A cargo that would not start, which both hold as cargo's own answer: one sentence,
    // and it names what stopped it.
    let project = Builds {
        built: Some(Arc::new(cargo::Run::NoCargo("not found".to_owned()))),
        ..Builds::default()
    };
    let pad = pad_holding(Build {
        run: cargo::Run::NoCargo("not found".to_owned()),
        executable: None,
    });
    assert_eq!(project.verdict(), pad.verdict());
    assert_eq!(
        project.verdict().expect("a verdict").text,
        "could not run cargo: not found"
    );
    // And said once: neither pane repeats it under that line as cargo's own words.
    assert_eq!(project.refusal(), None);
    assert_eq!(pad.refusal(), None);

    // And while one is going, which is neither's build to describe. Said with a build
    // already held: the rule is "Building..." and nothing of the build before, and a pane
    // that fell through to the last verdict would report a finished build over one still
    // going.
    let last = built(&["target/debug/viewer"]);
    let mut pad = pad_holding(Build {
        run: last.clone(),
        executable: Some(PathBuf::from("target/debug/viewer")),
    });
    pad.building = true;
    let project = Builds {
        building: true,
        built: Some(Arc::new(last)),
        ..Builds::default()
    };
    assert_eq!(project.verdict(), pad.verdict());
    assert_eq!(project.verdict(), Some(Verdict::plain("Building...")));
    // Nothing built and nothing going is no line at all.
    assert_eq!(Builds::default().verdict(), PadState::default().verdict());
    assert_eq!(Builds::default().verdict(), None);
}

/// **A read fills each of the two manifest fields with its own file.** A project opened at
/// a workspace member names both: its own manifest, and the root above it, cargo taking
/// `[profile.*]` from the root alone.
///
/// Both are `Option<PathBuf>`, so the pairing is what is pinned. Crossed, the view would
/// say the root is what gets built and offer an edit to the member's own file, which cargo
/// ignores.
#[test]
fn a_members_read_names_its_own_manifest_and_the_root_the_profile_comes_from() {
    // Real files, this being one of the few things the filesystem itself is the question
    // (`AGENTS.md`): both manifests are read and parsed.
    let root = Temporary::fresh_directory("member-read");
    let member = root.join("app");
    std::fs::create_dir_all(&member).expect("the directory");
    std::fs::write(
        root.join(cargo::MANIFEST),
        "[workspace]\nmembers = [\"app\"]\n\n[profile.release]\ndebug = \"line-tables-only\"\n",
    )
    .expect("the root manifest");
    std::fs::write(
        member.join(cargo::MANIFEST),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .expect("the member manifest");

    let profiles = cargo::profile_manifest(&member);
    let BuildAnswer::Read(said) = read(&member, profiles, Profile::Release, None) else {
        panic!("a read answers with what the manifest said");
    };

    assert_eq!(
        said.path,
        Some(member.join(cargo::MANIFEST)),
        "what cargo is run over is the directory's own manifest"
    );
    assert_eq!(
        said.profiles,
        Some(root.join(cargo::MANIFEST)),
        "the profile is the root's, and that is the file the offer edits"
    );
    // From the root's table, release carrying no lines by default: the other way round
    // this would be false.
    assert!(said.debug_lines);
    assert_eq!(said.edit_refused, None, "nothing was refused");

    // And the same read again is no change, which is what keeps the hook from writing
    // ([`write_if`]): the value is compared whole.
    let mut state = Builds::default();
    assert!(state.read(said.clone()));
    assert!(!state.read(said));
}
