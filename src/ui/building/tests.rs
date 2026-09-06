//! What [`Builds`] does with a finished build.

use super::*;

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
    let reopening = state.finished(built(&["target/debug/viewer", "target/debug/other"]), &open);

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
    let reopening = state.finished(run, &previous);

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
