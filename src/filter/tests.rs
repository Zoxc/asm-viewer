use super::*;

/// Named `hits` rather than `matches` so the `matches!` macro next to it keeps its
/// own name.
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
    // Every toggle on and still nothing typed: a filter is what was typed, and the
    // toggles only say how to read it.
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

/// The point of the toggle being off: a pattern is text, not syntax, and a symbol
/// name is full of characters a regex would read.
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

/// `\b` sits between a word character and anything else, so a name's `_` binds and
/// its `::`, `<` and spaces do not — which is the whole reason the toggle is useful
/// on symbol names, where every interesting boundary is punctuation.
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

/// The alternation is why the wrapping is `\b(?:…)\b` and not `\b…\b`: without the
/// group the boundaries would bind to the first and last branch only, and `next`
/// would match anywhere.
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

/// All three at once, since composing is the property being claimed.
#[test]
fn the_toggles_compose() {
    let filter = Filter {
        pattern: "ITER(ator)?".to_owned(),
        case_sensitive: false,
        whole_word: true,
        regex: true,
    };
    assert!(hits(&filter, "core::iter::next"));
    assert!(hits(&filter, "core::Iterator::next"));
    assert!(!hits(&filter, "core::iteration"));

    let cased = Filter {
        case_sensitive: true,
        ..filter
    };
    assert!(!hits(&cased, "core::iter::next"));
    assert!(hits(&cased, "ITER::next"));
}

/// A regex carrying its own case flag overrides the toggle for the part it covers,
/// which is what setting the flag on the builder buys over a `(?i)` prefix.
#[test]
fn a_pattern_can_override_the_case_toggle() {
    let filter = Filter {
        regex: true,
        case_sensitive: true,
        ..plain("(?i)iter")
    };
    assert!(hits(&filter, "core::ITER::next"));
}

/// A half-typed pattern is the ordinary state of a filter box, so this is the case
/// that has to read as itself rather than as an empty list.
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

/// The same pattern with the toggle off is not invalid at all — it is a name with a
/// bracket in it, which is what a demangled name can look like.
#[test]
fn the_same_pattern_is_fine_as_text() {
    assert!(hits(&plain("core::(iter"), "core::(iter) something odd"));
}

/// The other way a `regex` build fails: a pattern that compiles to more than the
/// crate's size limit. It has to reach the same place a syntax error does rather
/// than panicking or being unwrapped.
#[test]
fn a_pattern_too_big_to_compile_is_invalid_too() {
    let matcher = Filter {
        regex: true,
        ..plain(r"(?:a{1000}){1000}")
    }
    .matcher();

    assert!(matcher.error().is_some());
    assert!(!matcher.matches("aaaa"));
}
