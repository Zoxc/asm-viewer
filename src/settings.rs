//! The settings the *user* gave: which fonts to draw with, and which theme to draw in.
//!
//! This is the other half of the storage split `notes/Goals.md` asks for under
//! *Projects* — user-given settings in one file, the session in another — and the split
//! is not tidiness. The session is what the app *noticed* (binaries opened, tabs left
//! open, where the reader was), it changes on every click, and losing it costs a few
//! clicks; a setting is what the user *said*, it changes when they say so, and losing it
//! is losing an instruction. They have different rates, different save policies (see
//! [`Settings::save`]) and different consequences when the file is corrupt, so they are
//! separate files rather than halves of one that a bad write takes down together. The rest
//! of that split is `project.rs`'s, which cuts the session itself in two along the same
//! line: what the user said about a project, and what the app noticed while they read it.
//!
//! Framework-free, exactly as `project.rs` is — no freya types — so it can move into a
//! crate beside it later.
//!
//! **Unspecified is a real third state.** Every setting here is an `Option`, and `None`
//! means "the user has not said — ask the desktop", which is neither an empty string nor
//! the desktop's current answer copied into the file. Copying the answer in would be the
//! easy mistake and the wrong one twice over: a font written down at install time would
//! stop following the desktop the moment the desktop changed, and the settings page has
//! to be able to *show* the difference between a value the reader chose and the one they
//! are inheriting. That is why an unspecified field is a key that is absent from the
//! TOML entirely, and why the two states are distinguishable after a round trip.
//!
//! There is no published version of this app, so a schema change here is just a schema
//! change: a file that no longer parses is the default, not a migration.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// The directory this app keeps its state in, the same one `project.rs` names: the
/// platform's state directory, falling back to its local data directory. Spelled out
/// again rather than shared, because the two modules are meant to be separable and a
/// constant is a cheaper duplicate than a dependency between them.
const APP_DIR: &str = "assembly-viewer";
const FILE_NAME: &str = "settings.toml";

/// Everything the user can set.
///
/// **The field order is load-bearing**, for `project.rs`'s reason: TOML cannot reopen a
/// table once a later one has begun, so a serializer must emit every plain value of a
/// table before the first sub-table of it, and a plain value written after a table fails
/// at *runtime* with "values must be emitted before tables" rather than at compile time.
/// `theme` is a bare string and the two fonts are tables, so `theme` comes first. The
/// round-trip test below is what keeps that true.
///
/// `serde(default)` at the container earns its place here in a way it does not in the
/// session file: this is a file a user may reasonably open and edit, every field is
/// independently optional by construction, and a file that names only the theme must
/// load as the theme rather than as nothing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Light, dark, or whatever the desktop prefers.
    pub theme: Theme,
    /// The interface font — the one everything but the code panes is drawn in.
    pub interface: FontSetting,
    /// The fixed-width font, for the assembly and source rows.
    pub fixed: FontSetting,
}

/// One font override: a family, a size, each independently unspecified.
///
/// Independently, because the two halves are asked for separately. A reader who wants
/// their editor's font at the desktop's size, or the desktop's font a little larger, is
/// asking for one of these and not both, and a page that could only set the pair would
/// make them write down a value they did not choose in order to change the one they did.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FontSetting {
    /// `skip_serializing_if` is what makes "unspecified" a *missing key*: TOML has no
    /// null, so the `toml` crate cannot write a bare `None` at all, and leaving the key
    /// out is both the only way to write it and the shape a reader editing this file by
    /// hand would expect. `default` is what reads it back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
}

impl FontSetting {
    /// The family this asks for, or `None` for "ask the desktop".
    ///
    /// Judged here rather than trusted, because the value can come from a text box or
    /// from a hand-edited file: a family of spaces is not a font, and pinning the UI to
    /// it would resolve to nothing at all — freya's own fallbacks being proportional,
    /// that silently takes the assembly view out of a monospaced face.
    pub fn family(&self) -> Option<&str> {
        self.family
            .as_deref()
            .map(str::trim)
            .filter(|family| !family.is_empty())
    }

    /// The size this asks for, **in points**, or `None` for "ask the desktop".
    ///
    /// Points and not pixels, because points are the unit the desktops answer in
    /// (`fonts.rs` converts once, at the end) and so the unit in which an override and
    /// the value it is overriding can be compared — a settings page showing "the desktop
    /// says 10, you set 11" cannot be built out of two different units. A size that is
    /// not a positive finite number is the same as no size at all, exactly as it is for
    /// a desktop's answer.
    pub fn size(&self) -> Option<f32> {
        self.size
            .filter(|points| points.is_finite() && *points > 0.0)
    }
}

/// Which theme the user asked for. Only the *choice* lives here; which colours that
/// means is the palette's business (`notes/Goals.md`, *UI*: a dark mode in the same
/// palette rather than a second design).
///
/// **Turning [`Theme::Desktop`] into an [`Appearance`] is deliberately not a method here**,
/// and that is not a layering nicety. "Which theme does this desktop prefer" is a question
/// the *windowing system* answers: winit reports it on Windows, macOS, X11 and Wayland
/// alike, and freya keeps the answer in `Platform::preferred_theme` — a `State`, so a
/// desktop that switches to dark at sunset repaints the window that is already open. This
/// module is framework-free and holds no window, so the only answer it could work out on
/// its own is the worse one: a subprocess, spawned once, whose answer would then be stale
/// for the rest of the session. `ui.rs` is where the choice and the window's answer meet
/// (`resolve_appearance`), which is also the only place that may change what is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    /// Follow the desktop's own preference, and the default: an app that starts in the
    /// wrong theme on a dark desktop is wrong before the user has been given anywhere to
    /// say so.
    #[default]
    Desktop,
    Light,
    Dark,
}

/// A resolved theme: what the palette is actually asked for. [`Theme`] has three answers
/// and this has two, which is the whole difference between them — "follow the desktop"
/// is a choice a user can make and not a set of colours anything can be drawn in.
///
/// It lives here rather than in `ui.rs` because it is the *shape* of the answer and not
/// the answer itself: this module owns the choice, and a choice with no vocabulary for
/// what it resolves to could not say what `Theme::Light` even means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

impl Settings {
    /// The file the settings are stored in — at the top of the app's own state directory,
    /// above the `projects/` the sessions live under, because a setting is the user's and
    /// not any one project's — or `None` on a system with no state or local data directory
    /// to put it in.
    pub fn path() -> Option<PathBuf> {
        let base = dirs::state_dir().or_else(dirs::data_local_dir)?;
        Some(base.join(APP_DIR).join(FILE_NAME))
    }

    /// Read the stored settings. A missing, unreadable or corrupt file is simply the
    /// default — never an error the user sees, and never a reason not to start: the
    /// default is a complete, usable set of answers by construction, since every field
    /// of it means "ask the desktop".
    pub fn load() -> Settings {
        Settings::path()
            .map(|path| Settings::load_from(&path))
            .unwrap_or_default()
    }

    /// Write the settings out, atomically. Any IO failure is logged and swallowed, for
    /// `project.rs`'s reason: failing to persist is never worth interrupting the user
    /// for.
    ///
    /// **Public, and with no policy in front of it** — this is the one place the two
    /// files differ in kind. The session has a [`crate::project::Saves`]-shaped policy
    /// because it changes on every click and most of those changes can wait; settings
    /// change only when the user changes one, so the write is already as rare as a
    /// deliberate action and every one of them is worth keeping. Hence: written at once,
    /// by whoever changed one, and *no second autosave timer* — a timer here would be a
    /// tick that finds nothing to do on every run of the app in which the user never
    /// opened the settings page, which is nearly all of them.
    ///
    /// Nothing *writes* a settings file yet: dark mode reads the theme choice, `fonts.rs`
    /// reads the two fonts, and the settings page is what will change one. So this is the
    /// last of the module's dead code and the allow is on it alone — `save_to` with it,
    /// being reachable only from here and from the tests — rather than on the module,
    /// where it would go on covering everything the app now really uses.
    #[allow(dead_code)]
    pub fn save(&self) {
        let Some(path) = Settings::path() else {
            log::warn!("no state directory to save the settings in");
            return;
        };
        if let Err(error) = self.save_to(&path) {
            log::warn!("could not save {}: {error}", path.display());
        }
    }

    fn load_from(path: &Path) -> Settings {
        fs::read_to_string(path)
            .ok()
            .and_then(|data| toml::from_str(&data).ok())
            .unwrap_or_default()
    }

    /// Write `path` by writing `path.tmp` first and renaming it over the top, so an
    /// interrupted write cannot leave a half-written file behind — and so a settings
    /// file that is being read by another copy of the app is either the old one or the
    /// new one, never a truncated one that would silently load as the default.
    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)?;
        }

        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);

        // Nothing here can fail to serialize the way a non-UTF-8 path can in the session
        // file — these are strings and numbers — but the error is still turned into an
        // IO error and swallowed by `save` rather than unwrapped, because a settings
        // write must not be a way to crash the app.
        let data = toml::to_string_pretty(self)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        fs::write(&temporary, data)?;
        fs::rename(&temporary, path)
    }
}

#[cfg(test)]
mod tests {
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
}
