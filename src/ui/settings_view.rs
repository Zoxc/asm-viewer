//! The settings page: the theme choice and the two font overrides, and the wiring
//! between what it edits and the appearance, the fonts and `settings.toml`.
//!
//! An override is drawn differently from the value it would replace, which is why
//! `settings.rs` keeps `None` as a real third state.

use super::*;

/// Resolve the appearance from the stored choice and the platform's own, and write it
/// through [`set_appearance`] -- the one function that may change it, and so the one that
/// empties `HIGHLIGHTED`.
///
/// **Not a `use_hook`**: reading `Platform::preferred_theme` subscribes this scope, so a
/// desktop that goes dark while the app is running repaints. It resolves in the render
/// body rather than in an effect, an effect being a frame late and a frame late on a dark
/// desktop a white flash; the write is idempotent, so that costs nothing.
pub(crate) fn use_theme(choice: ThemeChoice) {
    let preferred = *Platform::get().preferred_theme.read();

    set_appearance(resolve_appearance(choice, preferred));
}

/// The whole of the wiring between the settings and what they are settings of: the
/// appearance, the fonts, and `settings.toml`. The write is handed in because
/// [`Settings::save`] writes the machine's real settings file, so a test that mounted
/// this would be editing the settings of whoever ran it.
pub(crate) fn use_settings_with(
    prefs: State<EditedSettings>,
    mut save: impl FnMut(&Settings) + 'static,
) {
    // What the file currently says -- not what was loaded. It has to *move*, or a reader
    // who changes a setting and changes it back would leave the file holding the middle
    // answer. An `Rc<RefCell>` rather than a `State`, since nothing renders from it.
    let written = use_hook(|| Rc::new(RefCell::new(prefs.peek().settings())));
    let settings = prefs.read().settings();

    use_theme(settings.theme);

    use_side_effect_with_deps(&settings, move |settings: &Settings| {
        set_fonts(fonts::resolve(settings));

        let mut written = written.borrow_mut();
        if *settings != *written {
            *written = settings.clone();
            save(settings);
        }
    });
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
    rect()
        .width(Size::fill())
        .height(Size::px(list_row_height() + 8.0))
        .horizontal()
        .cross_align(Alignment::Center)
        .content(Content::Flex)
        .spacing(8.0)
        .child(
            label()
                .text(name.to_owned())
                .width(Size::px(FIELD_LABEL_WIDTH))
                .color(match overridden {
                    true => palette().text_fg,
                    false => palette().address_fg,
                })
                .max_lines(1),
        )
        .child(value)
        .child(
            rect()
                // Wide enough for the **Clear** button, so the value boxes above and
                // below one another end at the same x whichever state each is in.
                .width(Size::px(76.0))
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
    let inherited_family = inherited
        .families
        .first()
        .map(|family| family.to_string())
        .unwrap_or_default();
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
                        // A fixed column, so `+` does not move under the finger as the
                        // number beside it grows a digit.
                        .width(Size::px(52.0))
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
                .padding(Gaps::new(2.0, 0.0, 8.0, FIELD_LABEL_WIDTH + 8.0))
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
