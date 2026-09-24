use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static STOPPED: AtomicBool = AtomicBool::new(false);
static SAVED_AFTER_STOP: AtomicUsize = AtomicUsize::new(0);

/// **A save that panics keeps neither the stop nor the other saves from running.** The
/// stop used to come last, so a save that panicked on the shutdown thread left every
/// program the app started running, and the app up.
#[test]
fn a_panicking_save_stops_nothing_else() {
    in_order(
        || STOPPED.store(true, Ordering::SeqCst),
        [
            || panic!("a save's bug"),
            || {
                if STOPPED.load(Ordering::SeqCst) {
                    SAVED_AFTER_STOP.fetch_add(1, Ordering::SeqCst);
                }
            },
            || {
                if STOPPED.load(Ordering::SeqCst) {
                    SAVED_AFTER_STOP.fetch_add(1, Ordering::SeqCst);
                }
            },
        ],
    );
    assert!(
        STOPPED.load(Ordering::SeqCst),
        "the programs were not stopped"
    );
    assert_eq!(SAVED_AFTER_STOP.load(Ordering::SeqCst), 2);
}
