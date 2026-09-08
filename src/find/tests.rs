use super::*;
use crate::filter::Filter;

/// A pattern as a bar with nothing toggled compiles it.
fn matcher(pattern: &str) -> Matcher {
    Filter {
        pattern: pattern.to_owned(),
        ..Filter::default()
    }
    .matcher()
}

/// A line of plain text, as the source pane draws one.
fn text(line: &str) -> Line {
    Line::text(line)
}

/// The columns a pattern hits in a line.
fn hits(line: &Line, pattern: &str) -> Vec<Range<usize>> {
    hits_in(line, &matcher(pattern))
}

/// No hits, spelt so the type is known.
fn none() -> Vec<Range<usize>> {
    Vec::new()
}

/// A hit on one row, for the steps below.
fn hit(row: usize, columns: Range<usize>) -> Hit {
    Hit { row, columns }
}

#[test]
fn a_pattern_hits_the_columns_it_covers() {
    assert_eq!(hits(&text("mov rax, rbx"), "rax"), vec![4..7]);
    assert_eq!(hits(&text("aaa"), "a"), vec![0..1, 1..2, 2..3]);
    assert_eq!(hits(&text("mov rax"), "rcx"), none());
}

/// Nothing typed marks nothing: a wash over every row is not an answer.
#[test]
fn an_empty_pattern_hits_nothing() {
    assert_eq!(hits(&text("mov rax, rbx"), ""), none());
}

/// A pattern that will not compile hits nothing either, the bar saying why instead.
#[test]
fn an_invalid_pattern_hits_nothing() {
    let filter = Filter {
        pattern: "(".to_owned(),
        regex: true,
        ..Filter::default()
    };
    assert_eq!(hits_in(&text("(a)"), &filter.matcher()), none());
}

/// **A run of adjacent text is matched whole.** An assembly line is pushed one span at a
/// time, so a pattern crossing two of them is the ordinary case and not an edge one:
/// matching each piece on its own answers nothing here.
#[test]
fn a_pattern_crosses_the_spans_a_line_is_pushed_in() {
    let mut line = Line::default();
    line.push_text("mov");
    line.push_text(" ");
    line.push_text("rax");

    assert_eq!(hits_in(&line, &matcher("mov rax")), vec![0..7]);
    assert_eq!(hits_in(&line, &matcher("v r")), vec![2..5]);
}

/// An inline element is one column and its text is a whole symbol name, so the hit is the
/// column it is drawn at -- there are no columns inside it to mark.
#[test]
fn an_inline_element_hits_as_one_column() {
    let mut line = Line::default();
    line.push_text("call ");
    line.push_inline("some_function");

    assert_eq!(hits_in(&line, &matcher("some_fun")), vec![5..6]);
    assert_eq!(hits_in(&line, &matcher("call")), vec![0..4]);
}

/// And it ends the run: a pattern cannot straddle it, the columns it would cover not
/// existing.
#[test]
fn a_pattern_does_not_straddle_an_inline_element() {
    let mut line = Line::default();
    line.push_text("call ");
    line.push_inline("target");
    line.push_text(", 7");

    assert_eq!(hits_in(&line, &matcher("call target")), none());
    // The text on the far side of it is still its own run, at the columns past the one
    // the element takes.
    assert_eq!(hits_in(&line, &matcher(", 7")), vec![6..9]);
}

/// Columns are UTF-16 units, which is what the caret and the wash are placed by: a
/// character outside the basic plane is two of them.
#[test]
fn columns_are_counted_in_utf16_units() {
    assert_eq!(hits(&text("\u{1f600}ab"), "ab"), vec![2..4]);
    // And a run after an inline counts the element as one, whatever its name is.
    let mut line = Line::default();
    line.push_inline("\u{1f600}\u{1f600}");
    line.push_text("ab");
    assert_eq!(hits_in(&line, &matcher("ab")), vec![1..3]);
}

/// A step goes to the next hit and wraps at the end; back goes the other way.
#[test]
fn a_step_walks_the_hits_and_wraps() {
    let hits = [hit(0, 0..3), hit(4, 1..4), hit(9, 0..2)];
    let caret = Caret { row: 0, col: 0 };

    assert_eq!(step(&hits, Some(0), caret, false), Some(1));
    assert_eq!(step(&hits, Some(2), caret, false), Some(0));
    assert_eq!(step(&hits, Some(1), caret, true), Some(0));
    assert_eq!(step(&hits, Some(0), caret, true), Some(2));
}

/// With nothing to step to there is nowhere to go, and the bar says so instead.
#[test]
fn a_step_with_no_hits_goes_nowhere() {
    assert_eq!(step(&[], None, Caret { row: 0, col: 0 }, false), None);
    assert_eq!(step(&[], Some(0), Caret { row: 0, col: 0 }, true), None);
}

/// The **first** step reads the caret, so a find starts from where the reader is looking
/// rather than from the top of the pane.
#[test]
fn the_first_step_starts_from_the_caret() {
    let hits = [hit(0, 0..3), hit(4, 1..4), hit(9, 0..2)];

    let between = Caret { row: 2, col: 0 };
    assert_eq!(step(&hits, None, between, false), Some(1));
    assert_eq!(step(&hits, None, between, true), Some(0));

    // Past the last hit forward, and before the first backward, each wrap once.
    let past = Caret { row: 20, col: 0 };
    assert_eq!(step(&hits, None, past, false), Some(0));
    let before = Caret { row: 0, col: 0 };
    assert_eq!(step(&hits, None, before, true), Some(2));
}

/// A caret sitting inside a hit -- a click in the middle of one -- steps out of it, not
/// back onto it.
#[test]
fn a_caret_inside_a_hit_steps_out_of_it() {
    let hits = [hit(0, 0..3), hit(4, 1..4)];

    let inside = Caret { row: 4, col: 2 };
    assert_eq!(step(&hits, None, inside, true), Some(0));
    // Forward, the hit the caret is in is behind it too -- its start is -- so there is
    // nothing ahead and the step wraps.
    assert_eq!(step(&hits, None, inside, false), Some(0));
}

/// A stale index -- the pattern changed and there are fewer hits than there were -- falls
/// back to the caret rather than pointing at nothing.
#[test]
fn a_step_from_an_index_that_is_gone_reads_the_caret() {
    let hits = [hit(0, 0..3), hit(4, 1..4)];
    let caret = Caret { row: 4, col: 0 };

    assert_eq!(step(&hits, Some(9), caret, false), Some(1));
}
