use std::fs;

use super::*;
use crate::temporary::Temporary;

/// A directory of this test's own under the system temporary directory, named after
/// the line that asked for it, and gone when the test ends.
fn directory(line: u32) -> Temporary {
    Temporary::at(std::env::temp_dir().join(format!(
        "assembly-viewer-settings-test-{}-{line}",
        std::process::id()
    )))
}

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
fn a_missing_or_corrupt_file_is_the_default() {
    let directory = directory(line!());
    let path = directory.join(FILE_NAME);

    assert_eq!(Settings::load(&Store::at(&directory)), Settings::default());

    fs::create_dir_all(&directory).expect("creating the test directory");

    // Not TOML at all.
    fs::write(&path, b"{ not toml").expect("writing the corrupt file");
    assert_eq!(Settings::load(&Store::at(&directory)), Settings::default());

    // TOML, but not this schema: starting in the default beats refusing to start.
    fs::write(&path, b"theme = \"solarized\"\n").expect("writing the stale file");
    assert_eq!(Settings::load(&Store::at(&directory)), Settings::default());

    // Neither was left for the next save to write over: both are under `incompatible`,
    // the second under a name of its own.
    let moved = directory.join(crate::store::INCOMPATIBLE_DIR);
    assert!(moved.join(FILE_NAME).exists());
    assert!(moved.join(format!("2-{FILE_NAME}")).exists());
}

/// A partial file is not a corrupt one: `serde(default)` on the container, since a
/// settings file is one someone may write by hand.
#[test]
fn a_file_that_names_one_setting_keeps_it() {
    let directory = directory(line!());
    let path = directory.join(FILE_NAME);

    fs::create_dir_all(&directory).expect("creating the test directory");
    fs::write(&path, b"theme = \"light\"\n").expect("writing the partial file");

    let loaded = Settings::load(&Store::at(&directory));
    assert_eq!(loaded.theme, Theme::Light);
    assert_eq!(loaded.interface, FontSetting::default());
}

/// The field-order test: TOML cannot reopen a table, so `theme` has to be written
/// before `[interface]` — a runtime failure, hence a real serializer and a real file.
#[test]
fn writes_atomically_and_reads_back_with_its_tables_last() {
    let directory = directory(line!());
    let path = directory.join("nested").join(FILE_NAME);

    let settings = settings();
    Store::at(directory.join("nested"))
        .write_toml(FILE_NAME, &settings)
        .expect("saving");

    let text = fs::read_to_string(&path).expect("reading it back");
    let theme = text.find("theme").expect("the theme");
    let interface = text.find("[interface]").expect("the interface table");
    let fixed = text.find("[fixed]").expect("the fixed table");
    assert!(theme < interface && interface < fixed, "{text}");

    assert_eq!(
        Settings::load(&Store::at(directory.join("nested"))),
        settings
    );
    // The temporary was renamed, not left behind.
    assert!(!path.with_extension("toml.tmp").exists());
}

/// Unspecified is an *absent* key, so nothing can later mistake it for a value that was
/// chosen and happened to match the desktop's answer.
#[test]
fn unspecified_is_an_absent_key_and_not_an_empty_one() {
    let text = toml::to_string_pretty(&Settings::default()).expect("writing the default");
    assert!(!text.contains("family"), "{text}");
    assert!(!text.contains("size"), "{text}");

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

/// The other end of that distinction: a value that is present but says nothing is
/// judged at the accessor, so a box the reader emptied is not a font named "".
#[test]
fn a_family_of_nothing_is_not_a_family() {
    let empty = FontSetting {
        family: Some("   ".into()),
        size: Some(0.0),
    };
    assert_eq!(empty.family(), None);
    assert_eq!(empty.size(), None);

    let named = FontSetting {
        family: Some(" Fira Code ".into()),
        size: Some(10.5),
    };
    assert_eq!(named.family(), Some("Fira Code"));
    assert_eq!(named.size(), Some(10.5));
}

/// The ceiling: the half of the rule a positive number can still fail. `size = 900` in
/// a hand-edited file is a slip, and honouring it would draw one row taller than the
/// window.
#[test]
fn a_size_the_app_cannot_draw_at_is_no_size() {
    let huge = FontSetting {
        family: None,
        size: Some(900.0),
    };
    assert_eq!(huge.size(), None);

    assert_eq!(points(f32::INFINITY), None);
    assert_eq!(points(MAX_POINTS + 1.0), None);
    // Large but drawable, so the reader gets what they asked for.
    assert_eq!(points(MAX_POINTS), Some(MAX_POINTS));
}

/// What is owed is written by the next flush, the newest of it only, and once. The one
/// test that goes through the `OWED` static.
#[test]
fn a_flush_writes_the_settings_last_owed_once() {
    let directory = directory(line!());
    let store = Store::at(&directory);
    fs::create_dir_all(&directory).expect("creating the test directory");

    Settings::default().owe(&store);
    settings().owe(&store);
    assert!(!directory.join(FILE_NAME).exists(), "owing is not writing");

    flush();
    assert_eq!(Settings::load(&store), settings());

    fs::remove_file(directory.join(FILE_NAME)).expect("the file written");
    flush();
    assert!(!directory.join(FILE_NAME).exists(), "written twice");
}
