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
