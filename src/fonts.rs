//! The two fonts the UI uses: the interface font and the fixed-width one for the
//! assembly. KDE is asked for the desktop's own settings, so the interface matches
//! the rest of the desktop and the assembly view matches KWrite; everywhere else the
//! platform's standard fonts are named explicitly, at the sizes the floem version
//! used.

use std::{borrow::Cow, process::Command, sync::OnceLock};

/// The platform's own interface and fixed-width families. These have to be named:
/// freya's global fallbacks (`Segoe UI`, `Noto Sans`, `Arial`, ...) are all
/// proportional, so a font nothing resolves would silently take the assembly view out
/// of a monospaced face. Naming another platform's fonts here would be worse than
/// naming none -- a Windows machine that happens to have DejaVu or Liberation
/// installed would render in those instead of its own fonts.
#[cfg(target_os = "windows")]
const DEFAULT_UI: &str = "Segoe UI";
#[cfg(target_os = "windows")]
const DEFAULT_MONO: &str = "Consolas";

#[cfg(target_os = "macos")]
const DEFAULT_UI: &str = ".AppleSystemUIFont";
#[cfg(target_os = "macos")]
const DEFAULT_MONO: &str = "Menlo";

/// Everywhere else the generic families are the right answer: skia resolves them
/// through fontconfig, which is what has been configured with the system's choices.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const DEFAULT_UI: &str = "sans-serif";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const DEFAULT_MONO: &str = "monospace";

/// The sizes the floem version hardcoded, used wherever KDE has no say.
const DEFAULT_UI_SIZE: f32 = 12.0;
const DEFAULT_MONO_SIZE: f32 = 14.0;

pub struct Font {
    /// Families to try, most preferred first: the desktop's choice, if there is one,
    /// in front of the platform's own font.
    pub families: Vec<Cow<'static, str>>,
    pub size: f32,
}

impl Font {
    fn new(queried: Option<(String, f32)>, default: &'static str, default_size: f32) -> Self {
        let (family, size) = match queried {
            Some((family, size)) => (Some(family), size),
            None => (None, default_size),
        };

        Font {
            families: family
                .map(Cow::Owned)
                .into_iter()
                .chain([Cow::Borrowed(default)])
                .collect(),
            size,
        }
    }
}

pub struct Fonts {
    pub ui: Font,
    pub mono: Font,
}

/// KDE stores font sizes in points, freya wants logical pixels.
fn points_to_pixels(points: f32) -> f32 {
    points * 96.0 / 72.0
}

/// Font specs look like `Noto Sans Mono,10,-1,5,50,0,0,0,0,0`; only the family and
/// the point size are of interest here.
fn parse_spec(spec: &str) -> Option<(String, f32)> {
    let mut parts = spec.split(',');
    let family = parts.next()?.trim();
    let points: f32 = parts.next()?.trim().parse().ok()?;

    (!family.is_empty() && points > 0.0).then(|| (family.to_owned(), points_to_pixels(points)))
}

/// Ask KDE for a key in the `[General]` group of `kdeglobals`. Going through
/// `kreadconfig` rather than reading the file directly matters: neither `font` nor
/// `fixed` is written out until it is changed in System Settings, and only KDE knows
/// what its own defaults are.
fn query(key: &str) -> Option<(String, f32)> {
    ["kreadconfig6", "kreadconfig5"]
        .into_iter()
        .find_map(|bin| {
            let output = Command::new(bin)
                .args(["--group", "General", "--key", key])
                .output()
                .ok()?;

            if !output.status.success() {
                return None;
            }

            parse_spec(String::from_utf8(output.stdout).ok()?.trim())
        })
}

/// `font` is the general interface font and `fixed` the fixed-width one. Off a KDE
/// desktop there is nothing to ask, so both fonts are the platform's own at the
/// floem-era sizes.
fn load() -> Fonts {
    Fonts {
        ui: Font::new(query("font"), DEFAULT_UI, DEFAULT_UI_SIZE),
        mono: Font::new(query("fixed"), DEFAULT_MONO, DEFAULT_MONO_SIZE),
    }
}

pub fn fonts() -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(load)
}
