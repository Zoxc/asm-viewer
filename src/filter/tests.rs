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
