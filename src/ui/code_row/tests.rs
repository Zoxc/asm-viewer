//! What a press on a row's text means.

use super::*;

#[test]
fn one_press_is_a_caret_two_the_word_and_three_the_whole_row() {
    let word = |_| Some((4, 9));
    assert_eq!(pressed(PressEventType::Double, 6, word), Press::Span(4, 9));
    assert_eq!(
        pressed(PressEventType::Triple, 6, word),
        Press::Span(0, usize::MAX)
    );
    assert_eq!(
        pressed(PressEventType::Quadruple, 6, word),
        Press::Span(0, usize::MAX),
        "a fourth press is the row again, not the word back"
    );
    assert_eq!(
        pressed(PressEventType::Double, 6, |_| None),
        Press::At(6),
        "a row with no boundary to give"
    );
}

#[test]
fn a_single_press_asks_the_row_nothing() {
    assert_eq!(
        pressed(PressEventType::Single, 6, |_| panic!(
            "the word is asked for only where the answer turns on it"
        )),
        Press::At(6)
    );
}
