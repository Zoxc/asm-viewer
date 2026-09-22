//! What the app is holding turned into a session, and a session turned back.

use std::collections::BTreeMap;

use analysis::{
    Architecture, Bias, BinaryFormat, MadeUp, ObjectData, Section, SectionAddress, SectionIndex,
    SymbolData, SymbolIndex,
};

/// A placed address and one of a section's own, written as the numbers a test means by
/// them: what these assertions are about is which place is held, not the spelling.
fn placed_at(address: u64) -> PlacedAddress {
    PlacedAddress::new(address)
}

fn at(address: u64) -> SectionAddress {
    SectionAddress::new(address)
}

use super::*;
use crate::docs::Docs;
use crate::project::files::tests::*;
use crate::project::files::SavedMadeUp;

/// A bare `Object` with the given text symbols — only the fields the mapping reads.
fn object(path: &str, name: &str, symbols: &[(&str, u64)]) -> Arc<Object> {
    built(path, name, symbols, b"the first build")
}

/// The same, out of a named build of the file. `bytes` is only ever hashed, so "the file
/// was rebuilt" is spelt as two calls with different bytes.
fn built(path: &str, name: &str, symbols: &[(&str, u64)], bytes: &[u8]) -> Arc<Object> {
    with_symbols(path, name, bytes, symbols.len(), |section| {
        symbols
            .iter()
            .map(|(name, address)| {
                SymbolData::new(
                    (*name).to_owned(),
                    None,
                    SectionAddress::new(*address),
                    Some(section.clone()),
                    None,
                )
            })
            .collect()
    })
}

/// An object holding the symbols `make` builds in its one text section, of `code` bytes.
fn with_symbols(
    path: &str,
    name: &str,
    bytes: &[u8],
    code: usize,
    make: impl FnOnce(&Arc<Section>) -> Vec<SymbolData>,
) -> Arc<Object> {
    let section = Arc::new(Section::text(
        SectionIndex(0),
        ".text".into(),
        vec![0xC3; code],
        SectionAddress::new(0),
        BTreeMap::new(),
        Bias::NONE,
    ));

    // In any order: the constructor sorts them by name, which is what `find_symbol` searches by.
    let symbols = make(&section)
        .into_iter()
        .enumerate()
        .map(|(index, symbol)| (SymbolIndex(index), Arc::new(symbol)))
        .collect();

    Arc::new(Object::new(
        PathBuf::from(path),
        name.to_owned(),
        BinaryFormat::Elf,
        Architecture::X86_64,
        symbols,
        vec![section],
        ObjectData::from(bytes),
    ))
}

fn objects() -> Vec<Arc<Object>> {
    vec![
        object("/tmp/lib.a", "a.o", &[("caller", 0), ("target", 6)]),
        // Same path, different member: `path` alone cannot tell these apart.
        object("/tmp/lib.a", "b.o", &[("caller", 0)]),
    ]
}

/// [`Session::from_state`] over a session whose only open tab is the active document and
/// whose panes are at the top of what they show.
fn from_state(objects: &[Arc<Object>], document: Option<&Document>, visits: &Visits) -> Session {
    let document = document.cloned();
    session_of(
        objects,
        document
            .as_ref()
            .map(std::slice::from_ref)
            .unwrap_or_default(),
        &[],
        &[],
        &[],
        &[],
        document.as_ref(),
        visits,
    )
}

/// The app's state as these tests spell it -- one tab per document in strip order, each
/// a trail of one, its rows keyed by the document alone -- turned into what
/// [`Session::from_state`] takes, each row keyed by its tab's entry.
#[allow(clippy::too_many_arguments)]
fn session_of(
    objects: &[Arc<Object>],
    tabs: &[Document],
    asm: &[(&Document, usize)],
    src: &[(&Document, usize)],
    places: &[(&Document, Spot)],
    driven_from: &[(&Document, u32)],
    active: Option<&Document>,
    visits: &Visits,
) -> Session {
    let mut docs = Docs::default();
    let ids: Vec<DocId> = tabs.iter().map(|tab| docs.open(tab.clone())).collect();
    let entry = |document: &Document| -> Entry {
        let index = tabs
            .iter()
            .position(|tab| tab == document)
            .expect("a row of an open tab");
        (ids[index], Stop::whole(document.clone()))
    };
    let (mut asm_rows, mut src_rows, mut spots, mut driven) = (
        Positions::default(),
        Positions::default(),
        Positions::default(),
        Driven::default(),
    );
    for (document, row) in asm {
        asm_rows.remember(entry(document), TopRow::at(*row));
    }
    for (document, row) in src {
        src_rows.remember(entry(document), TopRow::at(*row));
    }
    for (document, spot) in places {
        spots.remember(entry(document), *spot);
    }
    for (document, line) in driven_from {
        driven.remember(entry(document), *line);
    }
    let trails: Vec<SavingTab<'_>> = ids
        .iter()
        .map(|id| SavingTab::Document {
            id: *id,
            trail: docs.trail(*id).expect("open"),
            temporal: false,
        })
        .collect();
    Session::from_state(
        objects,
        &trails,
        &LeftAt {
            asm_rows: &asm_rows,
            src_rows: &src_rows,
            places: &spots,
            driven: &driven,
        },
        match active {
            Some(document) => OnScreen::Document(document),
            None => OnScreen::Nothing,
        },
        visits,
        Noticed {
            trusted: false,
            artifacts: &[],
            ui: SavedUi::default(),
        },
    )
}

/// The document arm of a restored tab: what every test but the pages' is about.
fn as_document(tab: &RestoredTab) -> (bool, &History, &[RestoredEntry]) {
    match tab {
        RestoredTab::Document {
            temporal,
            trail,
            entries,
        } => (*temporal, trail, entries),
        RestoredTab::Page(page) => panic!("a page tab where a document was wanted: {page:?}"),
    }
}

/// The restored active document, when it is a place in a binary.
fn resolve_place(session: &Session, objects: &[Arc<Object>]) -> Option<Document> {
    session
        .restore(objects)
        .active
        .filter(|document| matches!(document, Document::Object(_) | Document::Symbol(_)))
}

/// Object `a.o`, then its `target` symbol, then object `b.o`, visited in that order.
fn visits(objects: &[Arc<Object>]) -> Visits {
    let mut visits = Visits::default();
    for document in places(objects) {
        visits.record(document);
    }
    visits
}

/// The three places [`visits`] records, oldest first.
fn places(objects: &[Arc<Object>]) -> Vec<Document> {
    vec![
        Document::Object(objects[0].clone()),
        Document::Symbol(Symbol {
            object: objects[0].clone(),
            data: objects[0].symbols_sorted[1].clone(),
        }),
        Document::Object(objects[1].clone()),
    ]
}

/// The same three places along one tab's trail, with the cursor wherever `back` calls
/// leave it.
fn trail(objects: &[Arc<Object>], back: usize) -> History {
    let mut trail = History::default();
    for document in places(objects) {
        trail.push(document);
    }
    for _ in 0..back {
        trail.back();
    }
    trail
}

fn tab(object: &Arc<Object>) -> Document {
    Document::Object(object.clone())
}

fn file_tab(path: &str) -> Document {
    Document::Source(Arc::from(path))
}

/// A saved record built by hand: entries no live `Visits` could have produced.
fn saved_history(entries: &[SavedDocument]) -> Session {
    Session {
        active: None,
        history: SavedHistory {
            entries: entries.to_vec(),
        },
        ..Session::default()
    }
}

/// A saved page tab, which has no trail at all.
fn saved_page(page: Page) -> SavedTab {
    SavedTab {
        page: Some(page.stored().to_owned()),
        temporal: false,
        cursor: 0,
        entries: Vec::new(),
    }
}

fn saved_file_tab(path: &str, asm_row: usize, src_row: usize) -> SavedTab {
    saved_one(SavedEntry {
        asm_row,
        asm_into: 0,
        src_row,
        src_into: 0,
        line: None,
        asm_address: None,
        code_address: None,
        src_line: None,
        document: SavedDocument::Source {
            path: path.to_owned(),
        },
    })
}

/// A tab as [`Session::restore`] hands it back: one place on its trail, and nothing
/// driving it.
fn restored(document: &Document, asm_row: usize, src_row: usize) -> RestoredTab {
    let mut trail = History::default();
    trail.push(document.clone());
    RestoredTab::Document {
        temporal: false,
        trail,
        entries: vec![RestoredEntry {
            document: document.clone(),
            asm_row: TopRow::at(asm_row),
            src_row: TopRow::at(src_row),
            line: None,
            address: None,
            code_address: None,
            src_line: None,
        }],
    }
}

/// What a restore resolves a saved place against: the objects loaded now, with each path
/// in `changed` named by a digest no build of it ever had.
fn loaded<'a>(objects: &'a [Arc<Object>], changed: &[&str]) -> Loaded<'a> {
    let session = Session {
        digests: changed
            .iter()
            .map(|path| (PathBuf::from(path), digest_of(b"never built")))
            .collect(),
        ..Session::default()
    };
    Loaded::of(&session, objects)
}

/// A saved session naming one binary at `row`, with the digest of `bytes`.
fn saved_against(bytes: Option<&[u8]>, saved: SavedDocument, row: usize) -> Session {
    Session {
        digests: bytes
            .map(|bytes| BTreeMap::from([(PathBuf::from("/tmp/lib.a"), digest_of(bytes))]))
            .unwrap_or_default(),
        active: Some(saved.clone()),
        tabs: vec![saved_one(saved_entry(saved.clone(), row))],
        history: SavedHistory {
            entries: vec![saved],
        },
        ..Session::default()
    }
}

/// `find_symbol` searches `symbols_sorted` by name, and an object sorts it itself: symbols
/// given out of order are all still found.
#[test]
fn symbols_given_out_of_order_are_all_found() {
    let objects = vec![object("/tmp/lib.a", "a.o", &[("target", 6), ("caller", 0)])];
    assert!(saved_symbol("a.o", "caller", 0)
        .resolve_by_name(&objects)
        .is_some());
    assert!(saved_symbol("a.o", "target", 6)
        .resolve_by_name(&objects)
        .is_some());
}

/// The binaries and their counts come off the one walk: a file is listed once, in the
/// order the files were opened, carrying every object that came out of it.
#[test]
fn every_binary_is_counted_once_in_the_order_it_was_opened() {
    let objects = vec![
        object("/tmp/lib.a", "a.o", &[("caller", 0)]),
        object("/tmp/one.o", "one.o", &[("one", 0)]),
        // A second member of the archive above and not a third binary.
        object("/tmp/lib.a", "b.o", &[("caller", 0)]),
    ];

    assert_eq!(
        binary_counts(&objects),
        vec![
            (PathBuf::from("/tmp/lib.a"), 2),
            (PathBuf::from("/tmp/one.o"), 1)
        ]
    );
    // The same list without the counts, off the same walk.
    assert_eq!(
        binaries(&objects),
        vec![PathBuf::from("/tmp/lib.a"), PathBuf::from("/tmp/one.o")]
    );
}

#[test]
fn saves_and_resolves_a_symbol() {
    let objects = objects();
    let selection = Document::Symbol(Symbol {
        object: objects[1].clone(),
        data: objects[1].symbols_sorted[0].clone(),
    });

    let session = from_state(&objects, Some(&selection), &Visits::default());
    assert_eq!(binaries(&objects), vec![PathBuf::from("/tmp/lib.a")]);
    assert_eq!(
        session.active,
        Some(SavedDocument::Symbol {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "b.o".into(),
            symbol_name: SavedName::File("caller".into()),
            address: 0,
        })
    );

    // The duplicate `caller` in `a.o` must not win.
    assert!(resolve_place(&session, &objects) == Some(selection));
}

#[test]
fn saves_and_resolves_an_object() {
    let objects = objects();
    let selection = Document::Object(objects[0].clone());
    let session = from_state(&objects, Some(&selection), &Visits::default());
    assert!(resolve_place(&session, &objects) == Some(selection));
}

#[test]
fn no_selection_round_trips_as_none() {
    let objects = objects();
    let session = from_state(&objects, None, &Visits::default());
    assert_eq!(session.active, None);
    assert!(resolve_place(&session, &objects).is_none());
}

#[test]
fn a_missing_symbol_falls_back_to_its_object() {
    let objects = objects();
    let session = Session {
        active: Some(SavedDocument::Symbol {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "a.o".into(),
            symbol_name: SavedName::File("gone".into()),
            address: 12,
        }),
        history: SavedHistory::default(),
        ..Session::default()
    };
    assert!(resolve_place(&session, &objects) == Some(Document::Object(objects[0].clone())));
}

#[test]
fn a_missing_object_falls_back_to_nothing() {
    let objects = objects();
    for saved in [
        SavedDocument::Object {
            path: PathBuf::from("/tmp/other.a"),
            object_name: "a.o".into(),
            shown: SavedShown::Symbols,
        },
        // Right path, but that member is no longer in the archive.
        SavedDocument::Object {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "c.o".into(),
            shown: SavedShown::Symbols,
        },
        SavedDocument::Symbol {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "c.o".into(),
            symbol_name: SavedName::File("caller".into()),
            address: 0,
        },
    ] {
        let session = Session {
            active: Some(saved),
            history: SavedHistory::default(),
            ..Session::default()
        };
        assert!(resolve_place(&session, &objects).is_none());
    }
}

#[test]
fn a_multi_entry_history_round_trips_as_an_array_of_tables() {
    let objects = objects();
    let session = from_state(&objects, None, &visits(&objects));
    assert_eq!(session.history.entries.len(), 3);
    let text = round_trip(&session);
    assert!(text.contains("[[history.entries]]"), "{text}");
}

#[test]
fn saves_and_restores_the_visits() {
    let objects = objects();
    let visits = visits(&objects);

    let session = from_state(&objects, None, &visits);
    assert_eq!(session.history.entries.len(), 3);

    let restored = session.restore(&objects).visits;
    assert!(restored.entries() == visits.entries());
}

#[test]
fn history_entries_that_no_longer_resolve_are_dropped() {
    let objects = objects();
    let session = saved_history(&[
        saved_object("a.o"),
        // A member that is no longer in the archive.
        saved_object("c.o"),
        // The object is there but the symbol is gone. Unlike the selection, which
        // would degrade to the object, an entry is dropped.
        SavedDocument::Symbol {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "a.o".into(),
            symbol_name: SavedName::File("gone".into()),
            address: 12,
        },
        saved_object("b.o"),
    ]);

    let restored = session.restore(&objects).visits;
    assert!(restored.entries() == [tab(&objects[0]), tab(&objects[1]),]);
}

#[test]
fn a_saved_history_with_duplicates_restores_without_them() {
    let objects = objects();
    // The same destination visited twice, saved twice.
    let session = saved_history(&[
        saved_object("a.o"),
        saved_object("b.o"),
        saved_object("a.o"),
    ]);

    let restored = session.restore(&objects).visits;
    // Collapsed onto the newest occurrence, which is the first.
    assert!(restored.entries() == [tab(&objects[0]), tab(&objects[1]),]);
}

/// A tab's whole trail goes out and comes back: every place on it, the cursor wherever
/// the reader had walked it, and the rows each place was left at, keyed by the place --
/// so Back after a restart comes back to the rows that were left.
#[test]
fn a_tabs_trail_comes_back_with_its_cursor_and_its_rows() {
    let objects = objects();

    // Every position the cursor can be in, including one the reader walked back to.
    for back in 0..3 {
        let trail = trail(&objects, back);
        let current = trail.current().expect("a current entry").document.clone();
        let mut docs = Docs::default();
        let id = docs.open_trail(trail.clone(), false).expect("a trail");
        let (mut asm, mut src) = (Positions::default(), Positions::default());
        for (index, place) in places(&objects).iter().enumerate() {
            asm.remember((id, Stop::whole(place.clone())), TopRow::at(10 + index));
            src.remember((id, Stop::whole(place.clone())), TopRow::at(20 + index));
        }
        let session = Session::from_state(
            &objects,
            &[SavingTab::Document {
                id,
                trail: docs.trail(id).expect("open"),
                temporal: false,
            }],
            &LeftAt {
                asm_rows: &asm,
                src_rows: &src,
                places: &Positions::default(),
                driven: &Driven::default(),
            },
            OnScreen::Document(&current),
            &Visits::default(),
            Noticed {
                trusted: false,
                artifacts: &[],
                ui: SavedUi::default(),
            },
        );
        // Newest first, so a cursor `back` steps from the newest is at index `back`.
        assert_eq!(session.tabs[0].cursor, back);
        assert_eq!(session.tabs[0].entries.len(), 3);
        let session: Session = toml::from_str(&round_trip(&session)).expect("reading back");

        let restored = session.restore(&objects).tabs;
        assert_eq!(restored.len(), 1);
        assert!(*as_document(&restored[0]).1 == trail);
        assert!(
            as_document(&restored[0])
                .1
                .current()
                .map(|stop| &stop.document)
                == Some(&current)
        );
        for (index, entry) in as_document(&restored[0]).2.iter().enumerate() {
            let place = 2 - index;
            assert!(entry.document == places(&objects)[place]);
            assert_eq!(entry.asm_row, TopRow::at(10 + place));
            assert_eq!(entry.src_row, TopRow::at(20 + place));
        }
        // What the restore raises is the tab showing the restored active document.
        assert!(session.restore(&objects).active.as_ref() == Some(&current));
    }
}

/// Which of [`LeftAt`]'s four maps each field of a saved place comes out of. Three of the
/// four are `&Positions` of near-identical type and a row is a number in any of them, so
/// two swapped would type-check and read back as a place left somewhere else entirely.
/// The four values here are distinct, which is what makes a swap show.
#[test]
fn each_map_a_place_was_left_in_lands_in_its_own_saved_field() {
    let objects = objects();
    let document = places(&objects)[0].clone();
    let spot = Spot {
        address: placed_at(0x40),
        past: TopRow {
            row: 3,
            into: Fraction(0x8000),
        },
    };
    let session = session_of(
        &objects,
        std::slice::from_ref(&document),
        &[(&document, 7)],
        &[(&document, 11)],
        &[(&document, spot)],
        &[(&document, 23)],
        Some(&document),
        &Visits::default(),
    );

    let saved = &session.tabs[0].entries[0];
    // An object's code keeps its assembly side as a place, and that wins over a row: the
    // rows past the address and how far into the last.
    assert_eq!(
        (saved.asm_row, saved.asm_into),
        (3, 0x8000),
        "the rows past the address it was scrolled to"
    );
    assert_eq!(saved.src_row, 11, "the source side's row");
    assert_eq!(
        saved.asm_address,
        Some(0x40),
        "the address it was scrolled to"
    );
    assert_eq!(saved.line, Some(23), "the line it was driven from");
}

/// A place that no longer resolves is dropped from its trail, the cursor carried to the
/// nearest older survivor -- the walk closing a file goes through -- and its rows go with
/// it; a tab with nothing left on its trail is dropped whole. The temporal flag rides
/// along.
#[test]
fn a_trail_drops_the_places_that_no_longer_resolve_and_a_tab_left_with_none() {
    let objects = objects();
    let gone = SavedDocument::Symbol {
        path: PathBuf::from("/tmp/lib.a"),
        object_name: "a.o".into(),
        symbol_name: SavedName::File("gone".into()),
        address: 12,
    };
    let session = Session {
        tabs: vec![
            // On the gone symbol, between two survivors: lands on the older one.
            SavedTab {
                page: None,
                temporal: true,
                cursor: 1,
                entries: vec![
                    saved_entry(saved_object("a.o"), 3),
                    saved_entry(gone.clone(), 4),
                    saved_entry(saved_object("b.o"), 5),
                ],
            },
            // Nothing survives: no tab.
            SavedTab {
                page: None,
                temporal: false,
                cursor: 0,
                entries: vec![saved_entry(gone, 6), saved_entry(saved_object("c.o"), 7)],
            },
        ],
        ..Session::default()
    };

    let restored = session.restore(&objects).tabs;
    assert_eq!(restored.len(), 1);
    assert!(as_document(&restored[0]).0);
    let entries: Vec<Document> = as_document(&restored[0])
        .1
        .entries()
        .iter()
        .map(|stop| stop.document.clone())
        .collect();
    assert!(entries == [tab(&objects[0]), tab(&objects[1])]);
    assert!(
        as_document(&restored[0])
            .1
            .current()
            .map(|stop| &stop.document)
            == Some(&tab(&objects[1]))
    );
    assert!(as_document(&restored[0]).1.ahead().is_some());
    let rows: Vec<usize> = as_document(&restored[0])
        .2
        .iter()
        .map(|entry| entry.asm_row.row)
        .collect();
    assert_eq!(rows, [3, 5]);
}

/// A hand-written or trimmed file: the `serde(default)`s keep a missing table from taking
/// the active document down with it, and the restore is exactly what it would have been.
#[test]
fn a_partial_file_still_loads() {
    let text = r#"
            binaries = ["/tmp/lib.a"]

            [active.Object]
            path = "/tmp/lib.a"
            object_name = "a.o"
            shown = "Symbols"
        "#;
    let session: Session = toml::from_str(text).expect("deserializing");

    assert_eq!(session.history, SavedHistory::default());
    assert!(session.tabs.is_empty());
    assert!(session.digests.is_empty());

    let objects = objects();
    assert!(resolve_place(&session, &objects) == Some(Document::Object(objects[0].clone())));
    assert!(session.restore(&objects).visits.entries().is_empty());
    assert!(session.restore(&objects).tabs.is_empty());
}

/// One strip of both kinds goes out in the reader's own order and comes back in it.
#[test]
fn saves_and_resolves_the_open_tabs() {
    let objects = objects();
    let tabs = vec![
        tab(&objects[0]),
        file_tab("/src/main.rs"),
        Document::Symbol(Symbol {
            object: objects[0].clone(),
            data: objects[0].symbols_sorted[1].clone(),
        }),
        tab(&objects[1]),
    ];

    let session = session_of(
        &objects,
        &tabs,
        &[],
        &[],
        &[],
        &[],
        Some(&tabs[3]),
        &Visits::default(),
    );

    assert_eq!(
        session.tabs,
        [
            saved_tab("a.o", 0),
            saved_file_tab("/src/main.rs", 0, 0),
            saved_one(saved_entry(
                SavedDocument::Symbol {
                    path: PathBuf::from("/tmp/lib.a"),
                    object_name: "a.o".into(),
                    symbol_name: SavedName::File("target".into()),
                    address: 6,
                },
                0,
            )),
            saved_tab("b.o", 0),
        ]
    );
    assert!(
        session.restore(&objects).tabs
            == [
                restored(&tabs[0], 0, 0),
                restored(&tabs[1], 0, 0),
                restored(&tabs[2], 0, 0),
                restored(&tabs[3], 0, 0),
            ]
    );
}

/// A tab that no longer resolves is dropped where the active document would degrade — a
/// **source-driven tab never is**, resolving against nothing. The rows travel with their
/// tab: a parallel array would have handed `b.o` the rows of a tab dropped before it.
#[test]
fn open_tabs_that_no_longer_resolve_are_dropped() {
    let objects = objects();
    let session = Session {
        tabs: vec![
            saved_tab("a.o", 3),
            // A member that is no longer in the archive.
            saved_tab("c.o", 4),
            // The object is still there; the symbol is not. The active document
            // would fall back to `a.o` here, and a tab must not.
            saved_one(saved_entry(
                SavedDocument::Symbol {
                    path: PathBuf::from("/tmp/lib.a"),
                    object_name: "a.o".into(),
                    symbol_name: SavedName::File("gone".into()),
                    address: 12,
                },
                5,
            )),
            saved_file_tab("/no/such/file.rs", 0, 9),
            saved_tab("b.o", 6),
        ],
        ..Session::default()
    };

    assert!(
        session.restore(&objects).tabs
            == [
                restored(&tab(&objects[0]), 3, 0),
                restored(&file_tab("/no/such/file.rs"), 0, 9),
                restored(&tab(&objects[1]), 6, 0),
            ]
    );
}

/// How far into a row each side was left survives the file: the part of a row a pane's
/// row stops short of, and, for an object's code, the rows past the address with it.
#[test]
fn the_part_of_a_row_a_place_was_left_into_comes_back() {
    let objects = objects();
    let tabs = vec![tab(&objects[0]), tab(&objects[1])];
    let spot = Spot {
        address: placed_at(0x40),
        past: TopRow {
            row: 3,
            into: Fraction(0x1234),
        },
    };
    let mut session = session_of(
        &objects,
        &tabs,
        &[(&tabs[0], 12)],
        &[(&tabs[0], 4)],
        &[(&tabs[1], spot)],
        &[],
        Some(&tabs[0]),
        &Visits::default(),
    );
    // The two maps' parts, which `session_of` states in whole rows.
    let first = &mut session.tabs[0].entries[0];
    first.asm_into = 0x8000;
    first.src_into = 0x0010;
    let session: Session = toml::from_str(&round_trip(&session)).expect("reading back");

    let restored = session.restore(&objects).tabs;
    let entry = &as_document(&restored[0]).2[0];
    assert_eq!(
        (entry.asm_row, entry.src_row),
        (
            TopRow {
                row: 12,
                into: Fraction(0x8000)
            },
            TopRow {
                row: 4,
                into: Fraction(0x0010)
            }
        )
    );
    let entry = &as_document(&restored[1]).2[0];
    assert_eq!(entry.address, Some(placed_at(0x40)));
    assert_eq!(
        entry.asm_row,
        TopRow {
            row: 3,
            into: Fraction(0x1234)
        }
    );
}

/// The round trip the app makes: out of the two maps, through TOML, and back into them.
#[test]
fn the_rows_come_back_against_the_tabs_they_belong_to() {
    let objects = objects();
    let tabs = vec![tab(&objects[0]), tab(&objects[1])];

    let session = session_of(
        &objects,
        &tabs,
        &[(&tabs[0], 12), (&tabs[1], 900)],
        &[(&tabs[1], 4)],
        &[],
        &[],
        Some(&tabs[0]),
        &Visits::default(),
    );
    let session: Session = toml::from_str(&round_trip(&session)).expect("reading back");

    let (mut asm, mut src): (Positions<Document, TopRow>, Positions<Document, TopRow>) =
        (Positions::default(), Positions::default());
    for tab in session.restore(&objects).tabs {
        for entry in as_document(&tab).2 {
            asm.remember(entry.document.clone(), entry.asm_row);
            src.remember(entry.document.clone(), entry.src_row);
        }
    }
    assert_eq!(asm.at(&tabs[0]), Some(TopRow::at(12)));
    assert_eq!(asm.at(&tabs[1]), Some(TopRow::at(900)));
    assert_eq!(src.at(&tabs[1]), Some(TopRow::at(4)));
    // And a hint it is: a listing that has since shrunk clamps to what it holds now.
    assert_eq!(asm.row(&tabs[1], 100), TopRow::at(99));
}

/// A row is a hint and not a fact, so a saved tab that does not name one is a tab at
/// the top rather than a file that will not load.
#[test]
fn a_saved_tab_with_no_rows_opens_at_the_top() {
    let text = r#"
            binaries = ["/tmp/lib.a"]

            [[tabs]]
            [[tabs.entries]]
            [tabs.entries.document.Object]
            path = "/tmp/lib.a"
            object_name = "a.o"
            shown = "Symbols"

            [[tabs]]
            [[tabs.entries]]
            [tabs.entries.document.Source]
            path = "/src/main.rs"
        "#;
    let session: Session = toml::from_str(text).expect("deserializing");

    let objects = objects();
    assert!(
        session.restore(&objects).tabs
            == [
                restored(&tab(&objects[0]), 0, 0),
                restored(&file_tab("/src/main.rs"), 0, 0),
            ]
    );
}

/// Nothing about a source file is resolved against this filesystem: the pane's own
/// "Source file not found" is the right answer for one that has been deleted.
#[test]
fn a_source_file_that_is_no_longer_there_still_comes_back() {
    let path = "/no/such/directory/gone.rs";
    assert!(!Path::new(path).exists());

    let objects = objects();
    let tabs = [file_tab(path)];
    let session = session_of(
        &objects,
        &tabs,
        &[],
        &[],
        &[],
        &[],
        Some(&tabs[0]),
        &Visits::default(),
    );

    assert_eq!(session.tabs, [saved_file_tab(path, 0, 0)]);
    assert_eq!(
        session.active,
        Some(SavedDocument::Source { path: path.into() })
    );
    assert!(session.restore(&objects).active == Some(file_tab(path)));
}

/// The field-order trap, which only a real serialization catches: a saved tab's two rows
/// are plain values and have to reach the file before its `document` sub-table.
#[test]
fn a_full_session_round_trips_through_toml() {
    let objects = objects();
    let tabs = vec![
        tab(&objects[0]),
        Document::Symbol(Symbol {
            object: objects[0].clone(),
            data: objects[0].symbols_sorted[1].clone(),
        }),
        file_tab("/src/main.rs"),
    ];
    let session = session_of(
        &objects,
        &tabs,
        &[(&tabs[0], 12), (&tabs[1], 34)],
        &[(&tabs[0], 56)],
        &[],
        &[(&tabs[2], 42)],
        Some(&tabs[1]),
        &visits(&objects),
    );

    let text = round_trip(&session);
    assert!(text.contains("[[tabs]]"), "{text}");
    assert!(text.contains("[[tabs.entries]]"), "{text}");
    assert!(text.contains("[tabs.entries.document.Source]"), "{text}");

    // Inside a tab, the flag and the cursor before the array its entries are written
    // as; inside an entry, the rows before the table its document is written as.
    let temporal = text.find("temporal = ").expect("the first tab's flag");
    let cursor = text.find("cursor = ").expect("the first tab's cursor");
    let entries = text
        .find("[[tabs.entries]]")
        .expect("the first tab's entries");
    assert!(temporal < entries, "temporal after the entries\n{text}");
    assert!(cursor < entries, "cursor after the entries\n{text}");
    let asm_row = text.find("asm_row = 12").expect("the first tab's row");
    let src_row = text
        .find("src_row = 56")
        .expect("the first tab's source row");
    let document = text
        .find("[tabs.entries.document")
        .expect("the first tab's document");
    assert!(asm_row < document, "asm_row after its document\n{text}");
    assert!(src_row < document, "src_row after its document\n{text}");

    // The driven line is written for one of the three kinds of tab and for no other.
    assert!(text.contains("line = 42"), "{text}");
    assert_eq!(text.matches("line = ").count(), 1, "{text}");
}

/// The round trip the app makes with it: out of `Driven`, through TOML, and back into
/// one — which is what makes a source-driven tab's saved `asm_row` mean anything, the
/// listing that row is a row of not being there until the line is asked again.
#[test]
fn the_line_a_source_tab_was_driven_from_comes_back() {
    let objects = objects();
    let tabs = vec![file_tab("/src/main.rs"), tab(&objects[0])];

    let session = session_of(
        &objects,
        &tabs,
        &[(&tabs[0], 7)],
        &[],
        &[],
        &[(&tabs[0], 42)],
        Some(&tabs[0]),
        &Visits::default(),
    );
    let session: Session = toml::from_str(&round_trip(&session)).expect("reading back");

    let mut lines: Vec<(Document, Option<u32>)> = Vec::new();
    for tab in session.restore(&objects).tabs {
        for entry in as_document(&tab).2 {
            lines.push((entry.document.clone(), entry.line));
        }
    }
    // Only the tab that was driven. An assembly-driven tab is never one.
    assert!(lines == [(tabs[0].clone(), Some(42)), (tabs[1].clone(), None)]);
    assert_eq!(
        as_document(&session.restore(&objects).tabs[0]).2[0].asm_row,
        TopRow::at(7)
    );
}

/// The bar goes out in its own order, pages and documents alike, and comes back in it:
/// a page is a tab with a name and no trail, and it resolves against no object at all.
#[test]
fn the_bar_saves_its_pages_where_they_stand() {
    let objects = objects();
    let documents = [tab(&objects[0]), file_tab("/src/main.rs")];
    let mut docs = Docs::default();
    let ids: Vec<DocId> = documents
        .iter()
        .map(|document| docs.open(document.clone()))
        .collect();
    let saving = vec![
        SavingTab::Page(Page::Project),
        SavingTab::Document {
            id: ids[0],
            trail: docs.trail(ids[0]).expect("open"),
            temporal: false,
        },
        SavingTab::Page(Page::Settings),
        SavingTab::Document {
            id: ids[1],
            trail: docs.trail(ids[1]).expect("open"),
            temporal: false,
        },
    ];
    let session = Session::from_state(
        &objects,
        &saving,
        &LeftAt {
            asm_rows: &Positions::default(),
            src_rows: &Positions::default(),
            places: &Positions::default(),
            driven: &Driven::default(),
        },
        OnScreen::Page(Page::Settings),
        &Visits::default(),
        Noticed {
            trusted: false,
            artifacts: &[],
            ui: SavedUi::default(),
        },
    );

    // The page on screen is written instead of an active document, never beside one.
    assert_eq!(session.active_page.as_deref(), Some("settings"));
    assert!(session.active.is_none());
    assert_eq!(
        session.pages().collect::<Vec<(usize, Page)>>(),
        [(0, Page::Project), (2, Page::Settings)]
    );

    // Through the file and back, in the order the bar was in.
    let text = round_trip(&session);
    let page = text.find("page = ").expect("a page tab");
    let entries = text.find("[[tabs.entries]]").expect("a document tab");
    assert!(
        page < entries,
        "the page name after an entries table\n{text}"
    );
    let session: Session = toml::from_str(&text).expect("reading back");
    let restored = session.restore(&objects).tabs;
    let names: Vec<Option<Page>> = restored
        .iter()
        .map(|tab| match tab {
            RestoredTab::Page(page) => Some(*page),
            RestoredTab::Document { .. } => None,
        })
        .collect();
    assert_eq!(
        names,
        [Some(Page::Project), None, Some(Page::Settings), None]
    );
    assert_eq!(session.shown_page(), Some(Page::Settings));
}

/// A page this build does not have is dropped, as a place that no longer resolves is,
/// and every other tab comes back. It is a string in the file for exactly this reason: a
/// serde enum would fail the parse and cost the reader the whole session.
#[test]
fn a_page_this_build_does_not_have_is_dropped() {
    let objects = objects();
    let session = Session {
        active_page: Some("terminal".to_owned()),
        tabs: vec![
            SavedTab {
                page: Some("terminal".to_owned()),
                temporal: false,
                cursor: 0,
                entries: Vec::new(),
            },
            saved_page(Page::Settings),
            saved_tab("a.o", 3),
        ],
        ..Session::default()
    };

    let restored = session.restore(&objects).tabs;
    assert_eq!(restored.len(), 2);
    assert!(matches!(restored[0], RestoredTab::Page(Page::Settings)));
    assert_eq!(as_document(&restored[1]).2[0].asm_row, TopRow::at(3));
    assert_eq!(session.shown_page(), None);
}

/// One digest per *file*, however many objects came out of it.
#[test]
fn saves_one_digest_per_binary_however_many_objects_it_holds() {
    let objects = objects();
    let session = from_state(&objects, None, &Visits::default());

    assert_eq!(binaries(&objects), vec![PathBuf::from("/tmp/lib.a")]);
    assert_eq!(
        session.digests,
        BTreeMap::from([(PathBuf::from("/tmp/lib.a"), digest_of(b"the first build"))])
    );
}

/// One file puts many objects in the list -- an archive's members -- and can put the same
/// member name in it twice. The **first** answers for the file wherever the list is
/// asked: the object a saved place resolves to, and the digest written down for the
/// binary. Indexing the objects has to keep what scanning them found.
#[test]
fn the_first_object_out_of_a_file_is_the_one_that_answers_for_it() {
    let objects = vec![
        built("/tmp/lib.a", "a.o", &[("target", 6)], b"the first build"),
        // The same path and the same member name.
        built("/tmp/lib.a", "a.o", &[("target", 96)], b"the second build"),
    ];
    let saved = saved_symbol("a.o", "target", 6);

    for found in [
        saved.resolve(&loaded(&objects, &[])),
        saved.resolve_by_name(&objects),
    ] {
        let symbol = found
            .as_ref()
            .and_then(Document::symbol)
            .expect("the symbol");
        assert!(Arc::ptr_eq(&symbol.object, &objects[0]), "the first member");
    }

    assert_eq!(binaries(&objects), vec![PathBuf::from("/tmp/lib.a")]);
    assert_eq!(
        digests(&objects),
        BTreeMap::from([(PathBuf::from("/tmp/lib.a"), digest_of(b"the first build"))])
    );
}

/// The file is the one the session was saved against, so the saved address is a fact
/// about it: an exact match resolves, a symbol that is not where it was said to be does
/// not, and the row the tab was left at is still that tab's row.
#[test]
fn an_unchanged_binary_is_still_matched_on_the_address() {
    let objects = objects();

    let session = saved_against(
        Some(b"the first build"),
        saved_symbol("a.o", "target", 6),
        42,
    );
    assert!(
        resolve_place(&session, &objects)
            == Some(Document::Symbol(Symbol {
                object: objects[0].clone(),
                data: objects[0].symbols_sorted[1].clone(),
            }))
    );
    assert_eq!(session.restore(&objects).tabs.len(), 1);
    assert_eq!(
        as_document(&session.restore(&objects).tabs[0]).2[0].asm_row,
        TopRow::at(42)
    );

    // The same name at an address it is not at, which this file does not explain.
    let moved = saved_against(
        Some(b"the first build"),
        saved_symbol("a.o", "target", 999),
        42,
    );
    assert!(resolve_place(&moved, &objects) == Some(Document::Object(objects[0].clone())));
    assert!(moved.restore(&objects).tabs.is_empty());
    assert!(moved.restore(&objects).visits.entries().is_empty());
}

/// The file has been rebuilt under the session, so a symbol that merely moved comes back
/// by name — and the saved row goes, naming a listing this build no longer has.
#[test]
fn a_rebuilt_binary_matches_by_name_and_forgets_the_row() {
    let objects = vec![
        built(
            "/tmp/lib.a",
            "a.o",
            &[("caller", 0), ("target", 96)],
            b"the second build",
        ),
        built("/tmp/lib.a", "b.o", &[("caller", 0)], b"the second build"),
    ];

    // Saved when `target` was at 6; it is at 96 now.
    let session = saved_against(
        Some(b"the first build"),
        saved_symbol("a.o", "target", 6),
        42,
    );

    let expected = Document::Symbol(Symbol {
        object: objects[0].clone(),
        data: objects[0].symbols_sorted[1].clone(),
    });
    assert!(resolve_place(&session, &objects) == Some(expected.clone()));
    assert!(session.restore(&objects).tabs == [restored(&expected, 0, 0)]);
    assert!(session.restore(&objects).visits.entries() == [expected]);
}

/// The refusal rather than the recovery: two symbols of one name in a rebuilt object and
/// a saved address that is now neither of theirs, so nothing is chosen.
#[test]
fn a_rebuilt_binary_will_not_guess_between_two_symbols_of_one_name() {
    let objects = vec![built(
        "/tmp/lib.a",
        "a.o",
        &[("helper", 32), ("helper", 64)],
        b"the second build",
    )];

    let session = saved_against(
        Some(b"the first build"),
        saved_symbol("a.o", "helper", 6),
        42,
    );
    // The selection degrades to the object; the tab and the history entry drop.
    assert!(resolve_place(&session, &objects) == Some(Document::Object(objects[0].clone())));
    assert!(session.restore(&objects).tabs.is_empty());
    assert!(session.restore(&objects).visits.entries().is_empty());

    // And where the address still names one of them, it is still the tie-breaker.
    let exact = saved_against(
        Some(b"the first build"),
        saved_symbol("a.o", "helper", 64),
        42,
    );
    assert!(
        resolve_place(&exact, &objects)
            == Some(Document::Symbol(Symbol {
                object: objects[0].clone(),
                data: objects[0].symbols_sorted[1].clone(),
            }))
    );
}

/// A session that never wrote a digest says nothing about the bytes: "not known to be
/// unchanged" is not "known to have changed".
#[test]
fn a_binary_with_no_saved_digest_is_believed_exactly_as_before() {
    let objects = vec![built(
        "/tmp/lib.a",
        "a.o",
        &[("caller", 0), ("target", 96)],
        b"the second build",
    )];

    let session = saved_against(None, saved_symbol("a.o", "target", 6), 42);
    assert!(resolve_place(&session, &objects) == Some(Document::Object(objects[0].clone())));
    assert!(session.restore(&objects).tabs.is_empty());
}

/// A digest for a path that is not loaded, and a loaded path with no digest, are both
/// "nothing to compare" rather than a mismatch.
#[test]
fn a_digest_for_a_binary_that_is_not_open_says_nothing() {
    let objects = objects();
    let mut session = saved_against(
        Some(b"the first build"),
        saved_symbol("a.o", "target", 6),
        42,
    );
    session
        .digests
        .insert(PathBuf::from("/tmp/some.dll"), digest_of(b"whatever"));

    assert_eq!(
        as_document(&session.restore(&objects).tabs[0]).2[0].asm_row,
        TopRow::at(42)
    );
}

/// The digests are a TOML *table* holding hex strings, a `u64` digest not fitting TOML's
/// signed integers at all.
#[test]
fn the_digests_round_trip_through_toml() {
    let objects = objects();
    let tabs = vec![tab(&objects[0]), file_tab("/src/main.rs")];
    let session = session_of(
        &objects,
        &tabs,
        &[(&tabs[0], 12)],
        &[],
        &[],
        &[],
        Some(&tabs[0]),
        &visits(&objects),
    );

    let text = round_trip(&session);
    assert!(text.contains("[digests]"), "{text}");
    assert!(
        text.contains(&format!("{}\"", digest_of(b"the first build"))),
        "{text}"
    );
}

/// An object's code is saved the way the object is -- by the file's path and the object's
/// name -- and found again by them, as its own kind of document and not as the object.
#[test]
fn a_code_document_is_saved_by_its_object_and_found_again() {
    let objects = objects();
    let document = Document::Code(objects[1].clone());

    let saved = SavedDocument::from_document(&document);
    assert_eq!(
        saved,
        SavedDocument::Object {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "b.o".into(),
            shown: SavedShown::Code,
        }
    );
    let found = saved.resolve(&loaded(&objects, &[]));
    assert!(
        found == Some(document.clone()),
        "the code document comes back"
    );
    assert!(
        found != Some(Document::Object(objects[1].clone())),
        "the object's code is not the object"
    );

    // Gone with its object, and degrading to nothing rather than to another object.
    let rest: Vec<Arc<Object>> = objects[..1].to_vec();
    assert!(saved.resolve(&loaded(&rest, &[])).is_none());
    assert!(saved.resolve_or_degrade(&loaded(&rest, &[])).is_none());

    // And it survives the round trip through TOML in a tab.
    let tab = saved_one(saved_entry(saved, 3));
    let text = toml::to_string(&tab).expect("serialises");
    let back: SavedTab = toml::from_str(&text).expect("parses back");
    assert_eq!(back, tab);
}

/// The two ways an object is shown are one saved document telling them apart, so a
/// session holding both still opens both after a round trip through the file.
#[test]
fn an_objects_symbols_and_its_code_come_back_as_two_tabs() {
    let objects = objects();
    let tabs = [
        Document::Object(objects[0].clone()),
        Document::Code(objects[0].clone()),
    ];
    let session = session_of(
        &objects,
        &tabs,
        &[],
        &[],
        &[],
        &[],
        Some(&tabs[1]),
        &Visits::default(),
    );

    let text = round_trip(&session);
    assert!(text.contains(r#"shown = "Symbols""#), "{text}");
    assert!(text.contains(r#"shown = "Code""#), "{text}");

    let session: Session = toml::from_str(&text).expect("reading back");
    let restored = session.restore(&objects).tabs;
    let documents: Vec<Document> = restored
        .iter()
        .map(|tab| as_document(tab).2[0].document.clone())
        .collect();
    assert!(documents == tabs, "the two tabs came back as one kind");
    assert!(session.restore(&objects).active == Some(tabs[1].clone()));
}

/// A place in a source file is the line of it, written before its document as the address
/// is, and it comes back as the place the tab is at -- so a trail through two lines of one
/// file survives a restart with both.
#[test]
fn a_source_places_line_is_written_before_its_document_and_comes_back() {
    let objects = objects();
    let file = file_tab("/src/main.rs");
    let mut trail = History::default();
    trail.push(Stop::whole(file.clone()));
    trail.push(Stop::on("/src/main.rs".into(), 42));
    assert_eq!(
        trail.entries().len(),
        2,
        "two lines of one file are two places"
    );

    let mut docs = Docs::default();
    let id = docs.open_trail(trail.clone(), false).expect("a trail");
    let session = Session::from_state(
        &objects,
        &[SavingTab::Document {
            id,
            trail: docs.trail(id).expect("open"),
            temporal: false,
        }],
        &LeftAt {
            asm_rows: &Positions::default(),
            src_rows: &Positions::default(),
            places: &Positions::default(),
            driven: &Driven::default(),
        },
        OnScreen::Nothing,
        &Visits::default(),
        Noticed {
            trusted: false,
            artifacts: &[],
            ui: SavedUi::default(),
        },
    );
    assert_eq!(session.tabs[0].entries[0].src_line, Some(42));

    let text = toml::to_string(&session).expect("serialises");
    let line = text.find("src_line = 42").expect("the line is written");
    let document = text[line..].find("document").expect("the document follows") + line;
    assert!(line < document, "{text}");

    let back: Session = toml::from_str(&text).expect("parses back");
    let restored = back.restore(&objects).tabs;
    assert!(
        *as_document(&restored[0]).1 == trail,
        "the lines did not come back"
    );
}

/// The address a code tab was left at is written before its document, as the rows are:
/// TOML puts plain values before tables, and a value after one would be read as the
/// table's.
#[test]
fn a_code_tabs_address_is_written_before_its_document() {
    let objects = objects();
    let code = Document::Code(objects[1].clone());
    let spot = Spot {
        address: placed_at(0x30),
        past: TopRow::at(2),
    };

    let session = session_of(
        &objects,
        &[code.clone()],
        &[],
        &[],
        &[(&code, spot)],
        &[],
        Some(&code),
        &Visits::default(),
    );
    assert_eq!(session.tabs[0].entries[0].asm_address, Some(0x30));
    let text = toml::to_string(&session).expect("serialises");
    let address = text
        .find("asm_address = 48")
        .expect("the address is written");
    let document = text.find("[[tabs]]").expect("the tab is a table");
    let inner = text[document..]
        .find("document")
        .expect("the document follows")
        + document;
    assert!(document < address && address < inner, "{text}");
    let back: Session = toml::from_str(&text).expect("parses back");
    assert_eq!(back.tabs, session.tabs);

    // And it comes back as the scroll of the place it was under, the rows past it being
    // a nicety.
    let restored = session.restore(&objects).tabs;
    assert!(as_document(&restored[0]).2[0].document == code);
    assert_eq!(
        as_document(&restored[0]).2[0].address,
        Some(placed_at(0x30))
    );
}

/// A tab that followed a link inside an object's code has that listing on its trail
/// twice, at two addresses, and both come back: the trail is places and not documents, so
/// the two do not collapse into one on the way out or on the way back.
///
/// **A place and the scroll under it are two facts, and are saved as two.** The listing a
/// tab opened at no place in particular and then scrolled away from comes back as the
/// whole listing it was, not as the address it was scrolled to; a place at an address
/// comes back at that address however far from it the reader then scrolled. Each keeps
/// its own scroll beside it.
#[test]
fn a_trail_through_one_listing_comes_back_with_both_places() {
    let objects = objects();
    let code = Document::Code(objects[1].clone());
    let (first, second) = (
        Stop::whole(code.clone()),
        Stop::at(objects[1].clone(), placed_at(0x40)),
    );

    let mut docs = Docs::default();
    let id = docs.open(first.clone());
    docs.trail_mut(id).expect("open").push(second.clone());
    // Neither side is where it was scrolled to: the whole listing was scrolled to an
    // address, and the place at one was scrolled past it.
    let mut spots = Positions::default();
    spots.remember(
        (id, first.clone()),
        Spot {
            address: placed_at(0x10),
            past: TopRow::at(3),
        },
    );
    spots.remember((id, second.clone()), Spot::at(placed_at(0x50)));

    let session = Session::from_state(
        &objects,
        &[SavingTab::Document {
            id,
            trail: docs.trail(id).expect("open"),
            temporal: false,
        }],
        &LeftAt {
            asm_rows: &Positions::default(),
            src_rows: &Positions::default(),
            places: &spots,
            driven: &Driven::default(),
        },
        OnScreen::Document(&code),
        &Visits::default(),
        Noticed {
            trusted: false,
            artifacts: &[],
            ui: SavedUi::default(),
        },
    );
    assert_eq!(session.tabs[0].entries.len(), 2, "the places collapsed");
    // Newest first, and each states its place apart from its scroll.
    let saved = &session.tabs[0].entries;
    assert_eq!(saved[0].code_address, Some(0x40));
    assert_eq!(saved[0].asm_address, Some(0x50));
    assert_eq!(
        saved[1].code_address, None,
        "a scroll made a place of its own"
    );
    assert_eq!(saved[1].asm_address, Some(0x10));
    let session: Session = toml::from_str(&round_trip(&session)).expect("reading back");

    let restored = session.restore(&objects).tabs;
    assert!(
        as_document(&restored[0]).1.entries() == [second, first],
        "the trail came back as other places than it went out as"
    );
    // And the scroll each was left at, beside the place and not as it.
    let addresses: Vec<Option<u64>> = as_document(&restored[0])
        .2
        .iter()
        .map(|entry| entry.address.map(PlacedAddress::get))
        .collect();
    assert_eq!(addresses, [Some(0x50), Some(0x10)]);
}

/// A place is where it is *in the document it is in*, so a saved entry whose half does
/// not belong to its document is the document itself and not a place of its own.
///
/// A file states the halves apart -- an address, a line and a document, each its own
/// value -- and so can state a pairing that means nothing. `RestoredEntry::stop` is where
/// they are put back together and the last place they are ever seen apart: two source
/// entries carrying an address are one place, where two addresses in an object's code are
/// two.
#[test]
fn a_saved_place_whose_half_is_not_its_documents_is_the_whole_document() {
    let text = r#"
            [[tabs]]

            [[tabs.entries]]
            code_address = 16
            [tabs.entries.document.Source]
            path = "/src/main.rs"

            [[tabs.entries]]
            code_address = 32
            [tabs.entries.document.Source]
            path = "/src/main.rs"
        "#;
    let session: Session = toml::from_str(text).expect("deserializing");

    let restored = session.restore(&objects()).tabs;
    assert!(
        as_document(&restored[0]).1.entries() == [Stop::whole(file_tab("/src/main.rs"))],
        "an address in a source file made a place of its own"
    );
}

/// An address is a claim about a layout: a rebuilt binary takes it with the rows and
/// leaves the tab.
#[test]
fn a_rebuilt_binary_takes_the_saved_address_with_it() {
    let objects = objects();
    let code = Document::Code(objects[1].clone());
    let spot = Spot::at(placed_at(0x30));
    let session = session_of(
        &objects,
        &[code.clone()],
        &[],
        &[],
        &[(&code, spot)],
        &[],
        Some(&code),
        &Visits::default(),
    );

    // The same file, rebuilt: a different digest under the same path.
    let rebuilt = vec![
        built("/tmp/lib.a", "a.o", &[("caller", 0)], b"the second build"),
        built("/tmp/lib.a", "b.o", &[("caller", 0)], b"the second build"),
    ];
    let restored = session.restore(&rebuilt).tabs;
    assert_eq!(restored.len(), 1);
    assert!(as_document(&restored[0]).2[0].document == Document::Code(rebuilt[1].clone()));
    assert_eq!(as_document(&restored[0]).2[0].address, None);
}

/// Only a code tab has an address to save; a symbol's tab keeps its row.
#[test]
fn a_symbol_tab_saves_no_address() {
    let objects = objects();
    let symbol = Document::Symbol(Symbol {
        object: objects[0].clone(),
        data: objects[0].symbols_sorted[0].clone(),
    });
    let session = session_of(
        &objects,
        &[symbol.clone()],
        &[(&symbol, 4)],
        &[],
        &[],
        &[],
        Some(&symbol),
        &Visits::default(),
    );
    assert_eq!(session.tabs[0].entries[0].asm_row, 4);
    assert_eq!(session.tabs[0].entries[0].asm_address, None);
    assert!(!toml::to_string(&session).unwrap().contains("asm_address"));
}

/// The binary search finds the whole run of a name wherever it sits in the list, and a
/// name that is not there — before, between or after the ones that are — finds nothing.
#[test]
fn a_symbol_is_found_by_binary_search_over_the_name_sorted_list() {
    let object = object(
        "/tmp/lib.a",
        "a.o",
        // Deliberately out of order: `built` sorts them as the parser would.
        &[
            ("zeta", 5),
            ("alpha", 1),
            ("mid", 3),
            ("mid", 4),
            ("beta", 2),
        ],
    );
    let objects = [object.clone()];
    let unchanged = loaded(&objects, &[]);
    let rebuilt = loaded(&objects, &["/tmp/lib.a"]);
    let find = |name: &str, address: u64, against: &Loaded| {
        saved_symbol("a.o", name, address)
            .resolve(against)
            .and_then(|found| found.symbol().map(|symbol| symbol.data.clone()))
    };

    for (name, address) in [
        ("alpha", 1),
        ("beta", 2),
        ("mid", 3),
        ("mid", 4),
        ("zeta", 5),
    ] {
        let found = find(name, address, &unchanged).expect(name);
        assert_eq!(
            (found.name.as_str(), found.address),
            (name, SectionAddress::new(address))
        );
    }
    // Two `mid`s and a stale address: neither is picked, rebuilt or not.
    assert!(find("mid", 9, &unchanged).is_none());
    assert!(find("mid", 9, &rebuilt).is_none());
    // A lone name at a stale address is, under a rebuild only.
    assert!(find("beta", 9, &unchanged).is_none());
    assert_eq!(
        find("beta", 9, &rebuilt).map(|data| data.address),
        Some(at(2))
    );
    for name in ["aardvark", "gamma", "omega"] {
        assert!(find(name, 1, &rebuilt).is_none(), "{name}");
    }
}

/// A bookmark on a name the app made up outlives the spelling it was made under. What is
/// saved is which name it is and the symbol's address, and the name is rendered again from
/// those two, so it is whatever the app calls one today. The other bookmark here is what an
/// earlier spelling would have left in the file: it finds nothing, which is what a stored
/// string does the day the app stops spelling it that way.
#[test]
fn a_bookmark_on_a_made_up_name_outlives_its_spelling() {
    const ADDRESS: u64 = 0x10;
    let address = SectionAddress::new(ADDRESS);
    // Named as the parser names it: spelled however `MadeUp` spells it now.
    let objects = vec![with_symbols(
        "/tmp/lib.a",
        "a.o",
        b"the first build",
        1,
        |section| {
            let made_up = MadeUp::Function(address);
            vec![SymbolData::new_made_up(
                made_up,
                address,
                Some(section.clone()),
                None,
            )]
        },
    )];

    let structure = SavedDocument::Symbol {
        path: PathBuf::from("/tmp/lib.a"),
        object_name: "a.o".into(),
        address: ADDRESS,
        symbol_name: SavedName::MadeUp(SavedMadeUp::Function),
    };
    let found = structure.resolve_by_name(&objects).expect("the symbol");
    assert!(
        found
            == Document::Symbol(Symbol {
                object: objects[0].clone(),
                data: objects[0].symbols_sorted[0].clone(),
            })
    );
    // And saving it again writes the structure back, not the name it was found under.
    assert!(SavedDocument::from_document(&found) == structure);

    let spelling = SavedDocument::Symbol {
        path: PathBuf::from("/tmp/lib.a"),
        object_name: "a.o".into(),
        address: ADDRESS,
        symbol_name: SavedName::File("<fn 0x10>".into()),
    };
    assert!(
        spelling.resolve_by_name(&objects).is_none(),
        "a spelling in the file is what a change of spelling loses"
    );
}

/// A saved place read by name alone, which is what a bookmark is: the same symbol as the
/// strict rule finds in an unchanged file, the moved symbol in a rebuilt one, and nothing
/// where two symbols share the name and neither sits at the saved address.
#[test]
fn resolving_by_name_agrees_with_the_strict_rule_and_survives_a_rebuild() {
    let objects = objects();
    let saved = saved_symbol("a.o", "target", 6);
    let strict = saved.resolve(&loaded(&objects, &[]));
    assert!(strict.is_some());
    assert!(saved.resolve_by_name(&objects) == strict);

    // Rebuilt: `target` moved from 6 to 96 and is still found; nothing in any session
    // says the file changed, and nothing has to.
    let rebuilt = vec![built(
        "/tmp/lib.a",
        "a.o",
        &[("caller", 0), ("target", 96)],
        b"the second build",
    )];
    let expected = Document::Symbol(Symbol {
        object: rebuilt[0].clone(),
        data: rebuilt[0].symbols_sorted[1].clone(),
    });
    assert!(saved.resolve_by_name(&rebuilt) == Some(expected));

    // Two of one name, neither at the saved address: refused, as under a rebuild.
    let twins = vec![built(
        "/tmp/lib.a",
        "a.o",
        &[("target", 32), ("target", 64)],
        b"the second build",
    )];
    assert!(saved.resolve_by_name(&twins).is_none());
    assert!(saved_symbol("a.o", "target", 64)
        .resolve_by_name(&twins)
        .is_some());

    // A symbol whose object is not loaded at all is nothing, and a file is always itself.
    assert!(saved_symbol("c.o", "target", 6)
        .resolve_by_name(&objects)
        .is_none());
    let file = SavedDocument::Source {
        path: "/src/main.rs".into(),
    };
    assert!(file.resolve_by_name(&[]) == Some(Document::Source(Arc::from("/src/main.rs"))));
}

/// What the last build produced is the app's own record and belongs to the session, so
/// [`Session::from_state`] is where it is written and an empty list leaves it out.
#[test]
fn the_session_records_what_the_last_build_produced() {
    let objects = Vec::new();
    let built = paths(&["/src/kernel/target/debug/vmlinux"]);
    let session = Session::from_state(
        &objects,
        &[],
        &LeftAt {
            asm_rows: &Positions::default(),
            src_rows: &Positions::default(),
            places: &Positions::default(),
            driven: &Driven::default(),
        },
        OnScreen::Nothing,
        &Visits::default(),
        Noticed {
            trusted: false,
            artifacts: &built,
            ui: SavedUi::default(),
        },
    );
    assert_eq!(
        session.cargo.as_ref().map(|cargo| cargo.artifacts.clone()),
        Some(built)
    );
}
