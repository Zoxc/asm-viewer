use super::*;
use std::path::Path;

/// The committed gcc object, relocatable, so its first code section is placed at 0, and
/// its code.
fn code() -> (Arc<Object>, Arc<CodeListing>) {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/analysis/tests/fixtures/line_fixture.o");
    let object = analysis::open_files(vec![path])
        .into_iter()
        .next()
        .expect("the fixture parses");
    let code = Arc::new(CodeListing::new(&object));
    (object, code)
}

/// What a walk found, if anything: the line and its columns.
fn found(
    object: &Object,
    code: &Arc<CodeListing>,
    pattern: &str,
    from: Option<(CodeLine, usize)>,
    direction: Direction,
) -> Option<(CodeLine, Range<usize>)> {
    let filter = Filter {
        pattern: pattern.to_owned(),
        ..Filter::default()
    };
    let mut found = None;
    hunt(object, code, &filter, from, direction, &mut |event| {
        if let Hunted::Found(line, columns) = event {
            found = Some((line, columns));
        }
        ControlFlow::Continue(())
    });
    found
}

/// The address of the line a walk found, if anything, from a caret at the start of the
/// line at `from`.
fn walk(
    object: &Object,
    code: &Arc<CodeListing>,
    pattern: &str,
    from: Option<PlacedAddress>,
    direction: Direction,
) -> Option<PlacedAddress> {
    let from = from.map(|address| (line_at(object, code, address), 0));
    found(object, code, pattern, from, direction).map(|(line, _)| line.address)
}

/// The last line drawn at `address`, which for a stretch's start is its first instruction.
fn line_at(object: &Object, code: &Arc<CodeListing>, address: PlacedAddress) -> CodeLine {
    let flat = code.at(address).expect("the address is in the code");
    let (_, kind, _) = section_view::stretch_texts(object, code, flat)
        .into_iter()
        .filter(|(at, _, _)| *at == address)
        .last()
        .expect("a line is drawn at the address");
    CodeLine { address, kind }
}

/// With no caret, a walk starts at the very top of the code, so a match at placed address
/// 0 is found. Address 0 is where a relocatable object's first section sits, and it used
/// to stand for "no caret" as well, which made the walk skip that line.
#[test]
fn a_walk_with_no_caret_finds_a_match_at_address_zero() {
    let (object, code) = code();
    let lines = section_view::stretch_texts(&object, &code, 0);
    let (address, _, line) = lines
        .iter()
        .min_by_key(|(address, _, _)| *address)
        .expect("the first stretch draws something");
    assert_eq!(*address, PlacedAddress::ZERO, "the fixture's layout moved");
    let pattern = line.to_string();
    // The pattern is only found at 0, or a match further on would hide the bug.
    let elsewhere = (0..code.stretch_count())
        .flat_map(|flat| section_view::stretch_texts(&object, &code, flat))
        .filter(|(at, _, text)| *at != PlacedAddress::ZERO && text.to_string().contains(&pattern))
        .count();
    assert_eq!(elsewhere, 0, "the pattern is not unique to address 0");

    for direction in [Direction::Forward, Direction::Back] {
        assert_eq!(
            walk(&object, &code, &pattern, None, direction),
            Some(PlacedAddress::ZERO),
            "{direction:?} with no caret did not find the line at 0",
        );
    }
}

/// A walk comes back round into the stretch it started in, for the lines on the reader's
/// side of the caret: a match earlier in the same function going forward, or later in it
/// going back, is still found. The walk used to stop one stretch short of that.
#[test]
fn a_walk_wraps_back_into_the_stretch_it_started_in() {
    let (object, code) = code();
    let every: Vec<(usize, PlacedAddress, String)> = (0..code.stretch_count())
        .flat_map(|flat| {
            section_view::stretch_texts(&object, &code, flat)
                .into_iter()
                .map(move |(address, _, line)| (flat, address, line.to_string()))
        })
        .collect();
    // A line no other line contains, with lines of its own stretch on both sides of it.
    let (flat, address, text) = every
        .iter()
        .find(|(flat, address, text)| {
            !text.is_empty()
                && every
                    .iter()
                    .filter(|(_, _, t)| t.contains(text.as_str()))
                    .count()
                    == 1
                && every.iter().any(|(f, a, _)| f == flat && a < address)
                && every.iter().any(|(f, a, _)| f == flat && a > address)
        })
        .expect("the fixture has a line only it says, inside a stretch");
    let (flat, address) = (*flat, *address);
    let below = every
        .iter()
        .find(|(f, a, _)| *f == flat && *a > address)
        .map(|(_, a, _)| *a);
    let above = every
        .iter()
        .find(|(f, a, _)| *f == flat && *a < address)
        .map(|(_, a, _)| *a);

    assert_eq!(
        walk(&object, &code, text, below, Direction::Forward),
        Some(address),
        "forward from below the match in its own stretch",
    );
    assert_eq!(
        walk(&object, &code, text, above, Direction::Back),
        Some(address),
        "back from above the match in its own stretch",
    );
}

/// A walk names the row it found and starts from the caret's row and column, not from an
/// address: a section's header, a label and the first instruction share one, and a line
/// can hit twice. A walk that knew only addresses answered a label as the instruction
/// under it, and stepped past every other hit at the address the caret was on.
#[test]
fn a_walk_tells_apart_the_rows_at_one_address_and_the_hits_on_one_line() {
    let (object, code) = code();
    let at = |address: u64, kind: section::Kind| CodeLine {
        address: PlacedAddress::ZERO.saturating_add(address),
        kind,
    };
    let (header, push) = (
        at(0, section::Kind::Header),
        at(0, section::Kind::Instruction(0)),
    );

    let (label, _) = found(&object, &code, "sum_to", None, Direction::Forward).expect("sum_to");
    assert_eq!(
        label,
        at(0x30, section::Kind::Label(0)),
        "not found on its label"
    );

    // `section .text` and `push rbp`, both at 0.
    assert_eq!(
        found(&object, &code, "s", None, Direction::Forward),
        Some((header, 0..1)),
        "the fixture's layout moved",
    );
    assert_eq!(
        found(&object, &code, "s", Some((header, 1)), Direction::Forward),
        Some((push, 2..3)),
        "forward past the header skipped the instruction at its address",
    );
    assert_eq!(
        found(&object, &code, "s", Some((push, 2)), Direction::Back),
        Some((header, 0..1)),
        "back from the instruction skipped the header at its address",
    );

    // `mov rbp, rsp` hits `r` twice.
    let mov = at(1, section::Kind::Instruction(1));
    let second = found(&object, &code, "r", Some((mov, 11)), Direction::Forward);
    assert_eq!(
        second,
        Some((mov, 15..16)),
        "forward skipped the second hit"
    );
    let first = found(&object, &code, "r", Some((mov, 15)), Direction::Back);
    assert_eq!(first, Some((mov, 10..11)), "back skipped the first hit");
}
