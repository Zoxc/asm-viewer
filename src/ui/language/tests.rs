//! The language server's transitions: what each answer and each press does to what the
//! app holds.

use super::*;

fn asking() -> Asking {
    Asking {
        directory: PathBuf::from("/project"),
        program: "rust-analyzer".to_owned(),
    }
}

#[test]
fn a_remark_from_a_stopped_server_says_nothing_about_the_one_that_is_on() {
    let mut state = Language {
        state: Lsp::Running,
        run: 4,
        ..Language::default()
    };
    assert!(!state.noted(3, true), "an older run's word is nobody's");
    assert!(!state.working);
    assert!(state.noted(4, true));
    assert!(!state.noted(4, true), "and saying it twice costs no render");
}

#[test]
fn a_start_counts_the_run_up_and_says_what_to_start_it_with() {
    let mut state = Language {
        settings: Some(Ok(lsp::Settings::none())),
        run: 2,
        ..Language::default()
    };
    let started = state.starting().expect("there is a server to start");
    assert_eq!(started.0, 3, "the run an answer will be matched by");
    assert_eq!(state.run, 3);
    assert!(matches!(state.state, Lsp::Starting));
}

#[test]
fn a_settings_file_that_could_not_be_read_starts_nothing() {
    let mut state = Language {
        settings: Some(Err(lsp::Unreadable::NotAnObject)),
        ..Language::default()
    };
    assert!(state.starting().is_none());
    assert!(
        matches!(state.state, Lsp::Failed(_)),
        "and the control says why"
    );
}

#[test]
fn a_stop_with_nothing_to_stop_does_not_have_to_tell_the_worker() {
    let mut state = Language::default();
    assert!(!state.stopped());

    // A failure still on the control is something to put back, server or no server.
    let mut failed = Language {
        state: Lsp::Failed("gone".to_owned()),
        run: 1,
        ..Language::default()
    };
    assert!(failed.stopped());
    assert!(matches!(failed.state, Lsp::Off));
    assert_eq!(failed.run, 2, "an answer in flight is for the run before");
}

#[test]
fn an_unanswered_question_goes_with_the_server_and_the_settings_stay() {
    let settings = Some(Ok(lsp::Settings::none()));
    let mut state = Language {
        state: Lsp::Starting,
        asking: Some(asking()),
        settings: settings.clone(),
        ..Language::default()
    };
    assert!(state.stopped());
    assert!(state.asking.is_none());
    assert!(
        state.settings == settings,
        "they are the project's, not the server's"
    );
}

#[test]
fn a_second_press_with_the_question_already_up_asks_it_again_and_that_is_nothing() {
    let mut state = Language::default();
    assert!(state.ask_to_start(asking()));
    assert!(!state.ask_to_start(asking()));
    assert!(state.declined());
    assert!(!state.declined(), "and there is nothing left to decline");
}

#[test]
fn a_failure_reported_for_a_server_already_replaced_is_not_shown() {
    let mut state = Language {
        state: Lsp::Running,
        run: 5,
        ..Language::default()
    };
    assert!(!state.failed(4, "it died".to_owned()));
    assert!(matches!(state.state, Lsp::Running));
    assert!(state.failed(5, "it died".to_owned()));
    assert!(matches!(state.state, Lsp::Failed(_)));
}
