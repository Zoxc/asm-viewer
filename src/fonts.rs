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
//! to something neither of them recognises. Windows has no such tool and is read out of
//! the registry instead.
//!
//! Nothing here fails: a missing binary, a key that was never written, a value in a shape
//! nobody expected and a font size of zero all come back as "the desktop said nothing",
//! which is what the platform defaults are for.

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

/// Which of the two fonts is being asked for. Every desktop names the same pair, and
/// names it differently, so the key belongs where the desktop is rather than at the call
/// site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Which {
    Ui,
    Fixed,
}

impl Which {
    /// What a font is when the desktop named a family but no size. Keeping the family and
    /// falling back on the size is deliberately not "no answer at all": a family is the
    /// half of the setting that is visible in every glyph on screen, and dropping it over
    /// a missing number would put the desktop's font back to `sans-serif`.
    const fn default_size(self) -> f32 {
        match self {
            Which::Ui => DEFAULT_UI_SIZE,
            Which::Fixed => DEFAULT_MONO_SIZE,
        }
    }
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

    fn sized(self, which: Which) -> (String, f32) {
        let size = self.points.map_or(which.default_size(), points_to_pixels);

        (self.family, size)
    }
}

/// Desktops store font sizes in points, freya wants logical pixels. The 96 is the
/// nominal DPI logical pixels are defined at -- the display's real one is winit's
/// business, applied to the whole window as a scale factor on top of this.
fn points_to_pixels(points: f32) -> f32 {
    points * 96.0 / 72.0
}

/// Run a tool and take its stdout, trimmed. Everything that can go wrong is one `None`:
/// the binary is not installed (the ordinary case for whichever desktop this is not), it
/// answered with a failure because the key does not exist, or it wrote something that is
/// not text.
fn output(command: &mut Command) -> Option<String> {
    let output = command.output().ok()?;

    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The desktop lookups, which are the whole of the story on everything that is not
/// Windows. Compiled on Windows too, but only so its parsers stay under test there.
#[cfg(any(not(target_os = "windows"), test))]
#[cfg_attr(target_os = "windows", allow(dead_code))]
mod desktop {
    use super::{output, Spec, Which};
    use std::{env, process::Command, sync::OnceLock};

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

/// Windows, read through `reg.exe` rather than through `SystemParametersInfo`.
///
/// The API call would be the better answer -- `NONCLIENTMETRICS` is what the desktop
/// actually uses, DPI-aware and always populated -- but it needs `windows-sys` as a
/// *direct* dependency, and this crate has none: the three copies in the tree are
/// winit's, accesskit's and skia's, and a dependency cannot be used through another
/// crate's. Undoing this is one `[target.'cfg(windows)'.dependencies] windows-sys` line
/// with the `Win32_UI_WindowsAndMessaging` and `Win32_Graphics_Gdi` features, and an
/// `unsafe` call to `SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, ..)` feeding the
/// `lfMessageFont` into [`parse_logfont`], which is the half of this that is under test.
///
/// What the registry costs by comparison: `MessageFont` is only written once something
/// has changed it, so a machine on its defaults answers nothing and gets `Segoe UI` at
/// the floem-era size -- which is what it would have got anyway.
///
/// Compiled on Linux too, but only for its tests: the parsing is the half of this that
/// can be checked from a machine that is not Windows, and a `cfg` that hid it from
/// `cargo test` would leave it checked nowhere at all.
#[cfg(any(target_os = "windows", test))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod windows {
    use super::{output, Spec, Which};
    use std::process::Command;

    const METRICS: &str = r"HKCU\Control Panel\Desktop\WindowMetrics";

    /// Windows has a message font and no fixed-width font: nothing in the shell settings
    /// names one. The nearest thing is `HKCU\Console`'s `FaceName`, which is the console
    /// host's own choice, is often the raster `Terminal` face that skia cannot use for a
    /// UI, and carries its size as a packed cell in pixels rather than a point size --
    /// three reasons it is not the desktop answering, so `Consolas` stands.
    pub fn query(which: Which) -> Option<Spec> {
        match which {
            Which::Ui => message_font(),
            Which::Fixed => None,
        }
    }

    /// The dialog font, as a `LOGFONTW` blob under `WindowMetrics`, whose sizes are
    /// stored at whatever DPI they were last written at -- `AppliedDPI`, right beside it.
    fn message_font() -> Option<Spec> {
        let font = registry(METRICS, "MessageFont")?;
        let dpi = registry(METRICS, "AppliedDPI")
            .as_deref()
            .and_then(parse_dword)
            .unwrap_or(96);

        parse_logfont(&parse_hex(&font)?, dpi)
    }

    /// The value of one registry entry, as `reg.exe` prints it.
    fn registry(key: &str, value: &str) -> Option<String> {
        let dump = output(Command::new("reg").args(["query", key, "/v", value]))?;

        parse_registry(&dump).map(|(_, value)| value.to_owned())
    }

    /// `reg query` writes a blank line, the key's full name, and then the entry as three
    /// columns: `    MessageFont    REG_BINARY    f5ffffff...`. The value is taken as the
    /// rest of the line rather than as a third whitespace-separated word, because a
    /// `REG_SZ` value contains spaces (`Lucida Console`) and the entry's name never does.
    pub fn parse_registry(dump: &str) -> Option<(&str, &str)> {
        dump.lines().find_map(|line| {
            let (_, rest) = line.split_once("REG_")?;
            let (kind, value) = rest.split_once(char::is_whitespace)?;

            Some((kind, value.trim()))
        })
    }

    /// A `REG_DWORD` prints as `0x60`.
    pub fn parse_dword(value: &str) -> Option<u32> {
        let digits = value.trim().strip_prefix("0x")?;

        u32::from_str_radix(digits, 16).ok()
    }

    /// A `REG_BINARY` prints as one unbroken run of hex digits.
    pub fn parse_hex(value: &str) -> Option<Vec<u8>> {
        let value = value.trim();
        if value.is_empty() || value.len() % 2 != 0 {
            return None;
        }

        (0..value.len() / 2)
            .map(|byte| u8::from_str_radix(&value[byte * 2..byte * 2 + 2], 16).ok())
            .collect()
    }

    /// A `LOGFONTW`: five `LONG`s, eight `BYTE`s, and 32 UTF-16 code units of face name.
    ///
    /// `lfHeight` is negative for the character height and positive for the cell height
    /// (which is taller by the font's internal leading). The sign is dropped rather than
    /// corrected for: the difference is a point either way on a UI font, and every writer
    /// of this value uses the negative form. It is in device units at `dpi`, which is why
    /// the DPI has to come out of the registry with it -- the same font is `-12` on a
    /// 96 DPI machine and `-18` on a 144 DPI one, and only one of them is 9pt.
    pub fn parse_logfont(bytes: &[u8], dpi: u32) -> Option<Spec> {
        const FACE_NAME: usize = 28;
        const SIZE: usize = FACE_NAME + 64;

        let bytes = bytes.get(..SIZE)?;
        let height = i32::from_le_bytes(bytes[..4].try_into().ok()?);

        let name: Vec<u16> = bytes[FACE_NAME..]
            .chunks_exact(2)
            .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
            .take_while(|unit| *unit != 0)
            .collect();
        let family = String::from_utf16(&name).ok()?;

        let dpi = if dpi == 0 { 96 } else { dpi };
        let points = height.unsigned_abs() as f32 * 72.0 / dpi as f32;

        Spec::new(&family, Some(points))
    }
}

/// The desktop's answer for one of the two fonts, in the shape [`Font::new`] wants:
/// a family, and a size already in logical pixels.
fn query(which: Which) -> Option<(String, f32)> {
    #[cfg(target_os = "windows")]
    let spec = windows::query(which);
    #[cfg(not(target_os = "windows"))]
    let spec = desktop::query(which);

    spec.map(|spec| spec.sized(which))
}

/// Off a desktop that has anything to say -- no `kreadconfig`, no `gsettings`, no
/// registry entry -- both fonts are the platform's own at the floem-era sizes.
fn load() -> Fonts {
    Fonts {
        ui: Font::new(query(Which::Ui), DEFAULT_UI, DEFAULT_UI_SIZE),
        mono: Font::new(query(Which::Fixed), DEFAULT_MONO, DEFAULT_MONO_SIZE),
    }
}

pub fn fonts() -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(load)
}

#[cfg(test)]
mod tests {
    use super::desktop::{order, parse_kde, parse_pango, Desktop};
    use super::windows::{parse_dword, parse_hex, parse_logfont, parse_registry};
    use super::{Spec, Which};

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

    #[test]
    fn a_spec_takes_the_default_size_only_when_it_has_none() {
        let sized = Spec::new("Cantarell", Some(12.0)).unwrap().sized(Which::Ui);
        assert_eq!(sized, ("Cantarell".to_owned(), 16.0));

        let defaulted = Spec::new("Cantarell", None).unwrap().sized(Which::Fixed);
        assert_eq!(defaulted, ("Cantarell".to_owned(), 14.0));
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

    /// A `LOGFONTW` with `height` in its first field and `family` in its face name.
    fn logfont(height: i32, family: &str) -> Vec<u8> {
        let mut bytes = height.to_le_bytes().to_vec();
        bytes.resize(28, 0);
        for unit in family.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.resize(92, 0);

        bytes
    }

    #[test]
    fn a_logfont_is_a_face_name_and_a_height_at_a_dpi() {
        assert_eq!(
            parse_logfont(&logfont(-12, "Segoe UI"), 96),
            spec("Segoe UI", Some(9.0))
        );
        // The same font written by a 150% machine: the point size is what the two have
        // in common, which is why the DPI is read out of the registry beside it.
        assert_eq!(
            parse_logfont(&logfont(-18, "Segoe UI"), 144),
            spec("Segoe UI", Some(9.0))
        );
        // A positive height is the cell height, taken as it stands.
        assert_eq!(
            parse_logfont(&logfont(12, "Segoe UI"), 96),
            spec("Segoe UI", Some(9.0))
        );
    }

    #[test]
    fn a_logfont_that_is_not_one_is_no_font() {
        assert_eq!(parse_logfont(&[], 96), None);
        assert_eq!(parse_logfont(&logfont(-12, "Segoe UI")[..91], 96), None);
        assert_eq!(parse_logfont(&logfont(-12, ""), 96), None);
        assert_eq!(
            parse_logfont(&logfont(0, "Segoe UI"), 96),
            spec("Segoe UI", None)
        );
    }

    #[test]
    fn a_registry_dump_is_read_by_its_type_column() {
        let dump = "\r\nHKEY_CURRENT_USER\\Control Panel\\Desktop\\WindowMetrics\r\n    \
                    MessageFont    REG_BINARY    f5ffffff00000000\r\n";
        assert_eq!(parse_registry(dump), Some(("BINARY", "f5ffffff00000000")));

        // A string value keeps its spaces: the value is the rest of the line.
        let dump = "\r\n    FaceName    REG_SZ    Lucida Console\r\n";
        assert_eq!(parse_registry(dump), Some(("SZ", "Lucida Console")));

        // What `reg query` writes for a value that is not there.
        assert_eq!(
            parse_registry("ERROR: The system was unable to find the specified registry key"),
            None
        );
    }

    #[test]
    fn registry_scalars() {
        assert_eq!(parse_dword("0x60"), Some(96));
        assert_eq!(parse_dword("96"), None);
        assert_eq!(parse_hex("f5ff"), Some(vec![0xf5, 0xff]));
        // An odd digit count, a stray character and an empty value are all no answer.
        assert_eq!(parse_hex("f5f"), None);
        assert_eq!(parse_hex("f5fg"), None);
        assert_eq!(parse_hex(""), None);
    }
}
