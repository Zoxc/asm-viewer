//! The language server's transitions: what each answer and each press does to what the
//! app holds.

use super::*;

impl Language {
    /// Whether the app is holding what would end a server, which is a process that
    /// exists. For the tests: nothing drawn asks it.
    pub(crate) fn holding(&self) -> bool {
        self.state.handle().is_some()
    }
}

impl Language {
    /// Whether the server is reading the project rather than answering about it. For the
    /// tests: what is drawn asks [`Language::busy`] instead, which a start is also.
    pub(crate) fn working(&self) -> bool {
        self.state.said().is_some_and(|said| said.working)
    }
}

impl Lsp {
    /// A server that is running, over a handle with no process behind it
    /// ([`process::Handle::to_nothing`]), started as a project that names no server would
    /// start it. For the tests: what they are about is the state the app holds, and none
    /// of them starts a program.
    pub(crate) fn running_to_nothing() -> Lsp {
        Lsp::running_as(OpenProject::default().serving())
    }

    /// The same, started as `serving`.
    pub(crate) fn running_as(serving: Serving) -> Lsp {
        Lsp::Running {
            server: process::Handle::to_nothing(),
            said: Remarks::default(),
            serving,
        }
    }
}

fn asking() -> Asking {
    Asking {
        directory: PathBuf::from("/project"),
        serving: OpenProject::default().serving(),
    }
}

#[test]
fn a_remark_from_a_stopped_server_says_nothing_about_the_one_that_is_on() {
    let mut state = Language {
        state: Lsp::running_to_nothing(),
        run: 4,
        ..Language::default()
    };
    let busy = lsp::Note::Busy(true);
    assert!(!state.remarked(3, &busy), "an older run's word is nobody's");
    assert!(!state.busy(), "nothing of it was written down");
    assert!(state.remarked(4, &busy));
    assert!(
        !state.remarked(4, &busy),
        "and saying it twice costs no render"
    );
}

/// One writer takes every remark, so what each one is about has to stay its own field.
/// The two are not the same thing either way round: a server reports progress after it
/// has said it settled, and going quiet is not the same as saying so.
#[test]
fn each_remark_is_written_on_the_field_it_is_about_and_no_other() {
    let mut state = Language {
        state: Lsp::running_to_nothing(),
        run: 1,
        ..Language::default()
    };
    assert!(state.remarked(1, &lsp::Note::Busy(true)));
    assert!(state.remarked(1, &lsp::Note::Settled(true)));
    assert!(state.working(), "settling ended the progress it reports");
    assert!(
        state.ready(),
        "what it said about settling beats its progress"
    );
    assert!(state.remarked(1, &lsp::Note::Busy(false)));
    assert!(state.remarked(1, &lsp::Note::Settled(false)));
    assert!(
        !state.ready(),
        "a quiet server that says it has not settled is not ready"
    );
    assert!(
        !state.remarked(1, &lsp::Note::Settled(false)),
        "saying it twice costs no render either"
    );
    assert!(
        !state.working(),
        "a remark about settling was written on the progress"
    );
}

#[test]
fn a_start_counts_the_run_up_and_says_what_to_start_it_with() {
    let mut state = Language {
        settings: Some(Ok(lsp::Settings::none())),
        run: 2,
        ..Language::default()
    };
    let started = state
        .starting(asking().serving)
        .expect("there is a server to start");
    assert_eq!(started.0, 3, "the run an answer will be matched by");
    assert_eq!(state.run, 3);
    assert!(matches!(state.state, Lsp::Starting { .. }));
}

#[test]
fn a_settings_file_that_could_not_be_read_starts_nothing() {
    let mut state = Language {
        settings: Some(Err(lsp::Unreadable::NotAnObject)),
        ..Language::default()
    };
    assert!(state.starting(asking().serving).is_none());
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
        state: Lsp::Starting {
            server: None,
            said: Remarks::default(),
            serving: asking().serving,
        },
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
        state: Lsp::running_to_nothing(),
        run: 5,
        ..Language::default()
    };
    assert!(!state.failed(4, "it died".to_owned()));
    assert!(matches!(state.state, Lsp::Running { .. }));
    assert!(state.failed(5, "it died".to_owned()));
    assert!(matches!(state.state, Lsp::Failed(_)));
}
