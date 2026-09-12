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
            })
            .collect(),
        diagnostics: Vec::new(),
    }
}

#[test]
fn only_the_previous_builds_artifacts_that_are_open_are_reopened() {
    let mut state = Builds {
        building: true,
        previous: vec![PathBuf::from("target/debug/viewer")],
        ..Builds::default()
    };
    // One of the two is open; the other file the reader has never opened, and a third is
    // theirs and was not built.
    let open = [
        PathBuf::from("target/debug/viewer"),
        PathBuf::from("theirs.o"),
    ];
    let reopening = state.finished(
        built(&["target/debug/viewer", "target/debug/other"]),
        HashSet::new(),
        &open,
    );

    assert_eq!(reopening, vec![PathBuf::from("target/debug/viewer")]);
    assert!(!state.building);
    assert_eq!(
        state.previous,
        vec![
            PathBuf::from("target/debug/viewer"),
            PathBuf::from("target/debug/other")
        ],
        "what this build made is what the next one replaces"
    );
}

#[test]
fn a_build_that_produced_nothing_leaves_the_previous_list_standing() {
    let previous = vec![PathBuf::from("target/debug/viewer")];
    let mut state = Builds {
        building: true,
        previous: previous.clone(),
        ..Builds::default()
    };
    let run = cargo::Run::Rejected {
        diagnostics: Vec::new(),
        message: "no".to_owned(),
    };
    let reopening = state.finished(run, HashSet::new(), &previous);

    assert_eq!(reopening, previous, "those paths are still what is open");
    assert_eq!(state.previous, previous);
    assert!(!state.building);
}

/// **A clone of the state shares the build rather than copying it.** The cargo section
/// clones the whole of `Builds` to draw it, and a keystroke in any of the pane's boxes is a
/// redraw: a build that said two hundred things would copy every diagnostic and the text
/// the compiler rendered for it, hundreds of kilobytes, per character typed. The worker's
/// hooks clone it too, to write two fields ([`write_if`]).
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
    state.finished(run, HashSet::from([PathBuf::from("src/main.rs")]), &[]);
    let copy = state.clone();

    assert!(
        std::ptr::eq(state.diagnostics().as_ptr(), copy.diagnostics().as_ptr()),
        "the clone copied every diagnostic"
    );
    // Typed, so what is compared is the sets and not the fields holding them.
    let one_set = |held: &HashSet<PathBuf>, also: &HashSet<PathBuf>| std::ptr::eq(held, also);
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
    let directory = Temporary::directory(
        std::env::temp_dir().join(format!("assembly-viewer-openable-{}", std::process::id())),
    );
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
        HashSet::from([directory.join("src/main.rs")]),
        "the set is the files this pane may open, and nothing else"
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
