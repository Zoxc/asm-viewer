//! The settings page: the theme choice and the two font overrides. What it edits reaches
//! the appearance, the fonts and `settings.toml` through `use_settings_with`, which lives
//! in `session.rs`.
//!
//! An override is drawn differently from the value it would replace, which is why
//! `settings.rs` keeps `None` as a real third state.

use super::*;

/// The user's settings as the settings page has them. [`OpenProject`]'s shape, and for its
/// reason: a family is a `String` here and an `Option<String>` in `Settings`, and
/// [`EditedSettings::settings`] is the one place the two spellings meet. A size is edited
/// by a stepper rather than a text box, so it needs no such treatment.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EditedSettings {
    pub(crate) theme: ThemeChoice,
    pub(crate) interface: EditedFont,
    pub(crate) fixed: EditedFont,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EditedFont {
    pub(crate) family: String,
    /// In points, like the file and like [`Font::points`], so the number on screen, the
    /// number the desktop answered and the number written down are one number.
    pub(crate) size: Option<f32>,
}

impl EditedSettings {
    /// The settings as they were read off disk.
    pub(crate) fn of(settings: &Settings) -> EditedSettings {
        EditedSettings {
            theme: settings.theme,
            interface: EditedFont::of(&settings.interface),
            fixed: EditedFont::of(&settings.fixed),
        }
    }

    /// What of this reaches `settings.toml` -- and, through [`fonts::resolve`], what is on
    /// screen.
    pub(crate) fn settings(&self) -> Settings {
        Settings {
            theme: self.theme,
            interface: self.interface.setting(),
            fixed: self.fixed.setting(),
        }
    }
}

impl EditedFont {
    pub(crate) fn of(setting: &FontSetting) -> EditedFont {
        EditedFont {
            family: setting.family().unwrap_or_default().to_owned(),
            size: setting.size(),
        }
    }

    pub(crate) fn setting(&self) -> FontSetting {
        FontSetting {
            family: given(&self.family).map(str::to_owned),
            size: self.size,
        }
    }
}

/// How far one press of the size stepper moves a font. Half a point is the granularity
/// the desktops themselves store.
const SIZE_STEP: f32 = 0.5;

/// A point size as the page writes it: `9`, `10.5`, and never `10.50` or `9.0`. Rounded
/// for display only -- the value stored is the value stepped.
fn points_text(points: f32) -> String {
    let rounded = (points * 10.0).round() / 10.0;

    match rounded.fract() == 0.0 {
        true => format!("{rounded:.0}"),
        false => format!("{rounded:.1}"),
    }
}

/// One overridable setting: its name, what it says, and whether that is the reader's own
/// answer or the one they are inheriting. Three cues -- the name's colour, real text
/// against a placeholder, and a **Clear** button that is the only way back to
/// unspecified.
fn setting_row(
    name: &str,
    overridden: bool,
    value: impl IntoElement,
    clear: impl FnMut(Event<PressEventData>) + 'static,
) -> impl IntoElement {
    field_row_in(
        name,
        match overridden {
            true => palette().text_fg,
            false => palette().address_fg,
        },
        value_row()
            // Taller than a plain field row: what these rows hold is a box to type in or
            // a stepper, either of which is taller than a line of text.
            .height(Size::px(text_box_height()))
            .child(value)
            .child(
                rect()
                    .width(Size::px(CLEAR_CELL_WIDTH))
                    .horizontal()
                    .main_align(Alignment::End)
                    .cross_align(Alignment::Center)
                    .child(match overridden {
                        true => Button::new()
                            .compact()
                            .on_press(clear)
                            .child("Clear")
                            .into_element(),
                        false => dim_line("inherited").into_element(),
                    }),
            ),
    )
}

/// Which of the two fonts a section of the page is about. Every difference between the
/// two sections is under it, so a section's boxes cannot come to be about two fonts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Interface,
    Fixed,
}

impl Which {
    /// The heading over the section.
    fn title(self) -> &'static str {
        match self {
            Which::Interface => "Interface font",
            Which::Fixed => "Fixed-width font",
        }
    }

    /// This font's half of what the reader has edited.
    fn edited(self, edited: &EditedSettings) -> &EditedFont {
        match self {
            Which::Interface => &edited.interface,
            Which::Fixed => &edited.fixed,
        }
    }

    /// The same half written: what the family box and the size stepper both go through.
    fn edited_mut(self, edited: &mut EditedSettings) -> &mut EditedFont {
        match self {
            Which::Interface => &mut edited.interface,
            Which::Fixed => &mut edited.fixed,
        }
    }

    /// This font's half of a resolved pair.
    fn font(self, fonts: &Fonts) -> &Font {
        match self {
            Which::Interface => &fonts.ui,
            Which::Fixed => &fonts.mono,
        }
    }
}

/// One of the two fonts as its section needs it: which one it is, the state its boxes
/// write, what the reader has said about it, what an unset field falls through to, what it
/// comes to on screen, and the box the family is typed in.
///
/// One struct built in one place ([`half`]) rather than six arguments, because every field
/// here has to be about the *same* font. Spelled out at the call site, that was the
/// caller's to get right once per section.
struct FontHalf {
    which: Which,
    prefs: State<EditedSettings>,
    edited: EditedFont,
    inherited: Font,
    resolved: Font,
    family: Writable<String>,
}

/// One section's worth of that, out of the one selector.
fn half(prefs: State<EditedSettings>, which: Which) -> FontHalf {
    // What the reader would get with nothing set: `resolve` of the default settings and
    // not a lookup of its own, so the value shown in an empty box is by construction the
    // value that would be used.
    let inherited = fonts::resolve(&Settings::default());
    // And what they are getting -- the pair the window is drawn in, which the root has
    // already resolved from this same state. Asking for it is also what repaints the
    // section when it changes, as everything else that draws a glyph repaints.
    let resolved = fonts();

    FontHalf {
        which,
        prefs,
        edited: which.edited(&prefs.read()).clone(),
        inherited: which.font(&inherited).clone(),
        resolved: which.font(&resolved).clone(),
        family: prefs.into_writable().map(
            move |edited: &EditedSettings| &which.edited(edited).family,
            move |edited: &mut EditedSettings| &mut which.edited_mut(edited).family,
        ),
    }
}

/// One of the two fonts, as three rows: the family, the size, and a line of the font
/// itself.
fn font_section(half: FontHalf) -> Element {
    let FontHalf {
        which,
        prefs,
        edited,
        inherited,
        resolved,
        family,
    } = half;
    let inherited_family = inherited.family();
    // What the stepper moves from: the reader's size where there is one, otherwise the
    // one being inherited.
    let points = edited.size.unwrap_or(inherited.points);
    // The one write the stepper and its **Clear** button both make.
    let set_size = move |size: Option<f32>| {
        let mut prefs = prefs;
        which.edited_mut(&mut prefs.write()).size = size;
    };
    let step = move |by: f32| {
        move |_: Event<PressEventData>| {
            // The bounds are on the *stepper* only: a hand-edited `settings.toml` may
            // still say anything.
            let moved = (points + by).clamp(5.0, 32.0);
            // Back onto the half-point grid, so stepping away from a desktop's 13.75 and
            // back lands on its neighbours rather than on a drift of its own.
            set_size(Some((moved / SIZE_STEP).round() * SIZE_STEP));
        }
    };

    section(which.title(), None)
        .child(setting_row(
            "Family",
            given(&edited.family).is_some(),
            Input::new(family.clone())
                .placeholder(inherited_family)
                .compact()
                .width(Size::flex(1.0))
                .on_pre_key_down(plain_keys()),
            move |_| family.clone().set(String::new()),
        ))
        .child(setting_row(
            "Size",
            edited.size.is_some(),
            value_row()
                .child(
                    Button::new()
                        .compact()
                        .on_press(step(-SIZE_STEP))
                        .child("-"),
                )
                .child(
                    label()
                        .text(format!("{} pt", points_text(points)))
                        .width(Size::px(SIZE_READOUT_WIDTH))
                        .text_align(TextAlign::Center)
                        .color(match edited.size {
                            Some(_) => palette().text_fg,
                            None => palette().address_fg,
                        })
                        .max_lines(1),
                )
                .child(Button::new().compact().on_press(step(SIZE_STEP)).child("+")),
            move |_| set_size(None),
        ))
        .child(
            rect()
                .width(Size::fill())
                .padding(Gaps::new(2.0, 0.0, 8.0, field_label_width() + 8.0))
                .overflow(Overflow::Clip)
                .child(
                    label()
                        .text("Disassembly 0123 l1I O0 {}")
                        .font(&resolved)
                        .color(palette().text_fg)
                        .max_lines(1),
                ),
        )
        .into()
}

/// The Settings pane: the theme, the two fonts, and which of those the reader has actually
/// chosen. Every control writes straight into `Prefs`, and [`use_settings_with`] at the root is
/// what turns that into a font, a theme and a file -- there is no Apply button, so the
/// whole window is the interface font's preview.
#[derive(PartialEq)]
pub(crate) struct SettingsTab;

impl Component for SettingsTab {
    fn render(&self) -> impl IntoElement {
        let mut prefs = use_consume::<Prefs>().0;
        let edited = prefs.read().clone();

        // Only a question at all under `Desktop`. Reading it here also subscribes this
        // pane, so the line follows a desktop that changes its mind.
        let following = (edited.theme == ThemeChoice::Desktop).then(|| {
            let preferred = *Platform::get().preferred_theme.read();

            info_line(format!(
                "Following the desktop, which prefers {}.",
                match preferred {
                    PreferredTheme::Light => "light",
                    PreferredTheme::Dark => "dark",
                }
            ))
            .into_element()
        });

        let themes = [
            (ThemeChoice::Light, "Light"),
            (ThemeChoice::Dark, "Dark"),
            (ThemeChoice::Desktop, "Desktop"),
        ];

        page(
            None,
            page_column()
                .child(
                    section("Appearance", None)
                        .child(field_row(
                            "Theme",
                            choice(&themes, edited.theme, move |theme| {
                                prefs.write().theme = theme;
                            }),
                        ))
                        .maybe_child(following),
                )
                .child(font_section(half(prefs, Which::Interface)))
                .child(font_section(half(prefs, Which::Fixed)))
                // The one consequence of a font change that is not a font, and two numbers
                // rather than one because each half of the page above moves exactly one of
                // them. Whole numbers, so nothing is lost rounding them: a row is its
                // font's size plus its leading, itself rounded (`row_height_for`).
                .child(info_line(format!(
                    "Rows follow the font they are drawn in: {:.0} pixels in the \
                     lists, {:.0} in the code panes.",
                    list_row_height(),
                    code_row_height()
                ))),
        )
        .into_element()
    }
}
