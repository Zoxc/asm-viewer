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

/// The rows a run touches come out in listing order whichever way round it was swept, so
/// a sweep upwards lights what one downwards does. They are the run the two panes point
/// at each other through, and there is no second copy of them.
#[test]
fn the_rows_are_in_listing_order_whichever_way_they_were_picked() {
    let forwards = CharSelection::at(caret(2, 6)).extended(caret(4, 1));
    let backwards = CharSelection::at(caret(4, 1)).extended(caret(2, 6));

    assert_eq!(forwards.rows(), 2..=4);
    assert_eq!(backwards.rows(), 2..=4);
    assert!(backwards.contains_row(3));
    assert!(!backwards.contains_row(5));

    // A run within one row is that row alone, which is what a press leaves.
    let pressed = CharSelection::at(caret(7, 3));
    assert_eq!(pressed.rows(), 7..=7);
    assert!(pressed.contains_row(7));
    assert!(!pressed.contains_row(6));
}

/// Reaching out moves the lead and leaves the anchor, so a second shift-click the other
/// side of the anchor corrects the first rather than running on from where it ended.
#[test]
fn extending_moves_the_lead_and_leaves_the_anchor() {
    let selection = CharSelection::at(caret(5, 0))
        .extended(caret(9, 2))
        .extended(caret(2, 4));

    assert_eq!(selection.rows(), 2..=5);
    assert_eq!(selection.anchor(), caret(5, 0));
    assert_eq!(selection.lead(), caret(2, 4));
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

/// How wide a piece draws is one rule, and counting a row's units, laying out its atoms
/// and slicing it all read it off the same place: the atoms run end to end from nothing to
/// the row's units, and slicing one atom's columns copies the one character it spans --
/// the whole name for an inline element's one column.
#[test]
fn the_units_the_atoms_and_the_slice_put_a_column_in_the_same_place() {
    let mut line = Line::default();
    line.push_text("mov a\u{1F600}, ");
    line.push_inline("core::fmt::write");
    line.push_text("+8");

    // What each column of the row copies, an inline element counting as one.
    let copied = [
        "m",
        "o",
        "v",
        " ",
        "a",
        "\u{1F600}",
        ",",
        " ",
        "core::fmt::write",
        "+",
        "8",
    ];
    let atoms = line.atoms();
    assert_eq!(atoms.len(), copied.len());

    let mut at = 0;
    for (atom, text) in atoms.iter().zip(copied) {
        assert_eq!(
            atom.start, at,
            "an atom starts where the one before it ended"
        );
        assert_eq!(
            line.slice(atom.start, atom.end),
            text,
            "columns {}..{} copy the character they span",
            atom.start,
            atom.end
        );
        at = atom.end;
    }
    assert_eq!(at, line.units(), "the atoms end where the row's units do");
    // The wide character is two columns and the inline element one, whatever its name.
    assert_eq!(atoms[5].end - atoms[5].start, 2);
    assert_eq!(atoms[8].end - atoms[8].start, 1);
}

/// A sweep that has left the rows reaches the row on screen nearest the pointer, at the
/// pointer's x clamped into the box -- and nothing while the pointer is over a row, which
/// answers for itself.
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
    let reach = |row: usize, x: f32| Some(Reach { row, x });

    assert_eq!(
        at(150.0, 70.0),
        None,
        "over a row, which answers for itself"
    );
    assert_eq!(at(50.0, 70.0), reach(3, 100.0));
    assert_eq!(at(350.0, 70.0), reach(3, 299.0));
    assert_eq!(at(150.0, 10.0), reach(1, 150.0));
    assert_eq!(at(150.0, 200.0), reach(5, 150.0));
    // Off a corner, the vertical side picks the row and the horizontal the x.
    assert_eq!(at(50.0, 200.0), reach(5, 100.0));
    assert_eq!(at(350.0, 10.0), reach(1, 299.0));

    // A listing shorter than its box: under its last row is that row, and the rows on
    // screen stop at the listing.
    let short = |x: f32, y: f32| beyond(bounds, 0.0, 10.0, 2, x, y);
    assert_eq!(short(150.0, 85.0), reach(1, 150.0));
    assert_eq!(short(150.0, 200.0), reach(1, 150.0));
    assert_eq!(short(150.0, 65.0), None, "over row 1");
    assert_eq!(short(50.0, 65.0), reach(1, 100.0));

    // Nothing to reach in an empty listing, and nothing with rows of no height.
    assert_eq!(beyond(bounds, 0.0, 10.0, 0, 50.0, 70.0), None);
    assert_eq!(beyond(bounds, 0.0, 0.0, 5, 50.0, 70.0), None);
}

/// A box the rows cannot be read in still answers: freya lays out in `f32`, and a pane
/// squeezed to a sliver -- a window resized very short, a pane mid-collapse -- gave a box
/// under half a pixel tall whose last row on screen came out above its first, which
/// `usize::clamp` panics on. A NaN edge is the same story for the x, `f32::clamp`
/// panicking on bounds it cannot order.
#[test]
fn a_box_the_rows_cannot_be_read_in_reaches_a_row_all_the_same() {
    // Two tenths of a pixel tall, across the boundary between rows 4 and 5: the first
    // row on screen is 5 and the last, before it is held to the first, is 4.
    let sliver = Bounds {
        left: 0.0,
        top: 100.0,
        right: 50.0,
        bottom: 100.3,
    };
    assert_eq!(
        beyond(sliver, -100.1, 20.0, 10, -5.0, 100.1),
        Some(Reach { row: 5, x: 0.0 })
    );

    let edgeless = Bounds {
        left: f32::NAN,
        ..sliver
    };
    assert_eq!(beyond(edgeless, -100.1, 20.0, 10, -5.0, 100.1), None);
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
    let wide = listing(3).atoms();
    assert_eq!(after(&wide, 0), Some(1));
    assert_eq!(after(&wide, 1), Some(3), "the wide character is one step");
    assert_eq!(after(&wide, 2), Some(3), "from inside it, its end");
    assert_eq!(after(&wide, 4), None);
    assert_eq!(before(&wide, 4), Some(3));
    assert_eq!(before(&wide, 3), Some(1));
    assert_eq!(before(&wide, 2), Some(1), "from inside it, its start");
    assert_eq!(before(&wide, 0), None);

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
    let row = listing(0).atoms();
    // "mov rax, [rbp-8]": rightward stops after mov, rax, ",", "[", rbp, "-", 8, "]".
    let mut stops = Vec::new();
    let mut col = 0;
    while let Some(next) = word_after(&row, col) {
        stops.push(next);
        col = next;
    }
    assert_eq!(stops, [3, 7, 8, 10, 13, 14, 15, 16]);
    // Leftward, the starts: the same words from the other side.
    let mut starts = Vec::new();
    let mut col = 16;
    while let Some(next) = word_before(&row, col) {
        starts.push(next);
        col = next;
    }
    assert_eq!(starts, [15, 14, 13, 10, 9, 7, 4, 0]);
    // Trailing and leading whitespace goes to the row's end or start.
    let padded = Line::text("  x  ").atoms();
    assert_eq!(word_after(&padded, 3), Some(5));
    assert_eq!(word_before(&padded, 2), Some(0));
    assert_eq!(word_after(&padded, 5), None);
    assert_eq!(word_before(&padded, 0), None);
    // Underscores are word characters, and an inline element is one word.
    let mut line = Line::default();
    line.push_text("call my_fn_2 ");
    line.push_inline("core::fmt::write");
    line.push_text("+8");
    let call = line.atoms();
    assert_eq!(word_after(&call, 5), Some(12));
    assert_eq!(word_after(&call, 12), Some(14), "the inline element");
    assert_eq!(word_before(&call, 14), Some(13));
    assert_eq!(word_after(&call, 14), Some(15));

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

/// A sweep by rows takes whole rows, the anchor at its row's start and the lead at the
/// swept row's end going down and the reverse going up; back on its own row it is the
/// caret the press left.
#[test]
fn a_sweep_by_rows_takes_whole_rows_either_way() {
    let pressed = CharSelection::at(Caret { row: 3, col: 0 });
    let down = pressed.by_rows(5);
    assert_eq!(
        down.ends(),
        (Caret { row: 3, col: 0 }, Caret { row: 5, col: END })
    );
    assert_eq!(down.of_row(4, 7), Some((0, 7)));
    let up = pressed.by_rows(1);
    assert_eq!(
        up.ends(),
        (Caret { row: 1, col: 0 }, Caret { row: 3, col: END })
    );
    assert_eq!(up.lead(), Caret { row: 1, col: 0 });
    assert_eq!(pressed.by_rows(3), pressed);
    assert!(down.by_rows(3).is_empty());
    assert_eq!(
        down.collapsed(),
        CharSelection::at(Caret { row: 5, col: END })
    );
}

/// Mapping a run's rows keeps which end is the caret and keeps the goal column the
/// vertical moves aim for, and answers `None` where a row has no row any more.
#[test]
fn a_mapped_run_keeps_its_lead_and_its_goal() {
    let up = CharSelection::at(caret(3, 5)).extended(caret(1, 2));
    let mapped = up.mapped(|row| Some(row + 1)).expect("every row answered");
    assert_eq!(
        mapped.lead(),
        caret(2, 2),
        "the lead came back at the bottom"
    );
    assert_eq!(mapped.ends(), (caret(2, 2), caret(4, 5)));
    assert_eq!(up.mapped(|row| (row != 3).then_some(row)), None);

    // The goal survives, so a move down through a short row still comes back to it.
    let down = moved(CharSelection::at(caret(0, 10)), Motion::Down, false);
    let mapped = down.mapped(|row| Some(row + 1)).expect("the row answered");
    assert_eq!(mapped.lead(), caret(2, 3));
    assert_eq!(moved(mapped, Motion::Down, false).lead(), caret(3, 4));
}

// A cursor put where the compiler pointed (`src/ui/pad.rs`).

/// A diagnostic's place is a line and a column the way rustc counts them; a cursor is one
/// number the way an editor counts it. This is the whole of the conversion, and the unit
/// is UTF-16 code units because that is what a cursor position is.
#[test]
fn a_span_is_a_cursor_position() {
    let source = "fn main() {\n    let x = 1;\n}\n";

    // One-based, both halves: line 2 column 5 is the `l` of `let`, which is char 16.
    assert_eq!(offset_of(source, 2, 5), 16);
    // The first character of the file, which is where a span with no useful place lands.
    assert_eq!(offset_of(source, 1, 1), 0);
    // The line break is not on the line: the last line is the empty one after it.
    assert_eq!(offset_of(source, 3, 1), 27);
    assert_eq!(offset_of(source, 4, 1), source.len());
}

/// A column is counted in characters and a cursor in UTF-16 code units, so a line with an
/// astral character in it is where the two disagree — one character, two code units. A
/// cursor placed by character count would sit one place left of the span for every one of
/// them before it.
#[test]
fn a_column_is_characters_and_a_cursor_is_code_units() {
    // `é` is one char and one code unit; `𝄞` is one char and two.
    let source = "// é𝄞 x\nlet y = 2;\n";

    // Column 7 is the `x`: six characters before it — `/`, `/`, ` `, `é`, `𝄞`, ` ` — which
    // are seven code units, the `𝄞` being two.
    assert_eq!(offset_of(source, 1, 7), 7);
    // And the line below starts after the whole of the line above, its break included:
    // eight characters, nine code units.
    assert_eq!(offset_of(source, 2, 1), 9);
}

/// The source is edited under a diagnostic — the reader has usually typed since the build —
/// so a span that no longer fits is clamped rather than dropped. Nowhere near a panic and
/// never past the end of the text.
#[test]
fn a_span_the_source_has_outgrown_is_clamped() {
    let source = "fn main() {}\n";

    // Past the end of its line: the end of that line, and not the line below.
    assert_eq!(offset_of(source, 1, 500), 12);
    // Past the end of the file: the end of the file.
    assert_eq!(offset_of(source, 99, 1), source.len());
    // Nothing to point at at all.
    assert_eq!(offset_of("", 1, 1), 0);
    // Zero is not a line rustc writes, and is the first line rather than a subtraction
    // that wraps.
    assert_eq!(offset_of(source, 0, 0), 0);
}

// The two units a column is counted in, and the conversion between them
// (`src/lsp.rs`, `src/ui/source_view.rs`).

/// A line where the two units part company: an emoji is four bytes and two UTF-16 units,
/// so every column after one is a different number in each. `// ` is three of both.
const WIDE: &str = "// \u{1f980} helper";

#[test]
fn a_byte_offset_and_a_column_are_the_same_number_until_a_wide_character() {
    // Before the crab the two agree, after it they are two apart.
    assert_eq!(columns_of(WIDE, 0..3), 0..3);
    assert_eq!(columns_of(WIDE, 8..14), 6..12);
    assert_eq!(bytes_of(WIDE, 6..12), 8..14);
    // And a line of nothing but ASCII never tells them apart.
    assert_eq!(columns_of("let x = 1;", 4..5), 4..5);
    assert_eq!(bytes_of("let x = 1;", 4..5), 4..5);
}

#[test]
fn a_column_inside_a_character_is_that_characters_start() {
    // Half of the crab, from either side: each end comes back at the start of the
    // character it is inside, and never at a boundary a slice would panic on. Column 4
    // is the crab's second unit, so it rounds back to the crab; byte 5 is inside it too.
    assert_eq!(columns_of(WIDE, 4..6), 3..3);
    assert_eq!(bytes_of(WIDE, 4..5), 3..7);
    // The units are what the row draws, so a whole crab is two of them.
    assert_eq!(columns_of(WIDE, 3..7), 3..5);
}

#[test]
fn a_run_the_line_is_too_short_for_stops_at_its_end() {
    // A line that changed under the answer, and one that has nothing to point at at all.
    assert_eq!(columns_of(WIDE, 90..99), 12..12);
    assert_eq!(bytes_of(WIDE, 90..99), 14..14);
    assert_eq!(columns_of("", 0..4), 0..0);
    assert_eq!(bytes_of("", 0..4), 0..0);
    // Ends the wrong way round come back empty at the start: not reversed, which panics
    // where the range is used, and not empty at the smaller end. Both ends are inside the
    // line, so this is the reversal and not the clamp above.
    assert_eq!(columns_of(WIDE, backwards(8, 3)), 6..6);
    assert_eq!(bytes_of(WIDE, backwards(6, 3)), 8..8);
}

/// A range whose ends are the wrong way round. One only ever comes from a stale answer,
/// so it is built from its ends rather than written `8..3`, which reads as a typo and is
/// an error to clippy.
fn backwards(start: usize, end: usize) -> Range<usize> {
    start..end
}

/// The two ways a cut lands inside a character, side by side: `bytes_of` rounds it back to
/// the character's start, `slice_of` refuses it, and a caller of `slice_of` can tell the
/// two cases apart because refusing is the only way it says nothing.
#[test]
fn a_cut_inside_a_character_rounds_one_way_and_is_refused_the_other() {
    // Column 4 is the crab's second unit. `bytes_of` takes the whole crab with it.
    assert_eq!(bytes_of(WIDE, 0..4), 0..3);
    assert_eq!(slice_of(WIDE, 0..4), None);
    // From inside it as well as up to inside it.
    assert_eq!(bytes_of(WIDE, 4..7), 3..9);
    assert_eq!(slice_of(WIDE, 4..7), None);
    // A cut on the boundaries either side of it is the same run for both.
    assert_eq!(&WIDE[bytes_of(WIDE, 3..5)], "\u{1f980}");
    assert_eq!(slice_of(WIDE, 3..5), Some("\u{1f980}"));
    assert_eq!(slice_of(WIDE, 0..3), Some("// "));
    assert_eq!(slice_of(WIDE, 5..12), Some(" helper"));
}

/// Nothing to cut is nothing to draw: an empty run and a reversed one both come back
/// `None`, where `bytes_of` answers an empty range at a place. Past the end is `None` too,
/// there being no character boundary there to land on.
#[test]
fn a_slice_of_nothing_is_refused() {
    assert_eq!(slice_of(WIDE, 3..3), None);
    assert_eq!(slice_of(WIDE, backwards(5, 3)), None);
    assert_eq!(bytes_of(WIDE, backwards(5, 3)), 7..7);
    assert_eq!(slice_of(WIDE, 0..99), None);
    assert_eq!(bytes_of(WIDE, 0..99), 0..14);
    assert_eq!(slice_of("", 0..0), None);
}

/// The nth character, in bytes: where an elision cuts, and the string's own length when
/// there is nothing to cut. The crab is one character and four bytes, so the count and the
/// offset part company at it.
#[test]
fn the_nth_character_is_where_an_elision_cuts() {
    assert_eq!(byte_of_char(WIDE, 3), 3);
    assert_eq!(&WIDE[..byte_of_char(WIDE, 3)], "// ");
    assert_eq!(&WIDE[..byte_of_char(WIDE, 4)], "// \u{1f980}");
    // Exactly as long as it is, and longer: both have nothing past the cut, which is what
    // an answer of the string's own length says.
    assert_eq!(byte_of_char(WIDE, WIDE.chars().count()), WIDE.len());
    assert_eq!(byte_of_char(WIDE, 99), WIDE.len());
    assert_eq!(byte_of_char("", 0), 0);
}
