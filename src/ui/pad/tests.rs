//! What [`Pads`] does with the scratchpad worker's answers.

use super::*;

fn id(name: &str) -> PadId {
    PadId::new(name).expect("a valid id")
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

    assert_eq!(opening.id(), &id("pad-a"));
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
    assert_eq!(opening.id(), &booted, "which is what seeds its baseline");
}

#[test]
fn a_pad_that_is_open_is_read_once_and_never_again() {
    let mut pads = Pads::default();
    let first = Scratchpad::new("pad-a").expect("a valid id");
    pads.show(id("pad-a"));

    assert!(pads.opened(&first, None), "the answer it was waiting for");
    let mut typed = first.clone();
    typed.source = "what the reader has since typed".to_owned();
    pads.state_mut().scratchpad = typed;

    // The second answer to the same question: the disk as it was read before the save of
    // what has been typed since.
    assert!(
        !pads.opened(&first, None),
        "taking it would put the older text back and make it the baseline"
    );
    assert_eq!(
        pads.state().scratchpad.source,
        "what the reader has since typed"
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
    assert!(pads.opened(&read, None));
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
    let build = Build::Unavailable(Failure::NoDirectory);
    // Named, not written: `built` only asks the store where the package would be.
    let store = Store::at("/nowhere");

    assert!(
        pads.built(&id("pad-a"), build.clone(), None, Some(&store))
            .is_none(),
        "the pad that asked has gone, and its id was handed out again"
    );
    assert!(pads.get(&id("pad-a")).expect("held").built.is_none());

    pads.state_mut().building = true;
    assert!(pads
        .built(&id("pad-a"), build, None, Some(&store))
        .is_some());
    let state = pads.get(&id("pad-a")).expect("held");
    assert!(!state.building && state.built.is_some());
}

#[test]
fn a_program_that_would_not_start_is_said_only_for_the_run_that_asked() {
    let mut pads = Pads::default();
    pads.show(id("pad-a"));
    pads.state_mut().run = 2;
    pads.state_mut().run_state = RunState::Starting;

    pads.started(&id("pad-a"), 1, Err(Failure::NoDirectory));
    assert!(
        matches!(pads.state().run_state, RunState::Starting),
        "a run the reader has left says nothing about the one that is on"
    );
    pads.started(&id("pad-a"), 2, Err(Failure::NoDirectory));
    assert!(matches!(pads.state().run_state, RunState::Over(_)));
}
