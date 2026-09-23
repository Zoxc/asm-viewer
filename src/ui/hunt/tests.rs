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

/// What a walk found, if anything.
fn walk(
    object: &Object,
    code: &Arc<CodeListing>,
    pattern: &str,
    from: Option<PlacedAddress>,
    direction: Direction,
) -> Option<PlacedAddress> {
    let filter = Filter {
        pattern: pattern.to_owned(),
        ..Filter::default()
    };
    let mut found = None;
    hunt(object, code, &filter, from, direction, &mut |event| {
        if let Hunted::Found(address, _) = event {
            found = Some(address);
        }
        ControlFlow::Continue(())
    });
    found
}

/// With no caret, a walk starts at the very top of the code, so a match at placed address
/// 0 is found. Address 0 is where a relocatable object's first section sits, and it used
/// to stand for "no caret" as well, which made the walk skip that line.
#[test]
fn a_walk_with_no_caret_finds_a_match_at_address_zero() {
    let (object, code) = code();
    let lines = section_view::stretch_texts(&object, &code, 0);
    let (address, line) = lines
        .iter()
        .min_by_key(|(address, _)| *address)
        .expect("the first stretch draws something");
    assert_eq!(*address, PlacedAddress::ZERO, "the fixture's layout moved");
    let pattern = line.to_string();
    // The pattern is only found at 0, or a match further on would hide the bug.
    let elsewhere = (0..code.stretch_count())
        .flat_map(|flat| section_view::stretch_texts(&object, &code, flat))
        .filter(|(at, text)| *at != PlacedAddress::ZERO && text.to_string().contains(&pattern))
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
                .map(move |(address, line)| (flat, address, line.to_string()))
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
