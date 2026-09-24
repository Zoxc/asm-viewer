//! What [`Pads`] does with the scratchpad worker's answers.

use super::*;

fn id(name: &str) -> PadId {
    PadId::new(name).expect("a valid id")
}

/// The number the state held for `name` asks its open with.
fn holding(pads: &Pads, name: &str) -> u64 {
    pads.get(&id(name)).expect("held").holding
}

fn listing(names: &[&str]) -> Vec<PadListing> {
    names
        .iter()
        .map(|name| PadListing {
            id: id(name),
            name: format!("{name}'s name"),
        })
        .collect()
}

#[test]
fn the_front_of_the_listing_is_what_a_restart_comes_back_to() {
    let mut pads = Pads::default();
    let opening = pads.listed(&listing(&["pad-a", "pad-b"]));

    assert_eq!(
        opening.expect("the front is to be read").pad(),
        Some(&id("pad-a"))
    );
    assert_eq!(pads.shown(), &id("pad-a"));
    assert_eq!(
        pads.get(&id("pad-b"))
            .expect("held for the panel")
            .scratchpad
            .name,
        "pad-b's name",
        "a pad nothing has opened is still drawn under its own name"
    );
}

#[test]
fn an_empty_listing_opens_the_pad_the_app_booted_holding() {
    let mut pads = Pads::default();
    let booted = pads.shown().clone();
    let opening = pads.listed(&[]);
    assert_eq!(
        opening
            .expect("the booted pad is read like any other")
            .pad(),
        Some(&booted),
        "which is what seeds its baseline"
    );
}

/// **The check is `show`'s, so every door into a pad makes it.** The listing, a pad just
/// made, a delete coming back to the next pad and the reader's own switch all show a pad
/// and then ask the worker for it, and the listing used to ask unconditionally: it was
/// right only because it is the first question the app puts, so nothing could be open
/// when it answered. The rule now holds whatever the order.
#[test]
fn a_listing_naming_a_pad_already_open_asks_for_nothing() {
    let mut pads = Pads::default();
    let read = Scratchpad::new("pad-a").expect("a valid id");
    assert!(
        pads.listed(&listing(&["pad-a"])).is_some(),
        "a pad the disk has never been read for"
    );
    assert!(
        pads.opened(holding(&pads, "pad-a"), &read, None),
        "the answer that seeds its baseline"
    );

    assert!(
        pads.listed(&listing(&["pad-a"])).is_none(),
        "a pad already read was asked for a second time"
    );
}

#[test]
fn a_pad_that_is_open_is_read_once_and_never_again() {
    let mut pads = Pads::default();
    let first = Scratchpad::new("pad-a").expect("a valid id");
    pads.show(id("pad-a"));

    assert!(
        pads.opened(holding(&pads, "pad-a"), &first, None),
        "the answer it was waiting for"
    );
    let mut typed = first.clone();
    typed.source = "what the reader has since typed".to_owned();
    pads.state_mut().scratchpad = typed;

    // The second answer to the same question: the disk as it was read before the save of
    // what has been typed since.
    assert!(
        !pads.opened(holding(&pads, "pad-a"), &first, None),
        "taking it would put the older text back and make it the baseline"
    );
    assert_eq!(
        pads.state().scratchpad.source,
        "what the reader has since typed"
    );
}

/// A pad deleted while its open was on the queue: the answer is its package, read before
/// the delete ran. Taken by the pad that now has its id, it would bring the deleted pad's
/// source back as that pad's, with a baseline saying the disk holds it.
#[test]
fn an_open_answered_for_a_deleted_pad_is_not_taken() {
    let mut pads = Pads::default();
    pads.show(id("pad-a"));
    pads.show(id("pad-b"));
    let asked = holding(&pads, "pad-a");
    let mut deleted = Scratchpad::new("pad-a").expect("a valid id");
    deleted.source = "the deleted pad's source".to_owned();

    pads.forget(&id("pad-a"));
    assert!(
        !pads.opened(asked, &deleted, None),
        "an answer for a pad the table no longer holds"
    );

    // The id handed out again, as New does with the lowest free one.
    pads.show(id("pad-a"));
    assert!(
        !pads.opened(asked, &deleted, None),
        "the new pad took the deleted one's package"
    );
    pads.unopened(&id("pad-a"), asked, Failure::Unreadable);
    let state = pads.state();
    assert!(!state.opened() && state.unsaved.is_none());
    assert_ne!(state.scratchpad.source, "the deleted pad's source");

    let read = Scratchpad::new("pad-a").expect("a valid id");
    assert!(
        pads.opened(holding(&pads, "pad-a"), &read, None),
        "the new pad's own answer"
    );
}

/// **Nothing is owed to the disk before the disk has been read.** What is on screen is
/// compared against the baseline the worker's answer seeds, and a pad with no baseline has
/// nothing to be compared against: the app boots holding the default scratchpad, and
/// writing that over a pad someone was keeping is the loss the rule exists to prevent.
#[test]
fn nothing_is_owed_to_the_disk_before_it_has_been_read() {
    let mut pads = Pads::default();
    pads.show(id("pad-a"));
    pads.state_mut().scratchpad.source = "the default the app booted with".to_owned();

    assert!(
        !pads.state().opened(),
        "a pad the worker has not answered for"
    );
    assert!(
        pads.unsaved_change(&id("pad-a")).is_none(),
        "a pad with no baseline was written over"
    );

    // The answer, which is both what opens the pad and what seeds its baseline.
    let read = Scratchpad::new("pad-a").expect("a valid id");
    assert!(pads.opened(holding(&pads, "pad-a"), &read, None));
    assert!(pads.state().opened());
    assert!(
        pads.unsaved_change(&id("pad-a")).is_none(),
        "the disk already holds what is on screen"
    );

    pads.state_mut().scratchpad.source = "typed since".to_owned();
    let owed = pads.unsaved_change(&id("pad-a")).expect("the edit");
    assert_eq!(owed.source, "typed since");
    assert!(
        pads.unsaved_change(&id("pad-a")).is_none(),
        "the baseline did not move to what was sent, so the edit is owed twice"
    );
}

#[test]
fn a_build_answered_for_a_pad_that_asked_for_none_is_not_taken() {
    let mut pads = Pads::default();
    pads.show(id("pad-a"));
    let build: Result<Build, Failure> = Err(Failure::NoDirectory);

    assert!(
        !pads.built(&id("pad-a"), build.clone(), None),
        "the pad that asked has gone, and its id was handed out again"
    );
    assert!(pads.get(&id("pad-a")).expect("held").built.is_none());

    pads.state_mut().building = true;
    assert!(pads.built(&id("pad-a"), build, None));
    let state = pads.get(&id("pad-a")).expect("held");
    assert!(!state.building && state.built.is_some());
}

#[test]
fn a_program_that_would_not_start_is_said_only_for_the_run_that_asked() {
    let mut runs = Runs::default();
    runs.start(&id("pad-a"));
    runs.start(&id("pad-a"));
    let state = |runs: &Runs| runs.get(&id("pad-a")).expect("held").state.clone();

    runs.started(&id("pad-a"), 1, Err(Failure::NoDirectory));
    assert!(
        matches!(state(&runs), RunState::Starting),
        "a run the reader has left says nothing about the one that is on"
    );
    runs.started(&id("pad-a"), 2, Err(Failure::NoDirectory));
    assert!(matches!(state(&runs), RunState::Over(_)));
}

/// A build writes the package on its way, so it is what says whether the disk has caught
/// up with the screen. `Err` is that write refused and nothing else, whatever refused it:
/// a bad dependency row, a directory that would not take the file, nowhere to write at
/// all.
#[test]
fn only_a_build_that_wrote_the_package_clears_the_unsaved_marker() {
    let refused = [
        Failure::Dependencies(1),
        Failure::Write("read-only".to_owned()),
        Failure::NoDirectory,
    ];
    for failure in refused {
        let mut pads = Pads::default();
        pads.show(id("pad-a"));
        pads.state_mut().building = true;
        pads.built(&id("pad-a"), Err(failure.clone()), None);
        assert_eq!(
            pads.state().unsaved,
            Some(failure),
            "a build that never wrote the package took the marker with it"
        );
    }

    // And one cargo answered, however it answered, wrote it first.
    let mut pads = Pads::default();
    pads.show(id("pad-a"));
    pads.state_mut().building = true;
    pads.state_mut().unsaved = Some(Failure::Write("read-only".to_owned()));
    let refused_by_cargo = Build {
        run: cargo::Run::Rejected {
            artifacts: Vec::new(),
            diagnostics: Vec::new(),
            message: String::new(),
        },
        executable: None,
    };
    pads.built(&id("pad-a"), Ok(refused_by_cargo), None);
    assert_eq!(pads.state().unsaved, None);
}

/// A pad whose open failed is asked for again the next time it is shown. When that
/// open works, the reason the first one failed no longer holds.
#[test]
fn an_open_that_works_clears_the_reason_the_last_one_failed() {
    let mut pads = Pads::default();
    pads.show(id("pad-a"));
    pads.unopened(&id("pad-a"), holding(&pads, "pad-a"), Failure::Unreadable);
    assert!(pads.state().unsaved.is_some());

    let read = Scratchpad::new("pad-a").expect("a valid id");
    assert!(pads.opened(holding(&pads, "pad-a"), &read, None));
    assert!(
        pads.state().unsaved.is_none(),
        "the panel still says the package could not be read"
    );
}

/// The line under the list says why the *last* New or Delete did not happen, so a New
/// that works takes away the one an earlier New left.
#[test]
fn a_new_that_works_clears_the_last_refusal() {
    let mut pads = Pads::default();
    pads.created(Err(Failure::NoDirectory));
    assert!(pads.refused.is_some());

    pads.created(Ok(Scratchpad::new("pad-b").expect("a valid id")));
    assert_eq!(
        pads.refused, None,
        "the panel still says the pad was not made"
    );
}
