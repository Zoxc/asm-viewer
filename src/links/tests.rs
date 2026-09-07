use super::*;

/// The token types these tests name, in an arbitrary order: nothing may depend on an
/// index, only on a name.
const TYPES: [&str; 10] = [
    "comment",
    "method",
    "builtinType",
    "struct",
    "property",
    "keyword",
    "variable",
    "interface",
    "typeAlias",
    "unresolvedReference",
];

/// Likewise the modifiers, standard and rust-analyzer's own mixed together as a real
/// legend mixes them.
const MODIFIERS: [&str; 6] = [
    "async",
    "declaration",
    "defaultLibrary",
    "trait",
    "public",
    "reference",
];

fn legend() -> lsp::Legend {
    lsp::Legend::of(&TYPES, &MODIFIERS)
}

/// A token of the named type, with the named modifiers set.
fn token(kind: &str, modifiers: &[&str]) -> lsp::Token {
    let at = |names: &[&str], name: &str| {
        names
            .iter()
            .position(|held| *held == name)
            .unwrap_or_else(|| panic!("{name} is not in this test's legend"))
    };
    lsp::Token {
        line: 1,
        columns: 0..4,
        kind: at(&TYPES, kind) as u32,
        modifiers: modifiers
            .iter()
            .fold(0u32, |bits, name| bits | 1 << at(&MODIFIERS, name)),
    }
}

fn says(kind: &str, modifiers: &[&str]) -> Named {
    classify(&legend(), &token(kind, modifiers))
}

#[test]
fn a_name_the_server_placed_is_a_link() {
    let definition = Named::Asks(lsp::Followed::Definition);
    assert_eq!(says("method", &["reference"]), definition);
    assert_eq!(says("struct", &["public"]), definition);
    assert_eq!(says("property", &["public"]), definition);
    assert_eq!(says("interface", &["public"]), definition);
    assert_eq!(says("typeAlias", &[]), definition);
}

/// The reader asked for locals as links: the server places a `let` binding as readily as
/// it places an item, and following one goes to where it was bound.
#[test]
fn a_local_is_a_link_where_it_is_used() {
    assert_eq!(
        says("variable", &[]),
        Named::Asks(lsp::Followed::Definition)
    );
    // Where it is *bound* it is a declaration, and so is not one.
    assert_eq!(says("variable", &["declaration"]), Named::Defined);
}

/// Where a name is defined there is nothing to follow, which the server says outright
/// rather than the app guessing it from the `fn` in front of it.
#[test]
fn a_name_where_one_is_defined_is_not_a_link() {
    assert_eq!(says("method", &["declaration", "public"]), Named::Defined);
    assert_eq!(says("struct", &["declaration", "public"]), Named::Defined);
    assert_eq!(says("property", &["declaration"]), Named::Defined);
}

/// An item in a trait `impl` is the exception: it is written out here and declared in the
/// trait, so it is a link, and it asks the other question.
#[test]
fn an_item_in_a_trait_impl_asks_for_the_declaration() {
    assert_eq!(
        says("method", &["declaration", "trait"]),
        Named::Asks(lsp::Followed::Declaration)
    );
    // `trait` without `declaration` is a call to a trait method, which is an ordinary
    // link: its definition is the `impl` that runs, which is where the reader wants to go.
    assert_eq!(
        says("method", &["trait"]),
        Named::Asks(lsp::Followed::Definition)
    );
    // `declaration` without `trait` is an inherent `impl`'s own method, which is defined
    // where it is written and declared nowhere else.
    assert_eq!(says("method", &["declaration"]), Named::Defined);
}

/// A built-in type is placed nowhere, and neither is a name the server could not resolve.
/// Nor is anything lexical: a keyword or a comment is not a name at all.
#[test]
fn what_the_server_places_nowhere_is_not_a_link() {
    assert_eq!(says("builtinType", &[]), Named::Not);
    assert_eq!(says("unresolvedReference", &[]), Named::Not);
    assert_eq!(says("keyword", &[]), Named::Not);
    assert_eq!(says("comment", &[]), Named::Not);
}

/// `defaultLibrary` marks a name from **std**, which has a definition like any other: it
/// must not be mistaken for the marker of a built-in type, or every link into std would go.
#[test]
fn a_name_from_the_standard_library_is_a_link() {
    assert_eq!(
        says("struct", &["defaultLibrary"]),
        Named::Asks(lsp::Followed::Definition),
        "a std type is placed where std defines it"
    );
    assert_eq!(
        says("method", &["defaultLibrary"]),
        Named::Asks(lsp::Followed::Definition)
    );
}

/// A type index the legend never declared says nothing, so it is nothing to follow. A
/// server that renumbers its legend between versions is why nothing here reads an index.
#[test]
fn a_type_the_legend_does_not_have_is_not_a_link() {
    let unknown = lsp::Token {
        line: 1,
        columns: 0..4,
        kind: 99,
        modifiers: 0,
    };
    assert_eq!(classify(&legend(), &unknown), Named::Not);
}

/// A modifier the legend does not have cannot have been said of anything, whatever bits
/// the answer set.
#[test]
fn a_modifier_the_legend_does_not_have_is_never_said() {
    let legend = lsp::Legend::of(&["method"], &["async"]);
    let every_bit = lsp::Token {
        line: 1,
        columns: 0..4,
        kind: 0,
        modifiers: u32::MAX,
    };
    assert!(!legend.says(&every_bit, "declaration"));
    // So a name whose declaration cannot be spoken of is taken as a use, which is the
    // link the reader can follow rather than the one they cannot.
    assert_eq!(
        classify(&legend, &every_bit),
        Named::Asks(lsp::Followed::Definition)
    );
}

/// The rows are found by a binary search, so the order matters more than the count.
#[test]
fn the_links_of_a_line_are_the_ones_on_it_in_the_order_they_are_drawn() {
    let legend = lsp::Legend::of(&["method"], &[]);
    let at = |line: u32, from: u32| lsp::Token {
        line,
        columns: from..from + 3,
        kind: 0,
        modifiers: 0,
    };
    // Out of order on purpose: the answer is sorted, not trusted.
    let links = Links::of(&legend, &[at(4, 9), at(2, 7), at(4, 2), at(9, 0)]);

    assert_eq!(links.on_line(4).len(), 2);
    assert_eq!(
        links
            .on_line(4)
            .iter()
            .map(|link| link.columns.start)
            .collect::<Vec<u32>>(),
        vec![2, 9]
    );
    assert_eq!(links.on_line(2).len(), 1);
    // A line with nothing on it, and one past everything: neither is a row that draws a
    // link, and neither may reach past the end of the answer.
    assert!(links.on_line(3).is_empty());
    assert!(links.on_line(99).is_empty());

    // The link a column is inside, which is what a press asks about.
    assert_eq!(links.at(4, 3).map(|link| link.columns.start), Some(2));
    assert_eq!(links.at(4, 5), None);
}

/// A name where one is defined is kept, with nothing to follow: the row's menu is offered
/// there as much as where the name is used. Only the names a press can follow are drawn as
/// links.
#[test]
fn a_name_with_nothing_to_follow_is_kept_and_is_not_drawn_as_a_link() {
    let legend = legend();
    let mut used = token("method", &[]);
    used.columns = 8..12;
    let mut defined = token("method", &["declaration"]);
    defined.columns = 0..4;
    let links = Links::of(&legend, &[used, defined]);

    assert_eq!(links.on_line(1).len(), 2);
    assert_eq!(
        links.followed_on(1).cloned().collect::<Vec<Range<u32>>>(),
        vec![8..12]
    );
}
