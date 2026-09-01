use std::collections::HashMap;

use analysis::{Architecture, BinaryFormat, ObjectData, Section, SectionIndex, SymbolData};

use super::*;

/// A bare `Object` with the given text symbols. The analysis crate's own fixtures
/// go through `parse_object`; here only the fields the mapping reads matter, and
/// every one of them is public, so the objects are built directly.
fn object(path: &str, name: &str, symbols: &[(&str, u64)]) -> Arc<Object> {
    built(path, name, symbols, b"the first build")
}

/// The same, out of a named build of the file. `bytes` is only ever hashed here —
/// these objects were not parsed from it — so "the file was rebuilt" is spelt as two
/// calls with different bytes.
fn built(path: &str, name: &str, symbols: &[(&str, u64)], bytes: &[u8]) -> Arc<Object> {
    let section = Arc::new(Section {
        index: SectionIndex(0),
        name: ".text".into(),
        data: vec![0xC3; symbols.len()],
        address: 0,
        relocations: HashMap::new(),
        symbols: symbols.iter().map(|(_, address)| *address).collect(),
    });

    let symbols_sorted: Vec<Arc<SymbolData>> = symbols
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

    Arc::new(Object {
        path: PathBuf::from(path),
        name: name.to_owned(),
        format: BinaryFormat::Elf,
        architecture: Architecture::X86_64,
        symbols: HashMap::new(),
        symbols_sorted,
        sections: vec![section],
        // The mapping never looks at the bytes themselves; what it does look at is
        // the digest they hash to, which is what says whether this is still the file
        // the session was saved against.
        data: ObjectData::from(bytes),
        dwarf: Default::default(),
    })
}

/// [`Session::from_state`] over a session whose only open tab is the active document
/// and whose panes are at the top of what they show — the state the tests written
/// before there were tabs to save were already describing, now spelt out.
///
/// It takes a [`Selection`] because every test that reaches for it is about a place in
/// a binary; a source-driven tab has its own tests below.
fn from_state(
    objects: &[Arc<Object>],
    selection: Option<&Selection>,
    history: &History,
) -> Session {
    let document = selection.cloned().map(Document::Assembly);
    Session::from_state(
        objects,
        document
            .as_ref()
            .map(std::slice::from_ref)
            .unwrap_or_default(),
        &Positions::default(),
        &Positions::default(),
        document.as_ref(),
        history,
    )
}

/// What [`Session::resolve`] answers, as the selection inside it. Every use of this
/// is a test about a place in a binary, which is what the app had before a document
/// could also be a file.
fn resolve_selection(session: &Session, objects: &[Arc<Object>]) -> Option<Selection> {
    session
        .resolve(objects)
        .and_then(|document| document.selection().cloned())
}

fn objects() -> Vec<Arc<Object>> {
    vec![
        object("/tmp/lib.a", "a.o", &[("caller", 0), ("target", 6)]),
        // Same path, different member: `path` alone cannot tell these apart.
        object("/tmp/lib.a", "b.o", &[("caller", 0)]),
    ]
}

/// The one question closing a file asks. A member is not a file, so both members of
/// `/tmp/lib.a` answer for it and a symbol answers for the file its object came out
/// of — closing the archive takes every one of them.
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

    // Nothing selected is in no file, so a close never has to special-case it.
}

#[test]
fn saves_and_resolves_a_symbol() {
    let objects = objects();
    let selection = Selection::Symbol(Symbol {
        object: objects[1].clone(),
        data: objects[1].symbols_sorted[0].clone(),
    });

    let session = from_state(&objects, Some(&selection), &History::default());
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
    let session = from_state(&objects, Some(&selection), &History::default());
    assert!(resolve_selection(&session, &objects) == Some(selection));
}

#[test]
fn no_selection_round_trips_as_none() {
    let objects = objects();
    let session = from_state(&objects, None, &History::default());
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
fn a_moved_symbol_falls_back_to_its_object() {
    let objects = objects();
    let session = Session {
        active: Some(SavedDocument::Symbol {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "a.o".into(),
            // Right name, recompiled to a different address.
            symbol_name: "target".into(),
            address: 999,
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

/// Serialize to TOML and read it straight back, which is the only way to catch the
/// `toml` crate's runtime failures: a bare `None`, and a value emitted after a table.
/// Generic over the two files, because the trap is a property of the serializer and
/// both halves are equally subject to it.
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
    // Nothing selected and nothing visited: the `None` the `toml` crate cannot write
    // has to be left out of the file entirely, and read back as `None`.
    let session = Session::new();
    let text = round_trip(&session);
    assert!(!text.contains("active"), "{text}");
}

#[test]
fn a_session_with_no_selection_round_trips() {
    let objects = objects();
    let session = from_state(&objects, None, &History::default());
    assert_eq!(session.active, None);
    let text = round_trip(&session);
    assert!(!text.contains("active"), "{text}");
}

#[test]
fn a_multi_entry_history_round_trips_as_an_array_of_tables() {
    let objects = objects();
    let session = from_state(&objects, None, &history(&objects, 1));
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

    assert_eq!(Session::load_from(&path), Some(session));
    // The temporary was renamed, not left behind.
    assert!(!path.with_extension("toml.tmp").exists());

    let _ = fs::remove_dir_all(&directory);
}

/// A history over the fixture objects: object `a.o`, then its `target` symbol, then
/// object `b.o`, with the cursor wherever `back` calls leave it.
fn history(objects: &[Arc<Object>], back: usize) -> History {
    let mut history = History::default();
    history.push(Document::Assembly(Selection::Object(objects[0].clone())));
    history.push(Document::Assembly(Selection::Symbol(Symbol {
        object: objects[0].clone(),
        data: objects[0].symbols_sorted[1].clone(),
    })));
    history.push(Document::Assembly(Selection::Object(objects[1].clone())));
    for _ in 0..back {
        history.back();
    }
    history
}

#[test]
fn saves_and_restores_the_history() {
    let objects = objects();
    let history = history(&objects, 0);

    let session = from_state(&objects, None, &history);
    assert_eq!(session.history.entries.len(), 3);
    assert_eq!(session.history.cursor, 2);

    let restored = session.resolve_history(&objects);
    assert!(restored.entries() == history.entries());
    assert_eq!(restored.cursor(), Some(2));
    assert!(restored.can_back());
    assert!(!restored.can_forward());
}

#[test]
fn a_history_the_user_walked_back_through_keeps_its_cursor() {
    let objects = objects();
    let history = history(&objects, 2);
    assert_eq!(history.cursor(), Some(0));

    let session = from_state(&objects, None, &history);
    let restored = session.resolve_history(&objects);

    assert_eq!(restored.cursor(), Some(0));
    // The two entries in front of the cursor survived, so they are still there to
    // go forward to.
    assert!(restored.can_forward());
    assert!(!restored.can_back());
}

/// Building a saved history by hand, since these are entries no live `History`
/// could have produced against these objects.
fn saved_history(entries: &[SavedDocument], cursor: usize) -> Session {
    Session {
        active: None,
        history: SavedHistory {
            entries: entries.to_vec(),
            cursor,
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
    let session = saved_history(
        &[
            saved_object("a.o"),
            // A member that is no longer in the archive.
            saved_object("c.o"),
            // The object is there but the symbol is gone. Unlike the selection,
            // which would degrade to the object, an entry is dropped: the user
            // never visited the object, and a list of places they did not go is
            // worse than a shorter list.
            SavedDocument::Symbol {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "a.o".into(),
                symbol_name: "gone".into(),
                address: 12,
            },
            saved_object("b.o"),
        ],
        3,
    );

    let restored = session.resolve_history(&objects);
    assert!(restored.entries() == [tab(&objects[0]), tab(&objects[1]),]);
    // The cursor was on the last entry, which survived: it must still be on it,
    // at its new index, and not on the entry that moved into its old one.
    assert_eq!(restored.cursor(), Some(1));
    assert!(!restored.can_forward());
}

#[test]
fn a_saved_history_with_duplicates_restores_without_them() {
    let objects = objects();
    // What a saved history written before entries were bumped rather than appended
    // looks like: the same destination visited twice, saved twice.
    let session = saved_history(
        &[
            saved_object("a.o"),
            saved_object("b.o"),
            saved_object("a.o"),
        ],
        2,
    );

    let restored = session.resolve_history(&objects);
    assert!(restored.entries() == [tab(&objects[1]), tab(&objects[0]),]);
    // The cursor was on the newest `a.o`, which is where the collapse left it.
    assert_eq!(restored.cursor(), Some(1));
    assert!(!restored.can_forward());
}

#[test]
fn duplicates_collapse_around_the_entries_that_were_dropped() {
    let objects = objects();
    let session = saved_history(
        &[
            saved_object("a.o"),
            // Gone, so it is dropped before anything is collapsed.
            saved_object("c.o"),
            saved_object("b.o"),
            saved_object("a.o"),
        ],
        3,
    );

    let restored = session.resolve_history(&objects);
    assert!(restored.entries() == [tab(&objects[1]), tab(&objects[0]),]);
    assert_eq!(restored.cursor(), Some(1));
}

#[test]
fn the_restored_cursor_follows_its_entry_through_the_collapse() {
    let objects = objects();
    // The cursor is on `b.o`, in the middle, and the collapse of the two `a.o`s
    // moves it to the front of the list.
    let session = saved_history(
        &[
            saved_object("a.o"),
            saved_object("b.o"),
            saved_object("a.o"),
        ],
        1,
    );

    let restored = session.resolve_history(&objects);
    assert!(restored.current() == Some(&tab(&objects[1])));
    assert_eq!(restored.cursor(), Some(0));
    // The newest `a.o` is still in front of it to go forward to.
    assert!(restored.can_forward());
    assert!(!restored.can_back());
}

#[test]
fn two_saved_symbols_naming_the_same_one_restore_as_one_entry() {
    let objects = objects();
    let symbol = || SavedDocument::Symbol {
        path: PathBuf::from("/tmp/lib.a"),
        object_name: "a.o".into(),
        symbol_name: "target".into(),
        address: 6,
    };
    let session = saved_history(&[symbol(), saved_object("b.o"), symbol()], 2);

    // Both resolve through the same lookup to the same `Arc`, so they are equal
    // entries however far apart they were saved.
    let restored = session.resolve_history(&objects);
    assert!(
        restored.entries()
            == [
                tab(&objects[1]),
                Document::Assembly(Selection::Symbol(Symbol {
                    object: objects[0].clone(),
                    data: objects[0].symbols_sorted[1].clone(),
                })),
            ]
    );
    assert_eq!(restored.cursor(), Some(1));
}

#[test]
fn a_collapsed_cursor_entry_is_still_the_restored_selection() {
    let objects = objects();
    // Every cursor position over a saved history that holds a duplicate.
    for cursor in 0..3 {
        let mut session = saved_history(
            &[
                saved_object("a.o"),
                saved_object("b.o"),
                saved_object("a.o"),
            ],
            cursor,
        );
        session.active = Some(session.history.entries[cursor].clone());

        let restored_history = session.resolve_history(&objects);
        let restored_active = session.resolve(&objects);

        assert!(restored_history.current() == restored_active.as_ref());
        assert!(
            !restored_history.would_push(restored_active.as_ref().expect("a restored document"))
        );
    }
}

#[test]
fn a_dropped_cursor_entry_falls_back_to_the_nearest_older_survivor() {
    let objects = objects();
    let session = saved_history(
        &[
            saved_object("a.o"),
            saved_object("c.o"),
            saved_object("b.o"),
        ],
        1,
    );

    let restored = session.resolve_history(&objects);
    assert!(restored.cursor() == Some(0));
    assert!(restored.current() == Some(&tab(&objects[0])));
    // `b.o` was in front of the cursor and still is.
    assert!(restored.can_forward());
}

#[test]
fn a_cursor_with_no_older_survivor_lands_on_the_oldest_entry_left() {
    let objects = objects();
    let session = saved_history(&[saved_object("c.o"), saved_object("b.o")], 0);

    let restored = session.resolve_history(&objects);
    assert!(restored.cursor() == Some(0));
    assert!(restored.current() == Some(&tab(&objects[1])));
}

#[test]
fn a_history_that_resolves_to_nothing_restores_as_empty() {
    let objects = objects();
    let session = saved_history(&[saved_object("c.o"), saved_object("d.o")], 1);

    let restored = session.resolve_history(&objects);
    assert!(restored.entries().is_empty());
    assert!(restored.cursor().is_none());
    assert!(!restored.can_back());
    assert!(!restored.can_forward());
}

#[test]
fn a_hand_edited_cursor_past_the_end_is_clamped() {
    let objects = objects();
    let session = saved_history(&[saved_object("a.o"), saved_object("b.o")], 99);

    let restored = session.resolve_history(&objects);
    assert_eq!(restored.cursor(), Some(1));
}

#[test]
fn the_restored_cursor_entry_is_the_restored_selection() {
    let objects = objects();

    // Every position the cursor can be in, including one the user walked back to.
    for back in 0..3 {
        let history = history(&objects, back);
        let current = history.current().expect("a current entry").clone();
        let selection = current.selection().expect("a place in a binary").clone();
        let session = from_state(&objects, Some(&selection), &history);

        let restored_history = session.resolve_history(&objects);
        let restored_active = session.resolve(&objects);

        assert!(restored_history.current() == restored_active.as_ref());
        // Which is what keeps the recording effect from pushing a duplicate the
        // moment the restore sets the active document.
        assert!(
            !restored_history.would_push(restored_active.as_ref().expect("a restored document"))
        );
    }
}

#[test]
fn a_file_with_no_history_still_loads() {
    // Hand-written, or trimmed: `serde(default)` is what keeps the missing table
    // from taking the binaries and the selection down with it.
    let text = r#"
            binaries = ["/tmp/lib.a"]

            [active.Object]
            path = "/tmp/lib.a"
            object_name = "a.o"
        "#;
    let session: Session = toml::from_str(text).expect("deserializing");

    assert_eq!(session.history, SavedHistory::new());

    // And it restores exactly as it would have: the selection back, no history.
    let objects = objects();
    assert!(resolve_selection(&session, &objects) == Some(Selection::Object(objects[0].clone())));
    assert!(session.resolve_history(&objects).entries().is_empty());
}

#[test]
fn a_history_with_no_entries_still_loads() {
    let text = r#"
            binaries = []

            [history]
            cursor = 0
        "#;
    let session: Session = toml::from_str(text).expect("deserializing");
    assert_eq!(session, Session::new());
}

/// A path TOML cannot spell is refused rather than mangled, in *both* files: the
/// binaries are the project half and the digests are keyed by the same paths, so the
/// one refusal has to hold on either side of the split.
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
        assert!(project.save_to(&directory.join(PROJECT_FILE)).is_err());
        assert!(session.save_to(&directory.join(SESSION_FILE)).is_err());
        // Nothing reached the disk, so a good earlier file would still be there.
        assert!(!directory.join(PROJECT_FILE).exists());
        assert!(!directory.join(SESSION_FILE).exists());

        let _ = fs::remove_dir_all(&directory);
    }
}

#[test]
fn the_history_round_trips_through_toml() {
    let objects = objects();
    let session = from_state(&objects, None, &history(&objects, 1));
    round_trip(&session);
}

// --- the open tabs ------------------------------------------------------

/// Where the panes were left, in the map the UI keeps it in.
fn positions<T: Clone + PartialEq>(at: &[(&T, usize)]) -> Positions<T> {
    let mut positions = Positions::default();
    for (tab, row) in at {
        positions.remember((*tab).clone(), *row);
    }
    positions
}

/// An assembly-driven tab over a whole object.
fn tab(object: &Arc<Object>) -> Document {
    Document::Assembly(Selection::Object(object.clone()))
}

/// A source-driven tab, in the `Arc<str>` the UI holds a file in.
fn file_tab(path: &str) -> Document {
    Document::Source(Arc::from(path))
}

fn saved_tab(object_name: &str, asm_row: usize) -> SavedTab {
    SavedTab {
        asm_row,
        src_row: 0,
        document: saved_object(object_name),
    }
}

fn saved_file_tab(path: &str, asm_row: usize, src_row: usize) -> SavedTab {
    SavedTab {
        asm_row,
        src_row,
        document: SavedDocument::Source {
            path: path.to_owned(),
        },
    }
}

/// One strip of both kinds goes out in the reader's own order and comes back in it,
/// through the very mapping the history already uses — which is the whole reason a
/// saved tab costs no new one.
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

    let session = Session::from_state(
        &objects,
        &tabs,
        &Positions::default(),
        &Positions::default(),
        Some(&tabs[3]),
        &History::default(),
    );

    assert_eq!(
        session.tabs,
        [
            saved_tab("a.o", 0),
            saved_file_tab("/src/main.rs", 0, 0),
            SavedTab {
                asm_row: 0,
                src_row: 0,
                document: SavedDocument::Symbol {
                    path: PathBuf::from("/tmp/lib.a"),
                    object_name: "a.o".into(),
                    symbol_name: "target".into(),
                    address: 6,
                },
            },
            saved_tab("b.o", 0),
        ]
    );
    assert!(
        session.resolve_tabs(&objects)
            == [
                (tabs[0].clone(), 0, 0),
                (tabs[1].clone(), 0, 0),
                (tabs[2].clone(), 0, 0),
                (tabs[3].clone(), 0, 0),
            ]
    );
}

/// The active tab is not written twice: it is `active`, and a saved session says
/// which tab is on screen only by naming it there.
#[test]
fn the_active_tab_is_only_the_active_document() {
    let objects = objects();
    let tabs = [tab(&objects[0])];
    let session = Session::from_state(
        &objects,
        &tabs,
        &Positions::default(),
        &Positions::default(),
        Some(&tabs[0]),
        &History::default(),
    );

    assert_eq!(session.active, Some(saved_object("a.o")));
    assert_eq!(session.tabs, [saved_tab("a.o", 0)]);
}

/// A tab is dropped exactly where the active document would degrade. There is one
/// active document and the app has to open somewhere, but a strip whose tabs lead to
/// places that are no longer there is worse than a shorter strip — and degrading
/// would be worse still, since two symbols of one object would degrade onto the
/// same tab and `Tabs::open` would collapse them into one.
///
/// A **source-driven tab is never dropped**: it resolves against nothing, so there is
/// nothing for it to fail against, and a file that has been deleted since is the
/// pane's own "Source file not found" rather than a tab that quietly went away.
///
/// The rows go with the tabs they belong to, which is the whole reason they are
/// fields of one: the second and third tabs here are dropped, and a parallel array of
/// rows would have handed `b.o` the rows of a tab that vanished before it.
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
            SavedTab {
                asm_row: 5,
                src_row: 0,
                document: SavedDocument::Symbol {
                    path: PathBuf::from("/tmp/lib.a"),
                    object_name: "a.o".into(),
                    symbol_name: "gone".into(),
                    address: 12,
                },
            },
            saved_file_tab("/no/such/file.rs", 0, 9),
            saved_tab("b.o", 6),
        ],
        ..Session::new()
    };

    assert!(
        session.resolve_tabs(&objects)
            == [
                (tab(&objects[0]), 3, 0),
                (file_tab("/no/such/file.rs"), 0, 9),
                (tab(&objects[1]), 6, 0),
            ]
    );
}

/// Where each *side* of each tab was left goes out with the tab it belongs to, and a
/// side that was never scrolled is written as the top rather than left out.
///
/// Two rows and not one, because a tab has two sides: keying the source position by
/// the file — which is what the Source pane's own strip did — made two functions
/// compiled from one file share a position they have no reason to share.
#[test]
fn saves_the_rows_each_side_of_a_tab_was_left_at() {
    let objects = objects();
    let tabs = vec![tab(&objects[0]), tab(&objects[1])];

    let session = Session::from_state(
        &objects,
        &tabs,
        &positions(&[(&tabs[1], 42)]),
        &positions(&[(&tabs[0], 7)]),
        Some(&tabs[0]),
        &History::default(),
    );

    assert_eq!(
        session.tabs,
        [
            SavedTab {
                asm_row: 0,
                src_row: 7,
                document: saved_object("a.o"),
            },
            saved_tab("b.o", 42),
        ]
    );
}

/// The round trip the app actually makes: out of the two maps, through TOML, and back
/// into them the way `use_restore_on_startup` does it.
#[test]
fn the_rows_come_back_against_the_tabs_they_belong_to() {
    let objects = objects();
    let tabs = vec![tab(&objects[0]), tab(&objects[1])];

    let session = Session::from_state(
        &objects,
        &tabs,
        &positions(&[(&tabs[0], 12), (&tabs[1], 900)]),
        &positions(&[(&tabs[1], 4)]),
        Some(&tabs[0]),
        &History::default(),
    );
    let session: Session = toml::from_str(&round_trip(&session)).expect("reading back");

    let (mut asm, mut src): (Positions<Document>, Positions<Document>) =
        (Positions::default(), Positions::default());
    for (tab, asm_row, src_row) in session.resolve_tabs(&objects) {
        asm.remember(tab.clone(), asm_row);
        src.remember(tab, src_row);
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
            [tabs.document.Object]
            path = "/tmp/lib.a"
            object_name = "a.o"

            [[tabs]]
            [tabs.document.Source]
            path = "/src/main.rs"
        "#;
    let session: Session = toml::from_str(text).expect("deserializing");

    let objects = objects();
    assert!(
        session.resolve_tabs(&objects)
            == [(tab(&objects[0]), 0, 0), (file_tab("/src/main.rs"), 0, 0),]
    );
}

/// Nothing about a source file is resolved against this filesystem, on purpose:
/// the pane's own "Source file not found" is the right answer for one that has been
/// deleted, and dropping the tab would lose a file the reader had open without ever
/// saying so.
#[test]
fn a_source_file_that_is_no_longer_there_still_comes_back() {
    let path = "/no/such/directory/gone.rs";
    assert!(!Path::new(path).exists());

    let objects = objects();
    let tabs = [file_tab(path)];
    let session = Session::from_state(
        &objects,
        &tabs,
        &Positions::default(),
        &Positions::default(),
        Some(&tabs[0]),
        &History::default(),
    );

    assert_eq!(session.tabs, [saved_file_tab(path, 0, 0)]);
    assert_eq!(
        session.active,
        Some(SavedDocument::Source { path: path.into() })
    );
    assert!(session.resolve(&objects) == Some(file_tab(path)));
}

/// The field-order trap, which only a real serialization catches: a saved tab's two
/// rows are plain values and have to reach the file before the `document` sub-table
/// under them. A session with every field set at once is the one that fails when they
/// do not.
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
    let session = Session::from_state(
        &objects,
        &tabs,
        &positions(&[(&tabs[0], 12), (&tabs[1], 34)]),
        &positions(&[(&tabs[0], 56)]),
        Some(&tabs[1]),
        &history(&objects, 1),
    );

    let text = round_trip(&session);
    assert!(text.contains("[[tabs]]"), "{text}");
    assert!(text.contains("[tabs.document.Source]"), "{text}");

    // Inside a tab, the rows before the table its document is written as.
    let asm_row = text.find("asm_row = 12").expect("the first tab's row");
    let src_row = text
        .find("src_row = 56")
        .expect("the first tab's source row");
    let document = text
        .find("[tabs.document")
        .expect("the first tab's document");
    assert!(asm_row < document, "asm_row after its document\n{text}");
    assert!(src_row < document, "src_row after its document\n{text}");
}

#[test]
fn a_file_with_no_tabs_still_loads() {
    // The `serde(default)`s, from the other side: a hand-written or trimmed file is
    // a session with an empty strip rather than a load failure.
    let text = r#"
            binaries = ["/tmp/lib.a"]

            [active.Object]
            path = "/tmp/lib.a"
            object_name = "a.o"
        "#;
    let session: Session = toml::from_str(text).expect("deserializing");

    assert!(session.tabs.is_empty());

    // And the active document still restores, opening its own tab through `activate`
    // the way a session saved before there were tabs to save would.
    let objects = objects();
    assert!(session.resolve(&objects) == Some(tab(&objects[0])));
    assert!(session.resolve_tabs(&objects).is_empty());
}

// --- the binaries' digests ---------------------------------------------

/// The digest of the objects the fixtures are built from, as it is written down.
fn digest_of(bytes: &[u8]) -> String {
    analysis::FileDigest::of(bytes).to_string()
}

/// A saved session naming one binary at `row`, with the digest of `bytes` — what a
/// run that had this file open would have written.
fn saved_against(bytes: Option<&[u8]>, saved: SavedDocument, row: usize) -> Session {
    Session {
        digests: bytes
            .map(|bytes| BTreeMap::from([(PathBuf::from("/tmp/lib.a"), digest_of(bytes))]))
            .unwrap_or_default(),
        active: Some(saved.clone()),
        tabs: vec![SavedTab {
            asm_row: row,
            src_row: 0,
            document: saved.clone(),
        }],
        history: SavedHistory {
            cursor: 0,
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

/// One digest per *file*, however many objects came out of it: the members of an
/// archive share one `ObjectData` and so one hash, and the map is keyed by the path
/// the rest of the session is expressed in.
#[test]
fn saves_one_digest_per_binary_however_many_objects_it_holds() {
    let objects = objects();
    let session = from_state(&objects, None, &History::default());

    assert_eq!(binaries(&objects), vec![PathBuf::from("/tmp/lib.a")]);
    assert_eq!(
        session.digests,
        BTreeMap::from([(PathBuf::from("/tmp/lib.a"), digest_of(b"the first build"))])
    );
}

/// The file is the one the session was saved against, so the saved address is a fact
/// about it: an exact match resolves, a symbol that is not where it was said to be
/// does not, and the row the tab was left at is still that tab's row. All of which is
/// what the app did before there were digests — an unchanged file changes nothing.
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
    assert_eq!(session.resolve_tabs(&objects)[0].1, 42);

    // The same name at an address it is not at. Nothing about this file explains
    // that, so it degrades exactly as it always has.
    let moved = saved_against(
        Some(b"the first build"),
        saved_symbol("a.o", "target", 999),
        42,
    );
    assert!(resolve_selection(&moved, &objects) == Some(Selection::Object(objects[0].clone())));
    assert!(moved.resolve_tabs(&objects).is_empty());
    assert!(moved.resolve_history(&objects).entries().is_empty());
}

/// The file has been rebuilt under the session. The name is what the reader meant, so
/// a symbol that has merely moved comes back — where an unchanged file would have
/// dropped it — and the saved row goes, because it named a row of a listing this
/// build no longer has.
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
    assert!(session.resolve_tabs(&objects) == [(document.clone(), 0, 0)]);
    assert!(session.resolve_history(&objects).entries() == [document]);
}

/// The half that is a refusal rather than a recovery: two symbols of one name in a
/// rebuilt object, and a saved address that is now neither of theirs. The address is
/// the only thing that could choose between them and it describes a layout this file
/// no longer has, so nothing is chosen — landing the reader on a function they never
/// opened is the failure this whole step exists to stop.
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

/// A session that never wrote a digest — a hand-edited file, or one saved before
/// there were any — says nothing about the bytes, so nothing new is done with it.
/// "Not known to be unchanged" is not "known to have changed".
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

    assert_eq!(session.resolve_tabs(&objects)[0].1, 42);
}

/// The digests are a TOML *table*, and a hex string is what it holds, since a `u64`
/// digest does not fit TOML's signed integers at all.
#[test]
fn the_digests_round_trip_through_toml() {
    let objects = objects();
    let tabs = vec![tab(&objects[0]), file_tab("/src/main.rs")];
    let session = Session::from_state(
        &objects,
        &tabs,
        &positions(&[(&tabs[0], 12)]),
        &Positions::default(),
        Some(&tabs[0]),
        &history(&objects, 1),
    );

    let text = round_trip(&session);
    assert!(text.contains("[digests]"), "{text}");
    assert!(
        text.contains(&format!("{}\"", digest_of(b"the first build"))),
        "{text}"
    );
}

/// And a file with no digests at all still loads, the way one with no tabs does.
#[test]
fn a_file_with_no_digests_still_loads() {
    let text = r#"
            binaries = ["/tmp/lib.a"]

            [active.Object]
            path = "/tmp/lib.a"
            object_name = "a.o"
        "#;
    let session: Session = toml::from_str(text).expect("deserializing");
    assert!(session.digests.is_empty());

    let objects = objects();
    assert!(resolve_selection(&session, &objects) == Some(Selection::Object(objects[0].clone())));
}

// --- the save policy ---------------------------------------------------

fn paths(binaries: &[&str]) -> Vec<PathBuf> {
    binaries.iter().map(PathBuf::from).collect()
}

fn session_with(selection: Option<&str>) -> Session {
    Session {
        active: selection.map(saved_object),
        history: SavedHistory::new(),
        ..Session::new()
    }
}

/// What `record` hands back to be written: the project half, the session half where
/// one is owed, or nothing at all.
///
/// The details handed in are the ones the project already has, so every test below
/// that uses this is asking about a change to the binaries or the session and nothing
/// else — which is what they were all asking before a rename could reach `record` at
/// all. The rename tests spell theirs out.
fn recorded(
    saves: &mut Saves,
    binaries: Vec<PathBuf>,
    session: Session,
) -> Option<(Project, Option<Session>)> {
    let unchanged = saves.given.clone();
    saves
        .record(unchanged, binaries, session)
        .map(|written| (written.project, written.session))
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
    // The save observer runs once on mount, before anything is restored, and this
    // is what it records. Nothing may come of it: the files on disk are the good
    // ones — and no project directory is allocated either, since only a write
    // allocates one.
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
            },
            Some(session_with(None)),
        ))
    );
    // And is not written a second time by the next flush.
    assert_eq!(saves.flush(), None);
}

/// Closing one takes the same path opening one does, which is the whole of what
/// makes 6d's "the save is immediate" true: `binaries` is what `record` looks at,
/// and it does not care in which direction the list changed.
#[test]
fn closing_a_binary_is_written_at_once() {
    let mut saves = Saves::new();
    written(&mut saves, &["/tmp/lib.a", "/tmp/some.dll"], Some("a.o"));

    // The selection is still pending from the open above; closing writes the lot,
    // so `session.toml` never names a place inside a binary `project.toml` has
    // already let go of.
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

/// Closing the last one is not "nothing changed": the empty project is a project,
/// and it has to reach the disk or the next run reopens what was just closed.
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

/// A tab is pending and not an immediate write, and nothing in `record` says so:
/// which file a field lives in is what decides it, and a tab lives in the session.
/// That is the answer wanted — `activate` opens a tab on the way to every selection
/// change, so an immediate write here would be one file per click.
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

/// And so is a source file, by the same route: the pane opens one whenever the
/// selection lands on a symbol with line info.
#[test]
fn opening_a_source_file_waits_for_the_flush() {
    let mut saves = Saves::new();
    written(&mut saves, &["/tmp/lib.a"], None);

    let mut session = session_with(None);
    session.tabs = vec![
        saved_file_tab("/src/main.rs", 0, 0),
        saved_file_tab("/src/lib.rs", 0, 0),
    ];
    session.active = Some(SavedDocument::Source {
        path: "/src/lib.rs".into(),
    });
    assert_eq!(
        recorded(&mut saves, paths(&["/tmp/lib.a"]), session.clone()),
        None
    );
    assert_eq!(saves.flush(), Some(session));
    assert_eq!(saves.flush(), None);
}

/// Closing a binary still writes at once, and now carries the tabs it closed with
/// it: they were pending, and `record` takes everything pending along with the
/// binaries change, so the session file never names a tab into a binary the project
/// file has already let go of.
#[test]
fn closing_a_binary_carries_the_tabs_it_closed_with_it() {
    let mut saves = Saves::new();
    let mut opened = session_with(Some("a.o"));
    opened.tabs = vec![saved_tab("a.o", 0)];
    recorded(&mut saves, paths(&["/tmp/lib.a", "/tmp/some.dll"]), opened);

    let closed = session_with(None);
    assert_eq!(
        recorded(&mut saves, paths(&["/tmp/lib.a"]), closed.clone()),
        Some((
            Project {
                binaries: paths(&["/tmp/lib.a"]),
                ..Project::default()
            },
            Some(closed)
        ))
    );
    assert_eq!(saves.flush(), None);
}

/// The name and the directory survive a record that is not about them: they are the
/// baseline, so a record handing back the same ones is handing back "unchanged" and
/// the write carries them rather than the absence a derived project would have.
#[test]
fn a_record_keeps_the_name_the_project_was_given() {
    let mut saves = Saves::new();
    let named = Project {
        name: Some("kernel".into()),
        directory: Some(PathBuf::from("/src/kernel")),
        binaries: paths(&["/tmp/vmlinux"]),
    };
    saves.opened(ProjectId::new("kernel-1").expect("an id"), &named);

    let (project, _) = written(&mut saves, &["/tmp/lib.a"], None).expect("a write");
    assert_eq!(project.name.as_deref(), Some("kernel"));
    assert_eq!(project.directory, Some(PathBuf::from("/src/kernel")));
    // And the binaries are the ones the app is showing, not the ones it was opened
    // with: that half *is* derived, on every record.
    assert_eq!(project.binaries, paths(&["/tmp/lib.a"]));
}

/// The other half of that: a reopen seeds the *name* and not the contents. Both are
/// the same rule — a baseline is the state the app boots into — applied to two fields
/// that are restored at different moments. The name is put on screen synchronously,
/// so the baseline holds it; the binaries arrive when a worker thread has finished
/// parsing them, so a baseline holding them would read the still-empty boot state as a
/// change and write an empty project over a good one.
#[test]
fn reopening_seeds_the_name_but_not_the_baseline() {
    let mut saves = Saves::new();
    let loaded = Project {
        name: Some("kernel".into()),
        directory: None,
        binaries: paths(&["/tmp/vmlinux"]),
    };
    saves.opened(ProjectId::new("kernel-1").expect("an id"), &loaded);

    // The boot state, recorded by the observer's first run: it equals the baseline,
    // so nothing is written and the good files are left alone.
    assert_eq!(recorded(&mut saves, Vec::new(), Session::new()), None);
    // And the restore that follows is an ordinary change, written at once.
    let (project, _) =
        recorded(&mut saves, paths(&["/tmp/vmlinux"]), Session::new()).expect("a write");
    assert_eq!(project, loaded);
}

/// Naming a project is a user project change, so it is on disk before the next
/// click — and it is a `project.toml` write and nothing else, since a rename lets go
/// of no binary and so cannot leave the two files disagreeing.
#[test]
fn a_rename_is_written_at_once_and_leaves_the_session_pending() {
    let mut saves = Saves::new();
    written(&mut saves, &["/tmp/lib.a"], None);
    // A selection, pending as ever.
    written(&mut saves, &["/tmp/lib.a"], Some("a.o"));

    let named = Details {
        name: Some("kernel".into()),
        directory: Some(PathBuf::from("/src/kernel")),
    };
    let written = saves
        .record(
            named.clone(),
            paths(&["/tmp/lib.a"]),
            session_with(Some("a.o")),
        )
        .expect("a write");
    assert_eq!(
        written.project,
        Project {
            name: named.name.clone(),
            directory: named.directory.clone(),
            binaries: paths(&["/tmp/lib.a"]),
        }
    );
    // The session was not owed and so was not written — and is still pending, which
    // is the half that says the rename did not quietly take it along.
    assert_eq!(written.session, None);
    assert_eq!(saves.flush(), Some(session_with(Some("a.o"))));

    // And the same name recorded again is not a second write: `given` is a baseline
    // like the binaries, so a re-render costs nothing.
    assert_eq!(
        saves.record(named, paths(&["/tmp/lib.a"]), session_with(Some("a.o"))),
        None
    );
}

/// Clearing a name is a change like any other, and writes the key away rather than
/// leaving the old one on disk.
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
        .record(Details::new(), Vec::new(), Session::new())
        .expect("a write");
    assert_eq!(written.project.name, None);
    assert_eq!(written.session, None);
}

/// A rename while the binaries are still being parsed — or after a restore that
/// opened none of them at all — writes back the list the file already holds.
///
/// The app holds no binary in that window and deliberately writes nothing to say so,
/// since a file that is only temporarily missing must not be forgotten. A rename is
/// an immediate write all the same, and writing the app's own empty list would forget
/// them through a change that had nothing to do with them.
#[test]
fn a_rename_before_the_binaries_have_loaded_does_not_forget_them() {
    let mut saves = Saves::new();
    let loaded = Project {
        name: None,
        directory: None,
        binaries: paths(&["/tmp/vmlinux", "/tmp/lib.a"]),
    };
    saves.opened(ProjectId::new("kernel-1").expect("an id"), &loaded);

    let named = Details {
        name: Some("kernel".into()),
        directory: None,
    };
    let written = saves
        .record(named, Vec::new(), Session::new())
        .expect("a write");
    assert_eq!(written.project.name.as_deref(), Some("kernel"));
    assert_eq!(written.project.binaries, loaded.binaries);

    // And once the parse lands, the binaries are the app's own again: that write *is*
    // about them, so it is the one kind that may replace the list.
    let written = saves
        .record(
            saves.given.clone(),
            paths(&["/tmp/vmlinux"]),
            Session::new(),
        )
        .expect("a write");
    assert_eq!(written.project.binaries, paths(&["/tmp/vmlinux"]));
    // Closing the last one is still a real change and still empties the file.
    let written = recorded(&mut saves, Vec::new(), Session::new()).expect("a write");
    assert_eq!(written.0.binaries, Vec::<PathBuf>::new());
}

/// Entering another project empties every baseline, because the app is about to be
/// emptied of everything that belonged to the last one. A baseline still describing
/// those binaries would read that emptying as a change and write it into the project
/// just entered.
#[test]
fn entering_a_project_empties_every_baseline() {
    let mut saves = Saves::new();
    written(&mut saves, &["/tmp/lib.a"], Some("a.o"));

    let entered = Project {
        name: Some("other".into()),
        ..Project::default()
    };
    saves.opened(ProjectId::new("other-2").expect("an id"), &entered);

    // The state a switch leaves the app in: nothing open, nothing selected, and the
    // name of the project just entered. Every one of those is the baseline, so
    // nothing is written into the new project before its own restore has run.
    assert_eq!(
        saves.record(entered.details(), Vec::new(), Session::new()),
        None
    );
    // Nor is the old project's pending session waiting to be written into the new
    // one: `switch` flushed it, and entering dropped whatever was left.
    assert_eq!(saves.flush(), None);
}

// --- the two files, the ids and the recent list -------------------------

/// A directory of this test's own under the system temporary directory, named after
/// the line that asked for it, exactly as the file tests above are.
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
    }
}

/// The field-order trap for the project half. Everything in it is a plain value
/// today, which is exactly the kind of thing that stops being true when a field is
/// added, so it is asserted against a real serializer rather than read off the struct.
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

/// Anonymous is an *absent* key, the way an unspecified font is in `settings.rs`:
/// it is what makes the project anonymous, so it must not be spelt as an empty name
/// that a later reader could mistake for one the user chose.
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

/// The whole of the split, seen from the disk: each half in its own file, neither
/// holding a word of the other's.
#[test]
fn the_two_halves_are_written_to_their_own_files() {
    let directory = directory(line!());
    let project = a_project();
    let session = Session {
        active: Some(saved_object("a.o")),
        tabs: vec![saved_tab("a.o", 7)],
        ..Session::new()
    };

    project
        .save_to(&directory.join(PROJECT_FILE))
        .expect("saving the project");
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
    assert_eq!(
        Session::load_from(&directory.join(SESSION_FILE)),
        Some(session)
    );

    let _ = fs::remove_dir_all(&directory);
}

/// The reason the split is worth two files rather than two tables: the half the app
/// rewrites every thirty seconds cannot take the half the user gave down with it.
#[test]
fn a_corrupt_session_leaves_the_project_readable() {
    let directory = directory(line!());
    let project = a_project();
    project
        .save_to(&directory.join(PROJECT_FILE))
        .expect("saving the project");
    fs::write(directory.join(SESSION_FILE), b"{ not toml").expect("writing the corrupt half");

    assert_eq!(
        Project::load_from(&directory.join(PROJECT_FILE)),
        Some(project)
    );
    assert_eq!(Session::load_from(&directory.join(SESSION_FILE)), None);

    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn a_missing_or_corrupt_file_is_none() {
    let directory = directory(line!());
    let path = directory.join(SESSION_FILE);

    assert_eq!(Session::load_from(&path), None);
    assert_eq!(Project::load_from(&directory.join(PROJECT_FILE)), None);

    fs::create_dir_all(&directory).expect("creating the test directory");
    fs::write(&path, b"{ not toml").expect("writing the corrupt file");
    assert_eq!(Session::load_from(&path), None);

    let _ = fs::remove_dir_all(&directory);
}

/// An id is interpolated into a path, so what it may be is the whole of what keeps
/// `recents.toml` from naming somewhere else on the disk.
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

/// Deserializing goes through the same check, so an id out of a hand-edited file
/// cannot be a path — and a file holding one is a corrupt file, which is the default.
#[test]
fn a_hand_edited_recent_that_is_not_an_id_is_refused() {
    assert!(toml::from_str::<Recents>(r#"projects = ["../elsewhere"]"#).is_err());
    assert!(toml::from_str::<Recents>(r#"projects = ["project-1"]"#).is_ok());

    let directory = directory(line!());
    let path = directory.join(RECENTS_FILE);
    fs::create_dir_all(&directory).expect("creating the test directory");
    fs::write(&path, br#"projects = ["../elsewhere"]"#).expect("writing");
    assert_eq!(Recents::load_from(&path), Recents::default());

    let _ = fs::remove_dir_all(&directory);
}

fn id(text: &str) -> ProjectId {
    ProjectId::new(text).expect("an id")
}

/// The order *is* the answer to "which project was last open", so touching the one
/// already at the front changes nothing — which is what keeps a startup that reopens
/// it from writing a file to say so.
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

/// Bounded, because this file is appended to for as long as the app is ever used.
/// What falls off the end is a place in the order and never a project: every one of
/// them is still a directory.
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
    assert_eq!(
        Recents::load_from(Path::new("/no/such/recents.toml")),
        Recents::default()
    );
}

/// The claim is the `create_dir`, so two allocations in the same directory cannot
/// land on the same name however the numbers are counted — and the directory is what
/// makes the id survive a restart.
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

/// The whole of "a project appears when there is something to put in it": nothing on
/// disk until the first write, then a directory, and the recent list pointing at it.
#[test]
fn the_first_write_creates_a_project_and_remembers_it() {
    let base = directory(line!());
    let mut saves = Saves::new();

    let id = open_project(&mut saves, &base).expect("a project");
    assert!(project_in(&base, &id).is_dir());
    assert_eq!(
        Recents::load_from(&recents_in(&base)).projects,
        vec![id.clone()]
    );

    // Every later write of the run goes into the same one rather than allocating
    // another, which is what makes the id the run's identity and not the write's.
    assert_eq!(open_project(&mut saves, &base), Some(id));

    let _ = fs::remove_dir_all(&base);
}

/// Startup: the front of the recent list, both halves of it, through the same
/// defaulting every other read here uses.
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
        project
            .save_to(&project_in(&base, &id).join(PROJECT_FILE))
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

    // A recent list naming a project whose directory has gone — deleted by hand, or
    // on another machine the state directory is synced from.
    remember(&base, &id("gone-1"));
    assert!(reopen_in(&base).is_none());

    let _ = fs::remove_dir_all(&base);
}

/// The directory *is* the project, so a run that was killed between creating one and
/// writing either file into it reopens as the empty project it is — rather than being
/// orphaned while a second one is allocated beside it. A corrupt session is the same
/// answer for the same reason, and this is the split earning its keep: the half the
/// app rewrites every thirty seconds cannot take the other half down with it.
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
    project
        .save_to(&project_in(&base, &id).join(PROJECT_FILE))
        .expect("saving the project");
    fs::write(project_in(&base, &id).join(SESSION_FILE), b"{ not toml")
        .expect("writing the corrupt half");

    let (_, reopened, session) = reopen_in(&base).expect("a project to reopen");
    assert_eq!(reopened, project);
    assert_eq!(session, Session::new());

    let _ = fs::remove_dir_all(&base);
}

/// The recent-projects view reads each row's name out of that project's own file,
/// in the order the list keeps, and not out of the list — which holds ids and nothing
/// else precisely so there is one copy of a name.
#[test]
fn the_recent_view_names_each_project_from_its_own_file() {
    let base = directory(line!());
    for (id, name) in [("first-1", "kernel"), ("second-2", "loader")] {
        let id = self::id(id);
        Project {
            name: Some(name.to_owned()),
            directory: Some(PathBuf::from("/src").join(name)),
            binaries: paths(&["/tmp/lib.a", "/tmp/some.dll"]),
        }
        .save_to(&project_in(&base, &id).join(PROJECT_FILE))
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

/// Two things that are not errors and not empty rows. A project whose directory has
/// gone is dropped here — the list never prunes itself on load, and this is the point
/// of use where the repair is free — while one that has a directory and no readable
/// file at all is a real project the reader can be put into, so it keeps its row and
/// describes itself as the empty project it is.
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

/// Anonymity is the missing name and not the shape of the id: a project the reader
/// later names keeps the directory it was allocated.
#[test]
fn an_anonymous_id_says_nothing_about_the_name() {
    let directory = directory(line!());
    let id = ProjectId::anonymous(&directory).expect("an id");

    let named = Project {
        name: Some("kernel".into()),
        ..Project::default()
    };
    let path = directory.join(id.as_str()).join(PROJECT_FILE);
    named.save_to(&path).expect("saving");
    assert_eq!(Project::load_from(&path), Some(named));

    let _ = fs::remove_dir_all(&directory);
}
