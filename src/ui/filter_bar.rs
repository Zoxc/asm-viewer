//! The bar over a sidebar list, and what a filter leaves of the symbols under it.
//!
//! One component with three uses, whose `Filter` is a `use_state` in the tab that owns the
//! list rather than a root context: a filter is a view of a list and never part of the
//! session, so nothing about it reaches `project.rs`.
//!
//! The three toggles *are* three regex constructs, which is why `filter.rs` compiles every
//! filter to one `Regex`. Only the symbol list earns a memo -- 115k names on
//! `viewer-sample` -- where Objects and History filter where their rows are built.

use super::*;

/// One of the three toggles beside a filter's text box.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Toggle {
    Case,
    Word,
    Regex,
}

impl Toggle {
    /// The three of them in the order the bar draws them.
    const ALL: [Toggle; 3] = [Toggle::Case, Toggle::Word, Toggle::Regex];

    /// What the button is drawn as.
    ///
    /// Still text, and looked at twice. The first answer leaned on the dependency, which
    /// the tab bar's icons have since brought in, and on Lucide having nothing for a regex
    /// flag, which is simply wrong: the set carries `case-sensitive`, `whole-word` and
    /// `regex`, which are VS Code's three toggles glyph for glyph. Rendered at
    /// [`toggle_size`] beside these, they lose anyway. `case-sensitive` is an `Aa` drawn as
    /// strokes, so it says exactly what the two letters say and no more; `regex` at 17px
    /// is a splayed asterisk over a rounded box, muddier than the two characters it stands
    /// for; and `\b` and `.*` *are* the regex the toggle turns on, written out, which in a
    /// window whose filter bar compiles to a `regex::Regex` and whose reader is reading
    /// disassembly is the more precise label rather than the more cryptic one. `whole-word`
    /// is the one that is arguably better than its text, and one of three is not a set.
    /// The words are in the tooltip either way.
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

/// One toggle button.
///
/// Whether it is on is a prop rather than something read here, so that typing a character
/// — which changes the one `Filter` all three of them share — re-renders the bar and none
/// of them.
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
                    // The text box beside this one gives its keyboard focus up from
                    // `on_global_pointer_press`, which is how an `Input` notices a click
                    // that landed outside it. A toggle is not outside it in the way that
                    // matters: turning "whole word" on halfway through typing a name must
                    // not send the rest of the name nowhere. A press's cancellable events
                    // include the global press it derives, and non-capture globals are
                    // sorted to run last (freya-core `events/name.rs`), so preventing the
                    // default here reaches the input before it acts on it.
                    e.prevent_default();
                    toggle.flip(&mut filter.write());
                })
                .child(label().text(toggle.glyph()).max_lines(1)),
        )
    }
}

/// The filter over one of the sidebar lists: a text box, and the three toggles that say
/// how to read what is in it.
///
/// One component and three uses. The state it edits belongs to the tab that owns the list
/// rather than to the root — a filter is a view of a list and not part of the session — so
/// it arrives as a prop and never as a context, and nothing about it reaches `project.rs`.
#[derive(Clone, PartialEq)]
struct FilterBar {
    filter: State<Filter>,
}

impl Component for FilterBar {
    fn render(&self) -> impl IntoElement {
        let filter = self.filter;
        // Reading subscribes the bar to the filter, which is what puts a typed character
        // back on screen and lights a toggle that was just pressed.
        let current = filter.read().clone();
        // Compiled here as well as wherever the list is actually filtered. A `Regex` is
        // not something the two can share through a `State`: it is not `PartialEq`, and a
        // compiled program is not a value to compare anyway. Compiling one costs
        // microseconds against the milliseconds a pass over a list of names does.
        let error = current.matcher().error().map(str::to_owned);

        rect()
            .width(Size::fill())
            .background(palette().header_bg)
            .border(bottom_hairline())
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(filter_height()))
                    .horizontal()
                    // The toggles take their own widths and the box takes the rest, which
                    // torin only works out for a `flex` child of a `Content::Flex` parent.
                    .content(Content::Flex)
                    .cross_align(Alignment::Center)
                    .padding(Gaps::new_symmetric(0.0, 5.0))
                    .spacing(2.0)
                    .child(
                        Input::new(
                            // The pattern is a field of the `Filter` rather than a state
                            // of its own, so that what was typed and how it is to be read
                            // are one value to compare and one thing to hand a memo.
                            // `Writable::map` is what lets the `Input` write into that
                            // field while still notifying everything watching the whole
                            // filter.
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
            // A pattern that will not compile has to read *as* one. Matching everything
            // would hide the half-typed `(` and matching nothing looks exactly like a
            // list with nothing in it, so the reason is written under the box it is in —
            // and the list below stays empty, which is now the truth rather than a
            // coincidence.
            .maybe_child(error.map(|error| {
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new(0.0, 6.0, 5.0, 6.0))
                    .overflow(Overflow::Clip)
                    .child(label().text(error).color(palette().invalid_fg).max_lines(1))
            }))
    }
}

/// A list under its own filter bar.
///
/// The bar goes above the list, which is where "filter bar under objects / symbols /
/// history" puts it: under the tab that names the list, the same place the assembly
/// goal's "bar under the Assembly tab" means. It takes its height off the top of the pane
/// rather than out of the list — the list is the `flex` child of a `Content::Flex` parent,
/// exactly as the source rows are under their path header — so a `VirtualScrollView`
/// inside it still starts at a row boundary whatever height the bar turns out to want,
/// which is not fixed: it grows by a line when the pattern will not compile.
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
/// that matched it are.
///
/// Indices rather than a second `Vec<Symbol>`, because the list is 115k entries on
/// `viewer-sample` and a row wants to be told which entry it is rather than handed a copy
/// of it. `None` rather than every index in order, because no filter at all is the state
/// the list is in most of the time and that case then costs exactly what it cost before
/// there was a filter: no pass over the names and no allocation to say "all of them".
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
    /// Filter on the name the row actually shows — the demangled one where there is one —
    /// because a filter the user cannot see the effect of on screen is not one.
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
