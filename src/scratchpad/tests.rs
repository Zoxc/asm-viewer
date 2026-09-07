use super::*;
use crate::store::MAX_ORDER;
use crate::temporary::Temporary;

/// A directory of this test's own under the system temporary directory, named after the
/// line that asked for it, and gone when the test ends.
fn directory(line: u32) -> Temporary {
    Temporary::at(std::env::temp_dir().join(format!(
        "assembly-viewer-scratchpad-test-{}-{line}",
        std::process::id()
    )))
}

fn scratchpad() -> Scratchpad {
    Scratchpad::new("sketch").expect("an id")
}

/// A row on its own, for the checks that are about one row and not about a list.
/// `NO_ROW` because nothing here looks it up.
fn dependency(name: impl Into<String>, version: impl Into<String>) -> Dependency {
    Dependency {
        id: NO_ROW,
        name: name.into(),
        version: version.into(),
    }
}

/// Rows on a scratchpad, and the ids they were handed.
fn dependencies<N: Into<String>, V: Into<String>>(
    scratchpad: &mut Scratchpad,
    rows: impl IntoIterator<Item = (N, V)>,
) -> Vec<RowId> {
    rows.into_iter()
        .map(|(name, version)| scratchpad.add_dependency(name, version))
        .collect()
}

/// The whole generated manifest, asserted as text rather than as a value: the field order
/// rule is a property of the *serializer*, and a round trip through a struct would not see
/// it. `[workspace]` being emitted at all is here for the same reason.
#[test]
fn a_package_is_a_manifest_and_a_main() {
    let mut scratchpad = scratchpad();
    // What the reader calls it is under `[package.metadata]`, the one place cargo lets a
    // tool of its own keep anything -- and it is under no obligation to be a crate name,
    // where `[package] name`, which is the id, is. Untrimmed on purpose, like the rows.
    scratchpad.name = " a name with spaces ".to_owned();
    // Out of order and untrimmed on purpose: the manifest sorts and trims, the list does
    // not.
    dependencies(&mut scratchpad, [("rand", "0.8"), (" anyhow ", " 1.0.86 ")]);

    assert_eq!(
        scratchpad.manifest().expect("a manifest"),
        "\
[package]
name = \"sketch\"
version = \"0.1.0\"
edition = \"2021\"

[package.metadata.scratchpad]
name = \"a name with spaces\"

[dependencies]
anyhow = \"1.0.86\"
rand = \"0.8\"

[workspace]
"
    );
}

/// The empty case is the one that actually ships: no `[dependencies]` header at all rather
/// than an empty one.
#[test]
fn a_scratchpad_with_no_crates_has_no_dependencies_table() {
    let manifest = scratchpad().manifest().expect("a manifest");
    assert!(!manifest.contains("[dependencies]"), "{manifest}");
}

#[test]
fn a_row_that_is_not_a_crate_name_says_which_row() {
    let mut scratchpad = scratchpad();
    let rows = dependencies(
        &mut scratchpad,
        [
            ("serde".to_owned(), "1"),
            (String::new(), "1"),
            ("1password".to_owned(), "1"),
            ("hello world".to_owned(), "1"),
            ("a".repeat(MAX_NAME + 1), "1"),
        ],
    );

    assert_eq!(
        scratchpad.problems(),
        vec![
            (rows[1], Problem::NoName),
            (rows[2], Problem::NameStart),
            (rows[3], Problem::NameCharacter(' ')),
            (rows[4], Problem::NameTooLong),
        ]
    );
}

#[test]
fn a_version_that_is_not_a_version_says_so() {
    for good in [
        "1",
        "1.2",
        "1.2.3",
        "^1.2.3",
        "~1.2",
        "=1.2.3",
        ">=1.2, <2.0",
        "1.0.0-rc.1",
        "1.0.0-alpha+build.5",
        " 1.0 ",
    ] {
        assert_eq!(dependency("serde", good).check(), Ok(()), "{good}");
    }

    for (bad, problem) in [
        ("", Problem::NoVersion),
        ("   ", Problem::NoVersion),
        // The whole point of the requirement, and its own answer.
        ("*", Problem::Wildcard),
        ("1.*", Problem::Wildcard),
        (">=1, <2.*", Problem::Wildcard),
        ("latest", Problem::NotAVersion),
        ("v1.2", Problem::NotAVersion),
        ("1.2.3.4", Problem::NotAVersion),
        ("1..2", Problem::NotAVersion),
        ("1.2-", Problem::NotAVersion),
        ("1.2-rc/1", Problem::NotAVersion),
        (">=1,", Problem::NotAVersion),
    ] {
        assert_eq!(dependency("serde", bad).check(), Err(problem), "{bad:?}");
    }
}

/// A table cannot hold a key twice, so the second row would silently win.
#[test]
fn the_same_crate_twice_is_a_row_that_says_so() {
    let mut scratchpad = scratchpad();
    // A second empty row is empty, not a duplicate: it has nothing to duplicate.
    let rows = dependencies(
        &mut scratchpad,
        [("serde", "1"), (" serde ", "2"), ("", ""), ("", "")],
    );

    assert_eq!(
        scratchpad.problems(),
        vec![
            (rows[1], Problem::Repeated),
            (rows[2], Problem::NoName),
            (rows[3], Problem::NoName),
        ]
    );
}

#[test]
fn a_scratchpad_with_a_bad_row_will_not_write() {
    let directory = directory(line!());
    let mut scratchpad = scratchpad();
    let row = scratchpad.add_dependency("rand", "");

    let failure = scratchpad.write_to(&directory).expect_err("a refusal");
    assert_eq!(
        failure,
        Failure::Dependencies(vec![(row, Problem::NoVersion)])
    );
    // And nothing was written on the way to refusing.
    assert!(!directory.exists());

    // A build refuses in the same terms rather than in cargo's.
    assert_eq!(scratchpad.build_in(&directory), Build::Unavailable(failure));
    assert!(!directory.exists());
}

/// The package is the storage, so this is the whole of the persistence test.
#[test]
fn writes_and_reads_back() {
    let directory = directory(line!());
    let mut scratchpad = scratchpad();
    scratchpad.source = "fn main() { /* edited */ }\n".to_owned();
    let anyhow = scratchpad.add_dependency("anyhow", "1.0.86");
    // A name nothing could file a pad under: it is a value in the package and not the
    // directory, so it may hold spaces, punctuation and any alphabet at all.
    scratchpad.name = "Sam's ✎ notes".to_owned();

    scratchpad.write_to(&directory).expect("writing");
    assert_eq!(Scratchpad::load_from(&directory), Some(scratchpad.clone()));

    // The temporaries were renamed, not left behind.
    assert!(!directory.join("Cargo.toml.tmp").exists());
    assert!(!directory.join("src").join("main.rs.tmp").exists());

    // Writing again replaces rather than merges -- the name included, a rename being an
    // ordinary edit now that nothing is filed under it.
    scratchpad.source = "fn main() {}\n".to_owned();
    scratchpad.name = "renamed".to_owned();
    scratchpad.remove_dependency(anyhow);
    scratchpad.add_dependency("rand", "0.8");
    scratchpad.write_to(&directory).expect("writing again");
    assert_eq!(Scratchpad::load_from(&directory), Some(scratchpad));

    // A directory with nothing in it is not a scratchpad, and neither is one with a
    // manifest and no source.
    assert_eq!(Scratchpad::load_from(&directory.join("src")), None);
    fs::remove_file(directory.join("src").join("main.rs")).expect("removing the source");
    assert_eq!(Scratchpad::load_from(&directory), None);
}

/// An id is the directory a pad lives in, so it is read back through the same check the app
/// generated it through. A file naming something that is not one is refused rather than
/// interpolated into a path.
#[test]
fn an_id_out_of_a_file_goes_through_the_same_check_a_generated_one_does() {
    assert_eq!(
        PadId::new("sketch").map(|id| id.as_str().to_owned()),
        Some("sketch".to_owned())
    );
    // Not trimmed, unlike a name: nobody types an id, so there is no stray space to
    // forgive, and a path component with a space at either end is a different directory.
    assert_eq!(PadId::new("  sketch  "), None);
    assert_eq!(PadId::new(""), None);
    assert_eq!(PadId::new("9lives"), None);
    assert_eq!(PadId::new("a/b"), None);
    assert_eq!(PadId::new(".."), None);

    // And the same through serde, which is the path a hand-edited file takes.
    let read = |text: &str| toml::from_str::<BTreeMap<String, PadId>>(text);
    assert_eq!(
        read("name = \"sketch\"").expect("an id")["name"].as_str(),
        "sketch"
    );
    assert!(read("name = \"../evil\"").is_err());
}

/// A directory whose manifest names something that could not be a directory is not a
/// scratchpad. That is the same sentence the pad listing is built on: a directory
/// [`Scratchpad::load_from`] answers for is a pad, anything else is not — and the crate's
/// name is the id, so this is where a hand-edited one is caught.
#[test]
fn a_manifest_naming_a_path_is_not_a_scratchpad() {
    let directory = directory(line!());
    let source = directory.join("src");
    fs::create_dir_all(&source).expect("the directory");
    fs::write(
        directory.join("Cargo.toml"),
        "[package]\nname = \"../evil\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("the manifest");
    fs::write(source.join("main.rs"), "fn main() {}\n").expect("the source");

    assert_eq!(Scratchpad::load_from(&directory), None);
}

/// The reason [`Scratchpad::default`] may hand out an id without an `Option`: it is an id
/// this module would generate, and a package it would agree to write. It carries no name at
/// all, a pad nobody has named having an empty one and what stands in for it on screen
/// being the UI's business.
#[test]
fn the_default_scratchpad_is_one_this_module_would_write() {
    let scratchpad = Scratchpad::default();

    assert_eq!(scratchpad.id().as_str(), DEFAULT_ID);
    assert_eq!(scratchpad.name(), "");
    assert_eq!(Scratchpad::new(DEFAULT_ID), Some(scratchpad.clone()));
    assert!(scratchpad.problems().is_empty());
    assert!(scratchpad.manifest().is_ok());
}

/// Reopening: what is on disk wins over what the caller was holding, except for the name,
/// which is the directory the next write goes back to.
#[test]
fn a_scratchpad_opens_as_its_directory_has_it() {
    let directory = directory(line!());

    // Nothing there yet: what the caller was holding, unchanged.
    let fresh = Scratchpad::default().opened_in(&directory);
    assert_eq!(fresh, Ok(Scratchpad::default()));

    let mut written = scratchpad();
    written.source = "fn main() { /* saved */ }\n".to_owned();
    written.add_dependency("anyhow", "1.0.86");
    written.write_to(&directory).expect("writing");

    let opened = Scratchpad::default()
        .opened_in(&directory)
        .expect("a package this module wrote");
    assert_eq!(opened.source, written.source);
    assert_eq!(opened.dependencies(), written.dependencies());
    // The manifest's crate name says `sketch` and the caller asked for `scratch`: the
    // caller wins, because the id is where the next write goes. The *name* comes off the
    // disk like everything else, being a value and not a place.
    assert_eq!(opened.id().as_str(), DEFAULT_ID);
    assert_eq!(opened.name(), written.name());
}

/// A directory with a package in it this module cannot read is not an empty one. Answering
/// the caller's own scratchpad for it is what would put the default source and manifest
/// over the reader's own files on the next keystroke, and a dependency written by hand as a
/// table -- the ordinary way to ask for a feature -- is enough to bring it about.
#[test]
fn a_package_that_will_not_load_is_refused_and_not_read_as_an_empty_directory() {
    let directory = directory(line!());
    let source = directory.join("src");
    fs::create_dir_all(&source).expect("the directory");
    fs::write(source.join("main.rs"), "fn main() { /* kept */ }\n").expect("the source");

    // Only half a package: still not an empty directory.
    assert_eq!(
        Scratchpad::default().opened_in(&directory),
        Err(Failure::Unreadable)
    );

    fs::write(
        directory.join("Cargo.toml"),
        "[package]\nname = \"scratch\"\nedition = \"2021\"\n\n\
         [dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\n",
    )
    .expect("the manifest");

    assert_eq!(
        Scratchpad::default().opened_in(&directory),
        Err(Failure::Unreadable)
    );
}

/// An id for the tests below, which all deal in ids this module would generate.
fn id(text: &str) -> PadId {
    PadId::new(text).expect("an id")
}

/// What the listing says a pad is, for asserting against.
fn row(id_text: &str, name: &str) -> PadListing {
    PadListing {
        id: id(id_text),
        name: name.to_owned(),
    }
}

/// The cap is the **file**'s and not the list's. The panel draws this order and the listing
/// it is built from is every pad there is, so an order that dropped its own tail would
/// leave the pads past the fiftieth with no row to open them from. What the file keeps is
/// bounded all the same: an order is what falls off it, never a pad.
#[test]
fn the_order_keeps_every_pad_and_the_file_keeps_fifty() {
    // One past the cap, so the last of them is what a bounded order would lose.
    let listing: Vec<PadListing> = (0..=MAX_ORDER)
        .map(|n| row(&format!("pad-{n}"), ""))
        .collect();
    let mut order = PadOrder::of(&listing);
    assert_eq!(order.entries().len(), listing.len());

    // Showing one pad is not an occasion to drop another.
    assert!(order.touch(id("pad-9")));
    assert_eq!(order.entries().len(), listing.len());
    assert!(order.entries().contains(&id(&format!("pad-{MAX_ORDER}"))));

    let base = directory(line!());
    let store = Store::at(&base);
    let scratchpads = store.scratchpads();
    fs::create_dir_all(&scratchpads).expect("the directory");
    store
        .write_toml(pad_recents_in(&store), &order)
        .expect("the order");
    // An id with a directory, which is what `remember` asks of one before it writes.
    fs::create_dir(scratchpads.join("fresh")).expect("the directory");

    remember(&store, &id("fresh"));

    let written = load_order(&store);
    assert_eq!(written.entries().len(), MAX_ORDER);
    assert_eq!(written.first(), Some(&id("fresh")));
}

/// Every pad is reachable, which is the difference from the recent-projects list: that one
/// is the projects a reader has *opened*, where this is the scratchpads there are. So an id
/// the order has kept whose directory is not a package is dropped, and a package the order
/// has never heard of is listed anyway. Each row carries the name out of that pad's own
/// package, which is what lets the panel draw a pad nothing has opened.
#[test]
fn the_listing_drops_what_is_not_a_pad_and_keeps_what_the_order_forgot() {
    let base = directory(line!());
    let store = Store::at(&base);
    let scratchpads = store.scratchpads();
    fs::create_dir_all(&scratchpads).expect("the directory");

    for (pad, name) in [("kept", "Kept one"), ("stray", "")] {
        let mut scratchpad = Scratchpad::of(id(pad));
        scratchpad.name = name.to_owned();
        scratchpad
            .write_to(&scratchpads.join(pad))
            .expect("writing");
    }
    // A directory with nothing in it, which `load_from` does not answer for.
    fs::create_dir(scratchpads.join("empty")).expect("the directory");

    let mut order = PadOrder::default();
    order.touch(id("empty"));
    order.touch(id("gone"));
    order.touch(id("kept"));
    store
        .write_toml(pad_recents_in(&store), &order)
        .expect("the order");

    // `kept` from the order, then the pad the order never named. `gone` has no directory
    // and `empty` is not a package, so neither is a row. A pad the reader never named
    // comes back with an empty name rather than with its id.
    assert_eq!(pads(&store), [row("kept", "Kept one"), row("stray", "")]);
}

/// A new pad claims its directory with the `create_dir` that fails rather than opens, so an
/// id another copy of the app is already using is stepped over rather than taken. The
/// package goes in at once: a claimed directory with nothing in it is not a pad, and the
/// listing above would repair it away.
#[test]
fn a_new_pad_steps_over_what_is_already_claimed() {
    let base = directory(line!());
    let store = Store::at(&base);
    let scratchpads = store.scratchpads();
    fs::create_dir_all(scratchpads.join("pad-1")).expect("the squatter");

    let made = new_pad(&store).expect("a pad");
    assert_eq!(made.id().as_str(), "pad-2");
    // And no name: naming it is the reader's, and until they do the pane calls it
    // `<pad-2>` without anything having been written down.
    assert_eq!(made.name(), "");
    assert_eq!(
        Scratchpad::load_from(&scratchpads.join("pad-2")),
        Some(made.clone())
    );
    // And it is at the front of the order, so it is what a restart would open.
    assert_eq!(load_order(&store).first(), Some(made.id()));
}

/// A delete takes the pad's whole directory, cargo's leavings included — and reaches
/// nothing else. The path is the id's, and an id is a checked crate name, so the only way
/// left to aim a `remove_dir_all` at something that is not a pad is for the directory to
/// have stopped being one, which is what the load answers. The pad beside it is untouched,
/// which is the assertion that would fail if the path were ever built from anything but the
/// id.
#[test]
fn a_delete_takes_the_package_and_only_the_package() {
    let base = directory(line!());
    let store = Store::at(&base);
    let scratchpads = store.scratchpads();
    fs::create_dir_all(&scratchpads).expect("the directory");

    for pad in ["going", "staying"] {
        Scratchpad::of(id(pad))
            .write_to(&scratchpads.join(pad))
            .expect("writing");
    }
    // What cargo leaves behind, which goes with the pad rather than being left orphaned.
    fs::create_dir_all(scratchpads.join("going").join("target")).expect("the directory");

    assert_eq!(delete_pad(&store, &id("going")), Ok(()));
    assert!(!scratchpads.join("going").exists());
    assert!(Scratchpad::load_from(&scratchpads.join("staying")).is_some());

    // Gone already is not a failure: the pad a first run holds has no directory until
    // something is typed into it.
    assert_eq!(delete_pad(&store, &id("going")), Ok(()));

    // A directory that is not a package is refused rather than removed, whatever the order
    // beside it says about it.
    let stranger = scratchpads.join("stranger");
    fs::create_dir(&stranger).expect("the directory");
    fs::write(stranger.join("notes.txt"), "someone's own").expect("the file");
    assert!(matches!(
        delete_pad(&store, &id("stranger")),
        Err(Failure::Delete(_))
    ));
    assert!(stranger.join("notes.txt").exists());

    // A link where a pad's directory should be is refused as well: `symlink_metadata` does
    // not follow one, so a delete reaches the directory itself or nothing.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(scratchpads.join("staying"), scratchpads.join("linked"))
            .expect("a link");
        assert!(matches!(
            delete_pad(&store, &id("linked")),
            Err(Failure::Delete(_))
        ));
        assert!(Scratchpad::load_from(&scratchpads.join("staying")).is_some());
    }
}

/// Nothing to run is an answer and not a panic — the executable a build named can be gone
/// by the time the reader presses the button.
#[test]
fn a_program_that_is_not_there_says_so() {
    let directory = directory(line!());
    let failure = run_in(&directory.join("not-a-program"), &directory, |_| {})
        .err()
        .expect("a refusal");

    assert!(matches!(failure, Failure::NoProgram(_)), "{failure:?}");
}

/// The pad's source file is the two names its package is written from, so the rule and the
/// package cannot drift apart.
#[test]
fn the_source_file_is_the_two_names_it_is_made_of() {
    assert_eq!(SOURCE_FILE, format!("{SOURCE_DIR}/{SOURCE_NAME}"));
}

/// A diagnostic names the pad's own file relatively, and on Windows with the other
/// separator. Nothing above it and nothing beside it is that file.
#[test]
fn a_diagnostic_names_the_pads_own_file_by_itself() {
    assert!(is_source_file("src/main.rs"));
    assert!(is_source_file(r"src\main.rs"));

    // A dependency's, and the pad's own file named from somewhere else: a diagnostic in
    // the pad's file is spelled relatively and nothing else is it.
    assert!(!is_source_file("src/lib.rs"));
    assert!(!is_source_file("main.rs"));
    assert!(!is_source_file("/pads/pad-1/src/main.rs"));
}

/// A program spells it with everything the compiler's own directory put in front, which is
/// why this is a tail and the diagnostic's rule is not.
#[test]
fn a_program_names_the_pads_own_file_with_what_is_in_front_of_it() {
    assert!(ends_in_source_file("/pads/pad-1/src/main.rs"));
    assert!(ends_in_source_file(r"C:\pads\pad-1\src\main.rs"));
    // Still the pad's own where nothing is in front of it: a producer that recorded the
    // relative name and a unit with no `comp_dir`.
    assert!(ends_in_source_file("src/main.rs"));

    // The tail is two names and not one, so a `main.rs` in any other directory is not it.
    assert!(!ends_in_source_file("/pads/pad-1/main.rs"));
    assert!(!ends_in_source_file("/pads/pad-1/tests/main.rs"));
    assert!(!ends_in_source_file("/registry/anyhow-1.0/src/lib.rs"));
    // And it is the whole of the last name, not a prefix of it.
    assert!(!ends_in_source_file("/pads/pad-1/src/main.rs.bak"));
}

/// Which of a program's files is the pad's, out of what a real one names: the standard
/// library's, a crates.io dependency's, and the pad's own.
#[test]
fn the_pads_own_file_is_the_one_ending_in_it() {
    let files = [
        "/rustc/1.83.0/library/std/src/rt.rs",
        "/registry/anyhow-1.0.95/src/lib.rs",
        "/pads/pad-3/src/main.rs",
    ];
    assert_eq!(own_source(files), Some("/pads/pad-3/src/main.rs"));

    // A program naming none of them, and one naming nothing at all.
    assert_eq!(own_source(["/registry/anyhow-1.0.95/src/lib.rs"]), None);
    assert_eq!(own_source([]), None);
}

/// What a build was of is the source and the crates, and **not** the name: cargo compiles
/// nothing from `[package.metadata]`, so a rename must not make a program out of date.
#[test]
fn what_a_build_was_of_is_the_source_and_the_crates() {
    let mut pad = Scratchpad::new("pad-1").expect("a valid id");
    pad.source = "fn main() {}".to_owned();
    let built = pad.compiled();

    let mut renamed = pad.clone();
    renamed.name = "something else".to_owned();
    assert_eq!(renamed.compiled(), built, "a rename is not an edit");

    let mut edited = pad.clone();
    edited.source.push('\n');
    assert_ne!(edited.compiled(), built, "an edit is one");

    let mut crated = pad.clone();
    crated.add_dependency("rand", "0.8");
    assert_ne!(crated.compiled(), built, "a crate row is one too");
}

/// What a build made goes into the package, so a later run opens the pad on its program
/// rather than on nothing -- and comes back out of it exactly as it went in, `load_from`
/// being `write_to`'s inverse.
#[test]
fn what_the_last_build_made_is_written_and_read_back() {
    let directory = directory(line!());
    let mut pad = Scratchpad::new("pad-1").expect("a valid id");
    pad.built = Some(Built {
        path: PathBuf::from("/elsewhere/target/debug/pad-1"),
        digest: pad.compiled().digest(),
    });
    pad.write_to(&directory).expect("the package is written");

    let read = Scratchpad::load_from(&directory).expect("the package loads");
    assert_eq!(read.built, pad.built);

    // A pad nothing has built says nothing, rather than an empty table nobody reads.
    let fresh = Scratchpad::new("pad-2").expect("a valid id");
    fresh.write_to(&directory).expect("the package is written");
    assert_eq!(
        Scratchpad::load_from(&directory)
            .expect("the package loads")
            .built,
        None
    );
    assert!(
        !fresh.manifest().expect("a manifest").contains("built"),
        "a pad nothing has built writes an empty table"
    );
}

/// The digest is of what a build compiles and nothing else, and it is the written form the
/// package keeps: sixteen lowercase hex digits, compared as text.
#[test]
fn the_digest_says_what_a_build_was_of() {
    let mut pad = Scratchpad::new("pad-1").expect("a valid id");
    pad.source = "fn main() {}".to_owned();
    let digest = pad.compiled().digest();
    assert_eq!(digest.len(), 16);
    assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));

    let mut renamed = pad.clone();
    renamed.name = "something else".to_owned();
    assert_eq!(
        renamed.compiled().digest(),
        digest,
        "a rename is not an edit"
    );

    let mut edited = pad.clone();
    edited.source.push(' ');
    assert_ne!(edited.compiled().digest(), digest);

    // The rows are ended one by one, so two lists that would run together as one string
    // are still two.
    let mut one = pad.clone();
    dependencies(&mut one, [("ab", "1"), ("c", "2")]);
    let mut other = pad.clone();
    dependencies(&mut other, [("a", "bc"), ("1", "2")]);
    assert_ne!(one.compiled().digest(), other.compiled().digest());
}

/// **A row that has gone is still somewhere to write.** The boxes of a deleted row go on
/// taking events out of the press that deleted it, so the lookup answers with the spare
/// rather than not answering, and what lands there is out of the list's reach. Ids are
/// never handed out twice either, so the next row added is not the one that went.
#[test]
fn a_write_to_a_row_that_has_gone_lands_on_the_spare() {
    let mut scratchpad = scratchpad();
    let rows = dependencies(&mut scratchpad, [("rand", "0.8"), ("anyhow", "1.0.86")]);
    scratchpad.remove_dependency(rows[0]);

    scratchpad.dependency_mut(rows[0]).name = "left behind".to_owned();
    assert_eq!(scratchpad.dependencies(), [dependency("anyhow", "1.0.86")]);
    assert_eq!(scratchpad.dependency(rows[1]).name(), "anyhow");
    assert_ne!(scratchpad.add_dependency("serde", "1"), rows[0]);
}
