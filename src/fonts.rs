//! The two fonts the UI uses: the interface font and the fixed-width one for the
//! assembly, asked of the desktop and merged under the user's own settings field by
//! field. Nothing here fails: a missing binary, a call that returns `FALSE` or a value
//! in a shape nobody expected all come back as "the desktop said nothing".
//!
//! **Which desktop to ask is a runtime question**: one Linux build runs on KDE and on
//! Gnome, so `XDG_CURRENT_DESKTOP` only decides the order the two tools are tried in and
//! the other is tried anyway.
//!
//! Everything here is **in points** — the unit the desktops answer in and the unit an
//! override is stored in — up to the one conversion at [`Font::size`].

use std::{borrow::Cow, sync::OnceLock};

use crate::settings::{FontSetting, Settings};

/// The platform's own interface and fixed-width families. These have to be *named*:
/// freya's global fallbacks are all proportional, so a font nothing resolves would
/// silently take the assembly view out of a monospaced face. Naming another platform's
/// fonts here would be worse than naming none -- a Windows machine that happens to have
/// DejaVu installed would render in it instead of its own fonts.
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

/// The app's own sizes, in points: the 12 and 14 logical pixels the floem version drew
/// at. Used wherever the desktop has no say, and where it named a family but no size.
const DEFAULT_UI_POINTS: f32 = 9.0;
const DEFAULT_MONO_POINTS: f32 = 10.5;

#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    /// Families to try, most preferred first: the desktop's choice, if there is one, in
    /// front of the platform's own font.
    pub families: Vec<Cow<'static, str>>,
    /// The size **in points**.
    pub points: f32,
}

impl Font {
    /// What this comes to on screen, in the logical pixels freya asks for: the only
    /// place points become pixels. The 96 is the nominal DPI logical pixels are defined
    /// at -- the display's real one is winit's business, applied to the whole window as
    /// a scale factor on top of this.
    pub fn size(&self) -> f32 {
        self.points * 96.0 / 72.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fonts {
    pub ui: Font,
    pub mono: Font,
}

/// Which of the two fonts is being asked for. Every desktop names the same pair and
/// names it differently, so the key belongs where the desktop is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Which {
    Ui,
    Fixed,
}

/// A font as a desktop wrote it down. The size is optional because Gnome's spec allows
/// `Cantarell` with no number after it.
#[derive(Debug, PartialEq)]
struct Spec {
    family: String,
    points: Option<f32>,
}

impl Spec {
    /// The one place a parsed spec is judged: an empty family is not an answer, and a
    /// size that is not a positive finite number is the same as no size at all.
    fn new(family: &str, points: Option<f32>) -> Option<Spec> {
        let family = family.trim();

        (!family.is_empty()).then(|| Spec {
            family: family.to_owned(),
            points: points.filter(|points| points.is_finite() && *points > 0.0),
        })
    }
}

/// The desktop lookups. Compiled on Windows too, but only so its parsers stay under test
/// there.
#[cfg(any(not(target_os = "windows"), test))]
#[cfg_attr(target_os = "windows", allow(dead_code))]
mod desktop {
    use super::{Spec, Which};
    use std::{env, process::Command, sync::OnceLock};

    /// Run a tool and take its stdout, trimmed. Everything that can go wrong -- the
    /// binary is not installed, the key does not exist, the output is not text -- is one
    /// `None`.
    fn output(command: &mut Command) -> Option<String> {
        let output = command.output().ok()?;

        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Desktop {
        Kde,
        Gnome,
    }

    /// Both desktops, in the order this one should be asked in. `XDG_CURRENT_DESKTOP` is
    /// frequently absent, which is why it only sorts the two rather than choosing one:
    /// getting the order wrong costs one failed `exec`, choosing wrong costs the setting.
    pub fn order(current: &str) -> [Desktop; 2] {
        let gnome_first = current.split(':').any(|name| {
            let name = name.trim();

            // `GNOME-Classic`, `GNOME-Flashback:GNOME` and the like all still answer to
            // `gsettings`, so this is a prefix and not an equality. Unity, Budgie and
            // Pantheon are GTK desktops that keep their fonts in the same schema.
            //
            // `get` and not a slice: the variable is whatever launched the session, and
            // five bytes into a name is not always a character boundary.
            name.get(..5)
                .is_some_and(|head| head.eq_ignore_ascii_case("GNOME"))
                || name.eq_ignore_ascii_case("Unity")
                || name.eq_ignore_ascii_case("Budgie")
                || name.eq_ignore_ascii_case("Pantheon")
        });

        if gnome_first {
            [Desktop::Gnome, Desktop::Kde]
        } else {
            [Desktop::Kde, Desktop::Gnome]
        }
    }

    /// The first desktop that has an answer wins.
    pub fn query(which: Which) -> Option<Spec> {
        let current = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();

        order(&current)
            .into_iter()
            .find_map(|desktop| match desktop {
                Desktop::Kde => kde(which),
                Desktop::Gnome => gnome(which),
            })
    }

    /// Ask KDE for a key in the `[General]` group of `kdeglobals`. Going through
    /// `kreadconfig` rather than reading the file matters: neither `font` nor `fixed` is
    /// written out until it is changed, and only KDE knows its own defaults.
    fn kde(which: Which) -> Option<Spec> {
        let key = match which {
            Which::Ui => "font",
            Which::Fixed => "fixed",
        };

        ["kreadconfig6", "kreadconfig5"]
            .into_iter()
            .find_map(|bin| {
                let value = output(Command::new(bin).args(["--group", "General", "--key", key]))?;

                parse_kde(&value)
            })
    }

    /// KDE's specs look like `Noto Sans Mono,10,-1,5,50,0,0,0,0,0`: the first two fields
    /// are the family and the point size, the rest are flags nothing here reads.
    pub fn parse_kde(spec: &str) -> Option<Spec> {
        let mut parts = spec.split(',');
        let family = parts.next()?;
        let points = parts.next().and_then(|points| points.trim().parse().ok());

        Spec::new(family, points)
    }

    /// Ask Gnome, through the GTK schema every GTK desktop shares.
    fn gnome(which: Which) -> Option<Spec> {
        let key = match which {
            Which::Ui => "font-name",
            Which::Fixed => "monospace-font-name",
        };

        let mut spec = parse_pango(&gsettings(key)?)?;

        // `text-scaling-factor` is how Gnome says "make text bigger": `font-name` keeps
        // its nominal size and the accessibility slider moves this instead. It multiplies
        // the point size and nothing else, as GTK does -- the *window* scale is winit's,
        // taken from the display, and multiplying that too would compound.
        if let Some(points) = spec.points.as_mut() {
            *points *= text_scaling();
        }

        Some(spec)
    }

    fn gsettings(key: &str) -> Option<String> {
        output(Command::new("gsettings").args(["get", "org.gnome.desktop.interface", key]))
    }

    /// Cached, because it is the same answer for both fonts and each call is a process.
    /// Out-of-range values are ignored rather than clamped: honouring one would produce a
    /// window nothing on screen fits in.
    fn text_scaling() -> f32 {
        static SCALING: OnceLock<f32> = OnceLock::new();

        *SCALING.get_or_init(|| {
            gsettings("text-scaling-factor")
                .and_then(|value| value.trim().parse::<f32>().ok())
                .filter(|scale| (0.5..=4.0).contains(scale))
                .unwrap_or(1.0)
        })
    }

    /// Gnome's specs are Pango font descriptions -- `Cantarell 11`, `Source Code Pro
    /// Semi-Bold 10`, `Cantarell` -- which is `Family [Styles] [Size]` with spaces
    /// throughout: the family itself contains spaces, so the size is the last word if the
    /// last word is a number, and there may be no size at all. Style words are a weight
    /// to Pango rather than part of the name, so they are dropped -- except where they
    /// are all there is, since an empty family is no answer.
    pub fn parse_pango(value: &str) -> Option<Spec> {
        let mut words: Vec<&str> = unquote(value.trim()).split_whitespace().collect();

        let points = words.last().and_then(|last| last.parse::<f32>().ok());
        if points.is_some() {
            words.pop();
        }

        while words.len() > 1 && is_style(words[words.len() - 1]) {
            words.pop();
        }

        Spec::new(&words.join(" "), points)
    }

    /// Pango's weight, style, variant and stretch words, which is the closed set its own
    /// parser recognises. A word that is not one of these is part of the family.
    fn is_style(word: &str) -> bool {
        const STYLES: &[&str] = &[
            "thin",
            "ultra-light",
            "extra-light",
            "light",
            "semi-light",
            "demi-light",
            "book",
            "regular",
            "normal",
            "medium",
            "semi-bold",
            "demi-bold",
            "demibold",
            "bold",
            "ultra-bold",
            "extra-bold",
            "heavy",
            "black",
            "ultra-heavy",
            "extra-black",
            "italic",
            "oblique",
            "roman",
            "small-caps",
            "all-small-caps",
            "petite-caps",
            "all-petite-caps",
            "unicase",
            "title-caps",
            "ultra-condensed",
            "extra-condensed",
            "condensed",
            "semi-condensed",
            "semi-expanded",
            "expanded",
            "extra-expanded",
            "ultra-expanded",
        ];

        STYLES
            .iter()
            .any(|style| style.eq_ignore_ascii_case(word.trim_end_matches(',')))
    }

    /// `gsettings get` prints `'Cantarell 11'`. The quote is only stripped when it is on
    /// both ends, so a family with an apostrophe in it survives being read.
    fn unquote(value: &str) -> &str {
        ['\'', '"']
            .into_iter()
            .find_map(|quote| {
                value
                    .strip_prefix(quote)
                    .and_then(|inner| inner.strip_suffix(quote))
            })
            .unwrap_or(value)
    }
}

/// Windows, asked through `user32`. Compiled on Linux too, but only so [`font_spec`] --
/// the half that does not touch the API -- stays under test; everything that does sits
/// behind a second `cfg` inside.
#[cfg(any(target_os = "windows", test))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod windows {
    use super::Spec;

    #[cfg(target_os = "windows")]
    use super::Which;
    #[cfg(target_os = "windows")]
    use windows_sys::Win32::{
        Graphics::Gdi::{GetDC, GetDeviceCaps, ReleaseDC, LOGPIXELSY},
        UI::WindowsAndMessaging::{
            SystemParametersInfoW, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS,
        },
    };

    /// The DPI logical pixels are defined at, and so also the one to assume wherever the
    /// real one cannot be had.
    const NOMINAL_DPI: u32 = 96;

    /// Windows stores no desktop-wide fixed-width font: the nearest thing is the console
    /// host's own `FaceName`, which is often a raster face and carries a pixel cell size,
    /// so `Consolas` stands.
    #[cfg(target_os = "windows")]
    pub fn query(which: Which) -> Option<Spec> {
        match which {
            Which::Ui => message_font(),
            Which::Fixed => None,
        }
    }

    /// `lfMessageFont` is the font dialogs and message boxes use, which is what "the
    /// interface font" means on Windows.
    #[cfg(target_os = "windows")]
    fn message_font() -> Option<Spec> {
        // `cbSize` is how `user32` tells the two layouts of this struct apart; a call
        // carrying neither returns `FALSE` having written nothing. `windows-sys` declares
        // only the post-XP layout, so the whole struct is the right size here. The rest is
        // zeroed, which `SPI_GETNONCLIENTMETRICS` then overwrites in full.
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };

        // SAFETY: `SPI_GETNONCLIENTMETRICS` is a read that writes `cbSize` bytes into
        // `pvparam`. That is this stack `NONCLIENTMETRICSW`, which is exactly that many
        // bytes long and already zeroed, and `uiparam` carries the same size; `fwinini`
        // says what to do with a *changed* setting and is ignored by every `SPI_GET*`.
        let read = unsafe {
            SystemParametersInfoW(
                SPI_GETNONCLIENTMETRICS,
                std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
                (&mut metrics as *mut NONCLIENTMETRICSW).cast(),
                0,
            )
        };

        // A `FALSE` is one more "the desktop said nothing".
        if read == 0 {
            return None;
        }

        let font = metrics.lfMessageFont;

        font_spec(&font.lfFaceName, font.lfHeight, metrics_dpi())
    }

    /// The DPI the metrics just read are in. `SystemParametersInfoW` answers in whatever
    /// DPI space the *process* is in, and `GetDeviceCaps(LOGPIXELSY)` on the screen DC is
    /// virtualised the same way, so the two agree without this file knowing which that is.
    ///
    /// `SystemParametersInfoForDpi(.., 96)` would drop the division outright and is
    /// deliberately not used: it and `GetDpiForSystem` exist only from Windows 10 1607,
    /// and `windows-sys` links its imports statically, so naming one would turn "no font
    /// setting" into a process that will not start. winit `GetProcAddress`es that family
    /// for the same reason.
    #[cfg(target_os = "windows")]
    fn metrics_dpi() -> u32 {
        // SAFETY: `GetDC(null)` is the documented way to ask for the screen's own DC and
        // these three are the documented sequence; `GetDeviceCaps` only reads, and the DC
        // is released on the one path that obtained it.
        let dpi = unsafe {
            let screen = GetDC(std::ptr::null_mut());
            if screen.is_null() {
                return NOMINAL_DPI;
            }

            let dpi = GetDeviceCaps(screen, LOGPIXELSY as i32);
            ReleaseDC(std::ptr::null_mut(), screen);

            dpi
        };

        u32::try_from(dpi)
            .ok()
            .filter(|dpi| *dpi > 0)
            .unwrap_or(NOMINAL_DPI)
    }

    /// The family and the point size inside a `LOGFONTW`.
    ///
    /// `lfFaceName` is a fixed `[u16; 32]` and is NUL-terminated only when the name is
    /// shorter than that, so the name runs to the first NUL *or* to the end of the array.
    /// `lfHeight` is in logical units at `dpi`: negative for the character height and
    /// positive for the cell height, and the sign is dropped rather than corrected for --
    /// a point either way on a UI font, and every writer of it uses the negative form.
    pub fn font_spec(face: &[u16; 32], height: i32, dpi: u32) -> Option<Spec> {
        let name: Vec<u16> = face.iter().copied().take_while(|unit| *unit != 0).collect();
        let family = String::from_utf16(&name).ok()?;

        let dpi = if dpi == 0 { NOMINAL_DPI } else { dpi };
        let points = height.unsigned_abs() as f32 * 72.0 / dpi as f32;

        Spec::new(&family, Some(points))
    }
}

/// The desktop's answer for one of the two fonts, asked once per process: a lookup is one
/// or two subprocesses, and the settings page re-[`resolve`]s both fonts on every change.
///
/// One `OnceLock` per font rather than one for the pair, because [`font`] declines to ask
/// about a font both of whose halves the reader has chosen, and a shared cell would make
/// the first such question answer for both.
fn desktop_answer(which: Which) -> Option<&'static Spec> {
    static UI: OnceLock<Option<Spec>> = OnceLock::new();
    static FIXED: OnceLock<Option<Spec>> = OnceLock::new();

    let cell = match which {
        Which::Ui => &UI,
        Which::Fixed => &FIXED,
    };

    cell.get_or_init(|| {
        #[cfg(target_os = "windows")]
        let answer = windows::query(which);
        #[cfg(not(target_os = "windows"))]
        let answer = desktop::query(which);

        answer
    })
    .as_ref()
}

/// One font, merged: the user's overrides in front of the desktop's answer, field by
/// field, with the platform's own family behind both. Pure, and handed the desktop's
/// answer rather than asking for it, so the merge is testable with no desktop at all.
fn resolve_font(
    setting: &FontSetting,
    desktop: Option<&Spec>,
    default: &'static str,
    default_points: f32,
) -> Font {
    let family = setting
        .family()
        .map(str::to_owned)
        .or_else(|| desktop.map(|desktop| desktop.family.clone()));

    Font {
        // A family with no size keeps the family and takes the app's own size: dropping
        // it over a missing number would put the chosen font back to the platform's.
        families: family
            .map(Cow::Owned)
            .into_iter()
            .chain([Cow::Borrowed(default)])
            .collect(),
        points: setting
            .size()
            .or_else(|| desktop.and_then(|desktop| desktop.points))
            .unwrap_or(default_points),
    }
}

fn font(setting: &FontSetting, which: Which, default: &'static str, default_points: f32) -> Font {
    // The desktop is asked only where it has something left to answer: a font whose
    // family *and* size the user has chosen has no unanswered half, so a fully configured
    // app spawns no process.
    let desktop = (setting.family().is_none() || setting.size().is_none())
        .then(|| desktop_answer(which))
        .flatten();

    resolve_font(setting, desktop, default, default_points)
}

/// The two fonts these settings and this desktop come to. Takes the settings by argument
/// rather than reading them, so the settings page can resolve what it is editing.
pub fn resolve(settings: &Settings) -> Fonts {
    Fonts {
        ui: font(
            &settings.interface,
            Which::Ui,
            DEFAULT_UI,
            DEFAULT_UI_POINTS,
        ),
        mono: font(
            &settings.fixed,
            Which::Fixed,
            DEFAULT_MONO,
            DEFAULT_MONO_POINTS,
        ),
    }
}

#[cfg(test)]
mod tests;
