//! The bar over a sidebar list, its three toggles, and the pane the list is drawn in.
//!
//! One component with three uses, whose `Filter` is a `use_state` in the tab that owns the
//! list rather than a root context: a filter is a view of a list, never part of the
//! session. What a filter leaves of a list is `filter::Filtered`; only the symbol lists
//! earn a memo over it, Objects and History filtering where their rows are built.

use super::*;

/// One of the three toggles beside a filter's text box.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Toggle {
    Case,
    Word,
    Regex,
}

impl Toggle {
    pub(crate) const ALL: [Toggle; 3] = [Toggle::Case, Toggle::Word, Toggle::Regex];

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

    pub(crate) fn is_on(self, filter: &Filter) -> bool {
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
pub(crate) struct FilterToggle {
    pub(crate) filter: State<Filter>,
    pub(crate) toggle: Toggle,
    pub(crate) on: bool,
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
    /// The box's own id, minted by the pane so the rows' handler can ask for it.
    a11y: AccessibilityId,
    /// What the empty box says it is for.
    placeholder: &'static str,
    /// Bumped by Enter, for a box that asks a question rather than filtering as it is
    /// typed. `None` in a filter bar, where there is nothing to submit. A counter and not
    /// a callback: a `Callback` is never equal to another, so a bar holding one would
    /// re-render on every render of whatever holds it.
    submits: Option<State<u64>>,
}

impl Component for FilterBar {
    fn render(&self) -> impl IntoElement {
        let filter = self.filter;
        let a11y = self.a11y;
        let submits = self.submits;
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
                        .placeholder(self.placeholder)
                        .compact()
                        .width(Size::flex(1.0))
                        .a11y_id(a11y)
                        // The window's chords declined before the edit, and Enter
                        // answered where the bar has something to submit (`chords.rs`).
                        .on_pre_key_down(box_keys(Boxed::Input, &[], move |key, _| {
                            if let (Key::Named(NamedKey::Enter), Some(mut submits)) = (key, submits)
                            {
                                // Bound before the write, so the read guard is gone by it.
                                let next = submits.peek().wrapping_add(1);
                                submits.set(next);
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

/// The box a panel's list is drawn in: the rows' own focusable node, the box over them,
/// the scroll the arrows move, and how tall the rows came out.
///
/// The list's counterpart to [`ListBox`] (`ui/list_box.rs`), which is the same four things
/// for a code listing, and minted the same way: before the list is built, the scroll view
/// being handed the controller. `use_hook`, so [`use_list_pane`] is a `use_` function and
/// is called once and unconditionally by each panel.
#[derive(Clone, Copy)]
pub(crate) struct ListPane {
    /// The rows' own node: what a press focuses, what the keys are answered on, and what
    /// [`RowsBox`] hands down so a row knows whether the keyboard is in its list.
    rows: AccessibilityId,
    /// The filter box over them, where the pane has one.
    box_id: AccessibilityId,
    /// The scroll the arrows move, handed to the panel's own scroll view.
    pub(crate) controller: ScrollController,
    /// How tall the rows' box is, which is what says whether the row an arrow moved to is
    /// on screen at all. A `VirtualScrollView` measures itself but keeps the answer, so
    /// the box around it is what is measured -- [`ListBox`]'s own reason.
    viewport: State<f32>,
    picking: Picking,
}

/// The box for the list `panel` draws.
pub(crate) fn use_list_pane(panel: Panel) -> ListPane {
    let rows = use_hook(AccessibilityId::new_unique);
    // Provided rather than passed as a prop: it is one fact about the pane and every row
    // of every list in it wants it (`ui/picks.rs`).
    use_provide_context(|| RowsBox(rows));
    ListPane {
        rows,
        box_id: use_hook(AccessibilityId::new_unique),
        controller: use_scroll_controller(ScrollConfig::default),
        viewport: use_state(|| 0.0f32),
        picking: use_picking(panel),
    }
}

impl ListPane {
    /// A list under its own filter bar. The bar takes its height off the top of the pane
    /// rather than out of the list, so a `VirtualScrollView` inside still starts at a row
    /// boundary however tall the bar turns out to be -- it grows a line for a bad pattern.
    pub(crate) fn filtered(
        &self,
        filter: State<Filter>,
        keys: ListKeys,
        list: impl IntoElement,
    ) -> Element {
        self.boxed(Some((filter, "Filter", None)), keys, list)
    }

    /// The Search panel's list under its own box: [`ListPane::filtered`] where Enter asks a
    /// question rather than the typing filtering as it goes, and where the box's id comes
    /// back, since the chord that reaches it is answered at the root and not on the rows.
    pub(crate) fn searched(
        &self,
        filter: State<Filter>,
        submits: State<u64>,
        keys: ListKeys,
        list: impl IntoElement,
    ) -> (Element, AccessibilityId) {
        (
            self.boxed(Some((filter, "Search", Some(submits))), keys, list),
            self.box_id,
        )
    }

    /// A list with no bar over it: the Files tree, which has nothing to filter by. Ctrl+F
    /// leads to a box, so it does nothing here.
    pub(crate) fn plain(&self, keys: ListKeys, list: impl IntoElement) -> Element {
        self.boxed(None, keys, list)
    }

    /// All three: the pane on the one ground every panel is drawn on, the bar where there
    /// is one, and the rows under it.
    fn boxed(
        &self,
        bar: Option<(State<Filter>, &'static str, Option<State<u64>>)>,
        keys: ListKeys,
        list: impl IntoElement,
    ) -> Element {
        rect()
            .expanded()
            .content(Content::Flex)
            .background(palette().pane_bg)
            .maybe(bar.is_some(), |pane| {
                let (filter, placeholder, submits) = bar.expect("the bar is there");
                pane.child(FilterBar {
                    filter,
                    a11y: self.box_id,
                    placeholder,
                    submits,
                })
            })
            .child(self.rows(keys, list))
            .into()
    }

    /// The rows: the focusable node the list is drawn in, and every key it answers.
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
    fn rows(&self, keys: ListKeys, list: impl IntoElement) -> Rect {
        let (rows, box_id, picking) = (self.rows, self.box_id, self.picking);
        let (controller, viewport) = (self.controller, self.viewport);
        let mut measured = viewport;
        rect()
            .width(Size::fill())
            .height(Size::flex(1.0))
            .a11y_id(rows)
            .a11y_focusable(true)
            .on_pointer_down(move |_| {
                rows.request_focus();
                picking.unasked();
            })
            .on_sized(move |e: Event<SizedEventData>| {
                measured.set_if_modified(e.area.height());
            })
            .on_key_down(move |e: Event<KeyboardEventData>| {
                answer(picking, &keys, controller, viewport, box_id, &e);
            })
            .child(list)
    }
}

/// What a focused list does with a key: the chord to its box, the arrows over its rows,
/// and Enter on the row they left the pick on.
///
/// The scroll follows the arrows for the finder's reason: the panel is a screenful of rows
/// and the arrows walk past it, so a pick nobody can see is a row Enter opens unnamed.
fn answer(
    picking: Picking,
    keys: &ListKeys,
    mut controller: ScrollController,
    viewport: State<f32>,
    box_id: AccessibilityId,
    e: &Event<KeyboardEventData>,
) {
    if Chord::Find.is(&e.key, e.modifiers) {
        box_id.request_focus();
        return;
    }
    let by = match &e.key {
        Key::Named(NamedKey::ArrowDown) => 1,
        Key::Named(NamedKey::ArrowUp) => -1,
        Key::Named(NamedKey::Enter) => return picking.entered(keys),
        _ => return,
    };
    if let Some(at) = picking.stepped(keys, by) {
        reveal_caret(
            &mut controller,
            *viewport.peek(),
            list_row_height(),
            keys.length,
            at,
        );
    }
}

/// The compiled filter, as the rows of a list are handed it: made once per render of the
/// panel and shared by every row it builds, since compiling a regex per row is not free.
///
/// Compared by the pointer, as everything else in the UI with an `Rc` or an `Arc` behind
/// it is. A fresh one every render is not equal to the last, so the scroll view builds its
/// rows again -- which costs the rows themselves nothing, their own props being what says
/// whether one has to be drawn again.
#[derive(Clone)]
pub(crate) struct Marking(Rc<Matcher>);

impl PartialEq for Marking {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Marking {
    pub(crate) fn new(matcher: Matcher) -> Self {
        Self(Rc::new(matcher))
    }

    /// Where the filter matched in `text`, for the row to mark.
    pub(crate) fn marks(&self, text: &str) -> Vec<Range<usize>> {
        self.0.marks(text)
    }

    /// The columns it matched in a line of code, for a code row to wash. The same
    /// question as [`marks`](Self::marks) asked of a line rather than a name, so the one
    /// compiled matcher answers both (`src/find.rs`).
    pub(crate) fn hits(&self, line: &Line) -> Vec<Range<usize>> {
        crate::find::hits_in(line, &self.0)
    }
}
