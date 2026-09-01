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
/// whatever is chosen, and 10.5 points -- the 14 logical pixels the floem version drew
/// at -- when nothing names a size.
fn resolved(setting: &FontSetting, desktop: Option<Spec>) -> Font {
    resolve_font(setting, desktop.as_ref(), DEFAULT_MONO, DEFAULT_MONO_POINTS)
}

#[test]
fn nothing_chosen_and_nothing_answered_is_the_platforms_own() {
    let font = resolved(&setting(None, None), None);

    assert_eq!(font.families, ["monospace"]);
    assert_eq!(font.points, DEFAULT_MONO_POINTS);
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
    assert_eq!(font.points, DEFAULT_MONO_POINTS);
}

#[test]
fn an_override_wins_over_the_desktop() {
    let chosen = setting(Some("Fira Code"), Some(12.0));
    let font = resolved(&chosen, Spec::new("Noto Sans Mono", Some(10.0)));

    // The desktop's family is not even a fallback: the reader named one, so the only
    // thing behind it is the platform's own, which is there so that a family that
    // resolves to nothing cannot leave the assembly view proportional.
    assert_eq!(font.families, ["Fira Code", "monospace"]);
    assert_eq!(font.points, 12.0);
}

#[test]
fn an_override_stands_where_the_desktop_said_nothing() {
    let font = resolved(&setting(Some("Fira Code"), Some(12.0)), None);

    assert_eq!(font.families, ["Fira Code", "monospace"]);
    assert_eq!(font.points, 12.0);
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
    assert_eq!(family_only.points, 10.0);

    // A size with no family: the desktop's family.
    let size_only = resolved(&setting(None, Some(12.0)), desktop());
    assert_eq!(size_only.families, ["Noto Sans Mono", "monospace"]);
    assert_eq!(size_only.points, 12.0);
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
    assert_eq!(font.points, 10.0);
}

/// Points in, pixels out, once and at the end.
///
/// The two numbers on the right are the sizes the floem version drew at, which is what
/// makes 9pt and 10.5pt a change of unit rather than of value -- and the reason the
/// unit matters at all is the settings page: an override is stored in points, so a
/// default spelled in pixels would be the one value on that page that could not be
/// compared with the box above it.
#[test]
fn points_become_pixels_once_and_at_the_end() {
    let ui = resolved(&setting(None, None), None);
    assert_eq!(DEFAULT_UI_POINTS * 96.0 / 72.0, 12.0);
    assert_eq!(DEFAULT_MONO_POINTS * 96.0 / 72.0, 14.0);
    assert_eq!(ui.points, DEFAULT_MONO_POINTS);
    assert_eq!(ui.size(), 14.0);

    // And nothing else converts: a size that came from an override is the same kind of
    // number as one that came from a desktop.
    let chosen = resolved(&setting(None, Some(12.0)), None);
    assert_eq!(chosen.points, 12.0);
    assert_eq!(chosen.size(), 16.0);
}

/// What [`inherited`] means, which is the settings page's whole empty state: it is what
/// [`resolve`] answers with nothing said, so a field the reader has not set shows the
/// value it is actually falling through to and not a guess at one.
///
/// Asserted as a *relationship* rather than against any family or size, since what this
/// machine's desktop answers is not something a test may know: overriding one half
/// leaves the other half exactly as inherited, and overriding neither leaves both.
#[test]
fn an_unset_field_is_showing_what_it_falls_through_to() {
    let inherited = inherited();
    assert_eq!(resolve(&Settings::default()), inherited);

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
