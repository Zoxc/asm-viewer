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

/// The filter over one of the sidebar lists: a text box, and the three toggles that say
/// how to read what is in it. The state it edits arrives as a prop, never as a context.
#[derive(Clone, PartialEq)]
struct FilterBar {
    filter: State<Filter>,
}

impl Component for FilterBar {
    fn render(&self) -> impl IntoElement {
        let filter = self.filter;
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
pub(crate) fn filter_pane(
    filter: State<Filter>,
    background: Color,
    list: impl IntoElement,
) -> Element {
    rect()
        .expanded()
        .content(Content::Flex)
        .background(background)
        .child(FilterBar { filter })
        .child(
            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .child(list),
        )
        .into()
}

/// What a filter leaves of the symbol list: the list itself, and where in it the names
/// that matched are. Indices rather than a second `Vec<Symbol>` (115k entries on
/// `viewer-sample`), and `None` for no filter at all, which costs no pass and no
/// allocation.
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
    /// Filters on the name the row actually shows -- the demangled one where there is one.
    pub(crate) fn new(symbols: SymbolList, matcher: &Matcher) -> Self {
        let matches = match matcher {
            Matcher::Everything => None,
            matcher => Some(Arc::new(
                symbols
                    .0
                    .iter()
                    .enumerate()
                    .filter(|(_, symbol)| matcher.matches(symbol.data.display()))
                    .map(|(index, _)| index)
                    .collect(),
            )),
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
