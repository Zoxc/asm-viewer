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
        rect()
            .width(Size::flex(1.0))
            // Taller than a plain field row: what these rows hold is a box to type in or
            // a stepper, either of which is taller than a line of text.
            .height(Size::px(text_box_height()))
            .horizontal()
            .cross_align(Alignment::Center)
            .content(Content::Flex)
            .spacing(8.0)
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
                        false => label()
                            .text("inherited")
                            .color(palette().address_fg)
                            .max_lines(1)
                            .into_element(),
                    }),
            ),
    )
}

/// One of the two fonts, as three rows: the family, the size, and a line of the font
/// itself.
fn font_section(
    title: &str,
    edited: EditedFont,
    inherited: &Font,
    resolved: &Font,
    family: Writable<String>,
    size: impl FnMut(Option<f32>) + Clone + 'static,
) -> Element {
    let inherited_family = inherited.family();
    // What the stepper moves from: the reader's size where there is one, otherwise the
    // one being inherited.
    let points = edited.size.unwrap_or(inherited.points);
    let step = |by: f32| {
        let mut size = size.clone();
        move |_: Event<PressEventData>| {
            // The bounds are on the *stepper* only: a hand-edited `settings.toml` may
            // still say anything.
            let moved = (points + by).clamp(5.0, 32.0);
            // Back onto the half-point grid, so stepping away from a desktop's 13.75 and
            // back lands on its neighbours rather than on a drift of its own.
            size(Some((moved / SIZE_STEP).round() * SIZE_STEP));
        }
    };
    let mut clear_size = size.clone();

    rect()
        .width(Size::fill())
        .child(section_heading(title, None))
        .child(setting_row(
            "Family",
            given(&edited.family).is_some(),
            Input::new(family.clone())
                .placeholder(inherited_family)
                .compact()
                .width(Size::flex(1.0)),
            move |_| family.clone().set(String::new()),
        ))
        .child(setting_row(
            "Size",
            edited.size.is_some(),
            rect()
                .width(Size::flex(1.0))
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(6.0)
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
            move |_| clear_size(None),
        ))
        .child(
            rect()
                .width(Size::fill())
                .padding(Gaps::new(2.0, 0.0, 8.0, field_label_width() + 8.0))
                .overflow(Overflow::Clip)
                .child(
                    label()
                        .text("Disassembly 0123 l1I O0 {}")
                        .font(resolved)
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
        // What the reader would get with nothing set, and what they are getting now.
        // What every unspecified field is falling through to, which is what the page
        // draws in an empty box: `resolve` of the default settings and not a lookup of
        // its own, so the value shown is by construction the value that would be used.
        let inherited = fonts::resolve(&Settings::default());
        let resolved = fonts::resolve(&edited.settings());

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

        rect()
            .expanded()
            .background(palette().pane_bg)
            .child(
                ScrollView::new().child(
                    rect()
                        .width(Size::fill())
                        .padding(Gaps::new_symmetric(8.0, 12.0))
                        .spacing(6.0)
                        .child(section_heading("Appearance", None))
                        .child(field_row(
                            "Theme",
                            SegmentedButton::new().children(themes.map(|(choice, text)| {
                                ButtonSegment::new()
                                    .key(text)
                                    .selected(edited.theme == choice)
                                    .on_press(move |_| {
                                        prefs.write().theme = choice;
                                    })
                                    .child(text)
                                    .into()
                            })),
                        ))
                        .maybe_child(following)
                        .child(font_section(
                            "Interface font",
                            edited.interface.clone(),
                            &inherited.ui,
                            &resolved.ui,
                            prefs.into_writable().map(
                                |edited| &edited.interface.family,
                                |edited| &mut edited.interface.family,
                            ),
                            move |size| prefs.write().interface.size = size,
                        ))
                        .child(font_section(
                            "Fixed-width font",
                            edited.fixed.clone(),
                            &inherited.mono,
                            &resolved.mono,
                            prefs.into_writable().map(
                                |edited| &edited.fixed.family,
                                |edited| &mut edited.fixed.family,
                            ),
                            move |size| prefs.write().fixed.size = size,
                        ))
                        // The one consequence of a font change that is not a font, and two
                        // numbers rather than one because each half of the page above
                        // moves exactly one of them.
                        .child(info_line(format!(
                            "Rows follow the font they are drawn in: {} pixels in the \
                             lists, {} in the code panes.",
                            points_text(list_row_height()),
                            points_text(code_row_height())
                        ))),
                ),
            )
            .into_element()
    }
}
