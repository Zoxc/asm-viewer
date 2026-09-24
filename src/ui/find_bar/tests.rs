//! What a bar's hits are about.

use super::*;
use analysis::{Extent, SectionAddress};

/// A symbol's listing with no instructions: all a bar needs of one is its pointer.
fn symbol() -> Searchable {
    Searchable::Symbol {
        assembly: Arc::new(Assembly {
            instructions: Vec::new(),
            edges: Vec::new(),
            undecodable: None,
            range: SectionAddress::ZERO..SectionAddress::ZERO,
            extent: Extent {
                bytes: 0,
                capped: false,
            },
        }),
        lanes: Lanes::none(),
    }
}

/// The hits a bar keeps after the pane has left their listing keep that listing alive, so
/// the next listing cannot be put at its address and take the hits for its own.
#[test]
fn a_bars_hits_keep_the_listing_they_are_about() {
    let filter = Filter {
        pattern: "mov".to_owned(),
        ..Filter::default()
    };
    let first = symbol();
    let Searchable::Symbol { assembly, .. } = &first else {
        unreachable!()
    };
    let freed = Arc::downgrade(assembly);
    let mut bar = Find {
        filter: filter.clone(),
        listing: Some(first.clone()),
        ..Find::default()
    };
    assert!(bar.take(About::of(&first, &filter), Shared::default()));
    drop(first);

    // The pane moves on; only the answer still names the first listing.
    bar.listing = None;
    assert!(freed.upgrade().is_some());

    bar.listing = Some(symbol());
    assert!(bar.hits().is_none());
    assert!(bar.pending().is_some());
}

/// A seed is text to find, in either mode: with Regex on, `[rip+0x2f]` finds itself and
/// not any one of its characters.
#[test]
fn a_seed_in_regex_mode_finds_the_text_it_is() {
    let mut bar = Find {
        filter: Filter {
            regex: true,
            ..Filter::default()
        },
        ..Find::default()
    };
    bar.seed("[rip+0x2f]");
    let matcher = bar.filter.matcher();
    assert_eq!(matcher.marks("lea rax, [rip+0x2f]"), vec![9..19]);
    assert!(!matcher.matches("mov rax, rcx"));
}
