use std::collections::{HashMap, HashSet};

use analysis::{Architecture, BinaryFormat, ObjectData, Section, SectionIndex, SymbolData};

use super::*;
use crate::bookmarks::Bookmark;
use crate::docs::Docs;

/// A bare `Object` with the given text symbols — only the fields the mapping reads.
fn object(path: &str, name: &str, symbols: &[(&str, u64)]) -> Arc<Object> {
    built(path, name, symbols, b"the first build")
}

/// The same, out of a named build of the file. `bytes` is only ever hashed, so "the file
/// was rebuilt" is spelt as two calls with different bytes.
fn built(path: &str, name: &str, symbols: &[(&str, u64)], bytes: &[u8]) -> Arc<Object> {
    let section = Arc::new(Section {
        index: SectionIndex(0),
        name: ".text".into(),
        data: vec![0xC3; symbols.len()],
        address: 0,
        relocations: HashMap::new(),
        symbols: symbols.iter().map(|(_, address)| *address).collect(),
        unwind: Vec::new(),
        code: true,
        bias: 0,
    });

    let mut symbols_sorted: Vec<Arc<SymbolData>> = symbols
        .iter()
        .map(|(name, address)| {
            Arc::new(SymbolData {
                name: (*name).to_owned(),
                demangled: None,
                address: *address,
                section: Some(section.clone()),
                size: 0,
            })
        })
        .collect();
    // The order the parser leaves them in, and what `find_symbol` searches by; a fixture
    // may give them in any order.
    symbols_sorted.sort_by(|a, b| a.name.cmp(&b.name));

    Arc::new(Object {
        path: PathBuf::from(path),
        name: name.to_owned(),
        format: BinaryFormat::Elf,
        architecture: Architecture::X86_64,
        symbols: HashMap::new(),
        symbols_sorted,
        sections: vec![section],
        data: ObjectData::from(bytes),
        debug_info: Default::default(),
        by_address: Default::default(),
    })
}

/// [`Session::from_state`] over a session whose only open tab is the active document and
/// whose panes are at the top of what they show.
fn from_state(objects: &[Arc<Object>], selection: Option<&Selection>, visits: &Visits) -> Session {
    let document = selection.cloned().map(Document::Assembly);
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
        (ids[index], document.clone())
    };
    let (mut asm_rows, mut src_rows, mut spots, mut driven) = (
        Positions::default(),
        Positions::default(),
        Positions::default(),
        Driven::default(),
    );
    for (document, row) in asm {
        asm_rows.remember(entry(document), *row);
    }
    for (document, row) in src {
        src_rows.remember(entry(document), *row);
    }
    for (document, spot) in places {
        spots.remember(entry(document), *spot);
    }
    for (document, line) in driven_from {
        driven.remember(entry(document), *line);
    }
    let trails: Vec<(DocId, &History, bool)> = ids
        .iter()
        .map(|id| (*id, docs.trail(*id).expect("open"), false))
        .collect();
    Session::from_state(
        objects,
        &trails,
        &asm_rows,
        &src_rows,
        &spots,
        &driven,
        active,
        visits,
        &[],
    )
}

/// What [`Session::resolve`] answers, as the selection inside it.
fn resolve_selection(session: &Session, objects: &[Arc<Object>]) -> Option<Selection> {
    session
        .resolve(objects)
        .and_then(|document| match document {
            Document::Assembly(selection) => Some(selection),
            Document::Source(_) | Document::Code(_) => None,
        })
}

/// What `load_project` does with `session.toml`: a missing or corrupt file is `None`.
fn load_session(path: &Path) -> Option<Session> {
    let data = fs::read_to_string(path).ok()?;
    toml::from_str(&data).ok()
}

fn objects() -> Vec<Arc<Object>> {
    vec![
        object("/tmp/lib.a", "a.o", &[("caller", 0), ("target", 6)]),
        // Same path, different member: `path` alone cannot tell these apart.
        object("/tmp/lib.a", "b.o", &[("caller", 0)]),
    ]
}

/// A member is not a file, so both members of `/tmp/lib.a` answer for it and a symbol
/// answers for the file its object came out of.
#[test]
fn everything_in_a_file_says_so() {
    let objects = objects();
    let lib = Path::new("/tmp/lib.a");
    let other = Path::new("/tmp/some.dll");

    let member = Selection::Object(objects[1].clone());
    assert!(member.in_file(lib));
    assert!(!member.in_file(other));

    let symbol = Selection::Symbol(Symbol {
        object: objects[0].clone(),
        data: objects[0].symbols_sorted[0].clone(),
    });
    assert!(symbol.in_file(lib));
    assert!(!symbol.in_file(other));
}

#[test]
fn saves_and_resolves_a_symbol() {
    let objects = objects();
    let selection = Selection::Symbol(Symbol {
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
            symbol_name: "caller".into(),
            address: 0,
        })
    );

    // The duplicate `caller` in `a.o` must not win.
    assert!(resolve_selection(&session, &objects) == Some(selection));
}

#[test]
fn saves_and_resolves_an_object() {
    let objects = objects();
    let selection = Selection::Object(objects[0].clone());
    let session = from_state(&objects, Some(&selection), &Visits::default());
    assert!(resolve_selection(&session, &objects) == Some(selection));
}

#[test]
fn no_selection_round_trips_as_none() {
    let objects = objects();
    let session = from_state(&objects, None, &Visits::default());
    assert_eq!(session.active, None);
    assert!(resolve_selection(&session, &objects).is_none());
}

#[test]
fn a_missing_symbol_falls_back_to_its_object() {
    let objects = objects();
    let session = Session {
        active: Some(SavedDocument::Symbol {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "a.o".into(),
            symbol_name: "gone".into(),
            address: 12,
        }),
        history: SavedHistory::default(),
        ..Session::new()
    };
    assert!(resolve_selection(&session, &objects) == Some(Selection::Object(objects[0].clone())));
}

#[test]
fn a_missing_object_falls_back_to_nothing() {
    let objects = objects();
    for saved in [
        SavedDocument::Object {
            path: PathBuf::from("/tmp/other.a"),
            object_name: "a.o".into(),
        },
        // Right path, but that member is no longer in the archive.
        SavedDocument::Object {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "c.o".into(),
        },
        SavedDocument::Symbol {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "c.o".into(),
            symbol_name: "caller".into(),
            address: 0,
        },
    ] {
        let session = Session {
            active: Some(saved),
            history: SavedHistory::default(),
            ..Session::new()
        };
        assert!(resolve_selection(&session, &objects).is_none());
    }
}

/// Serialize to TOML and read it straight back, which is the only way to catch the `toml`
/// crate's runtime failures: a bare `None`, and a value emitted after a table.
fn round_trip<T>(value: &T) -> String
where
    T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let text = toml::to_string_pretty(value).expect("serializing");
    let back: T = toml::from_str(&text).unwrap_or_else(|error| {
        panic!("deserializing\n--- {text}--- failed: {error}");
    });
    assert_eq!(*value, back);
    text
}

#[test]
fn toml_round_trips() {
    let session = Session {
        active: Some(SavedDocument::Symbol {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "b.o".into(),
            symbol_name: "caller".into(),
            address: 0x1234,
        }),
        history: SavedHistory::default(),
        ..Session::new()
    };
    let text = round_trip(&session);
    // The externally tagged enum is a table named after its variant.
    assert!(text.contains("[active.Symbol]"), "{text}");
}

#[test]
fn an_empty_session_round_trips() {
    // The `None` the `toml` crate cannot write has to be left out of the file entirely,
    // and read back as `None`.
    let session = Session::new();
    let text = round_trip(&session);
    assert!(!text.contains("active"), "{text}");
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
fn writes_atomically_and_reads_back() {
    let directory = std::env::temp_dir().join(format!(
        "assembly-viewer-test-{}-{}",
        std::process::id(),
        line!()
    ));
    let path = directory.join("nested").join(SESSION_FILE);

    let session = Session {
        active: Some(SavedDocument::Object {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "a.o".into(),
        }),
        history: SavedHistory::default(),
        ..Session::new()
    };
    session.save_to(&path).expect("saving");

    assert_eq!(load_session(&path), Some(session));
    // The temporary was renamed, not left behind.
    assert!(!path.with_extension("toml.tmp").exists());

    let _ = fs::remove_dir_all(&directory);
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
        Document::Assembly(Selection::Object(objects[0].clone())),
        Document::Assembly(Selection::Symbol(Symbol {
            object: objects[0].clone(),
            data: objects[0].symbols_sorted[1].clone(),
        })),
        Document::Assembly(Selection::Object(objects[1].clone())),
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

#[test]
fn saves_and_restores_the_visits() {
    let objects = objects();
    let visits = visits(&objects);

    let session = from_state(&objects, None, &visits);
    assert_eq!(session.history.entries.len(), 3);

    let restored = session.resolve_history(&objects);
    assert!(restored.entries() == visits.entries());
}

/// A saved record built by hand: entries no live `Visits` could have produced.
fn saved_history(entries: &[SavedDocument]) -> Session {
    Session {
        active: None,
        history: SavedHistory {
            entries: entries.to_vec(),
        },
        ..Session::new()
    }
}

fn saved_object(name: &str) -> SavedDocument {
    SavedDocument::Object {
        path: PathBuf::from("/tmp/lib.a"),
        object_name: name.to_owned(),
    }
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
            symbol_name: "gone".into(),
            address: 12,
        },
        saved_object("b.o"),
    ]);

    let restored = session.resolve_history(&objects);
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

    let restored = session.resolve_history(&objects);
    // Collapsed onto the newest occurrence.
    assert!(restored.entries() == [tab(&objects[1]), tab(&objects[0]),]);
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
        let current = trail.current().expect("a current entry").clone();
        let mut docs = Docs::default();
        let id = docs.open_trail(trail.clone(), false).expect("a trail");
        let (mut asm, mut src) = (Positions::default(), Positions::default());
        for (index, place) in places(&objects).iter().enumerate() {
            asm.remember((id, place.clone()), 10 + index);
            src.remember((id, place.clone()), 20 + index);
        }
        let session = Session::from_state(
            &objects,
            &[(id, docs.trail(id).expect("open"), false)],
            &asm,
            &src,
            &Positions::default(),
            &Driven::default(),
            Some(&current),
            &Visits::default(),
            &[],
        );
        assert_eq!(session.tabs[0].cursor, 2 - back);
        assert_eq!(session.tabs[0].entries.len(), 3);
        let session: Session = toml::from_str(&round_trip(&session)).expect("reading back");

        let restored = session.resolve_tabs(&objects);
        assert_eq!(restored.len(), 1);
        assert!(restored[0].trail == trail);
        assert!(restored[0].trail.current() == Some(&current));
        for (index, entry) in restored[0].entries.iter().enumerate() {
            assert!(entry.document == places(&objects)[index]);
            assert_eq!(entry.asm_row, 10 + index);
            assert_eq!(entry.src_row, 20 + index);
        }
        // What the restore raises is the tab showing the restored active document.
        assert!(session.resolve(&objects).as_ref() == Some(&current));
    }
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
        symbol_name: "gone".into(),
        address: 12,
    };
    let session = Session {
        tabs: vec![
            // On the gone symbol, between two survivors: lands on the older one.
            SavedTab {
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
                temporal: false,
                cursor: 0,
                entries: vec![saved_entry(gone, 6), saved_entry(saved_object("c.o"), 7)],
            },
        ],
        ..Session::new()
    };

    let restored = session.resolve_tabs(&objects);
    assert_eq!(restored.len(), 1);
    assert!(restored[0].temporal);
    assert!(restored[0].trail.entries() == [tab(&objects[0]), tab(&objects[1])]);
    assert!(restored[0].trail.current() == Some(&tab(&objects[0])));
    assert!(restored[0].trail.can_forward());
    let rows: Vec<usize> = restored[0]
        .entries
        .iter()
        .map(|entry| entry.asm_row)
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
        "#;
    let session: Session = toml::from_str(text).expect("deserializing");

    assert_eq!(session.history, SavedHistory::default());
    assert!(session.tabs.is_empty());
    assert!(session.digests.is_empty());

    let objects = objects();
    assert!(resolve_selection(&session, &objects) == Some(Selection::Object(objects[0].clone())));
    assert!(session.resolve_history(&objects).entries().is_empty());
    assert!(session.resolve_tabs(&objects).is_empty());
}

/// A path TOML cannot spell is refused rather than mangled, in *both* files.
#[test]
fn a_non_utf8_path_is_not_written_rather_than_mangled() {
    // Only Unix has a `PathBuf` that can hold one at all.
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        let path = PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/\xff\xfe.a"));
        let project = Project {
            binaries: vec![path.clone()],
            ..Project::default()
        };
        let session = Session {
            digests: BTreeMap::from([(path, digest_of(b"whatever"))]),
            ..Session::new()
        };
        // An error, not a panic and not a lossy path silently written in its place.
        assert!(toml::to_string_pretty(&project).is_err());
        assert!(toml::to_string_pretty(&session).is_err());

        let directory = std::env::temp_dir().join(format!(
            "assembly-viewer-test-{}-{}",
            std::process::id(),
            line!()
        ));
        assert!(write_toml(&directory.join(PROJECT_FILE), &project).is_err());
        assert!(session.save_to(&directory.join(SESSION_FILE)).is_err());
        // Nothing reached the disk, so a good earlier file would still be there.
        assert!(!directory.join(PROJECT_FILE).exists());
        assert!(!directory.join(SESSION_FILE).exists());

        let _ = fs::remove_dir_all(&directory);
    }
}

fn tab(object: &Arc<Object>) -> Document {
    Document::Assembly(Selection::Object(object.clone()))
}

fn file_tab(path: &str) -> Document {
    Document::Source(Arc::from(path))
}

/// A saved place with its assembly row and nothing else.
fn saved_entry(document: SavedDocument, asm_row: usize) -> SavedEntry {
    SavedEntry {
        asm_row,
        src_row: 0,
        line: None,
        asm_address: None,
        document,
    }
}

/// A saved tab that stays, with `entry` alone on its trail.
fn saved_one(entry: SavedEntry) -> SavedTab {
    SavedTab {
        temporal: false,
        cursor: 0,
        entries: vec![entry],
    }
}

fn saved_tab(object_name: &str, asm_row: usize) -> SavedTab {
    saved_one(saved_entry(saved_object(object_name), asm_row))
}

fn saved_file_tab(path: &str, asm_row: usize, src_row: usize) -> SavedTab {
    saved_one(SavedEntry {
        asm_row,
        src_row,
        line: None,
        asm_address: None,
        document: SavedDocument::Source {
            path: path.to_owned(),
        },
    })
}

/// A tab as [`Session::resolve_tabs`] hands it back: one place on its trail, and nothing
/// driving it.
fn restored(document: &Document, asm_row: usize, src_row: usize) -> RestoredTab {
    let mut trail = History::default();
    trail.push(document.clone());
    RestoredTab {
        temporal: false,
        trail,
        entries: vec![RestoredEntry {
            document: document.clone(),
            asm_row,
            src_row,
            line: None,
            address: None,
        }],
    }
}

/// One strip of both kinds goes out in the reader's own order and comes back in it.
#[test]
fn saves_and_resolves_the_open_tabs() {
    let objects = objects();
    let tabs = vec![
        tab(&objects[0]),
        file_tab("/src/main.rs"),
        Document::Assembly(Selection::Symbol(Symbol {
            object: objects[0].clone(),
            data: objects[0].symbols_sorted[1].clone(),
        })),
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
                    symbol_name: "target".into(),
                    address: 6,
                },
                0,
            )),
            saved_tab("b.o", 0),
        ]
    );
    assert!(
        session.resolve_tabs(&objects)
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
                    symbol_name: "gone".into(),
                    address: 12,
                },
                5,
            )),
            saved_file_tab("/no/such/file.rs", 0, 9),
            saved_tab("b.o", 6),
        ],
        ..Session::new()
    };

    assert!(
        session.resolve_tabs(&objects)
            == [
                restored(&tab(&objects[0]), 3, 0),
                restored(&file_tab("/no/such/file.rs"), 0, 9),
                restored(&tab(&objects[1]), 6, 0),
            ]
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

    let (mut asm, mut src): (Positions<Document>, Positions<Document>) =
        (Positions::default(), Positions::default());
    for tab in session.resolve_tabs(&objects) {
        for entry in tab.entries {
            asm.remember(entry.document.clone(), entry.asm_row);
            src.remember(entry.document, entry.src_row);
        }
    }
    assert_eq!(asm.at(&tabs[0]), Some(12));
    assert_eq!(asm.at(&tabs[1]), Some(900));
    assert_eq!(src.at(&tabs[1]), Some(4));
    // And a hint it is: a listing that has since shrunk clamps to what it holds now.
    assert_eq!(asm.row(&tabs[1], 100), 99);
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

            [[tabs]]
            [[tabs.entries]]
            [tabs.entries.document.Source]
            path = "/src/main.rs"
        "#;
    let session: Session = toml::from_str(text).expect("deserializing");

    let objects = objects();
    assert!(
        session.resolve_tabs(&objects)
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
    assert!(session.resolve(&objects) == Some(file_tab(path)));
}

/// The field-order trap, which only a real serialization catches: a saved tab's two rows
/// are plain values and have to reach the file before its `document` sub-table.
#[test]
fn a_full_session_round_trips_through_toml() {
    let objects = objects();
    let tabs = vec![
        tab(&objects[0]),
        Document::Assembly(Selection::Symbol(Symbol {
            object: objects[0].clone(),
            data: objects[0].symbols_sorted[1].clone(),
        })),
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
    for tab in session.resolve_tabs(&objects) {
        for entry in tab.entries {
            lines.push((entry.document, entry.line));
        }
    }
    // Only the tab that was driven. An assembly-driven tab is never one.
    assert!(lines == [(tabs[0].clone(), Some(42)), (tabs[1].clone(), None)]);
    assert_eq!(session.resolve_tabs(&objects)[0].entries[0].asm_row, 7);
}

/// The digest of the objects the fixtures are built from, as it is written down.
fn digest_of(bytes: &[u8]) -> String {
    analysis::FileDigest::of(bytes).to_string()
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
        ..Session::new()
    }
}

fn saved_symbol(object_name: &str, symbol_name: &str, address: u64) -> SavedDocument {
    SavedDocument::Symbol {
        path: PathBuf::from("/tmp/lib.a"),
        object_name: object_name.to_owned(),
        symbol_name: symbol_name.to_owned(),
        address,
    }
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
        resolve_selection(&session, &objects)
            == Some(Selection::Symbol(Symbol {
                object: objects[0].clone(),
                data: objects[0].symbols_sorted[1].clone(),
            }))
    );
    assert_eq!(session.resolve_tabs(&objects).len(), 1);
    assert_eq!(session.resolve_tabs(&objects)[0].entries[0].asm_row, 42);

    // The same name at an address it is not at, which this file does not explain.
    let moved = saved_against(
        Some(b"the first build"),
        saved_symbol("a.o", "target", 999),
        42,
    );
    assert!(resolve_selection(&moved, &objects) == Some(Selection::Object(objects[0].clone())));
    assert!(moved.resolve_tabs(&objects).is_empty());
    assert!(moved.resolve_history(&objects).entries().is_empty());
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

    let expected = Selection::Symbol(Symbol {
        object: objects[0].clone(),
        data: objects[0].symbols_sorted[1].clone(),
    });
    assert!(resolve_selection(&session, &objects) == Some(expected.clone()));
    let document = Document::Assembly(expected.clone());
    assert!(session.resolve_tabs(&objects) == [restored(&document, 0, 0)]);
    assert!(session.resolve_history(&objects).entries() == [document]);
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
    assert!(resolve_selection(&session, &objects) == Some(Selection::Object(objects[0].clone())));
    assert!(session.resolve_tabs(&objects).is_empty());
    assert!(session.resolve_history(&objects).entries().is_empty());

    // And where the address still names one of them, it is still the tie-breaker.
    let exact = saved_against(
        Some(b"the first build"),
        saved_symbol("a.o", "helper", 64),
        42,
    );
    assert!(
        resolve_selection(&exact, &objects)
            == Some(Selection::Symbol(Symbol {
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
    assert!(resolve_selection(&session, &objects) == Some(Selection::Object(objects[0].clone())));
    assert!(session.resolve_tabs(&objects).is_empty());
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

    assert_eq!(session.resolve_tabs(&objects)[0].entries[0].asm_row, 42);
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

fn paths(binaries: &[&str]) -> Vec<PathBuf> {
    binaries.iter().map(PathBuf::from).collect()
}

fn session_with(selection: Option<&str>) -> Session {
    Session {
        active: selection.map(saved_object),
        history: SavedHistory::default(),
        ..Session::new()
    }
}

/// `record` with the details the project already has, so every test using this is asking
/// about a change to the binaries or the session and nothing else. The rename tests spell
/// theirs out.
fn recorded(
    saves: &mut Saves,
    binaries: Vec<PathBuf>,
    session: Session,
) -> Option<(Project, Option<Session>)> {
    let unchanged = saves.given.clone();
    let bookmarks = saves.bookmarks.clone();
    saves.record(unchanged, binaries, bookmarks, session)
}

fn written(
    saves: &mut Saves,
    binaries: &[&str],
    selection: Option<&str>,
) -> Option<(Project, Option<Session>)> {
    recorded(saves, paths(binaries), session_with(selection))
}

#[test]
fn the_state_the_app_boots_into_is_never_written() {
    let mut saves = Saves::new();
    // The save observer's first run, before anything is restored. Nothing may come of
    // it: the files on disk are the good ones, and no project directory is allocated.
    assert_eq!(recorded(&mut saves, Vec::new(), Session::new()), None);
    assert_eq!(saves.flush(), None);
}

#[test]
fn opening_a_binary_is_written_at_once() {
    let mut saves = Saves::new();

    let written = written(&mut saves, &["/tmp/lib.a"], None);
    assert_eq!(
        written,
        Some((
            Project {
                name: None,
                directory: None,
                binaries: paths(&["/tmp/lib.a"]),
                cargo: None,
                bookmarks: Vec::new(),
            },
            Some(session_with(None)),
        ))
    );
    // And is not written a second time by the next flush.
    assert_eq!(saves.flush(), None);
}

/// Closing one takes the same path opening one does: `binaries` is what `record` looks
/// at, and it does not care in which direction the list changed.
#[test]
fn closing_a_binary_is_written_at_once() {
    let mut saves = Saves::new();
    written(&mut saves, &["/tmp/lib.a", "/tmp/some.dll"], Some("a.o"));

    // The selection is still pending from the open above; closing writes the lot, so
    // `session.toml` never names a place inside a binary `project.toml` has let go of.
    let written = written(&mut saves, &["/tmp/lib.a"], Some("a.o"));
    assert_eq!(
        written.as_ref().map(|(project, _)| &project.binaries),
        Some(&paths(&["/tmp/lib.a"]))
    );
    assert_eq!(
        written.and_then(|(_, session)| session),
        Some(session_with(Some("a.o")))
    );
    assert_eq!(saves.flush(), None);
}

/// The empty project is a project, and it has to reach the disk or the next run reopens
/// what was just closed.
#[test]
fn closing_the_only_binary_is_written_too() {
    let mut saves = Saves::new();
    written(&mut saves, &["/tmp/lib.a"], Some("a.o"));

    let written = recorded(&mut saves, Vec::new(), Session::new());
    assert_eq!(written, Some((Project::default(), Some(Session::new()))));
    assert_eq!(saves.flush(), None);
}

#[test]
fn a_selection_change_waits_for_the_flush() {
    let mut saves = Saves::new();
    written(&mut saves, &["/tmp/lib.a"], None);

    assert_eq!(written(&mut saves, &["/tmp/lib.a"], Some("a.o")), None);
    assert_eq!(saves.flush(), Some(session_with(Some("a.o"))));
    assert_eq!(saves.flush(), None);
}

#[test]
fn recording_the_same_project_again_changes_nothing() {
    let mut saves = Saves::new();
    written(&mut saves, &["/tmp/lib.a"], None);

    // A pending change re-recorded unchanged, as the save observer does whenever
    // something it does not persist wakes it.
    written(&mut saves, &["/tmp/lib.a"], Some("a.o"));
    assert_eq!(written(&mut saves, &["/tmp/lib.a"], Some("a.o")), None);
    // Still pending, and still exactly one write.
    assert_eq!(saves.flush(), Some(session_with(Some("a.o"))));
    assert_eq!(saves.flush(), None);

    // And once written, re-recording it is not a second write either.
    assert_eq!(written(&mut saves, &["/tmp/lib.a"], Some("a.o")), None);
    assert_eq!(saves.flush(), None);
}

#[test]
fn opening_a_binary_carries_the_pending_change_with_it() {
    let mut saves = Saves::new();
    written(&mut saves, &["/tmp/lib.a"], Some("a.o"));

    // The selection is pending; opening a second binary writes the lot.
    let written = written(&mut saves, &["/tmp/lib.a", "/tmp/some.dll"], Some("a.o"));
    assert_eq!(
        written.and_then(|(_, session)| session),
        Some(session_with(Some("a.o")))
    );
    assert_eq!(saves.flush(), None);
}

/// A tab is pending and not an immediate write, and nothing in `record` says so: which
/// file a field lives in is what decides it, and a tab lives in the session.
#[test]
fn opening_a_tab_waits_for_the_flush() {
    let mut saves = Saves::new();
    written(&mut saves, &["/tmp/lib.a"], None);

    let mut session = session_with(Some("a.o"));
    session.tabs = vec![saved_tab("a.o", 0)];
    assert_eq!(
        recorded(&mut saves, paths(&["/tmp/lib.a"]), session.clone()),
        None
    );
    assert_eq!(saves.flush(), Some(session));
    assert_eq!(saves.flush(), None);
}

/// The name and the directory survive a record that is not about them, the write carrying
/// them rather than the absence a derived project would have.
#[test]
fn a_record_keeps_the_name_the_project_was_given() {
    let mut saves = Saves::new();
    let named = Project {
        name: Some("kernel".into()),
        directory: Some(PathBuf::from("/src/kernel")),
        binaries: paths(&["/tmp/vmlinux"]),
        cargo: None,
        bookmarks: Vec::new(),
    };
    saves.opened(ProjectId::new("kernel-1").expect("an id"), &named);

    let (project, _) = written(&mut saves, &["/tmp/lib.a"], None).expect("a write");
    assert_eq!(project.name.as_deref(), Some("kernel"));
    assert_eq!(project.directory, Some(PathBuf::from("/src/kernel")));
    // And the binaries are the ones the app is showing: that half *is* derived.
    assert_eq!(project.binaries, paths(&["/tmp/lib.a"]));
}

/// A reopen seeds the *name* and not the contents: the name is restored synchronously,
/// while the binaries arrive from a worker thread — so a baseline holding them would read
/// the still-empty boot state as a change and write an empty project over a good one.
#[test]
fn reopening_seeds_the_name_but_not_the_baseline() {
    let mut saves = Saves::new();
    let loaded = Project {
        name: Some("kernel".into()),
        directory: None,
        binaries: paths(&["/tmp/vmlinux"]),
        cargo: None,
        bookmarks: Vec::new(),
    };
    saves.opened(ProjectId::new("kernel-1").expect("an id"), &loaded);

    // The boot state equals the baseline, so nothing is written.
    assert_eq!(recorded(&mut saves, Vec::new(), Session::new()), None);
    // And the restore that follows is an ordinary change, written at once.
    let (project, _) =
        recorded(&mut saves, paths(&["/tmp/vmlinux"]), Session::new()).expect("a write");
    assert_eq!(project, loaded);
}

/// A rename is on disk before the next click, and is a `project.toml` write and nothing
/// else: it lets go of no binary and so cannot leave the two files disagreeing.
#[test]
fn a_rename_is_written_at_once_and_leaves_the_session_pending() {
    let mut saves = Saves::new();
    written(&mut saves, &["/tmp/lib.a"], None);
    // A selection, pending as ever.
    written(&mut saves, &["/tmp/lib.a"], Some("a.o"));

    let named = Details {
        name: Some("kernel".into()),
        directory: Some(PathBuf::from("/src/kernel")),
        cargo: None,
    };
    let written = saves
        .record(
            named.clone(),
            paths(&["/tmp/lib.a"]),
            Vec::new(),
            session_with(Some("a.o")),
        )
        .expect("a write");
    assert_eq!(
        written.0,
        Project {
            name: named.name.clone(),
            directory: named.directory.clone(),
            binaries: paths(&["/tmp/lib.a"]),
            cargo: None,
            bookmarks: Vec::new(),
        }
    );
    // The session was not owed, and is still pending: the rename did not take it along.
    assert_eq!(written.1, None);
    assert_eq!(saves.flush(), Some(session_with(Some("a.o"))));

    // And the same name recorded again is not a second write.
    assert_eq!(
        saves.record(
            named,
            paths(&["/tmp/lib.a"]),
            Vec::new(),
            session_with(Some("a.o"))
        ),
        None
    );
}

/// Clearing a name writes the key away rather than leaving the old one on disk.
#[test]
fn clearing_a_name_is_a_change_too() {
    let mut saves = Saves::new();
    saves.opened(
        ProjectId::new("kernel-1").expect("an id"),
        &Project {
            name: Some("kernel".into()),
            ..Project::default()
        },
    );

    let written = saves
        .record(Details::default(), Vec::new(), Vec::new(), Session::new())
        .expect("a write");
    assert_eq!(written.0.name, None);
    assert_eq!(written.1, None);
}

/// A rename while the binaries are still being parsed writes back the list the file
/// already holds: the app holds none in that window, and writing its own empty list would
/// forget them through a change that had nothing to do with them.
#[test]
fn a_rename_before_the_binaries_have_loaded_does_not_forget_them() {
    let mut saves = Saves::new();
    let loaded = Project {
        name: None,
        directory: None,
        binaries: paths(&["/tmp/vmlinux", "/tmp/lib.a"]),
        cargo: None,
        bookmarks: Vec::new(),
    };
    saves.opened(ProjectId::new("kernel-1").expect("an id"), &loaded);

    let named = Details {
        name: Some("kernel".into()),
        directory: None,
        cargo: None,
    };
    let written = saves
        .record(named, Vec::new(), Vec::new(), Session::new())
        .expect("a write");
    assert_eq!(written.0.name.as_deref(), Some("kernel"));
    assert_eq!(written.0.binaries, loaded.binaries);

    // Once the parse lands the write *is* about the binaries, which is the one kind that
    // may replace the list.
    let written = saves
        .record(
            saves.given.clone(),
            paths(&["/tmp/vmlinux"]),
            Vec::new(),
            Session::new(),
        )
        .expect("a write");
    assert_eq!(written.0.binaries, paths(&["/tmp/vmlinux"]));
    // Closing the last one is still a real change and still empties the file.
    let written = recorded(&mut saves, Vec::new(), Session::new()).expect("a write");
    assert_eq!(written.0.binaries, Vec::<PathBuf>::new());
}

/// Entering another project empties every baseline, the app being about to be emptied: a
/// baseline still describing the old binaries would write that emptying into the new
/// project.
#[test]
fn entering_a_project_empties_every_baseline() {
    let mut saves = Saves::new();
    written(&mut saves, &["/tmp/lib.a"], Some("a.o"));

    let entered = Project {
        name: Some("other".into()),
        ..Project::default()
    };
    saves.opened(ProjectId::new("other-2").expect("an id"), &entered);

    // The state a switch leaves the app in: nothing open, nothing selected, and the name
    // of the project just entered — every one of them the baseline.
    assert_eq!(
        saves.record(
            Details {
                name: entered.name.clone(),
                directory: entered.directory.clone(),
                cargo: None,
            },
            Vec::new(),
            Vec::new(),
            Session::new()
        ),
        None
    );
    // Nor is the old project's pending session waiting to be written into the new one.
    assert_eq!(saves.flush(), None);
}

/// A directory of this test's own, named after the line that asked for it.
fn directory(line: u32) -> PathBuf {
    std::env::temp_dir().join(format!(
        "assembly-viewer-project-test-{}-{line}",
        std::process::id()
    ))
}

fn a_project() -> Project {
    Project {
        name: Some("kernel".into()),
        directory: Some(PathBuf::from("/src/kernel")),
        binaries: paths(&["/tmp/lib.a", "/tmp/some.dll"]),
        cargo: None,
        bookmarks: Vec::new(),
    }
}

/// The field-order trap for the project half, asserted against a real serializer rather
/// than read off the struct.
#[test]
fn a_project_round_trips_through_toml() {
    let project = a_project();
    let text = round_trip(&project);
    assert!(!text.contains("\n["), "a table in the project file\n{text}");

    let name = text.find("name = ").expect("the name");
    let directory = text.find("directory = ").expect("the directory");
    let binaries = text.find("binaries = ").expect("the binaries");
    assert!(name < directory && directory < binaries, "{text}");
}

/// Anonymous is an *absent* key, never an empty name a later reader could mistake for one
/// the user chose.
#[test]
fn an_anonymous_project_writes_no_name() {
    let project = Project {
        binaries: paths(&["/tmp/lib.a"]),
        ..Project::default()
    };
    let text = round_trip(&project);
    assert!(!text.contains("name"), "{text}");
    assert!(!text.contains("directory"), "{text}");
}

/// The split seen from the disk: each half in its own file, neither holding a word of the
/// other's.
#[test]
fn the_two_halves_are_written_to_their_own_files() {
    let directory = directory(line!());
    let project = a_project();
    let session = Session {
        active: Some(saved_object("a.o")),
        tabs: vec![saved_tab("a.o", 7)],
        ..Session::new()
    };

    write_toml(&directory.join(PROJECT_FILE), &project).expect("saving the project");
    session
        .save_to(&directory.join(SESSION_FILE))
        .expect("saving the session");

    let project_text = fs::read_to_string(directory.join(PROJECT_FILE)).expect("reading");
    let session_text = fs::read_to_string(directory.join(SESSION_FILE)).expect("reading");
    assert!(project_text.contains("/tmp/lib.a"), "{project_text}");
    assert!(!project_text.contains("active"), "{project_text}");
    assert!(session_text.contains("active"), "{session_text}");
    assert!(!session_text.contains("binaries"), "{session_text}");

    assert_eq!(
        Project::load_from(&directory.join(PROJECT_FILE)),
        Some(project)
    );
    assert_eq!(load_session(&directory.join(SESSION_FILE)), Some(session));

    let _ = fs::remove_dir_all(&directory);
}

/// Why the split is worth two files: the half the app rewrites every thirty seconds cannot
/// take the half the user gave down with it.
#[test]
fn a_corrupt_session_leaves_the_project_readable() {
    let directory = directory(line!());
    let project = a_project();
    write_toml(&directory.join(PROJECT_FILE), &project).expect("saving the project");
    fs::write(directory.join(SESSION_FILE), b"{ not toml").expect("writing the corrupt half");

    assert_eq!(
        Project::load_from(&directory.join(PROJECT_FILE)),
        Some(project)
    );
    assert_eq!(load_session(&directory.join(SESSION_FILE)), None);

    let _ = fs::remove_dir_all(&directory);
}

/// An id is interpolated into a path, so what it may be is what keeps `recents.toml` from
/// naming somewhere else on the disk.
#[test]
fn an_id_is_one_ordinary_path_component() {
    for good in ["project-1", "kernel_2", "a", "9lives"] {
        assert_eq!(
            ProjectId::new(good).map(|id| id.as_str().to_owned()),
            Some(good.to_owned())
        );
    }
    for bad in [
        "", "..", ".", "-leading", "a/b", "a\\b", "a b", "a.toml", "é",
    ] {
        assert_eq!(ProjectId::new(bad), None, "{bad}");
    }
    assert_eq!(ProjectId::new("x".repeat(MAX_ID + 1)), None);
}

/// Deserializing goes through the same check, so a file holding an id that is not one is a
/// corrupt file, which is the default.
#[test]
fn a_hand_edited_recent_that_is_not_an_id_is_refused() {
    assert!(toml::from_str::<Recents>(r#"projects = ["../elsewhere"]"#).is_err());
    assert!(toml::from_str::<Recents>(r#"projects = ["project-1"]"#).is_ok());

    let directory = directory(line!());
    let path = directory.join(RECENTS_FILE);
    fs::create_dir_all(&directory).expect("creating the test directory");
    fs::write(&path, br#"projects = ["../elsewhere"]"#).expect("writing");
    assert_eq!(Recents::load_in(&directory), Recents::default());
    // And moved aside, the next `remember` being what would otherwise write over it.
    assert!(directory
        .join(crate::rescue::INCOMPATIBLE_DIR)
        .join(RECENTS_FILE)
        .exists());

    let _ = fs::remove_dir_all(&directory);
}

fn id(text: &str) -> ProjectId {
    ProjectId::new(text).expect("an id")
}

/// The order *is* the answer to "which project was last open", so touching the one already
/// at the front changes nothing and writes no file.
#[test]
fn touching_a_project_moves_it_to_the_front_once() {
    let mut recents = Recents::default();
    assert!(recents.touch(&id("a")));
    assert!(recents.touch(&id("b")));
    assert_eq!(recents.projects, vec![id("b"), id("a")]);
    assert_eq!(recents.first(), Some(&id("b")));

    // Already first: no change, and so no write.
    assert!(!recents.touch(&id("b")));
    // And one that is in the list is moved rather than repeated.
    assert!(recents.touch(&id("a")));
    assert_eq!(recents.projects, vec![id("a"), id("b")]);
}

/// Bounded, because this file is appended to for as long as the app is ever used. What
/// falls off the end is a place in the order and never a project.
#[test]
fn the_recent_list_is_bounded() {
    let mut recents = Recents::default();
    for n in 0..MAX_RECENTS + 10 {
        recents.touch(&id(&format!("project-{n}")));
    }
    assert_eq!(recents.projects.len(), MAX_RECENTS);
    assert_eq!(
        recents.first(),
        Some(&id(&format!("project-{}", MAX_RECENTS + 9)))
    );
}

#[test]
fn the_recent_list_round_trips_through_toml() {
    let mut recents = Recents::default();
    recents.touch(&id("project-1"));
    recents.touch(&id("kernel_2"));
    let text = round_trip(&recents);
    assert!(text.contains(r#""kernel_2""#), "{text}");

    // A missing or unreadable file is the empty list, never an error.
    assert_eq!(Recents::load_in(Path::new("/no/such")), Recents::default());
}

/// The claim is the `create_dir`, so two allocations in the same directory cannot land on
/// the same name.
#[test]
fn anonymous_projects_do_not_collide() {
    let directory = directory(line!());

    let first = ProjectId::anonymous(&directory).expect("an id");
    let second = ProjectId::anonymous(&directory).expect("a second id");
    assert_ne!(first, second);
    assert!(directory.join(first.as_str()).is_dir());
    assert!(directory.join(second.as_str()).is_dir());

    // A directory that is already there is stepped over rather than opened, whether
    // this app made it or not.
    fs::create_dir(directory.join(format!("{ANONYMOUS_STEM}-3"))).expect("a squatter");
    let third = ProjectId::anonymous(&directory).expect("a third id");
    assert_ne!(third.as_str(), format!("{ANONYMOUS_STEM}-3"));

    let _ = fs::remove_dir_all(&directory);
}

/// A project appears when there is something to put in it: nothing on disk until the first
/// write, then a directory, and the recent list pointing at it.
#[test]
fn the_first_write_creates_a_project_and_remembers_it() {
    let base = directory(line!());
    let mut saves = Saves::new();

    let id = open_project(&mut saves, &base).expect("a project");
    assert!(project_in(&base, &id).is_dir());
    assert_eq!(Recents::load_in(&base).projects, vec![id.clone()]);

    // Every later write of the run goes into the same one rather than allocating another.
    assert_eq!(open_project(&mut saves, &base), Some(id));

    let _ = fs::remove_dir_all(&base);
}

/// Startup: the front of the recent list, both halves of it.
#[test]
fn the_last_project_is_the_one_reopened() {
    let base = directory(line!());
    let project = a_project();
    let session = Session {
        active: Some(saved_object("a.o")),
        ..Session::new()
    };

    for id in ["other-1", "wanted-2"] {
        let id = self::id(id);
        write_toml(&project_in(&base, &id).join(PROJECT_FILE), &project)
            .expect("saving the project");
        session
            .save_to(&project_in(&base, &id).join(SESSION_FILE))
            .expect("saving the session");
        remember(&base, &id);
    }

    let (id, reopened, restored) = reopen_in(&base).expect("a project to reopen");
    assert_eq!(id, self::id("wanted-2"));
    assert_eq!(reopened, project);
    assert_eq!(restored, session);

    let _ = fs::remove_dir_all(&base);
}

/// Three ways for there to be nothing to reopen, all of them silence.
#[test]
fn nothing_to_reopen_is_not_an_error() {
    let base = directory(line!());
    // No recent list at all: a first run, or one whose file was deleted.
    assert!(reopen_in(&base).is_none());

    // A recent list naming a project whose directory has gone.
    remember(&base, &id("gone-1"));
    assert!(reopen_in(&base).is_none());

    let _ = fs::remove_dir_all(&base);
}

/// The directory *is* the project, so a run killed between creating one and writing either
/// file into it reopens as the empty project it is rather than being orphaned. A corrupt
/// session is the same answer.
#[test]
fn a_project_missing_a_half_still_reopens() {
    let base = directory(line!());
    let mut saves = Saves::new();
    let id = open_project(&mut saves, &base).expect("a project");

    // Neither file written yet.
    let (reopened, project, session) = reopen_in(&base).expect("a project to reopen");
    assert_eq!(reopened, id);
    assert_eq!(project, Project::default());
    assert_eq!(session, Session::new());

    // The user's half good, the app's half corrupt.
    let project = a_project();
    write_toml(&project_in(&base, &id).join(PROJECT_FILE), &project).expect("saving the project");
    fs::write(project_in(&base, &id).join(SESSION_FILE), b"{ not toml")
        .expect("writing the corrupt half");

    let (_, reopened, session) = reopen_in(&base).expect("a project to reopen");
    assert_eq!(reopened, project);
    assert_eq!(session, Session::new());

    // And the corrupt half was moved aside under the path it had, rather than left for
    // the next flush to write over.
    let moved = base
        .join(crate::rescue::INCOMPATIBLE_DIR)
        .join(PROJECTS_DIR)
        .join(id.as_str())
        .join(SESSION_FILE);
    assert_eq!(fs::read(&moved).ok().as_deref(), Some(&b"{ not toml"[..]));

    let _ = fs::remove_dir_all(&base);
}

/// The recent-projects view reads each row's name out of that project's own file, in the
/// order the list keeps, so there is one copy of a name.
#[test]
fn the_recent_view_names_each_project_from_its_own_file() {
    let base = directory(line!());
    for (id, name) in [("first-1", "kernel"), ("second-2", "loader")] {
        let id = self::id(id);
        write_toml(
            &project_in(&base, &id).join(PROJECT_FILE),
            &Project {
                name: Some(name.to_owned()),
                directory: Some(PathBuf::from("/src").join(name)),
                binaries: paths(&["/tmp/lib.a", "/tmp/some.dll"]),
                cargo: None,
                bookmarks: Vec::new(),
            },
        )
        .expect("saving the project");
        remember(&base, &id);
    }

    let recents = recent_projects_in(&base);
    assert_eq!(
        recents
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["second-2", "first-1"]
    );
    assert_eq!(recents[0].name.as_deref(), Some("loader"));
    assert_eq!(recents[0].directory, Some(PathBuf::from("/src/loader")));
    assert_eq!(recents[0].binaries, 2);

    let _ = fs::remove_dir_all(&base);
}

/// Reading a project to draw its row is not opening it: a file that will not parse is
/// left where it is, since nothing is about to write over a project nobody has entered.
#[test]
fn listing_a_project_does_not_move_its_file_aside() {
    let base = directory(line!());
    let id = self::id("broken-1");
    let path = project_in(&base, &id).join(PROJECT_FILE);
    fs::create_dir_all(project_in(&base, &id)).expect("creating the test directory");
    fs::write(&path, b"{ not toml").expect("writing the corrupt file");
    remember(&base, &id);

    // The row is drawn, as the project it will behave as once opened.
    let recents = recent_projects_in(&base);
    assert_eq!(recents.len(), 1);
    assert_eq!(recents[0].name, None);

    assert!(path.exists());
    assert!(!base.join(crate::rescue::INCOMPATIBLE_DIR).exists());

    let _ = fs::remove_dir_all(&base);
}

/// A project whose directory has gone is dropped here, where the repair is free; one with
/// a directory and no readable file is a real project and keeps its row.
#[test]
fn a_recent_project_that_is_gone_is_dropped_and_an_empty_one_is_not() {
    let base = directory(line!());
    let mut saves = Saves::new();
    let empty = open_project(&mut saves, &base).expect("a project");
    remember(&base, &id("gone-1"));

    let recents = recent_projects_in(&base);
    assert_eq!(recents.len(), 1);
    assert_eq!(recents[0].id, empty);
    assert_eq!(recents[0].name, None);
    assert_eq!(recents[0].binaries, 0);

    let _ = fs::remove_dir_all(&base);
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
        SavedDocument::Code {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "b.o".into(),
        }
    );
    let found = saved.resolve(&objects, &Rebuilt::Paths(Default::default()));
    assert!(
        found == Some(document.clone()),
        "the code document comes back"
    );
    assert!(
        found != Some(Document::Assembly(Selection::Object(objects[1].clone()))),
        "the object's code is not the object"
    );

    // Gone with its object, and degrading to nothing rather than to another object.
    let rest: Vec<Arc<Object>> = objects[..1].to_vec();
    assert!(saved
        .resolve(&rest, &Rebuilt::Paths(Default::default()))
        .is_none());
    assert!(saved
        .resolve_or_degrade(&rest, &Rebuilt::Paths(Default::default()))
        .is_none());

    // And it survives the round trip through TOML in a tab.
    let tab = saved_one(saved_entry(saved, 3));
    let text = toml::to_string(&tab).expect("serialises");
    let back: SavedTab = toml::from_str(&text).expect("parses back");
    assert_eq!(back, tab);
}

/// An object's code points into the file its object came out of, so it closes with it and
/// with nothing else.
#[test]
fn a_code_document_closes_with_its_file() {
    let objects = objects();
    let code = Document::Code(objects[1].clone());
    assert!(code.in_file(Path::new("/tmp/lib.a")));
    assert!(!code.in_file(Path::new("/tmp/some.dll")));
    assert!(code.symbol().is_none(), "no symbol to ask the worker about");
}

/// The address a code tab was left at is written before its document, as the rows are:
/// TOML puts plain values before tables, and a value after one would be read as the
/// table's.
#[test]
fn a_code_tabs_address_is_written_before_its_document() {
    let objects = objects();
    let code = Document::Code(objects[1].clone());
    let spot = Spot {
        address: 0x30,
        rows: 2,
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

    // And it comes back as the tab's address, the rows past it being a nicety.
    let restored = session.resolve_tabs(&objects);
    assert!(restored[0].entries[0].document == code);
    assert_eq!(restored[0].entries[0].address, Some(0x30));
}

/// An address is a claim about a layout: a rebuilt binary takes it with the rows and
/// leaves the tab.
#[test]
fn a_rebuilt_binary_takes_the_saved_address_with_it() {
    let objects = objects();
    let code = Document::Code(objects[1].clone());
    let spot = Spot {
        address: 0x30,
        rows: 0,
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

    // The same file, rebuilt: a different digest under the same path.
    let rebuilt = vec![
        built("/tmp/lib.a", "a.o", &[("caller", 0)], b"the second build"),
        built("/tmp/lib.a", "b.o", &[("caller", 0)], b"the second build"),
    ];
    let restored = session.resolve_tabs(&rebuilt);
    assert_eq!(restored.len(), 1);
    assert!(restored[0].entries[0].document == Document::Code(rebuilt[1].clone()));
    assert_eq!(restored[0].entries[0].address, None);
}

/// Only a code tab has an address to save; a symbol's tab keeps its row.
#[test]
fn a_symbol_tab_saves_no_address() {
    let objects = objects();
    let symbol = Document::Assembly(Selection::Symbol(Symbol {
        object: objects[0].clone(),
        data: objects[0].symbols_sorted[0].clone(),
    }));
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
    let unchanged = Rebuilt::Paths(Default::default());
    let rebuilt = Rebuilt::Paths(HashSet::from([PathBuf::from("/tmp/lib.a")]));
    let find = |name: &str, address: u64, rebuilt: &Rebuilt| {
        saved_symbol("a.o", name, address)
            .resolve(&objects, rebuilt)
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
        assert_eq!((found.name.as_str(), found.address), (name, address));
    }
    // Two `mid`s and a stale address: neither is picked, rebuilt or not.
    assert!(find("mid", 9, &unchanged).is_none());
    assert!(find("mid", 9, &rebuilt).is_none());
    // A lone name at a stale address is, under a rebuild only.
    assert!(find("beta", 9, &unchanged).is_none());
    assert_eq!(find("beta", 9, &rebuilt).map(|data| data.address), Some(2));
    for name in ["aardvark", "gamma", "omega"] {
        assert!(find(name, 1, &rebuilt).is_none(), "{name}");
    }
}

/// A bookmark is what the user said, so it is `project.toml`'s: the one table in that file,
/// after every plain value, and inside it the name before its document's table.
#[test]
fn bookmarks_are_written_after_the_binaries_and_name_first() {
    let project = Project {
        bookmarks: vec![
            Bookmark {
                name: "kernel::start".into(),
                document: saved_symbol("a.o", "_ZN6kernel5startE", 6),
            },
            Bookmark {
                name: "main.rs".into(),
                document: SavedDocument::Source {
                    path: "/src/main.rs".into(),
                },
            },
        ],
        ..a_project()
    };
    let text = round_trip(&project);

    let binaries = text.find("binaries = ").expect("the binaries");
    let first = text.find("[[bookmarks]]").expect("a bookmark");
    assert!(binaries < first, "{text}");
    assert!(
        !text[..first].contains("\n["),
        "a table before the bookmarks\n{text}"
    );

    let name = text
        .find("name = \"kernel::start\"")
        .expect("the bookmark's name");
    let document = text
        .find("[bookmarks.document.Symbol]")
        .expect("its document");
    assert!(first < name && name < document, "{text}");
    assert!(text.contains("[bookmarks.document.Source]"), "{text}");

    // And none at all is a key that is absent, not an empty list.
    assert!(!round_trip(&a_project()).contains("bookmarks"));
}

/// A bookmarks change is written at once and to `project.toml` alone, like a rename: it
/// lets go of no binary, so it cannot leave the two files disagreeing, and it writes back
/// the binaries the file already lists rather than the app's own.
#[test]
fn a_bookmarks_change_writes_the_project_file_alone() {
    let mut saves = Saves::new();
    let reopened = Project {
        bookmarks: vec![Bookmark {
            name: "caller".into(),
            document: saved_symbol("a.o", "caller", 0),
        }],
        ..a_project()
    };
    saves.opened(id("project-1"), &reopened);

    // Seeded: the same bookmarks are no change, while the parse has yet to land.
    let unchanged = saves.record(
        saves.given.clone(),
        Vec::new(),
        reopened.bookmarks.clone(),
        Session::new(),
    );
    assert!(unchanged.is_none());

    let mut added = reopened.bookmarks.clone();
    added.push(Bookmark {
        name: "target".into(),
        document: saved_symbol("a.o", "target", 6),
    });
    let (project, session) = saves
        .record(
            saves.given.clone(),
            Vec::new(),
            added.clone(),
            Session::new(),
        )
        .expect("a write");
    assert!(session.is_none(), "the session went with it");
    assert_eq!(project.bookmarks, added);
    assert_eq!(
        project.binaries, reopened.binaries,
        "the listed binaries, not the app's"
    );

    // Removing them all is a change too, written as an absent key.
    let (project, _) = saves
        .record(saves.given.clone(), Vec::new(), Vec::new(), Session::new())
        .expect("a write");
    assert!(project.bookmarks.is_empty());
}

/// A saved place read by name alone, which is what a bookmark is: the same symbol as the
/// strict rule finds in an unchanged file, the moved symbol in a rebuilt one, and nothing
/// where two symbols share the name and neither sits at the saved address.
#[test]
fn resolving_by_name_agrees_with_the_strict_rule_and_survives_a_rebuild() {
    let objects = objects();
    let saved = saved_symbol("a.o", "target", 6);
    let strict = saved.resolve(&objects, &Rebuilt::Paths(Default::default()));
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
    let expected = Document::Assembly(Selection::Symbol(Symbol {
        object: rebuilt[0].clone(),
        data: rebuilt[0].symbols_sorted[1].clone(),
    }));
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

/// The section a build puts in each file, as it is spelled: `[cargo]` in both, a profile
/// by name in the project's and the paths in the session's, both read back as they were
/// written. The profile is written as a word rather than as a number, so a file a reader
/// opens says what it means.
#[test]
fn a_cargo_section_is_written_where_toml_can_read_it_back() {
    let project = Project {
        name: Some("kernel".into()),
        directory: Some(PathBuf::from("/src/kernel")),
        binaries: paths(&["/tmp/vmlinux"]),
        cargo: Some(Cargo {
            profile: Profile::Debug,
        }),
        bookmarks: vec![Bookmark {
            name: "start".to_owned(),
            document: SavedDocument::Source {
                path: "/src/kernel/main.rs".to_owned(),
            },
        }],
    };
    let text = round_trip(&project);
    assert!(text.contains("[cargo]"), "{text}");
    assert!(text.contains("profile = \"debug\""), "{text}");

    let session = Session {
        cargo: Some(SessionCargo {
            artifacts: paths(&["/src/kernel/target/debug/vmlinux"]),
        }),
        ..session_with(None)
    };
    let text = round_trip(&session);
    assert!(text.contains("[cargo]"), "{text}");
}

/// Absent rather than empty: a project whose profile is the default one and a session in
/// which nothing was built each write no section at all, so a file nothing has chosen in
/// says nothing.
#[test]
fn nothing_chosen_and_nothing_built_write_no_section() {
    let project = Project {
        name: None,
        directory: None,
        binaries: Vec::new(),
        cargo: None,
        bookmarks: Vec::new(),
    };
    assert!(!round_trip(&project).contains("[cargo]"));
    assert!(!round_trip(&Session::new()).contains("[cargo]"));
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
        &Positions::default(),
        &Positions::default(),
        &Positions::default(),
        &Driven::default(),
        None,
        &Visits::default(),
        &built,
    );
    assert_eq!(
        session.cargo.as_ref().map(|cargo| cargo.artifacts.clone()),
        Some(built)
    );
}
