//! What the filter bar under each of the sidebar lists is asking for, and what answers it.
//!
//! This module is deliberately **framework-free** — no freya types appear here — so it can
//! move into a crate of its own alongside the rest of the non-UI code. A [`Filter`] is a
//! view of a list and never part of the session, so nothing here is serialized either.
//!
//! Every filter compiles to a [`regex::Regex`], the plain ones included: the three toggles
//! the goal asks for *are* three regex constructs — `(?i)` is "match case" turned off,
//! `\b…\b` is "whole word", and the third one is whether the pattern is read as a pattern
//! at all — so escaping a literal pattern and letting one engine answer all four
//! combinations is what makes them compose instead of four hand-written search loops that
//! agree with each other by inspection. It is also the faster answer, measured on
//! `viewer-sample`'s 151k demangled names (18 MB of text): 3 ms for a case-sensitive
//! literal against 3.7 ms for `str::contains`, and 8 ms case-insensitively against 7.4 ms
//! for lowercasing both sides — with the worst case seen, a whole-word single letter, at
//! 22 ms, which is one keystroke's worth of work and not one frame's.

use regex::{Regex, RegexBuilder};

/// One list's filter: what was typed, and the three toggles that say how to read it.
///
/// The default is what a list that has never been filtered has, and `matcher` answers
/// [`Matcher::Everything`] for it — so "no filter" costs no pass over the list at all.
#[derive(Clone, Default, PartialEq)]
pub struct Filter {
    pub pattern: String,
    /// Case matters. Off by default, which is what a search box is expected to do.
    pub case_sensitive: bool,
    /// The pattern has to be a whole word: `\b` on both ends of it. With `regex` on it
    /// wraps the user's whole pattern rather than being abandoned, which is what an
    /// alternation needs (`\b(?:a|b)\b`, never `\ba|b\b`).
    pub whole_word: bool,
    /// The pattern is a regular expression rather than text to be found literally.
    pub regex: bool,
}

impl Filter {
    /// The three toggles compose, so this is one expression built in three steps and
    /// never four cases.
    ///
    /// `case_insensitive` is set as a flag on the builder rather than as a `(?i)` prefix
    /// on purpose: a flag is the pattern's starting state, so a regex carrying its own
    /// `(?i)`/`(?-i)` still overrides it for the part it covers, which a prefix wrapped
    /// around the whole thing could not.
    pub fn matcher(&self) -> Matcher {
        if self.pattern.is_empty() {
            return Matcher::Everything;
        }

        let mut expression = if self.regex {
            self.pattern.clone()
        } else {
            regex::escape(&self.pattern)
        };
        if self.whole_word {
            expression = format!(r"\b(?:{expression})\b");
        }

        match RegexBuilder::new(&expression)
            .case_insensitive(!self.case_sensitive)
            .build()
        {
            Ok(regex) => Matcher::Pattern(regex),
            Err(error) => Matcher::Invalid(message(&error)),
        }
    }
}

/// A [`Filter`] compiled into the question it asks of each row.
pub enum Matcher {
    /// Nothing was typed. Kept apart from a pattern that happens to match everything so
    /// that a list with no filter on it can skip the pass entirely.
    Everything,
    Pattern(Regex),
    /// A pattern that will not compile, with what is wrong with it.
    ///
    /// A state of its own rather than a fallback to one of the other two, because both of
    /// those are lies a half-typed `(` would tell: matching everything hides the mistake
    /// and matching nothing looks like a list with nothing in it. The bar shows the
    /// message instead.
    Invalid(String),
}

impl Matcher {
    pub fn matches(&self, text: &str) -> bool {
        match self {
            Matcher::Everything => true,
            Matcher::Pattern(regex) => regex.is_match(text),
            Matcher::Invalid(_) => false,
        }
    }

    /// What is wrong with the pattern, for the bar to show.
    pub fn error(&self) -> Option<&str> {
        match self {
            Matcher::Invalid(message) => Some(message),
            _ => None,
        }
    }
}

/// The one line of a `regex` error worth putting in a filter bar.
///
/// `regex::Error`'s own `Display` is a four-line report — the pattern, a caret under the
/// offending byte, and then the sentence — which is right for a terminal and far too tall
/// for a strip under a list. The sentence is the last non-empty line of it, and the crate
/// prefixes that with `error:` where the whole thing is already labelled as one.
fn message(error: &regex::Error) -> String {
    let text = error.to_string();
    let line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(&text)
        .trim();

    line.strip_prefix("error:")
        .unwrap_or(line)
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
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
}
