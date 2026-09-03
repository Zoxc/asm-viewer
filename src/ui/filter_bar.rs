//! The bar over a sidebar list, and what a filter leaves of the symbols under it.
//!
//! One component with three uses, whose `Filter` is a `use_state` in the tab that owns the
//! list rather than a root context: a filter is a view of a list, never part of the
//! session. Only the symbol list earns a memo; Objects and History filter where their rows
//! are built.

use super::*;

/// One of the three toggles beside a filter's text box.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Toggle {
    Case,
    Word,
    Regex,
}

impl Toggle {
    const ALL: [Toggle; 3] = [Toggle::Case, Toggle::Word, Toggle::Regex];

    /// What the button is drawn as: text rather than an icon, since `\b` and `.*` *are*
    /// the regex the toggle turns on. The words are in the tooltip.
    fn glyph(self) -> &'static str {
        match self {
            Toggle::Case => "Aa",
            Toggle::Word => "\\b",
            Toggle::Regex => ".*",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Toggle::Case => "Match case",
            Toggle::Word => "Whole word",
            Toggle::Regex => "Regular expression",
        }
    }

    fn is_on(self, filter: &Filter) -> bool {
        match self {
            Toggle::Case => filter.case_sensitive,
            Toggle::Word => filter.whole_word,
            Toggle::Regex => filter.regex,
        }
    }

    fn flip(self, filter: &mut Filter) {
        match self {
            Toggle::Case => filter.case_sensitive = !filter.case_sensitive,
            Toggle::Word => filter.whole_word = !filter.whole_word,
            Toggle::Regex => filter.regex = !filter.regex,
        }
    }
}

/// One toggle button. Whether it is on is a prop rather than something read here, so that
/// typing a character re-renders the bar and none of the toggles.
#[derive(Clone, PartialEq)]
struct FilterToggle {
    filter: State<Filter>,
    toggle: Toggle,
    on: bool,
}

impl Component for FilterToggle {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut filter = self.filter;
        let toggle = self.toggle;

        let background = if self.on {
            palette().toggle_on_bg
        } else if hovering() {
            palette().toggle_hover_bg
        } else {
            Color::TRANSPARENT
        };

        TooltipContainer::new(Tooltip::new(toggle.tooltip())).child(
            rect()
                .width(Size::px(toggle_size()))
                .height(Size::px(toggle_size()))
                .center()
                .corner_radius(4.0)
                .background(background)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |e: Event<PressEventData>| {
                    // **Load-bearing**: the `Input` beside this one gives its keyboard
                    // focus up from `on_global_pointer_press`, so without this a toggle
                    // pressed mid-word sends the rest of the name nowhere. The global
                    // press a press derives is cancellable and sorts last.
                    e.prevent_default();
                    toggle.flip(&mut filter.write());
                })
                .child(label().text(toggle.glyph()).max_lines(1)),
        )
    }
}

/// The one chord that reaches a filter box: Ctrl+F -- Ctrl or Meta, and neither Shift
/// nor Alt, so Ctrl+Shift+F stays free for the source search. `F` as well as `f`, which
/// is what Caps Lock makes of it.
pub(crate) fn is_find_chord(key: &Key, modifiers: Modifiers) -> bool {
    modifiers.contains(Modifiers::ctrl_or_meta())
        && !modifiers.intersects(Modifiers::SHIFT | Modifiers::ALT)
        && matches!(key, Key::Character(character) if character.eq_ignore_ascii_case("f"))
}

/// The filter over one of the sidebar lists: a text box, and the three toggles that say
/// how to read what is in it. The state it edits arrives as a prop, never as a context.
#[derive(Clone, PartialEq)]
struct FilterBar {
    filter: State<Filter>,
    /// The box's own id, minted by the pane so the rows' handler can ask for it.
    a11y: AccessibilityId,
}

impl Component for FilterBar {
    fn render(&self) -> impl IntoElement {
        let filter = self.filter;
        let a11y = self.a11y;
        // Reading subscribes the bar to the filter.
        let current = filter.read().clone();
        // Compiled here as well as wherever the list is filtered: a `Regex` is not
        // `PartialEq`, so the two cannot share one through a `State`.
        let error = current.matcher().error().map(str::to_owned);

        rect()
            .width(Size::fill())
            .background(palette().header_bg)
            .border(bottom_hairline())
            .child(
                rect()
                    .width(Size::fill())
                    // Taller than a row by the room an `Input`'s border and inner margin
                    // need. The **list** height, a filter bar sitting only ever over a
                    // sidebar list.
                    .height(Size::px(list_row_height() + 6.0))
                    .horizontal()
                    // A `flex` child needs a `Content::Flex` parent for torin to size it.
                    .content(Content::Flex)
                    .cross_align(Alignment::Center)
                    .padding(Gaps::new_symmetric(0.0, 5.0))
                    .spacing(2.0)
                    .child(
                        Input::new(
                            // `Writable::map` lets the `Input` write into the one field
                            // while still notifying everything watching the whole filter.
                            filter
                                .into_writable()
                                .map(|filter| &filter.pattern, |filter| &mut filter.pattern),
                        )
                        .placeholder("Filter")
                        .compact()
                        .width(Size::flex(1.0))
                        .a11y_id(a11y)
                        // An `Input` inserts a character it has no chord of its own for,
                        // so Ctrl+F in the box would type an `f` into the pattern.
                        // Declined here, before the edit. The rest is freya's default,
                        // which the hook replaces wholesale (`notes/upstream/freya.md`).
                        .on_pre_key_down(Callback::new(|e: Event<KeyboardEventData>| {
                            if is_find_chord(&e.key, e.modifiers) {
                                return false;
                            }
                            match &e.key {
                                Key::Named(NamedKey::Enter)
                                | Key::Named(NamedKey::Escape)
                                | Key::Named(NamedKey::Shift) => true,
                                Key::Named(NamedKey::Tab) => false,
                                _ => {
                                    e.stop_propagation();
                                    e.prevent_default();
                                    true
                                }
                            }
                        }))
                        .maybe(error.is_some(), |input| {
                            input
                                .color(palette().invalid_fg)
                                .focus_border_fill(palette().invalid_fg)
                        }),
                    )
                    .children(Toggle::ALL.map(|toggle| {
                        FilterToggle {
                            filter,
                            toggle,
                            on: toggle.is_on(&current),
                        }
                        .into()
                    })),
            )
            // A pattern that will not compile has to read *as* one: matching nothing
            // looks exactly like a list with nothing in it, so the reason is written out.
            .maybe_child(error.map(|error| {
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new(0.0, 6.0, 5.0, 6.0))
                    .overflow(Overflow::Clip)
                    .child(label().text(error).color(palette().invalid_fg).max_lines(1))
            }))
    }
}

/// A list under its own filter bar. The bar takes its height off the top of the pane
/// rather than out of the list, so a `VirtualScrollView` inside still starts at a row
/// boundary however tall the bar turns out to be -- it grows a line for a bad pattern.
///
/// **Ctrl+F puts the keyboard in the box over the list it is pressed in**, and nowhere
/// else: the binding is on the rows and not on the root, so a code pane keeps its keys
/// and its own Ctrl+F for the source search. A press on the rows is what focuses them,
/// or a list could not be reached with the keyboard at all and the chord would have
/// nothing to fire from. In the box the chord does nothing, the box being where it
/// leads; the bar declines it there so it is not typed in as an `f`.
///
/// The handler goes on the rows themselves rather than over both halves of the pane
/// because a key event is emitted only for a **focused node that listens for it** --
/// bubbling to an ancestor's handler comes after that, and never happens when the
/// focused node has no handler of its own (`notes/upstream/freya.md`).
///
/// The two ids are minted here and not in the bar, the pane being what holds them both.
/// `use_hook`, so this is a `use_` function: it is called once and unconditionally by
/// each of the five tabs, at the end of a render.
pub(crate) fn use_filter_pane(
    filter: State<Filter>,
    background: Color,
    list: impl IntoElement,
) -> Element {
    let rows = use_hook(AccessibilityId::new_unique);
    let box_id = use_hook(AccessibilityId::new_unique);
    rect()
        .expanded()
        .content(Content::Flex)
        .background(background)
        .child(FilterBar {
            filter,
            a11y: box_id,
        })
        .child(
            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .a11y_id(rows)
                .a11y_focusable(true)
                .on_pointer_down(move |_| rows.request_focus())
                .on_key_down(move |e: Event<KeyboardEventData>| {
                    if is_find_chord(&e.key, e.modifiers) {
                        box_id.request_focus();
                    }
                })
                .child(list),
        )
        .into()
}

/// What a filter leaves of the symbol list: the list itself, and where in it the names
/// that matched are, best match first. Indices rather than a second `Vec<Symbol>` (115k
/// entries on `viewer-sample`), and `None` for no filter at all, which costs no pass, no
/// sort and no allocation, and keeps the list in its own order.
#[derive(Clone)]
pub(crate) struct Filtered {
    pub(crate) symbols: SymbolList,
    matches: Option<Arc<Vec<usize>>>,
}

impl PartialEq for Filtered {
    fn eq(&self, other: &Self) -> bool {
        self.symbols == other.symbols
            && match (&self.matches, &other.matches) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

impl Filtered {
    /// Filters on the name the row actually shows -- the demangled one where there is one
    /// -- and orders what is left by its [`Rank`], the list's own order breaking ties, so
    /// the sort is deterministic and `sort_unstable` is safe.
    pub(crate) fn new(symbols: SymbolList, matcher: &Matcher) -> Self {
        let matches = match matcher {
            Matcher::Everything => None,
            matcher => {
                let mut ranked: Vec<(Rank, usize)> = symbols
                    .0
                    .iter()
                    .enumerate()
                    .filter_map(|(index, symbol)| {
                        let rank = matcher.rank(symbol.data.display())?;
                        Some((rank, index))
                    })
                    .collect();
                ranked.sort_unstable();
                Some(Arc::new(
                    ranked.into_iter().map(|(_, index)| index).collect(),
                ))
            }
        };

        Filtered { symbols, matches }
    }

    /// How many rows there are, which is what the `VirtualScrollView` is given.
    pub(crate) fn len(&self) -> usize {
        self.matches
            .as_ref()
            .map_or(self.symbols.0.len(), |matches| matches.len())
    }

    /// Which symbol the row at `row` is.
    pub(crate) fn index(&self, row: usize) -> usize {
        self.matches.as_ref().map_or(row, |matches| matches[row])
    }
}
