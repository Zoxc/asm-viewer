//! The two fonts the UI uses: the interface font and the fixed-width one for the
//! assembly. The desktop is asked for its own settings, so the interface matches the rest
//! of the desktop and the assembly view matches the desktop's editor; where there is
//! nothing to ask, the platform's standard fonts are named explicitly, at the sizes the
//! floem version used.
//!
//! **Which desktop to ask is a runtime question, not a compile-time one.** One Linux
//! build runs on KDE and on Gnome, so `XDG_CURRENT_DESKTOP` only decides the *order* the
//! two tools are tried in and the other one is tried anyway: a tool that is not installed
//! is already a `None` here rather than an error, so asking both costs one failed `exec`
//! on a desktop that has neither and gets an answer on the desktops that set the variable
//! to something neither of them recognises. Windows has no such tool and no such
//! question: there is one shell, and `user32` is asked directly.
//!
//! Nothing here fails: a missing binary, a call that returns `FALSE`, a value in a shape
//! nobody expected and a font size of zero all come back as "the desktop said nothing",
//! which is what the platform defaults are for.
//!
//! **The user comes before the desktop.** `Settings` holds a family and a size for each
//! of the two fonts, each independently unspecified, and [`resolve`] merges them over the
//! desktop's answer *field by field*: an override wins, an unspecified field falls
//! through to the desktop, and the platform's own family stands behind both. Merging by
//! field rather than by font is what lets a reader take their editor's family at the
//! desktop's size, or the desktop's family a little larger, without writing down a value
//! they did not choose.

use std::{borrow::Cow, sync::OnceLock};

use crate::settings::{FontSetting, Settings};

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

/// The sizes the floem version hardcoded, used wherever the desktop has no say -- and
/// also where it named a family but no size, which is a shape Gnome's setting allows.
const DEFAULT_UI_SIZE: f32 = 12.0;
const DEFAULT_MONO_SIZE: f32 = 14.0;

pub struct Font {
    /// Families to try, most preferred first: the desktop's choice, if there is one,
    /// in front of the platform's own font.
    pub families: Vec<Cow<'static, str>>,
    pub size: f32,
}

impl Font {
    /// A family to put in front of the platform's own, and a size in points -- either of
    /// which may be missing, and separately.
    ///
    /// A family with no size keeps the family and takes the app's own size, which is
    /// deliberately not "no answer at all": a family is the half of the setting that is
    /// visible in every glyph on screen, and dropping it over a missing number would put
    /// the chosen font back to `sans-serif`. `default_size` is already in logical pixels
    /// -- it is this app's own number and not anybody's point size -- which is why only
    /// the answered size goes through [`points_to_pixels`].
    fn new(
        family: Option<String>,
        points: Option<f32>,
        default: &'static str,
        default_size: f32,
    ) -> Self {
        Font {
            families: family
                .map(Cow::Owned)
                .into_iter()
                .chain([Cow::Borrowed(default)])
                .collect(),
            size: points.map_or(default_size, points_to_pixels),
        }
    }
}

pub struct Fonts {
    pub ui: Font,
    pub mono: Font,
}

/// Which of the two fonts is being asked for. Every desktop names the same pair, and
/// names it differently, so the key belongs where the desktop is rather than at the call
/// site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Which {
    Ui,
    Fixed,
}

/// A font as a desktop wrote it down. The size is optional because Gnome's spec allows
/// `Cantarell` with no number after it, and because a size of zero is a key that has
/// never been set rather than an invisible font.
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

/// Desktops store font sizes in points, freya wants logical pixels. The 96 is the
/// nominal DPI logical pixels are defined at -- the display's real one is winit's
/// business, applied to the whole window as a scale factor on top of this.
fn points_to_pixels(points: f32) -> f32 {
    points * 96.0 / 72.0
}

/// The desktop lookups, which are the whole of the story on everything that is not
/// Windows. Compiled on Windows too, but only so its parsers stay under test there.
#[cfg(any(not(target_os = "windows"), test))]
#[cfg_attr(target_os = "windows", allow(dead_code))]
mod desktop {
    use super::{Spec, Which};
    use std::{env, process::Command, sync::OnceLock};

    /// Run a tool and take its stdout, trimmed. Everything that can go wrong is one
    /// `None`: the binary is not installed (the ordinary case for whichever desktop this
    /// is not), it answered with a failure because the key does not exist, or it wrote
    /// something that is not text.
    fn output(command: &mut Command) -> Option<String> {
        let output = command.output().ok()?;

        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    /// The two desktops that can be asked. Nothing else is: every other desktop either
    /// answers to `gsettings` (it is the GTK setting, not a Gnome-only one) or has no
    /// tool to ask, and a list of desktops to keep current is worse than two tools that
    /// each say `None` when they are not there.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Desktop {
        Kde,
        Gnome,
    }

    impl Desktop {
        fn query(self, which: Which) -> Option<Spec> {
            match self {
                Desktop::Kde => kde(which),
                Desktop::Gnome => gnome(which),
            }
        }
    }

    /// Both desktops, in the order this one should be asked in. `XDG_CURRENT_DESKTOP` is
    /// a colon-separated list of names and is frequently absent (a bare X session, a
    /// window manager that sets nothing, a terminal launched outside the session), which
    /// is why it only sorts the two rather than choosing one: getting the order wrong
    /// costs one failed `exec`, while choosing wrong costs the setting.
    ///
    /// KDE stays first when the variable says nothing, because that is the order this
    /// file already had and a KDE session is the one that most reliably sets the
    /// variable anyway.
    pub fn order(current: &str) -> [Desktop; 2] {
        let gnome_first = current.split(':').any(|name| {
            let name = name.trim();

            // `GNOME-Classic`, `GNOME-Flashback:GNOME` and the like all still answer to
            // `gsettings`, so this is a prefix and not an equality. Unity, Budgie and
            // Pantheon are GTK desktops that keep their fonts in the same schema.
            name.len() >= 5 && name[..5].eq_ignore_ascii_case("GNOME")
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

    /// The first desktop that has an answer wins; a desktop with no tool installed, or
    /// with the key unset, is not an answer.
    pub fn query(which: Which) -> Option<Spec> {
        let current = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();

        order(&current)
            .into_iter()
            .find_map(|desktop| desktop.query(which))
    }

    /// Ask KDE for a key in the `[General]` group of `kdeglobals`. Going through
    /// `kreadconfig` rather than reading the file directly matters: neither `font` nor
    /// `fixed` is written out until it is changed in System Settings, and only KDE knows
    /// what its own defaults are.
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

    /// KDE's specs look like `Noto Sans Mono,10,-1,5,50,0,0,0,0,0` -- a comma-separated
    /// list whose first two fields are the family and the point size, and whose remaining
    /// eight are the weight, style and hinting flags nothing here has an opinion about.
    pub fn parse_kde(spec: &str) -> Option<Spec> {
        let mut parts = spec.split(',');
        let family = parts.next()?;
        let points = parts.next().and_then(|points| points.trim().parse().ok());

        Spec::new(family, points)
    }

    /// Ask Gnome, through the GTK schema every GTK desktop shares. `gsettings` prints the
    /// GVariant rather than the string inside it, so the value arrives quoted.
    fn gnome(which: Which) -> Option<Spec> {
        let key = match which {
            Which::Ui => "font-name",
            Which::Fixed => "monospace-font-name",
        };

        let mut spec = parse_pango(&gsettings(key)?)?;

        // `text-scaling-factor` is applied here, and this is the decision worth writing
        // down. It is how Gnome says "make text bigger": `font-name` keeps its nominal
        // 11, and the accessibility slider (and every fractional font scale) moves this
        // instead -- so a reader who set 1.5 and got no larger text would be looking at
        // an app that ignored the one setting they changed. It multiplies the point size
        // and nothing else, which is exactly what GTK does with it: the *window* scale is
        // winit's, taken from the display, and multiplying that too would compound.
        if let Some(points) = spec.points.as_mut() {
            *points *= text_scaling();
        }

        Some(spec)
    }

    fn gsettings(key: &str) -> Option<String> {
        output(Command::new("gsettings").args(["get", "org.gnome.desktop.interface", key]))
    }

    /// Cached, because it is the same answer for both fonts and each call is a process.
    /// Out-of-range values are ignored rather than clamped: a scale outside this range is
    /// a value that was not written by the settings app, and honouring it would produce a
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
    /// throughout, and *not* KDE's list: the family itself contains spaces, so the size
    /// is the last word if the last word is a number, and there may be no size at all.
    ///
    /// The style words are dropped rather than kept in the family, because that is what
    /// they are to Pango -- a weight, not part of the name -- and skia is being asked to
    /// resolve a family. `words.len() > 1` guards the degenerate case: a description of
    /// nothing but style words keeps the first one rather than becoming empty, since
    /// there is nothing better to answer and an empty family is no answer at all.
    pub fn parse_pango(value: &str) -> Option<Spec> {
        let mut words: Vec<&str> = unquote(value.trim()).split_whitespace().collect();

        let points = match words.last() {
            Some(last) => last.parse::<f32>().ok(),
            None => None,
        };
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

/// Windows, asked through `user32` rather than through a tool: there is one shell here
/// and it answers `SystemParametersInfo`, so nothing is shelled out to and nothing is
/// read back out of another program's output.
///
/// Compiled on Linux too, but only for its tests: [`font_spec`] -- a `LOGFONTW`'s face
/// name and height turned into a family and a point size -- is the half of this that can
/// be checked from a machine that is not Windows, and a `cfg` that hid it from `cargo
/// test` would leave it checked nowhere at all. Everything that touches the API sits
/// behind a second `cfg` inside, because that half compiles nowhere else.
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

    /// Windows has a message font and no fixed-width font: nothing in the shell settings
    /// names one. The nearest thing is `HKCU\Console`'s `FaceName`, which is the console
    /// host's own choice, is often the raster `Terminal` face that skia cannot use for a
    /// UI, and carries its size as a packed cell in pixels rather than a point size --
    /// three reasons it is not the desktop answering, so `Consolas` stands.
    #[cfg(target_os = "windows")]
    pub fn query(which: Which) -> Option<Spec> {
        match which {
            Which::Ui => message_font(),
            Which::Fixed => None,
        }
    }

    /// `SPI_GETNONCLIENTMETRICS` fills a `NONCLIENTMETRICSW` with the fonts and widths the
    /// shell draws its own chrome with. `lfMessageFont` is the one dialogs and message
    /// boxes use, which is what "the interface font" means on Windows -- and unlike the
    /// registry copy of it, the API answers on a machine that has never changed a setting.
    #[cfg(target_os = "windows")]
    fn message_font() -> Option<Spec> {
        // `cbSize` is neither optional nor a formality: it is how `user32` tells the two
        // layouts of this struct apart, and a call carrying neither of them returns
        // `FALSE` having written nothing. The struct grew an `iPaddedBorderWidth` after
        // XP, so code that had to run there passed the *old* size -- this one less an
        // `i32` -- and got the short answer back. The whole struct is the right answer
        // here: `windows-sys` declares only the post-XP layout, so the short size would
        // leave a field this code can name uninitialised while gaining nothing, and the
        // oldest Windows this app runs on is far past XP anyway (freya wants an OpenGL
        // context XP's drivers do not give it). The rest is zeroed, which is what
        // `Default` is here: `SPI_GETNONCLIENTMETRICS` writes every field of it.
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

        // A `BOOL`, and a `FALSE` is one more "the desktop said nothing": the reason is
        // over in `GetLastError` and there is nothing this file would do differently for
        // any of them.
        if read == 0 {
            return None;
        }

        let font = metrics.lfMessageFont;

        font_spec(&font.lfFaceName, font.lfHeight, metrics_dpi())
    }

    /// The DPI the metrics just read are in, which is the part of this that has to be got
    /// right rather than merely written down.
    ///
    /// `SystemParametersInfoW` answers in whatever DPI space the *process* is in: 96 while
    /// it is still DPI-unaware, the system DPI once winit has made it per-monitor aware.
    /// `GetDeviceCaps(LOGPIXELSY)` on the screen DC is virtualised in exactly the same
    /// way, so the two agree whichever of those two states this was called in -- which is
    /// the reason to read the DPI here rather than to assume one.
    ///
    /// `SystemParametersInfoForDpi(.., 96)` would ask for the answer already in the units
    /// this file works in and drop the division outright. It is deliberately not used:
    /// `windows-sys` links its imports statically, and that entry point -- like
    /// `GetDpiForSystem`, the other way to skip the DC -- exists only from Windows 10
    /// 1607, so naming it turns "the desktop said nothing" into a process that will not
    /// start at all on anything older. winit reaches that same family of functions through
    /// `GetProcAddress` for precisely that reason, and a font setting is not worth
    /// lowering the app's floor under winit's own.
    ///
    /// Dividing at all matters however the DPI arrives: 9pt is `-12` at 96 and `-18` at
    /// 144, while freya is handed logical pixels at a nominal 96 with winit applying the
    /// display's real scale factor on top of them. Passing the 144 DPI number straight
    /// through would scale the font twice.
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

    /// The family and the point size inside a `LOGFONTW`, which is the half of this a
    /// machine that is not Windows can be asked about.
    ///
    /// `lfFaceName` is a fixed `[u16; 32]` and is NUL-terminated only when the name is
    /// shorter than that, so the name runs to the first NUL *or* to the end of the array.
    /// An empty one is not an answer, which [`Spec::new`] is already the judge of.
    ///
    /// `lfHeight` is in logical units at `dpi`: negative for the character height and
    /// positive for the cell height, which is taller by the font's internal leading. The
    /// sign is dropped rather than corrected for -- the difference is a point either way
    /// on a UI font, and every writer of this value uses the negative form.
    pub fn font_spec(face: &[u16; 32], height: i32, dpi: u32) -> Option<Spec> {
        let name: Vec<u16> = face.iter().copied().take_while(|unit| *unit != 0).collect();
        let family = String::from_utf16(&name).ok()?;

        let dpi = if dpi == 0 { NOMINAL_DPI } else { dpi };
        let points = height.unsigned_abs() as f32 * 72.0 / dpi as f32;

        Spec::new(&family, Some(points))
    }
}

/// The desktop's answer for one of the two fonts, as the desktop wrote it down: a
/// family, and a size still in points.
fn query(which: Which) -> Option<Spec> {
    #[cfg(target_os = "windows")]
    return windows::query(which);
    #[cfg(not(target_os = "windows"))]
    return desktop::query(which);
}

/// Whether the desktop has anything left to be asked. A font whose family *and* size the
/// user has both chosen has no unanswered half, so a fully configured app spawns no
/// process at startup at all -- which is the only reason this is a question rather than
/// two unconditional lookups.
fn needs_desktop(setting: &FontSetting) -> bool {
    setting.family().is_none() || setting.size().is_none()
}

/// One font, merged: the user's overrides in front of the desktop's answer, field by
/// field, with the platform's own family behind both.
///
/// Pure, and handed the desktop's answer rather than asking for it, so that the merge --
/// the part with the rules in it -- is testable on a machine with no desktop at all.
fn resolve_font(
    setting: &FontSetting,
    desktop: Option<Spec>,
    default: &'static str,
    default_size: f32,
) -> Font {
    let (family, points) = match desktop {
        Some(desktop) => (Some(desktop.family), desktop.points),
        None => (None, None),
    };

    Font::new(
        setting.family().map(str::to_owned).or(family),
        setting.size().or(points),
        default,
        default_size,
    )
}

fn font(setting: &FontSetting, which: Which, default: &'static str, default_size: f32) -> Font {
    let desktop = needs_desktop(setting).then(|| query(which)).flatten();

    resolve_font(setting, desktop, default, default_size)
}

/// The two fonts these settings and this desktop come to.
///
/// Public and taking the settings by argument, rather than reading them itself, because
/// this is the half of [`fonts`] that survives the settings page: a page that changes a
/// font calls this with the new settings and has the answer, with no cache to invalidate
/// and no process-wide state to write.
///
/// Off a desktop that has anything to say -- no `kreadconfig`, no `gsettings`, no
/// `SystemParametersInfo` -- and with nothing overridden, both fonts are the platform's
/// own at the floem-era sizes.
pub fn resolve(settings: &Settings) -> Fonts {
    Fonts {
        ui: font(&settings.interface, Which::Ui, DEFAULT_UI, DEFAULT_UI_SIZE),
        mono: font(
            &settings.fixed,
            Which::Fixed,
            DEFAULT_MONO,
            DEFAULT_MONO_SIZE,
        ),
    }
}

/// The fonts the UI draws with: the stored settings over the desktop's answer, worked out
/// once and handed out as a `&'static`.
///
/// **This is a `OnceLock`, and a setting the user can change at runtime cannot stay one.**
/// It is left as it is deliberately rather than papered over, because making *this*
/// function re-readable is not the missing piece and would only look like a fix. Two
/// things stand in the way, and both belong to the settings page:
///
/// - **Nothing subscribes to it.** freya re-renders a scope when a state it *read*
///   changes, and a `&'static` is not a read of anything. A `fonts()` that could answer
///   differently would therefore reach only the elements that happened to re-render for
///   some other reason -- half the window in the new font and half in the old, which is
///   worse than a window that is consistently stale. `palette()` had exactly this hole and
///   no longer does, which is the pattern to copy: the appearance behind it is a `State`,
///   so *asking for a colour is what subscribes the caller to the theme*, and a switch
///   repaints every scope that drew one and no other. A `fonts()` shaped like that -- a
///   function over a state rather than a handed-out `&'static` -- is what turns a font
///   change from a restart into a repaint. Note where `palette()` keeps that state: a
///   thread-local `State::create_global`, not a context, precisely because it is read from
///   free functions and render callbacks that cannot run a hook, and `icon_size` and
///   `FontExt` are the same kind of caller.
/// - **`&'static Fonts` is in the signatures around it.** `FontExt::font` in `ui.rs`
///   takes a `&'static Font` precisely because this hands one out, so a `Fonts` with a
///   lifetime shorter than the program cannot be threaded through today without either
///   leaking every rebuild or touching those call sites.
///
/// So what the settings page has to change, exactly:
///
/// 1. Hold the fonts in a state, built with [`resolve`] from the settings that are loaded
///    and replaced when the page writes a new [`Settings`]. A context (a `State<Arc<Fonts>>`
///    beside `Objects` and the rest) if every reader turns out to be a component; a
///    thread-local global, as the appearance is, if any of them stays a free function --
///    which the bullet above is the reason to expect.
/// 2. Move the four readers onto it: `icon_size`, `FontExt::interface_font` and
///    `::assembly_font` -- whose `font(&'static Font)` becomes `font(&Font)` -- and the
///    tooltip's `font_size` in the root `use_init_theme`, which is a hook and so wants the
///    fonts read before it rather than inside a closure that runs once.
/// 3. Decide what happens to `ROW_HEIGHT`. It is a `const` that must equal every
///    `VirtualScrollView`'s `item_size`, and it is sized against the fixed-width font --
///    so a size the reader can change makes the row height a *value* rather than a
///    constant, and the saved per-tab rows, the lane gutter and `reveal_row`'s
///    `CONTEXT_ROWS` are all downstream of it. This is the reason a font change may end up
///    wanting a restart even after the first two are done, and it is a decision, not an
///    oversight to inherit silently.
///
/// [`resolve`] is what survives all of that; this function goes away with its last caller.
pub fn fonts() -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(|| resolve(&Settings::load()))
}

#[cfg(test)]
mod tests {
    use super::desktop::{order, parse_kde, parse_pango, Desktop};
    use super::windows::font_spec;
    use super::*;

    /// What a parser is expected to have found, spelled out at the call site so the
    /// cases read as the specs they came from.
    fn spec(family: &str, points: Option<f32>) -> Option<Spec> {
        Some(Spec {
            family: family.to_owned(),
            points,
        })
    }

    #[test]
    fn a_kde_spec_is_a_family_and_a_size_in_a_list() {
        assert_eq!(
            parse_kde("Noto Sans Mono,10,-1,5,50,0,0,0,0,0"),
            spec("Noto Sans Mono", Some(10.0))
        );
    }

    #[test]
    fn a_kde_size_that_says_nothing_leaves_the_family() {
        // Both shapes a size can fail in. The family is still the desktop's answer, so
        // it survives with the app's own size behind it.
        assert_eq!(parse_kde("Noto Sans"), spec("Noto Sans", None));
        assert_eq!(parse_kde("Noto Sans,0,-1"), spec("Noto Sans", None));
        assert_eq!(parse_kde("Noto Sans,,-1"), spec("Noto Sans", None));
    }

    #[test]
    fn a_pango_description_is_quoted_and_ends_in_its_size() {
        assert_eq!(parse_pango("'Cantarell 11'"), spec("Cantarell", Some(11.0)));
        // Unquoted and fractional: `gsettings` is the only thing that quotes, and Pango
        // sizes are not integers.
        assert_eq!(parse_pango("Cantarell 11.5"), spec("Cantarell", Some(11.5)));
    }

    #[test]
    fn a_pango_family_keeps_its_spaces() {
        // The one thing that separates this spec from KDE's: the size is the last word,
        // not the second field.
        assert_eq!(
            parse_pango("'Source Code Pro 10'"),
            spec("Source Code Pro", Some(10.0))
        );
    }

    #[test]
    fn pango_style_words_are_not_part_of_the_family() {
        assert_eq!(
            parse_pango("'Source Code Pro Semi-Bold 10'"),
            spec("Source Code Pro", Some(10.0))
        );
        // Several of them, in any case, and with no size behind them to find them by.
        assert_eq!(
            parse_pango("'DejaVu Sans Condensed Bold Italic'"),
            spec("DejaVu Sans", None)
        );
        // A description of nothing else keeps one, rather than parsing to no family.
        assert_eq!(parse_pango("Bold 11"), spec("Bold", Some(11.0)));
    }

    #[test]
    fn a_pango_description_can_omit_its_size() {
        assert_eq!(parse_pango("'Cantarell'"), spec("Cantarell", None));
        // And a family whose last word merely looks like one is not a size.
        assert_eq!(parse_pango("'M+ 1m'"), spec("M+ 1m", None));
    }

    #[test]
    fn nothing_is_not_a_font() {
        assert_eq!(parse_kde(""), None);
        assert_eq!(parse_kde(",10"), None);
        assert_eq!(parse_pango(""), None);
        assert_eq!(parse_pango("''"), None);
        assert_eq!(parse_pango("   "), None);
    }

    /// A font setting, spelled at the call site the way the two cases read: what the user
    /// chose, and what they left alone.
    fn setting(family: Option<&str>, size: Option<f32>) -> FontSetting {
        FontSetting {
            family: family.map(str::to_owned),
            size,
        }
    }

    /// The mono defaults, since every case below is one font resolved: `monospace` behind
    /// whatever is chosen, and 14 logical pixels when nothing names a size.
    fn resolved(setting: &FontSetting, desktop: Option<Spec>) -> Font {
        resolve_font(setting, desktop, DEFAULT_MONO, DEFAULT_MONO_SIZE)
    }

    #[test]
    fn nothing_chosen_and_nothing_answered_is_the_platforms_own() {
        let font = resolved(&setting(None, None), None);

        assert_eq!(font.families, ["monospace"]);
        assert_eq!(font.size, DEFAULT_MONO_SIZE);
    }

    #[test]
    fn nothing_chosen_takes_the_desktops_answer() {
        let font = resolved(
            &setting(None, None),
            Spec::new("Noto Sans Mono", Some(10.0)),
        );

        assert_eq!(font.families, ["Noto Sans Mono", "monospace"]);
        assert_eq!(font.size, points_to_pixels(10.0));
    }

    #[test]
    fn a_desktop_answer_with_no_size_keeps_its_family() {
        let font = resolved(&setting(None, None), Spec::new("Noto Sans Mono", None));

        assert_eq!(font.families, ["Noto Sans Mono", "monospace"]);
        assert_eq!(font.size, DEFAULT_MONO_SIZE);
    }

    #[test]
    fn an_override_wins_over_the_desktop() {
        let chosen = setting(Some("Fira Code"), Some(12.0));
        let font = resolved(&chosen, Spec::new("Noto Sans Mono", Some(10.0)));

        // The desktop's family is not even a fallback: the reader named one, so the only
        // thing behind it is the platform's own, which is there so that a family that
        // resolves to nothing cannot leave the assembly view proportional.
        assert_eq!(font.families, ["Fira Code", "monospace"]);
        assert_eq!(font.size, points_to_pixels(12.0));
    }

    #[test]
    fn an_override_stands_where_the_desktop_said_nothing() {
        let font = resolved(&setting(Some("Fira Code"), Some(12.0)), None);

        assert_eq!(font.families, ["Fira Code", "monospace"]);
        assert_eq!(font.size, points_to_pixels(12.0));
    }

    /// The distinction the settings file exists to keep: unspecified is not a value, so
    /// the half the reader left alone still follows the desktop -- and follows it *later*,
    /// when the desktop changes its mind.
    #[test]
    fn an_unspecified_field_falls_through_to_the_desktop() {
        let desktop = || Spec::new("Noto Sans Mono", Some(10.0));

        // A family with no size: the desktop's size.
        let family_only = resolved(&setting(Some("Fira Code"), None), desktop());
        assert_eq!(family_only.families, ["Fira Code", "monospace"]);
        assert_eq!(family_only.size, points_to_pixels(10.0));

        // A size with no family: the desktop's family.
        let size_only = resolved(&setting(None, Some(12.0)), desktop());
        assert_eq!(size_only.families, ["Noto Sans Mono", "monospace"]);
        assert_eq!(size_only.size, points_to_pixels(12.0));
    }

    /// And a value that is present but says nothing is not a choice either, so it falls
    /// through exactly as an absent one does.
    #[test]
    fn a_setting_that_says_nothing_falls_through_too() {
        let font = resolved(
            &setting(Some("  "), Some(0.0)),
            Spec::new("Noto Sans Mono", Some(10.0)),
        );

        assert_eq!(font.families, ["Noto Sans Mono", "monospace"]);
        assert_eq!(font.size, points_to_pixels(10.0));
    }

    #[test]
    fn only_an_unanswered_half_is_worth_a_process() {
        assert!(needs_desktop(&setting(None, None)));
        assert!(needs_desktop(&setting(Some("Fira Code"), None)));
        assert!(needs_desktop(&setting(None, Some(12.0))));
        // Both chosen: there is nothing left to ask.
        assert!(!needs_desktop(&setting(Some("Fira Code"), Some(12.0))));
        // Both merely present: there is.
        assert!(needs_desktop(&setting(Some(""), Some(-1.0))));
    }

    #[test]
    fn the_desktop_variable_only_sorts_the_two() {
        // Both are always tried; the variable says which one first.
        assert_eq!(order("KDE"), [Desktop::Kde, Desktop::Gnome]);
        assert_eq!(order("ubuntu:GNOME"), [Desktop::Gnome, Desktop::Kde]);
        assert_eq!(order("GNOME-Classic:GNOME"), [Desktop::Gnome, Desktop::Kde]);
        assert_eq!(order("X-Cinnamon:Unity"), [Desktop::Gnome, Desktop::Kde]);
        // Absent, or a desktop neither of them recognises: KDE first, and Gnome after it.
        assert_eq!(order(""), [Desktop::Kde, Desktop::Gnome]);
        assert_eq!(order("sway:wlroots"), [Desktop::Kde, Desktop::Gnome]);
    }

    /// A `LOGFONTW`'s `lfFaceName`: UTF-16, NUL-padded, and with no terminator at all
    /// when the name fills all 32 units.
    fn face(name: &str) -> [u16; 32] {
        let mut units = [0u16; 32];

        for (slot, unit) in units.iter_mut().zip(name.encode_utf16()) {
            *slot = unit;
        }

        units
    }

    #[test]
    fn a_logfont_is_a_face_name_and_a_height_at_a_dpi() {
        assert_eq!(
            font_spec(&face("Segoe UI"), -12, 96),
            spec("Segoe UI", Some(9.0))
        );
        // The same font on a 150% machine: the point size is what the two have in common,
        // which is why the DPI the metrics came back in is read beside them.
        assert_eq!(
            font_spec(&face("Segoe UI"), -18, 144),
            spec("Segoe UI", Some(9.0))
        );
        // A positive height is the cell height, taken as it stands.
        assert_eq!(
            font_spec(&face("Segoe UI"), 12, 96),
            spec("Segoe UI", Some(9.0))
        );
        // No DPI is the nominal one, not a division by zero.
        assert_eq!(
            font_spec(&face("Segoe UI"), -12, 0),
            spec("Segoe UI", Some(9.0))
        );
    }

    #[test]
    fn a_face_name_runs_to_the_first_nul_or_to_the_end() {
        // The padding after a short name is not part of it.
        assert_eq!(
            font_spec(&face("MS Shell Dlg 2"), -12, 96),
            spec("MS Shell Dlg 2", Some(9.0))
        );
        // 32 units with no room left for a terminator: the whole array is the name.
        let full = "A".repeat(32);
        assert_eq!(font_spec(&face(&full), -12, 96), spec(&full, Some(9.0)));
    }

    #[test]
    fn a_logfont_that_names_nothing_is_no_font() {
        // An all-NUL face name is a struct nobody filled in rather than a font.
        assert_eq!(font_spec(&face(""), -12, 96), None);
        // A height of zero asks for the font's default height, which is not a size this
        // app can use. The family is still the desktop's answer, so it survives with the
        // app's own size behind it.
        assert_eq!(font_spec(&face("Segoe UI"), 0, 96), spec("Segoe UI", None));
    }
}
