use super::*;

/// The count is only up while the guarded call runs, and a panic inside one leaves it
/// where it was: what a hook asks during the *next* panic has to be the truth.
#[test]
fn a_thread_is_guarded_only_inside_a_guarded_call() {
    assert!(!guarded());
    assert_eq!(guard(|| guarded()), Some(true));
    assert!(!guarded());

    // The panic a guard is for: caught, and the count put back.
    let previous = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let answer: Option<()> = guard(|| panic!("a dependency's bug"));
    panic::set_hook(previous);
    assert_eq!(answer, None);
    assert!(!guarded());
}

/// Nested, as the demangle pool's per-job guard holds the per-name one: the inner call
/// leaving does not say the thread is unguarded while the outer one is still running.
#[test]
fn a_guard_inside_a_guard_stays_guarded_until_the_outer_one_leaves() {
    let inside = guard(|| {
        assert_eq!(guard(|| guarded()), Some(true));
        guarded()
    });
    assert_eq!(inside, Some(true));
    assert!(!guarded());
}

/// Per thread, since that is what a panic hook can ask about: one thread inside a guarded
/// call says nothing about another.
#[test]
fn a_guard_is_the_thread_s_own() {
    let answer = guard(|| std::thread::spawn(guarded).join().ok());
    assert_eq!(answer, Some(Some(false)));
}
