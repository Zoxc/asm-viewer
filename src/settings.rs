//! The settings the *user* gave: which fonts to draw with, and which theme to draw in.
//! Framework-free, like `project.rs`, and stored in its own file so that a corrupt
//! session cannot take a setting down with it.
//!
//! `None` is a real third state throughout: "the user has not said — ask the desktop",
//! which is written as an absent key rather than as an empty string or a copy of the
//! desktop's current answer.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::project::{base, write_toml};
use crate::rescue;

const FILE_NAME: &str = "settings.toml";

/// Everything the user can set.
///
/// **The field order is load-bearing**: TOML cannot reopen a table, so every plain value
/// must be emitted before the first sub-table, and getting it wrong fails at *runtime*.
/// `theme` is a bare string and the two fonts are tables, so `theme` comes first; the
/// round-trip test is what keeps that true.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    /// The interface font — the one everything but the code panes is drawn in.
    pub interface: FontSetting,
    /// The fixed-width font, for the assembly and source rows.
    pub fixed: FontSetting,
}

/// One font override: a family, a size, each independently unspecified.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FontSetting {
    /// `skip_serializing_if` is what makes "unspecified" a *missing key*: TOML has no
    /// null, so this is the only way to write it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
}

impl FontSetting {
    /// The family this asks for, or `None` for "ask the desktop". A family of spaces is
    /// not a font: it can come from a text box or a hand-edited file, and pinning the UI
    /// to it resolves to nothing at all.
    pub fn family(&self) -> Option<&str> {
        self.family
            .as_deref()
            .map(str::trim)
            .filter(|family| !family.is_empty())
    }

    /// The size this asks for, **in points** — the unit the desktops answer in, so that
    /// an override and the value it overrides are comparable. `fonts.rs` converts once,
    /// at the end.
    pub fn size(&self) -> Option<f32> {
        self.size
            .filter(|points| points.is_finite() && *points > 0.0)
    }
}

/// Which theme the user asked for. Resolving [`Theme::Desktop`] to an [`Appearance`] is
/// `ui/palette.rs`'s job: this module holds no window and stays framework-free.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    #[default]
    Desktop,
    Light,
    Dark,
}

/// A resolved theme: what the palette is actually asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

impl Settings {
    /// The file the settings are stored in, or `None` on a system with no state or local
    /// data directory to put it in. Beside `recents.toml` and above the projects, since a
    /// setting is the user's and not any one project's.
    fn path() -> Option<PathBuf> {
        Some(base()?.join(FILE_NAME))
    }

    /// Read the stored settings. A missing or unreadable file is the default; one that
    /// will not parse is that too, and is moved aside first.
    pub fn load() -> Settings {
        base()
            .map(|base| Settings::load_in(&base))
            .unwrap_or_default()
    }

    /// The same from a given state directory, so a test can point it at one of its own.
    /// The file is moved aside rather than ignored because the next [`Settings::save`]
    /// would write over it.
    fn load_in(base: &Path) -> Settings {
        rescue::parse(base, &base.join(FILE_NAME)).unwrap_or_default()
    }

    /// Write the settings out at once — a settings change is already as rare as a
    /// deliberate action, so there is no `Saves`-shaped policy and no autosave timer.
    /// Any IO failure is logged and swallowed.
    pub fn save(&self) {
        let Some(path) = Settings::path() else {
            log::warn!("no state directory to save the settings in");
            return;
        };
        if let Err(error) = self.save_to(&path) {
            log::warn!("could not save {}: {error}", path.display());
        }
    }

    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        write_toml(path, self)
    }
}

#[cfg(test)]
mod tests;
