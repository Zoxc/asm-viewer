//! What the pool must not change about the answer: its order, its content, and the stack a
//! deep name is allowed to recurse on.

use super::*;

/// A batch big enough to be split, of names that are all safe on the caller's own stack, so
/// a test can compute the same answer sequentially to compare against.
fn mixed_batch(len: usize) -> Names {
    Arc::new(
        (0..len)
            .map(|index| match index % 4 {
                // Nothing to demangle: the entry point's kind of name.
                0 => None,
                // A real one, and a different one each time round.
                1 => Some(format!(
                    "_ZN4core3fmt9Formatter12pad_integral17h{index:016x}E"
                )),
                // A C name no demangler has anything to say about.
                2 => Some(format!("plain_c_function_{index}")),
                _ => Some(format!("_ZN3std2io5Write5write17h{index:016x}E")),
            })
            .collect(),
    )
}

#[test]
fn a_batch_is_split_across_the_threads_there_are_and_never_into_nothing() {
    assert_eq!(jobs_for(0, 8), 1);
    assert_eq!(jobs_for(1, 8), 1);
    assert_eq!(jobs_for(GRAIN, 8), 1);
    assert_eq!(jobs_for(GRAIN + 1, 8), 2);
    assert_eq!(jobs_for(GRAIN * 4, 8), 4);
    // Never more jobs than there are threads to run them...
    assert_eq!(jobs_for(GRAIN * 1000, 8), 8);
    // ...and never none, whatever it is asked with.
    assert_eq!(jobs_for(usize::MAX, 0), 1);
    assert_eq!(jobs_for(0, 0), 1);
}

/// The point of the whole exercise: which thread got which grain is not visible in the
/// answer. Both that it matches the sequential one and that it is the same twice.
#[test]
fn the_answer_is_the_batch_s_own_order_however_it_was_split() {
    // Past `GRAIN`, so this is the parallel path and not the caller's own stack.
    let names = mixed_batch(GRAIN * 3 + 7);
    assert!(names.len() > GRAIN);

    let sequential = demangle_range(&names, 0..names.len());
    assert_eq!(batch(&names), sequential);
    assert_eq!(batch(&names), sequential);

    // And it is an answer and not a row of `None`s: the Rust names came back demangled and
    // the C ones came back as the file wrote them.
    assert_eq!(sequential[0], None);
    assert_eq!(
        sequential[1].as_deref(),
        Some("core::fmt::Formatter::pad_integral")
    );
    assert_eq!(sequential[2], None);
}

/// The constraint the pool exists under: a name recurses as deep as the file says, so the
/// thread it lands on has to be one of the pool's own and not whatever thread the batch was
/// submitted from. Before there was a pool this was a fresh 64 MiB thread per object; if a
/// grain ever ran on the submitter's stack instead, this test would not fail, it would
/// **abort** the test binary.
#[test]
fn a_deep_name_in_a_split_batch_is_demangled_on_a_pool_thread() {
    let deep = format!("?f@@YAX{}@Z", "P".repeat(1000));
    let over_cap = format!("?g@@YAX{}@Z", "P".repeat(4000));

    let mut names: Vec<Option<String>> = (0..GRAIN * 2).map(|_| None).collect();
    names[0] = Some("_ZN4core3fmt9Formatter12pad_integral17h0123456789abcdefE".to_owned());
    // In the last grain, so it is not the first thing the first job does.
    names[GRAIN * 2 - 1] = Some(deep);
    names[GRAIN + 1] = Some(over_cap);
    let names: Names = Arc::new(names);

    let demangled = batch(&names);
    assert_eq!(
        demangled[0].as_deref(),
        Some("core::fmt::Formatter::pad_integral")
    );
    // Past the cap: not demangled, and that is the whole cost of the guard.
    assert_eq!(demangled[GRAIN + 1], None);
    // Whatever `msvc-demangler` makes of the deep one, it made it without overflowing.
    let _ = &demangled[GRAIN * 2 - 1];
}

#[test]
fn a_batch_with_nothing_in_it_answers_one_none_per_name() {
    assert_eq!(batch(&Arc::new(Vec::new())), Vec::<Option<String>>::new());
    assert_eq!(batch(&Arc::new(vec![None, None, None])), vec![None; 3]);
    // An empty name is not a name either.
    assert_eq!(
        batch(&Arc::new(vec![Some(String::new()), None])),
        vec![None; 2]
    );
}
