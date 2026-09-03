use super::*;

fn caret(row: usize, col: usize) -> Caret {
    Caret { row, col }
}

/// A press leaves an empty run: nothing to draw on its row, and nothing to copy.
#[test]
fn a_press_selects_nothing_until_it_is_swept() {
    let selection = CharSelection::at(caret(3, 4));
    assert!(selection.is_empty());
    assert_eq!(selection.of_row(3, 10), None);
    assert_eq!(selection.copy(|_| Line::text("abcdefghij")), "");
}

/// The ends come out in listing order whichever way the sweep went, so a sweep upwards
/// highlights and copies what one downwards does.
#[test]
fn the_ends_are_in_listing_order_whichever_way_they_were_swept() {
    let down = CharSelection::at(caret(1, 2)).extended(caret(3, 5));
    let up = CharSelection::at(caret(3, 5)).extended(caret(1, 2));
    assert_eq!(down.ends(), up.ends());
    assert_eq!(up.ends(), (caret(1, 2), caret(3, 5)));

    // Within one row too: the columns swap.
    let back = CharSelection::at(caret(1, 7)).extended(caret(1, 2));
    assert_eq!(back.of_row(1, 10), Some((2, 7)));
}

/// The first row is drawn from the first end's column to its end, the last from its
/// start to the second end's column, and every row between whole. Rows outside get
/// nothing.
#[test]
fn each_row_draws_its_own_part_of_the_run() {
    let selection = CharSelection::at(caret(1, 2)).extended(caret(3, 5));
    assert_eq!(selection.of_row(0, 10), None);
    assert_eq!(selection.of_row(1, 10), Some((2, 10)));
    assert_eq!(selection.of_row(2, 10), Some((0, 10)));
    assert_eq!(selection.of_row(2, 0), Some((0, 0)));
    assert_eq!(selection.of_row(3, 10), Some((0, 5)));
    assert_eq!(selection.of_row(4, 10), None);
    // A column past the row's text -- a sweep to the right of it -- is its end.
    assert_eq!(selection.of_row(3, 3), Some((0, 3)));
}

/// What is copied is each row's own part, in listing order, joined with newlines.
#[test]
fn copying_joins_each_rows_part_with_newlines() {
    let lines = ["mov rax, 1", "ret", "", "jmp 4"];
    let line = |row: usize| Line::text(lines.get(row).copied().unwrap_or_default());
    let selection = CharSelection::at(caret(0, 4)).extended(caret(3, 3));
    assert_eq!(selection.copy(line), "rax, 1\nret\n\njmp");

    // Upwards is the same text.
    let up = CharSelection::at(caret(3, 3)).extended(caret(0, 4));
    assert_eq!(up.copy(line), "rax, 1\nret\n\njmp");

    // Past the end of the listing is empty rows.
    let past = CharSelection::at(caret(3, 0)).extended(caret(5, 0));
    assert_eq!(past.copy(line), "jmp 4\n\n");
}

/// Columns are UTF-16 units, since that is what the text engine counts in; a column inside
/// a character two units wide rounds outward rather than cutting it.
#[test]
fn a_slice_never_splits_a_character() {
    // 'a', then a character that is two units, then 'b'.
    let line = Line::text("a\u{1F600}b");
    assert_eq!(line.units(), 4);
    assert_eq!(line.slice(0, 4), "a\u{1F600}b");
    assert_eq!(line.slice(1, 3), "\u{1F600}");
    // Inside the character, either side: the character comes whole.
    assert_eq!(line.slice(2, 4), "\u{1F600}b");
    assert_eq!(line.slice(0, 2), "a\u{1F600}");
    assert_eq!(line.slice(1, 1), "");
    // Past the end is the end, and reversed ends are put right.
    assert_eq!(line.slice(3, 9), "b");
    assert_eq!(line.slice(4, 1), "\u{1F600}b");
}

/// A relocation link is one unit of the row to the text engine and the whole name to the
/// clipboard, and copies whole when its unit is inside the range.
#[test]
fn an_inline_element_is_one_unit_and_copies_as_its_name() {
    let mut line = Line::default();
    line.push_text("call ");
    line.push_inline("core::fmt::write");
    line.push_text(" ; tail");
    assert_eq!(line.units(), "call ".len() + 1 + " ; tail".len());
    assert_eq!(line.to_string(), "call core::fmt::write ; tail");
    assert_eq!(line.slice(0, 6), "call core::fmt::write");
    assert_eq!(line.slice(5, 6), "core::fmt::write");
    assert_eq!(line.slice(6, 8), " ;");
    assert_eq!(line.slice(0, 5), "call ");
}

/// A sweep that has left the rows reaches the row on screen nearest the pointer, from its
/// start on the left and above, to its end on the right and below -- and nothing while the
/// pointer is over a row, which answers for itself.
#[test]
fn a_sweep_beyond_the_rows_reaches_the_row_on_screen_nearest_the_pointer() {
    // A box of four rows of 10, scrolled down by one row and a half: rows 1 to 5 on
    // screen, the first cut.
    let bounds = Bounds {
        left: 100.0,
        top: 50.0,
        right: 300.0,
        bottom: 90.0,
    };
    let rows_top = -15.0;
    let at = |x: f32, y: f32| beyond(bounds, rows_top, 10.0, 20, x, y);
    let caret = |row: usize, col: usize| Some(Caret { row, col });

    assert_eq!(
        at(150.0, 70.0),
        None,
        "over a row, which answers for itself"
    );
    assert_eq!(at(50.0, 70.0), caret(3, 0));
    assert_eq!(at(350.0, 70.0), caret(3, END));
    assert_eq!(at(150.0, 10.0), caret(1, 0));
    assert_eq!(at(150.0, 200.0), caret(5, END));
    // Off a corner, the vertical side decides.
    assert_eq!(at(50.0, 200.0), caret(5, END));
    assert_eq!(at(350.0, 10.0), caret(1, 0));

    // A listing shorter than its box: under its last row is that row's end, and the
    // rows on screen stop at the listing.
    let short = |x: f32, y: f32| beyond(bounds, 0.0, 10.0, 2, x, y);
    assert_eq!(short(150.0, 85.0), caret(1, END));
    assert_eq!(short(150.0, 200.0), caret(1, END));
    assert_eq!(short(150.0, 65.0), None, "over row 1");
    assert_eq!(short(50.0, 65.0), caret(1, 0));

    // Nothing to reach in an empty listing, and nothing with rows of no height.
    assert_eq!(beyond(bounds, 0.0, 10.0, 0, 50.0, 70.0), None);
    assert_eq!(beyond(bounds, 0.0, 0.0, 5, 50.0, 70.0), None);
}

/// The listing the key tests move through: a row of words and punctuation, a short one,
/// an empty one, one with a character two units wide, and a last.
fn listing(row: usize) -> Line {
    match row {
        0 => Line::text("mov rax, [rbp-8]"),
        1 => Line::text("ret"),
        2 => Line::text(""),
        3 => Line::text("a\u{1F600}b"),
        4 => Line::text("jmp short 4Bh"),
        _ => Line::default(),
    }
}

fn moved(selection: CharSelection, motion: Motion, extend: bool) -> CharSelection {
    selection.moved(motion, extend, listing, 5, 2)
}

/// A step by character: one character at a time, whole, whatever its width -- and from a
/// row's start to the row above's end, from its end to the row below's start.
#[test]
fn left_and_right_step_by_character_and_cross_rows_at_their_ends() {
    let line = listing(3);
    assert_eq!(line.after(0), Some(1));
    assert_eq!(line.after(1), Some(3), "the wide character is one step");
    assert_eq!(line.after(2), Some(3), "from inside it, its end");
    assert_eq!(line.after(4), None);
    assert_eq!(line.before(4), Some(3));
    assert_eq!(line.before(3), Some(1));
    assert_eq!(line.before(2), Some(1), "from inside it, its start");
    assert_eq!(line.before(0), None);

    let at = |row, col| CharSelection::at(caret(row, col));
    assert_eq!(moved(at(0, 3), Motion::Right, false).lead(), caret(0, 4));
    assert_eq!(moved(at(0, 16), Motion::Right, false).lead(), caret(1, 0));
    assert_eq!(moved(at(1, 0), Motion::Left, false).lead(), caret(0, 16));
    // Through the empty row: on to it, and off it again.
    assert_eq!(moved(at(1, 3), Motion::Right, false).lead(), caret(2, 0));
    assert_eq!(moved(at(2, 0), Motion::Right, false).lead(), caret(3, 0));
    assert_eq!(moved(at(3, 0), Motion::Left, false).lead(), caret(2, 0));
    // The listing's ends hold.
    assert_eq!(moved(at(0, 0), Motion::Left, false).lead(), caret(0, 0));
    assert_eq!(moved(at(4, 13), Motion::Right, false).lead(), caret(4, 13));
    // A lead a sweep left past the row's end is the end, and a row past the listing is
    // its last.
    assert_eq!(moved(at(1, END), Motion::Left, false).lead(), caret(1, 2));
    assert_eq!(moved(at(9, 0), Motion::Right, false).lead(), caret(4, 1));
}

/// A step by word passes over whitespace and then over a run of one kind: an
/// identifier, a number, or a run of punctuation, each a word; an inline element is a
/// word of its own.
#[test]
fn a_step_by_word_takes_a_run_of_one_kind() {
    let line = listing(0);
    // "mov rax, [rbp-8]": rightward stops after mov, rax, ",", "[", rbp, "-", 8, "]".
    let mut stops = Vec::new();
    let mut col = 0;
    while let Some(next) = line.word_after(col) {
        stops.push(next);
        col = next;
    }
    assert_eq!(stops, [3, 7, 8, 10, 13, 14, 15, 16]);
    // Leftward, the starts: the same words from the other side.
    let mut starts = Vec::new();
    let mut col = 16;
    while let Some(next) = line.word_before(col) {
        starts.push(next);
        col = next;
    }
    assert_eq!(starts, [15, 14, 13, 10, 9, 7, 4, 0]);
    // Trailing and leading whitespace goes to the row's end or start.
    let padded = Line::text("  x  ");
    assert_eq!(padded.word_after(3), Some(5));
    assert_eq!(padded.word_before(2), Some(0));
    assert_eq!(padded.word_after(5), None);
    assert_eq!(padded.word_before(0), None);
    // Underscores are word characters, and an inline element is one word.
    let mut call = Line::default();
    call.push_text("call my_fn_2 ");
    call.push_inline("core::fmt::write");
    call.push_text("+8");
    assert_eq!(call.word_after(5), Some(12));
    assert_eq!(call.word_after(12), Some(14), "the inline element");
    assert_eq!(call.word_before(14), Some(13));
    assert_eq!(call.word_after(14), Some(15));

    // Through the selection, and across rows at the ends as a character step does.
    let at = |row, col| CharSelection::at(caret(row, col));
    assert_eq!(
        moved(at(0, 0), Motion::WordRight, false).lead(),
        caret(0, 3)
    );
    assert_eq!(
        moved(at(0, 16), Motion::WordRight, false).lead(),
        caret(1, 0)
    );
    assert_eq!(
        moved(at(1, 0), Motion::WordLeft, false).lead(),
        caret(0, 16)
    );
    assert_eq!(moved(at(1, 3), Motion::WordLeft, false).lead(), caret(1, 0));
}

/// A vertical move keeps the column it set out from through rows too short to hold it:
/// the goal is the column before the first of them, and the lead comes back to it.
#[test]
fn a_vertical_move_remembers_its_goal_column() {
    let at = CharSelection::at(caret(0, 10));
    let down = moved(at, Motion::Down, false);
    assert_eq!(down.lead(), caret(1, 3), "clamped to the short row");
    let down = moved(down, Motion::Down, false);
    assert_eq!(down.lead(), caret(2, 0));
    let down = moved(down, Motion::Down, false);
    assert_eq!(down.lead(), caret(3, 4));
    let down = moved(down, Motion::Down, false);
    assert_eq!(down.lead(), caret(4, 10), "the goal column, reached again");
    // And back up the same way.
    let up = moved(moved(down, Motion::Up, false), Motion::Up, false);
    assert_eq!(up.lead(), caret(2, 0));
    assert_eq!(moved(up, Motion::PageUp, false).lead(), caret(0, 10));

    // A sideways move sets a column of its own and forgets the goal.
    let aside = moved(moved(at, Motion::Down, false), Motion::Left, false);
    assert_eq!(aside.lead(), caret(1, 2));
    assert_eq!(moved(aside, Motion::Down, false).lead(), caret(2, 0));
    assert_eq!(
        moved(moved(aside, Motion::Down, false), Motion::Down, false).lead(),
        caret(3, 2)
    );
    // So does a sweep.
    let swept = moved(at, Motion::Down, false).extended(caret(1, 1));
    assert_eq!(
        moved(moved(swept, Motion::Down, false), Motion::Down, false).lead(),
        caret(3, 1)
    );
}

/// The ends: a row's, the listing's, and a page at a time; the listing's ends clamp
/// rather than wrap.
#[test]
fn the_ends_and_the_pages() {
    let at = |row, col| CharSelection::at(caret(row, col));
    assert_eq!(moved(at(0, 5), Motion::RowStart, false).lead(), caret(0, 0));
    assert_eq!(moved(at(0, 5), Motion::RowEnd, false).lead(), caret(0, 16));
    assert_eq!(
        moved(at(3, 2), Motion::ListingStart, false).lead(),
        caret(0, 0)
    );
    assert_eq!(
        moved(at(0, 5), Motion::ListingEnd, false).lead(),
        caret(4, 13)
    );
    // A page is two rows here, and the goal column carries.
    assert_eq!(moved(at(0, 5), Motion::PageDown, false).lead(), caret(2, 0));
    assert_eq!(moved(at(3, 1), Motion::PageDown, false).lead(), caret(4, 1));
    assert_eq!(moved(at(4, 5), Motion::PageDown, false).lead(), caret(4, 5));
    assert_eq!(moved(at(1, 1), Motion::PageUp, false).lead(), caret(0, 1));
    assert_eq!(moved(at(0, 1), Motion::Up, false).lead(), caret(0, 1));
    assert_eq!(moved(at(4, 1), Motion::Down, false).lead(), caret(4, 1));
    // A page of no rows is still a page of one.
    assert_eq!(
        at(1, 0)
            .moved(Motion::PageDown, false, listing, 5, 0)
            .lead(),
        caret(2, 0)
    );
    // And nothing moves in a listing of no rows.
    let none = at(0, 3);
    assert_eq!(none.moved(Motion::Right, false, listing, 0, 2), none);
}

/// Shift keeps the anchor and moves the lead; without it the run collapses to where the
/// lead went, whichever way round it was.
#[test]
fn a_move_extends_with_shift_and_collapses_without() {
    let at = CharSelection::at(caret(0, 4));
    let extended = moved(at, Motion::WordRight, true);
    assert_eq!(extended.ends(), (caret(0, 4), caret(0, 7)));
    let extended = moved(extended, Motion::Down, true);
    assert_eq!(extended.ends(), (caret(0, 4), caret(1, 3)));
    let back = moved(extended, Motion::Up, true);
    assert_eq!(back.ends(), (caret(0, 4), caret(0, 7)));
    let before = moved(moved(back, Motion::WordLeft, true), Motion::WordLeft, true);
    assert_eq!(
        before.ends(),
        (caret(0, 0), caret(0, 4)),
        "reached back past the anchor"
    );

    let collapsed = moved(before, Motion::Right, false);
    assert!(collapsed.is_empty());
    assert_eq!(collapsed.lead(), caret(0, 1));
}
