use super::*;
use crate::filter::Filter;

/// The filter a reader's typing compiles to, so these read as what was typed.
fn typed(pattern: &str) -> Matcher {
    Filter {
        pattern: pattern.to_owned(),
        ..Filter::default()
    }
    .matcher()
}

#[test]
fn nothing_typed_keeps_every_section_and_every_row() {
    let listed = matching(&typed(""));

    assert_eq!(listed.len(), SECTIONS.len());
    for (kept, section) in listed.iter().zip(SECTIONS) {
        assert_eq!(kept.gestures.len(), section.gestures.len());
    }
}

/// The point of matching both halves: the reader who knows the key and the reader who
/// knows what they want are both looking for the same row.
#[test]
fn a_row_is_found_by_its_keys_or_by_what_it_does() {
    let by_keys = matching(&typed("Ctrl+A"));
    let by_does = matching(&typed("Select the whole listing"));

    let one_row = |listed: &[Listed]| {
        assert_eq!(listed.len(), 1, "one section");
        assert_eq!(listed[0].gestures.len(), 1, "one row");
        listed[0].gestures[0].keys
    };

    assert_eq!(one_row(&by_keys), "Ctrl+A");
    assert_eq!(one_row(&by_does), "Ctrl+A");
}

/// A heading with nothing under it says the app has no gestures there, which is a lie the
/// filter would tell on every pattern.
#[test]
fn a_section_with_no_row_left_is_dropped() {
    let listed = matching(&typed("Wheel"));

    assert!(!listed.is_empty(), "the tab bar's wheel is a row");
    for kept in &listed {
        assert!(
            !kept.gestures.is_empty(),
            "{:?} was kept with nothing in it",
            kept.section.place
        );
    }
}

#[test]
fn a_pattern_nothing_matches_leaves_nothing() {
    assert!(matching(&typed("Ctrl+Meta+Q")).is_empty());
}

/// A pattern that will not compile matches nothing rather than everything, which is
/// `Matcher::Invalid`'s rule and worth pinning here: an empty page reads as a mistake, and
/// the whole list reads as the filter having been ignored.
#[test]
fn a_pattern_that_will_not_compile_leaves_nothing() {
    let broken = Filter {
        pattern: "(".to_owned(),
        regex: true,
        ..Filter::default()
    }
    .matcher();

    assert!(broken.error().is_some());
    assert!(matching(&broken).is_empty());
}

#[test]
fn every_section_has_a_name_and_rows() {
    for section in SECTIONS {
        assert!(!section.place.is_empty());
        assert!(
            !section.gestures.is_empty(),
            "{:?} has no rows",
            section.place
        );
        for gesture in section.gestures {
            assert!(!gesture.keys.is_empty(), "in {:?}", section.place);
            assert!(!gesture.does.is_empty(), "{:?}", gesture.keys);
        }
    }
}

/// Twice in one section is a row written twice; the same gesture in two sections is the
/// point, Escape and Tab each meaning something different in three places.
#[test]
fn no_gesture_is_written_twice_in_one_section() {
    for section in SECTIONS {
        for (at, gesture) in section.gestures.iter().enumerate() {
            let earlier = section.gestures[..at]
                .iter()
                .any(|before| before.keys == gesture.keys);

            assert!(!earlier, "{:?} twice in {:?}", gesture.keys, section.place);
        }
    }
}

/// The page names no place twice, so a filtered list cannot draw one heading over two
/// runs of rows.
#[test]
fn no_place_is_named_twice() {
    for (at, section) in SECTIONS.iter().enumerate() {
        let earlier = SECTIONS[..at]
            .iter()
            .any(|before| before.place == section.place);

        assert!(!earlier, "{:?} twice", section.place);
    }
}
