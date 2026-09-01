use super::*;

/// A directory of this test's own under the system temporary directory, named after
/// the line that asked for it, exactly as `project.rs`'s file tests are.
fn directory(line: u32) -> PathBuf {
    std::env::temp_dir().join(format!(
        "assembly-viewer-settings-test-{}-{line}",
        std::process::id()
    ))
}

/// Everything spelled out, so a round trip has something to lose.
fn settings() -> Settings {
    Settings {
        theme: Theme::Dark,
        interface: FontSetting {
            family: Some("Cantarell".into()),
            size: Some(11.0),
        },
        fixed: FontSetting {
            family: Some("Fira Code".into()),
            size: Some(10.5),
        },
    }
}

#[test]
fn no_file_at_all_is_the_default() {
    let path = directory(line!()).join(FILE_NAME);
    assert_eq!(Settings::load_from(&path), Settings::default());

    // And the default is "the user has said nothing", not a set of values.
    let default = Settings::default();
    assert_eq!(default.theme, Theme::Desktop);
    assert_eq!(default.interface.family(), None);
    assert_eq!(default.interface.size(), None);
    assert_eq!(default.fixed.family(), None);
    assert_eq!(default.fixed.size(), None);
}

#[test]
fn a_corrupt_file_is_the_default() {
    let directory = directory(line!());
    let path = directory.join(FILE_NAME);

    fs::create_dir_all(&directory).expect("creating the test directory");

    // Not TOML at all.
    fs::write(&path, b"{ not toml").expect("writing the corrupt file");
    assert_eq!(Settings::load_from(&path), Settings::default());

    // TOML, but not this schema: a theme this app has never heard of is a file it
    // cannot honour, and starting in the default beats refusing to start.
    fs::write(&path, b"theme = \"solarized\"\n").expect("writing the stale file");
    assert_eq!(Settings::load_from(&path), Settings::default());

    let _ = fs::remove_dir_all(&directory);
}

/// A partial file is not a corrupt one: this is the `serde(default)` on the container
/// earning its place, since a settings file is a file someone may write by hand.
#[test]
fn a_file_that_names_one_setting_keeps_it() {
    let directory = directory(line!());
    let path = directory.join(FILE_NAME);

    fs::create_dir_all(&directory).expect("creating the test directory");
    fs::write(&path, b"theme = \"light\"\n").expect("writing the partial file");

    let loaded = Settings::load_from(&path);
    assert_eq!(loaded.theme, Theme::Light);
    assert_eq!(loaded.interface, FontSetting::default());

    let _ = fs::remove_dir_all(&directory);
}

/// The field-order test. TOML cannot reopen a table, so `theme` has to be written
/// before `[interface]` — and that is a runtime failure, which is why it is asserted
/// against a real serializer and a real file rather than reasoned about.
#[test]
fn writes_atomically_and_reads_back_with_its_tables_last() {
    let directory = directory(line!());
    let path = directory.join("nested").join(FILE_NAME);

    let settings = settings();
    settings.save_to(&path).expect("saving");

    let text = fs::read_to_string(&path).expect("reading it back");
    let theme = text.find("theme").expect("the theme");
    let interface = text.find("[interface]").expect("the interface table");
    let fixed = text.find("[fixed]").expect("the fixed table");
    assert!(theme < interface && interface < fixed, "{text}");

    assert_eq!(Settings::load_from(&path), settings);
    // The temporary was renamed, not left behind.
    assert!(!path.with_extension("toml.tmp").exists());

    let _ = fs::remove_dir_all(&directory);
}

/// Unspecified is not the desktop's answer, and the file is where that has to
/// survive: a font nobody has chosen is a key that is *absent*, so nothing can later
/// mistake it for a value that was chosen and happened to match.
#[test]
fn unspecified_is_an_absent_key_and_not_an_empty_one() {
    let text = toml::to_string_pretty(&Settings::default()).expect("writing the default");
    assert!(!text.contains("family"), "{text}");
    assert!(!text.contains("size"), "{text}");

    // The same font written down explicitly — as a settings page that filled its
    // boxes with what the desktop answered would write it — is a different file and
    // a different value, and stays one across the round trip.
    let chosen = Settings {
        interface: FontSetting {
            family: Some("Cantarell".into()),
            size: Some(11.0),
        },
        ..Settings::default()
    };
    let text = toml::to_string_pretty(&chosen).expect("writing the chosen font");
    assert!(text.contains("family = \"Cantarell\""), "{text}");
    assert_eq!(toml::from_str::<Settings>(&text).ok(), Some(chosen));
}

/// The other end of the same distinction: a value that is present but says nothing is
/// judged at the accessor, so a box the reader emptied is not a font named "".
#[test]
fn a_family_of_nothing_is_not_a_family() {
    let empty = FontSetting {
        family: Some("   ".into()),
        size: Some(0.0),
    };
    assert_eq!(empty.family(), None);
    assert_eq!(empty.size(), None);

    // And a real one is taken, trimmed.
    let named = FontSetting {
        family: Some(" Fira Code ".into()),
        size: Some(10.5),
    };
    assert_eq!(named.family(), Some("Fira Code"));
    assert_eq!(named.size(), Some(10.5));
}

#[test]
fn the_theme_choice_is_a_plain_string() {
    let text = toml::to_string_pretty(&Settings::default()).expect("writing the default");
    assert!(text.contains("theme = \"desktop\""), "{text}");

    for (choice, spelling) in [
        (Theme::Desktop, "desktop"),
        (Theme::Light, "light"),
        (Theme::Dark, "dark"),
    ] {
        let settings = Settings {
            theme: choice,
            ..Settings::default()
        };
        let text = toml::to_string_pretty(&settings).expect("writing the theme");
        assert!(text.contains(&format!("theme = \"{spelling}\"")), "{text}");
        assert_eq!(toml::from_str::<Settings>(&text).ok(), Some(settings));
    }
}
