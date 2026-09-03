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
