use super::*;

/// Named `hits` rather than `matches` so the `matches!` macro next to it keeps its own
/// name.
fn hits(filter: &Filter, text: &str) -> bool {
    filter.matcher().matches(text)
}

fn plain(pattern: &str) -> Filter {
    Filter {
        pattern: pattern.to_owned(),
        ..Filter::default()
    }
}

#[test]
fn empty_pattern_matches_everything() {
    assert!(matches!(Filter::default().matcher(), Matcher::Everything));
    // Every toggle on and still nothing typed.
    assert!(matches!(
        Filter {
            regex: true,
            whole_word: true,
            case_sensitive: true,
            ..plain("")
        }
        .matcher(),
        Matcher::Everything
    ));
}

#[test]
fn substring_ignores_case_until_told_not_to() {
    assert!(hits(&plain("iter"), "core::iter::Iterator::next"));
    assert!(hits(&plain("ITER"), "core::iter::Iterator::next"));

    let cased = Filter {
        case_sensitive: true,
        ..plain("Iter")
    };
    assert!(hits(&cased, "core::iter::Iterator::next"));
    assert!(!hits(&cased, "core::iter::next"));
}

/// A pattern is text and not syntax until the toggle is on, and a symbol name is full of
/// characters a regex would read.
#[test]
fn metacharacters_are_literal_until_regex_is_on() {
    assert!(hits(&plain("Vec<u8>"), "alloc::vec::Vec<u8>::push"));
    assert!(!hits(&plain("a.c"), "abc"));
    assert!(hits(
        &Filter {
            regex: true,
            ..plain("a.c")
        },
        "abc"
    ));
}

/// `\b` sits between a word character and anything else, so a name's `_` binds and its
/// `::`, `<` and spaces do not.
#[test]
fn whole_word_is_bounded_by_word_characters() {
    let word = Filter {
        whole_word: true,
        ..plain("iter")
    };
    assert!(hits(&word, "core::iter::Iterator"));
    assert!(hits(&word, "fn iter(&self)"));
    assert!(hits(&word, "<Vec<T> as Iter>"));
    assert!(!hits(&word, "core::iterator"));
    assert!(!hits(&word, "into_iter"));
    assert!(!hits(&word, "iter_mut"));
}

/// Why the wrapping is `\b(?:…)\b` and not `\b…\b`: without the group the boundaries bind
/// to the first and last branch only, and `next` would match anywhere.
#[test]
fn whole_word_wraps_the_whole_regex() {
    let filter = Filter {
        regex: true,
        whole_word: true,
        ..plain("iter|next")
    };
    assert!(hits(&filter, "core::iter::Iterator"));
    assert!(hits(&filter, "Iterator::next"));
    assert!(!hits(&filter, "iterator::nextish"));
}

/// A regex carrying its own case flag overrides the toggle for the part it covers, which
/// is what setting the flag on the builder buys over a `(?i)` prefix.
#[test]
fn a_pattern_can_override_the_case_toggle() {
    let filter = Filter {
        regex: true,
        case_sensitive: true,
        ..plain("(?i)iter")
    };
    assert!(hits(&filter, "core::ITER::next"));
}

/// A half-typed pattern is the ordinary state of a filter box, so this is the case that
/// has to read as itself rather than as an empty list.
#[test]
fn an_invalid_regex_says_so_and_matches_nothing() {
    let matcher = Filter {
        regex: true,
        ..plain("core::(iter")
    }
    .matcher();

    let error = matcher.error().expect("should not compile");
    assert!(!error.is_empty());
    assert!(!error.contains('\n'));
    assert!(!error.starts_with("error:"));
    assert!(!matcher.matches("core::iter::Iterator"));
    assert!(!matcher.matches("anything at all"));
}

/// The rank a filter gives a name, `None` where the name does not match.
fn rank(filter: &Filter, text: &str) -> Rank {
    filter
        .matcher()
        .rank(text)
        .unwrap_or_else(|| panic!("{text:?} should match"))
}

/// The tiers dominate the lengths: the prefix here is the longest name and the substring
/// the shortest, and the order is still prefix, word start, substring.
#[test]
fn a_prefix_outranks_a_word_start_outranks_a_substring() {
    let filter = plain("iter");
    let prefix = rank(&filter, "iterator::Iterator::next");
    let word = rank(&filter, "core::iter");
    let inside = rank(&filter, "into_iter");
    assert!(prefix < word);
    assert!(word < inside);
    assert_eq!(prefix.tier, Tier::Prefix);
    assert_eq!(word.tier, Tier::Word);
    assert_eq!(inside.tier, Tier::Inside);
    assert!(filter.matcher().rank("Vec::push").is_none());
}

/// Within a tier the shorter name is the one the pattern says more of.
#[test]
fn a_shorter_name_wins_a_tie() {
    let filter = plain("next");
    let short = rank(&filter, "a::next");
    let long = rank(&filter, "abc::next");
    assert_eq!(short.tier, long.tier);
    assert!(short < long);
    assert_eq!(rank(&filter, "x::next"), rank(&filter, "y::next"));
}

/// The word start is regex's `\b`, the Word toggle's notion, asked of the match's start
/// alone: `_` binds, so `into_iter` matches inside a word while `iter_mut` is a prefix;
/// `::`, `<` and a space bound, so `core::iterator` starts at a word even though the
/// toggle would reject it for how it ends.
#[test]
fn a_word_start_is_the_word_toggles_boundary() {
    let filter = plain("iter");
    assert_eq!(rank(&filter, "core::iter::Iterator").tier, Tier::Word);
    assert_eq!(rank(&filter, "fn iter(&self)").tier, Tier::Word);
    assert_eq!(rank(&filter, "<Vec<T> as Iter>").tier, Tier::Word);
    assert_eq!(rank(&filter, "core::iterator").tier, Tier::Word);
    assert_eq!(rank(&filter, "into_iter").tier, Tier::Inside);
    assert_eq!(rank(&filter, "iter_mut").tier, Tier::Prefix);
}

/// A regex ranks by where its first match lands, whatever it matched; one that matches
/// nothing at all -- an empty match -- starts nowhere and ranks last.
#[test]
fn a_regex_ranks_by_where_its_first_match_lands() {
    let regex = |pattern: &str| Filter {
        regex: true,
        ..plain(pattern)
    };
    assert_eq!(rank(&regex("[a-z]+::"), "core::iter").tier, Tier::Prefix);
    assert_eq!(rank(&regex("::n\\w+"), "core::next").tier, Tier::Word);
    assert_eq!(rank(&regex("e::"), "core::next").tier, Tier::Inside);
    assert_eq!(rank(&regex("x*"), "abc").tier, Tier::Inside);
}

/// Nothing typed ranks everything alike, and a pattern that will not compile ranks
/// nothing, as it matches nothing.
#[test]
fn nothing_typed_ranks_alike_and_an_invalid_pattern_ranks_nothing() {
    let none = Filter::default();
    assert_eq!(rank(&none, "abc").tier, Tier::Inside);
    assert!(rank(&none, "abc") < rank(&none, "abcd"));
    let invalid = Filter {
        regex: true,
        ..plain("core::(iter")
    };
    assert!(invalid.matcher().rank("core::iter").is_none());
}

/// The case toggle changes what matches and not how a match ranks.
#[test]
fn case_folding_does_not_change_the_rank() {
    assert_eq!(rank(&plain("iter"), "ITER::x").tier, Tier::Prefix);
    assert_eq!(rank(&plain("iter"), "x::ITER").tier, Tier::Word);
    let sensitive = Filter {
        case_sensitive: true,
        ..plain("iter")
    };
    assert!(sensitive.matcher().rank("ITER::x").is_none());
}

/// **What matched, and where.** Every occurrence and not only the first, since a name is
/// marked wherever the pattern is in it; nothing at all for a filter that lets everything
/// through, which is what an empty box is; and nothing for a pattern that will not
/// compile, which matches nothing to begin with.
#[test]
fn the_marks_are_every_occurrence_and_nothing_where_nothing_was_typed() {
    let marks = |filter: &Filter, text: &str| filter.matcher().marks(text);

    assert_eq!(marks(&plain("iter"), "iter::iter_mut"), [0..4, 6..10]);
    assert!(marks(&plain("nope"), "iter::iter_mut").is_empty());
    assert!(marks(&Filter::default(), "iter").is_empty());
    let invalid = Filter {
        regex: true,
        ..plain("core::(iter")
    };
    assert!(marks(&invalid, "core::iter").is_empty());

    // Case folds by default, so a mark lands where the fold matched and not where the
    // pattern would have.
    assert_eq!(marks(&plain("iter"), "ITER"), [0..4]);

    // A pattern that can match nothing marks nothing: a wash of no width is not a mark,
    // and `find_iter` would hand back one at every position.
    let empty = Filter {
        regex: true,
        ..plain("x*")
    };
    assert_eq!(marks(&empty, "axxb"), [1..3]);
}

/// The list the two tests below filter: against `next` it holds a prefix, two word starts
/// of different lengths, a substring, and a name that does not match at all.
fn names() -> Shared<String> {
    ["zz::next", "next_to", "std::next", "connext", "push"]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<String>>()
        .into()
}

fn filtered(list: Shared<String>, filter: &Filter) -> Filtered<String> {
    Filtered::new(list, &filter.matcher(), String::as_str)
}

/// Nothing typed leaves the list in its own order, and does it by keeping no indices at
/// all: no pass, no sort and no allocation, which is what a list under an empty box has to
/// cost.
#[test]
fn an_unfiltered_list_keeps_its_own_order_and_no_indices() {
    let list = filtered(names(), &Filter::default());

    assert!(list.matches.is_none());
    assert_eq!(list.len(), 5);
    assert!((0..5).all(|row| list.index(row) == row));
    assert_eq!(list.at(0).map(String::as_str), Some("zz::next"));
    assert_eq!(list.at(5), None);
}

/// Under a filter the rows come back by how well they matched -- a prefix, then a word
/// start, then a substring -- with the shorter name first among equals, and what did not
/// match at all is gone.
#[test]
fn a_filtered_list_puts_the_best_match_first() {
    let list = filtered(names(), &plain("next"));

    let rows: Vec<usize> = (0..list.len()).map(|row| list.index(row)).collect();
    assert_eq!(rows, [1, 0, 2, 3]);
    assert_eq!(list.at(0).map(String::as_str), Some("next_to"));
    assert_eq!(list.at(4), None);
}

/// Two [`Filtered`]s are the same one only where both halves are the same build: the list
/// compared by its pointer, and the indices kept from it by theirs. What the panels' props
/// rest on, a fresh build being what tells a scroll view to draw its rows again.
#[test]
fn a_filtered_list_is_equal_only_to_the_same_build() {
    let list = names();
    let one = filtered(list.clone(), &plain("next"));

    assert!(one == one.clone());
    assert!(one != filtered(list.clone(), &plain("next")));
    assert!(one != filtered(names(), &plain("next")));

    let all = filtered(list.clone(), &Filter::default());
    assert!(all == all.clone());
    // Both unfiltered, so both hold no indices: the list alone decides.
    assert!(all == filtered(list, &Filter::default()));
    assert!(all != filtered(names(), &Filter::default()));
}
