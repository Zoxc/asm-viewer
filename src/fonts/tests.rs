use super::desktop_parse::{kde, order, pango, Desktop};
use super::windows_parse::logfont;
use super::*;

fn spec(family: &str, points: Option<f32>) -> Option<Spec> {
    Some(Spec {
        family: family.to_owned(),
        points,
    })
}

#[test]
fn a_kde_spec_is_a_family_and_a_size_in_a_list() {
    assert_eq!(
        kde("Noto Sans Mono,10,-1,5,50,0,0,0,0,0"),
        spec("Noto Sans Mono", Some(10.0))
    );
}

#[test]
fn a_kde_size_that_says_nothing_leaves_the_family() {
    assert_eq!(kde("Noto Sans"), spec("Noto Sans", None));
    assert_eq!(kde("Noto Sans,0,-1"), spec("Noto Sans", None));
    assert_eq!(kde("Noto Sans,,-1"), spec("Noto Sans", None));
}

#[test]
fn a_pango_description_is_quoted_and_ends_in_its_size() {
    assert_eq!(pango("'Cantarell 11'"), spec("Cantarell", Some(11.0)));
    // Unquoted and fractional: `gsettings` is the only thing that quotes, and Pango
    // sizes are not integers.
    assert_eq!(pango("Cantarell 11.5"), spec("Cantarell", Some(11.5)));
}

/// The one thing that separates this spec from KDE's: the size is the last word, not
/// the second field, so the family keeps its spaces.
#[test]
fn a_pango_family_keeps_its_spaces() {
    assert_eq!(
        pango("'Source Code Pro 10'"),
        spec("Source Code Pro", Some(10.0))
    );
}

#[test]
fn pango_style_words_are_not_part_of_the_family() {
    assert_eq!(
        pango("'Source Code Pro Semi-Bold 10'"),
        spec("Source Code Pro", Some(10.0))
    );
    // Several of them, and with no size behind them to find them by.
    assert_eq!(
        pango("'DejaVu Sans Condensed Bold Italic'"),
        spec("DejaVu Sans", None)
    );
    // A description of nothing else keeps one, rather than parsing to no family.
    assert_eq!(pango("Bold 11"), spec("Bold", Some(11.0)));
}

#[test]
fn a_pango_description_can_omit_its_size() {
    assert_eq!(pango("'Cantarell'"), spec("Cantarell", None));
    // And a family whose last word merely looks like one is not a size.
    assert_eq!(pango("'M+ 1m'"), spec("M+ 1m", None));
}

/// Pango's family is a list: the first name in it is the family, and the commas are not
/// part of it.
#[test]
fn a_pango_family_list_names_its_first_family() {
    assert_eq!(pango("'Cantarell, 11'"), spec("Cantarell", Some(11.0)));
    assert_eq!(
        pango("'Noto Sans,Noto Color Emoji 11'"),
        spec("Noto Sans", Some(11.0))
    );
    assert_eq!(
        pango("'DejaVu Sans, Bold 10'"),
        spec("DejaVu Sans", Some(10.0))
    );
    // A comma ends the style words: what is before it is all family.
    assert_eq!(pango("'Foo Bold, 10'"), spec("Foo Bold", Some(10.0)));
    assert_eq!(pango("', 10'"), None);
}

#[test]
fn nothing_is_not_a_font() {
    assert_eq!(kde(""), None);
    assert_eq!(kde(",10"), None);
    assert_eq!(pango(""), None);
    assert_eq!(pango("''"), None);
    assert_eq!(pango("   "), None);
}

fn setting(family: Option<&str>, size: Option<f32>) -> FontSetting {
    FontSetting {
        family: family.map(str::to_owned),
        size,
    }
}

/// One font resolved against the mono defaults, since that is what every case below is.
fn resolved(setting: &FontSetting, desktop: Option<Spec>) -> Font {
    resolve_font(setting, desktop.as_ref(), Which::Fixed)
}

#[test]
fn nothing_chosen_and_nothing_answered_is_the_platforms_own() {
    let font = resolved(&setting(None, None), None);

    assert_eq!(font.families, ["monospace"]);
    assert_eq!(font.points, FIXED_FACTS.points);
}

#[test]
fn nothing_chosen_takes_the_desktops_answer() {
    let font = resolved(
        &setting(None, None),
        Spec::new("Noto Sans Mono", Some(10.0)),
    );

    assert_eq!(font.families, ["Noto Sans Mono", "monospace"]);
    assert_eq!(font.points, 10.0);
}

#[test]
fn a_desktop_answer_with_no_size_keeps_its_family() {
    let font = resolved(&setting(None, None), Spec::new("Noto Sans Mono", None));

    assert_eq!(font.families, ["Noto Sans Mono", "monospace"]);
    assert_eq!(font.points, FIXED_FACTS.points);
}

#[test]
fn an_override_wins_over_the_desktop() {
    let chosen = setting(Some("Fira Code"), Some(12.0));
    let font = resolved(&chosen, Spec::new("Noto Sans Mono", Some(10.0)));

    // The desktop's family is not even a fallback: the only thing behind a chosen one is
    // the platform's own, so that a family resolving to nothing cannot leave the assembly
    // view proportional.
    assert_eq!(font.families, ["Fira Code", "monospace"]);
    assert_eq!(font.points, 12.0);

    // And it stands where the desktop said nothing at all.
    let alone = resolved(&setting(Some("Fira Code"), Some(12.0)), None);
    assert_eq!(alone.families, ["Fira Code", "monospace"]);
    assert_eq!(alone.points, 12.0);
}

/// Unspecified is not a value, so the half the reader left alone still follows the
/// desktop.
#[test]
fn an_unspecified_field_falls_through_to_the_desktop() {
    let desktop = || Spec::new("Noto Sans Mono", Some(10.0));

    // A family with no size: the desktop's size.
    let family_only = resolved(&setting(Some("Fira Code"), None), desktop());
    assert_eq!(family_only.families, ["Fira Code", "monospace"]);
    assert_eq!(family_only.points, 10.0);

    // A size with no family: the desktop's family.
    let size_only = resolved(&setting(None, Some(12.0)), desktop());
    assert_eq!(size_only.families, ["Noto Sans Mono", "monospace"]);
    assert_eq!(size_only.points, 12.0);
}

/// And a value that is present but says nothing falls through exactly as an absent one
/// does.
#[test]
fn a_setting_that_says_nothing_falls_through_too() {
    let font = resolved(
        &setting(Some("  "), Some(0.0)),
        Spec::new("Noto Sans Mono", Some(10.0)),
    );

    assert_eq!(font.families, ["Noto Sans Mono", "monospace"]);
    assert_eq!(font.points, 10.0);
}

/// Points in, pixels out, once and at the end: 10.5pt is the 14 logical pixels the floem
/// version drew at, and a size that came from an override converts the same way.
#[test]
fn points_become_pixels_once_and_at_the_end() {
    assert_eq!(resolved(&setting(None, None), None).size(), 14.0);
    assert_eq!(resolved(&setting(None, Some(12.0)), None).size(), 16.0);
}

/// What an unspecified field falls through to is [`resolve`] with nothing said, asserted
/// as a *relationship* since what this machine's desktop answers is not something a test
/// may know: overriding one half leaves the other exactly as inherited.
#[test]
fn an_unset_field_is_showing_what_it_falls_through_to() {
    let inherited = resolve(&Settings::default());

    let one_half = Settings {
        fixed: FontSetting {
            family: None,
            size: Some(13.0),
        },
        ..Settings::default()
    };
    let resolved = resolve(&one_half);

    // The interface font was not mentioned at all, so it is the inherited one entire.
    assert_eq!(resolved.ui, inherited.ui);
    // The half that was: the size is the reader's, the family is still inherited.
    assert_eq!(resolved.mono.points, 13.0);
    assert_eq!(resolved.mono.families, inherited.mono.families);
}

/// The key names are the whole of the contract with each desktop, and a wrong one fails
/// quietly: a key `kreadconfig` or `gsettings` does not know is the same `None` as a tool
/// that is not installed, so the app draws the fallback font and says nothing. The rest of
/// each row is pinned by what the merge falls back to; the keys are asserted here because
/// nothing else on this machine reads them.
#[test]
fn each_font_carries_the_key_each_desktop_keeps_it_under() {
    assert_eq!((UI_FACTS.kde, UI_FACTS.gnome), ("font", "font-name"));
    assert_eq!(
        (FIXED_FACTS.kde, FIXED_FACTS.gnome),
        ("fixed", "monospace-font-name")
    );
}

#[test]
fn the_desktop_variable_only_sorts_the_two() {
    assert_eq!(order("KDE"), [Desktop::Kde, Desktop::Gnome]);
    assert_eq!(order("ubuntu:GNOME"), [Desktop::Gnome, Desktop::Kde]);
    assert_eq!(order("GNOME-Classic:GNOME"), [Desktop::Gnome, Desktop::Kde]);
    assert_eq!(order("X-Cinnamon:Unity"), [Desktop::Gnome, Desktop::Kde]);
    // Absent, or a desktop neither of them recognises: KDE first, and Gnome after it.
    assert_eq!(order(""), [Desktop::Kde, Desktop::Gnome]);
    assert_eq!(order("sway:wlroots"), [Desktop::Kde, Desktop::Gnome]);
    // Whatever launched the session set the variable, so a name need not be ASCII and its
    // fifth byte need not be a character boundary. "ÖÖÖ" is six bytes, with boundaries
    // at 0, 2, 4 and 6.
    assert_eq!(order("ÖÖÖ"), [Desktop::Kde, Desktop::Gnome]);
    assert_eq!(order("ÖÖÖ:GNOME"), [Desktop::Gnome, Desktop::Kde]);
}

/// A `LOGFONTW`'s `lfFaceName`: UTF-16, NUL-padded, and with no terminator at all when
/// the name fills all 32 units.
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
        logfont(&face("Segoe UI"), -12, 96),
        spec("Segoe UI", Some(9.0))
    );
    // The same font on a 150% machine: the point size is what the two have in common,
    // which is why the DPI the metrics came back in is read beside them.
    assert_eq!(
        logfont(&face("Segoe UI"), -18, 144),
        spec("Segoe UI", Some(9.0))
    );
    // A positive height is the cell height, taken as it stands.
    assert_eq!(
        logfont(&face("Segoe UI"), 12, 96),
        spec("Segoe UI", Some(9.0))
    );
    // No DPI is the nominal one, not a division by zero.
    assert_eq!(
        logfont(&face("Segoe UI"), -12, 0),
        spec("Segoe UI", Some(9.0))
    );
}

#[test]
fn a_face_name_runs_to_the_first_nul_or_to_the_end() {
    assert_eq!(
        logfont(&face("MS Shell Dlg 2"), -12, 96),
        spec("MS Shell Dlg 2", Some(9.0))
    );
    // 32 units with no room left for a terminator: the whole array is the name.
    let full = "A".repeat(32);
    assert_eq!(logfont(&face(&full), -12, 96), spec(&full, Some(9.0)));
}

#[test]
fn a_logfont_that_names_nothing_is_no_font() {
    // An all-NUL face name is a struct nobody filled in rather than a font.
    assert_eq!(logfont(&face(""), -12, 96), None);
    // A height of zero asks for the font's default height, which is not a size this app
    // can use; the family still survives, with the app's own size behind it.
    assert_eq!(logfont(&face("Segoe UI"), 0, 96), spec("Segoe UI", None));
}

/// Both halves of the `or_else` in [`resolve_font`] judge a size by the one rule, so a
/// size neither source could mean is refused whichever side it came from. Two copies of
/// the rule could be tightened one at a time, and the chain would then take from one
/// source what it had just refused from the other.
#[test]
fn a_size_out_of_range_is_refused_from_either_source() {
    let huge = settings::MAX_POINTS + 1.0;

    // The reader's, out of `settings.toml` or the settings page.
    assert_eq!(
        resolved(&setting(None, Some(huge)), None).points,
        FIXED_FACTS.points
    );

    // The desktop's, refused as the answer is parsed, so the family it named still
    // stands and only the size falls back.
    let desktop = pango(&format!("'Fira Code {huge}'"));
    assert_eq!(desktop, spec("Fira Code", None));

    let font = resolved(&setting(None, None), desktop);
    assert_eq!(font.families, ["Fira Code", "monospace"]);
    assert_eq!(font.points, FIXED_FACTS.points);
}
