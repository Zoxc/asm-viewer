//! The settings page, and the wiring between what it edits and what those settings are
//! settings *of* -- the appearance, the two fonts, and `settings.toml`.
//!
//! **An override is drawn differently from the value it would replace**, which is why
//! `settings.rs` keeps `None` as a real third state: an unspecified field is a key that is
//! absent from the file, not the desktop's current answer copied into it, and the page can
//! therefore say which of the two the reader is looking at.
//!
//! A family is a text box and a size is a stepper, so only one of the two has a "the reader
//! has not said" spelling to preserve: an empty box *is* how a family is unset, and there
//! is nothing a stepper could be nudged to that would mean the same, which is what the
//! **Clear** button is for.
//!
//! The write is compared against what the file currently says rather than against what was
//! loaded, `Saves::written`'s rule: a reader who changes a setting and changes it back
//! leaves the file alone, and a run that never opened this page writes no file at all.

use super::*;

/// The whole of the wiring between the stored choice and what is drawn: read both inputs,
/// resolve them, and write the answer down through [`set_appearance`] -- the one function
/// that may change the appearance, and so the one that empties `HIGHLIGHTED`. There is
/// deliberately no second path: a switch that reached the palette without passing through
/// there would leave the source pane's spans in the colours of the theme before it.
///
/// **Not a `use_hook`, and that is the point.** `Platform::preferred_theme` is a `State`
/// freya keeps from the windowing system itself -- winit answers `Window::theme()` on
/// Windows, macOS, X11 and Wayland alike, and freya re-sets the state on a `ThemeChanged`
/// event -- so *reading* it here subscribes this scope to it, and a desktop that goes dark
/// while the app is running re-runs this and repaints. That is a real gain over what this
/// replaced: the old answer came from a subprocess (`kreadconfig`, `gsettings`,
/// `defaults`) asked once at startup, which could not follow the desktop it was asking
/// about and could not be asked at all from a window that had not been opened yet. A
/// `use_hook` here would put that limitation back, one line at a time.
///
/// The *choice* arrives as a value rather than being loaded here, and since 9c it is a
/// value that can change: `Prefs` holds it, the settings page writes it, and the root
/// reads it -- so the same two-hop path that carries a desktop switch carries a click on
/// the Dark button. That is also what lets a test hand this a choice without the machine's
/// own settings file having a vote in what the test asserts.
///
/// Written from the render body rather than from an effect, deliberately: an effect lands
/// a frame late, and a frame late on a dark desktop is a white window flashing at someone
/// who asked for neither. The write is idempotent (`set_if_modified_and_then`), so the
/// render this runs in and every render after it that resolves the same way cost nothing.
pub(crate) fn use_theme(choice: ThemeChoice) {
    let preferred = *Platform::get().preferred_theme.read();

    set_appearance(resolve_appearance(choice, preferred));
}

/// The whole of the wiring between the settings and what they are settings *of*: the
/// appearance, the fonts, and `settings.toml`.
///
/// Three things come out of one state, and they are deliberately not three mechanisms.
/// The theme resolves in the render body, because `use_theme` must (a frame late is a
/// white flash); the fonts and the write go in one effect, because both are consequences
/// of the settled value rather than of the keystroke, and `fonts::resolve` allocates.
///
/// **The baseline is why a run that never opens the page writes no file.** `Settings::save`
/// has no policy in front of it by design -- a settings change is already as rare as a
/// deliberate action -- but "the settings as they were loaded" is not a change, and saving
/// it would create `settings.toml` on every first launch, which is `project.rs`'s rule
/// about a directory made by the first write that has something to say. So what the file
/// says is kept beside the hook and compared, exactly as `Saves::written` is.
///
/// `set_fonts` runs unconditionally, baseline or not: it is idempotent
/// (`set_if_modified`), and the alternative -- trusting that the thread-local was
/// initialised from the same file this hook loaded -- is two readers of one file agreeing
/// by luck.
pub(crate) fn use_settings(prefs: State<EditedSettings>) {
    use_settings_with(prefs, |settings: &Settings| settings.save());
}

/// The same, with the write handed in -- `use_analysis`/`use_analysis_with`'s shape and
/// for the same reason: [`Settings::save`] writes to the machine's real settings file, so
/// a test that mounted this would be editing the settings of whoever ran it.
pub(crate) fn use_settings_with(
    prefs: State<EditedSettings>,
    mut save: impl FnMut(&Settings) + 'static,
) {
    // What the file currently says: the settings as they were loaded, and thereafter
    // whatever was last written. It has to *move*, not sit at the loaded value -- a reader
    // who changes a setting and changes it back would otherwise leave the file holding the
    // middle answer, which is `Saves::written`'s rule and the same bug it exists for. An
    // `Rc<RefCell>` rather than a `State`, since nothing renders from it.
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

/// The column a setting's status sits in, on the right of the value: wide enough for the
/// **Clear** button that appears there when the setting is the reader's own, so that the
/// value boxes above and below one another end at the same x whichever state each is in.
const SETTING_STATUS_WIDTH: f32 = 76.0;

/// How far one press of the size stepper moves a font, and the range it may be moved in.
///
/// Half a point, because that is the granularity the desktops themselves store (KDE writes
/// integers, Gnome's Pango descriptions and the Windows `LOGFONTW` conversion both produce
/// fractions) and because a whole point is a visible jump at nine of them. The bounds are
/// not a claim about taste: below five points the window's own chrome stops being legible
/// enough to change the setting back, and above thirty-two a row is taller than the
/// toolbar. A hand-edited `settings.toml` may still say anything, and is honoured -- these
/// bound the *stepper*, not the file.
const SIZE_STEP: f32 = 0.5;
const MIN_POINTS: f32 = 5.0;

const MAX_POINTS: f32 = 32.0;

/// The column the size is written in, between the two stepper buttons.
const SIZE_VALUE_WIDTH: f32 = 52.0;

/// A point size as the page writes it: `9`, `10.5`, and never `10.50` or `9.0`.
///
/// One decimal, because that is what the stepper's half-points need and what a desktop's
/// answer can carry (Gnome multiplies its size by `text-scaling-factor`, so 11 at 1.25 is
/// 13.75). Rounded for display only -- the value stored is the value stepped.
pub(crate) fn points_text(points: f32) -> String {
    let rounded = (points * 10.0).round() / 10.0;

    match rounded.fract() == 0.0 {
        true => format!("{rounded:.0}"),
        false => format!("{rounded:.1}"),
    }
}

/// One overridable setting: its name, what it says, and -- the whole point of this page --
/// whether what it says is the reader's answer or the one they are inheriting.
///
/// `notes/Goals.md` asks for "a default being unspecified with clear visual distinction",
/// and this is where that is cashed out. Three cues, deliberately more than one, because a
/// single quiet difference is one a reader has to be told about:
///
/// - **The name changes colour.** An overridden setting is written in `name_fg`, the
///   colour a function's name is drawn in; an inherited one in `address_fg`, the colour
///   everything that recedes is drawn in. That is the cue that reads down the column
///   without looking at any one row.
/// - **The value reads as text or as a placeholder.** An override is real text in the box;
///   an unspecified field shows what it is falling through to, in the box's placeholder
///   colour, so the reader is never asked to remember what the desktop said.
/// - **The Clear button is only there when there is something to clear.** It is also the
///   *only* way back to unspecified, which is why it is a button and not a keystroke: an
///   empty family box is unspecified, but a size has no empty state to type.
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
                // The same pair the value beside it uses: what the reader said is
                // ordinary interface text, what they are inheriting recedes into the
                // colour everything secondary in this app is written in.
                .color(match overridden {
                    true => palette().text_fg,
                    false => palette().address_fg,
                })
                .max_lines(1),
        )
        .child(value)
        .child(
            rect()
                .width(Size::px(SETTING_STATUS_WIDTH))
                .horizontal()
                .main_align(Alignment::End)
                .cross_align(Alignment::Center)
                .child(match overridden {
                    true => Button::new()
                        .compact()
                        .on_press(clear)
                        .child("Clear")
                        .into_element(),
                    // Not "unset" and not blank: the reader is being told where the value
                    // in the box beside this came from, which is the question the page
                    // exists to answer.
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
///
/// The preview earns its place on the fixed-width half and is kept on both for symmetry:
/// the interface font is already every label in the window, but the fixed-width one is
/// only visible when a symbol with code in it is open, and a reader changing it with the
/// Assembly pane on a placeholder would otherwise be typing family names at nothing. The
/// digits and the `l1I`/`O0` pairs are in it because they are what a monospaced face is
/// actually chosen for.
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
    // What the stepper moves from: the reader's size where there is one, and otherwise the
    // one being inherited -- so the first press is one step away from what is on screen
    // rather than a jump to some number of this file's own choosing.
    let points = edited.size.unwrap_or(inherited.points);
    let step = |by: f32| {
        let mut size = size.clone();
        move |_: Event<PressEventData>| {
            let moved = (points + by).clamp(MIN_POINTS, MAX_POINTS);
            // Back onto the half-point grid, so that stepping away from a desktop's
            // 13.75 and back again lands on 13.75's neighbours rather than on a drift of
            // its own.
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
                        // A fixed column, so that `+` does not move under the finger as
                        // the number beside it grows a digit or loses a decimal -- the
                        // reason `SourceRow`'s line-number gutter is a fixed width and not
                        // a minimum, and it matters more here, where the thing that would
                        // move is the button being pressed again.
                        .width(Size::px(SIZE_VALUE_WIDTH))
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
/// chosen.
///
/// **A view and not a document**, which is the rule 8e settled and this inherits: the
/// content strip holds `Selection`s -- a place in a binary -- and there is one settings
/// page, resolving against no object, that neither code pane could draw. So it is a `Tab`,
/// the mechanism the app already has for "a pane with its own state the reader can put
/// where they like", and it is excluded from the saved session for free, a dock layout not
/// being persisted.
///
/// **What it writes and when.** Every control writes straight into `Prefs`, and
/// [`use_settings`] at the root is what turns that into a font, a theme and a file --
/// there is no Apply button and no autosave timer, `Settings::save` writing at once by
/// design. So a press here is on disk and on screen before the finger is off the button,
/// which is what makes the page its own preview: there is no "sample text" widget for the
/// interface font because the whole window is one.
#[derive(PartialEq)]
pub(crate) struct SettingsTab;

impl Component for SettingsTab {
    fn render(&self) -> impl IntoElement {
        let mut prefs = use_consume::<Prefs>().0;
        let edited = prefs.read().clone();
        // Both halves of what the page draws, from the same two functions the root
        // resolves with: what the reader would be getting with nothing set, and what they
        // are getting now. Cheap -- the desktop lookups behind them are cached for the
        // life of the process (`fonts::desktop_answer`).
        let inherited = fonts::inherited();
        let resolved = fonts::resolve(&edited.settings());

        // Only a question at all under `Desktop`, which is exactly what `resolve_appearance`
        // says: a reader who named a theme is answered by their own answer, so telling them
        // what the desktop prefers would be telling them about something that is not
        // happening. Reading it here also subscribes this pane, so the line follows a
        // desktop that changes its mind while the page is open.
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
                        // Said here rather than left to be discovered, because it is the
                        // one consequence of a font change that is not a font: a row is
                        // its own font's size plus `ROW_LEADING`, and that is the
                        // `item_size` of the views over it, so a list gets taller with the
                        // font it is drawn in rather than clipping it. Two numbers and not
                        // one blended answer, because each half of the page above moves
                        // exactly one of them -- which is the whole of what a reader wants
                        // to know before stepping a size.
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
