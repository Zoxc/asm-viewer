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

    /// The chord that flips it from the box beside it. The toggles are one thing in all
    /// four boxes -- the three filter bars and the find bar -- so the three keys are too,
    /// and each is answered on the bar the box is in and nowhere else.
    fn chord(self) -> Chord {
        match self {
            Toggle::Case => Chord::MatchCase,
            Toggle::Word => Chord::WholeWord,
            Toggle::Regex => Chord::Regex,
        }
    }

    /// The toggle a key is the chord of, where it is one of the three. The key is read as
    /// a chord once and the three are compared against that, so [`Toggle::chord`] stays
    /// the one table.
    pub(crate) fn pressed(key: &Key, modifiers: Modifiers) -> Option<Toggle> {
        let chord = Chord::of(key, modifiers)?;
        Toggle::ALL
            .into_iter()
            .find(|toggle| toggle.chord() == chord)
    }

    pub(crate) fn flip(self, filter: &mut Filter) {
        match self {
            Toggle::Case => filter.case_sensitive = !filter.case_sensitive,
            Toggle::Word => filter.whole_word = !filter.whole_word,
            Toggle::Regex => filter.regex = !filter.regex,
        }
    }
}

/// One toggle button. Whether it is on is a prop rather than something read here, so that
/// typing a character re-renders the bar and none of the toggles.
#[derive(Clone)]
pub(crate) struct FilterToggle {
    pub(crate) filter: State<Filter>,
    pub(crate) toggle: Toggle,
    pub(crate) on: bool,
}

/// Written out because the filter cannot be compared: a `State` compares by the box it is
/// and never by what is in it, so a derive would put a line there that reads as a
/// comparison of the filter and is not one. Nothing is lost by leaving it out -- the
/// toggle only writes the filter, and `on` is what says it has to be drawn again.
impl PartialEq for FilterToggle {
    fn eq(&self, other: &Self) -> bool {
        self.toggle == other.toggle && self.on == other.on
    }
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
#[derive(Clone)]
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
    /// What is wrong with the pattern, out of the one [`Marking`] the panel narrowed its
    /// list with. A prop and not compiled here: the reason the bar prints has to be the
    /// reason the list refused.
    error: Option<String>,
}

/// Written out because neither state can be compared: a `State` compares by the box it is
/// and never by what is in it, so a derive would put two lines there that read as
/// comparisons and are not. What makes the bar draw again is reading the filter in
/// `render`, which is what subscribes it. Whether there is a `submits` at all is a real
/// difference and is still compared, and so is the error, which is what the box is
/// coloured by.
impl PartialEq for FilterBar {
    fn eq(&self, other: &Self) -> bool {
        self.a11y == other.a11y
            && self.placeholder == other.placeholder
            && self.submits.is_some() == other.submits.is_some()
            && self.error == other.error
    }
}

impl Component for FilterBar {
    fn render(&self) -> impl IntoElement {
        let filter = self.filter;
        let a11y = self.a11y;
        let submits = self.submits;
        // Reading subscribes the bar to the filter, which is what draws the toggles
        // again. The pattern is not compiled here: `error` is the panel's own
        // [`Marking`], so the bar and the list cannot disagree about a bad pattern.
        let current = filter.read().clone();
        let error = self.error.clone();

        rect()
            .width(Size::fill())
            .background(palette().header_bg)
            .border(bottom_hairline())
            .child(
                rect()
                    .width(Size::fill())
                    // A row and the room an `Input` needs around it: the **list** height,
                    // a filter bar sitting only ever over a sidebar list.
                    .height(Size::px(text_box_height()))
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
                        //
                        // **The three keys the list under the box answers are declined
                        // too**, so they reach the bar's own handler ([`handed`]) rather
                        // than moving the caret or unfocusing the box: the arrows are the
                        // pick's, and Escape puts the keyboard back on the list with what
                        // was typed still here. Enter is not among them -- it is the
                        // box's where the bar submits -- and arrives at that handler
                        // anyway, an `Input` neither stopping nor cancelling it.
                        .on_pre_key_down(box_keys(
                            Boxed::Input,
                            &[NamedKey::ArrowUp, NamedKey::ArrowDown, NamedKey::Escape],
                            move |key, modifiers| {
                                let plain = chords::held(modifiers).is_empty();
                                if let (Key::Named(NamedKey::Enter), true, Some(mut submits)) =
                                    (key, plain, submits)
                                {
                                    // Bound before the write, so the read guard is gone
                                    // by it.
                                    let next = submits.peek().wrapping_add(1);
                                    submits.set(next);
                                }
                            },
                        ))
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
            .maybe_child(error.map(invalid_line))
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
    /// The scroll the arrows move. Private: every list of this pane's is drawn through
    /// [`ListPane::virtual_rows`] or [`ListPane::short_list`], which are what hand it over.
    controller: ScrollController,
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
    let box_id = use_hook(AccessibilityId::new_unique);
    // Which of the two a chord that reaches this panel puts the keyboard in
    // ([`Panel::filters`]), registered here and not by whichever of `filtered`, `plain`
    // and `searched` the panel calls: this is the one function every panel calls once and
    // unconditionally, and a hook behind a panel's early return is a hook that shifts.
    use_panel_keyboard(panel, if panel.filters() { box_id } else { rows });
    ListPane {
        rows,
        box_id,
        controller: use_scroll_controller(ScrollConfig::default),
        viewport: use_state(|| 0.0f32),
        picking: use_picking(panel),
    }
}

/// The bar over a list, where it has one: the state it edits, what the empty box says it
/// is for, whether Enter submits, and what is wrong with the pattern.
struct Bar {
    filter: State<Filter>,
    placeholder: &'static str,
    submits: Option<State<u64>>,
    error: Option<String>,
}

impl Bar {
    /// The bar, with its error taken off the [`Marking`] the panel's list was narrowed
    /// with rather than compiled again here.
    fn new(
        filter: State<Filter>,
        placeholder: &'static str,
        submits: Option<State<u64>>,
        marking: &Marking,
    ) -> Self {
        Bar {
            filter,
            placeholder,
            submits,
            error: marking.matcher().error().map(str::to_owned),
        }
    }
}

impl ListPane {
    /// A list under its own filter bar. The bar takes its height off the top of the pane
    /// rather than out of the list, so a `VirtualScrollView` inside still starts at a row
    /// boundary however tall the bar turns out to be -- it grows a line for a bad pattern.
    ///
    /// `marking` is the compiled filter the panel narrowed its list with, handed over so
    /// that the reason the bar prints is the reason the list refused.
    pub(crate) fn filtered(
        &self,
        filter: State<Filter>,
        marking: &Marking,
        keys: ListKeys,
        list: impl IntoElement,
    ) -> Element {
        self.boxed(Some(Bar::new(filter, "Filter", None, marking)), keys, list)
    }

    /// The Search panel's list under its own box: [`ListPane::filtered`] where Enter asks a
    /// question rather than the typing filtering as it goes.
    pub(crate) fn searched(
        &self,
        filter: State<Filter>,
        submits: State<u64>,
        marking: &Marking,
        keys: ListKeys,
        list: impl IntoElement,
    ) -> Element {
        self.boxed(
            Some(Bar::new(filter, "Search", Some(submits), marking)),
            keys,
            list,
        )
    }

    /// A list with no bar over it: the Files tree, which has nothing to filter by. Ctrl+F
    /// leads to a box, so it does nothing here.
    pub(crate) fn plain(&self, keys: ListKeys, list: impl IntoElement) -> Element {
        self.boxed(None, keys, list)
    }

    /// A handful of rows under a filter, or the reason there are none: `empty` when the
    /// list has nothing in it at all, "No matches" when the filter left nothing of it.
    /// Which of the two it is has to be asked of the whole list: the rows are only what
    /// the filter left of it.
    ///
    /// A plain `ScrollView` and not a `VirtualScrollView`: the lists drawn this way are a
    /// few one-label rows, built straight from the state rather than routed through
    /// `new_with_data`. The scroll is the pane's own all the same, so the arrows reach a
    /// row below the fold.
    pub(crate) fn short_list(&self, rows: Vec<Element>, any: bool, empty: &str) -> Element {
        match (any, rows.is_empty()) {
            (false, _) => placeholder(empty),
            (true, true) => placeholder("No matches"),
            (true, false) => ScrollView::new_controlled(self.controller)
                .child(rect().width(Size::fill()).children(rows).into_element())
                .into_element(),
        }
    }

    /// The rows of a long list, drawn through a `VirtualScrollView`.
    ///
    /// **The one place a sidebar list's `item_size` and its scroll are written.** A row's
    /// height must equal the `item_size` over it or the scrolling misaligns, and there are
    /// two heights (`AGENTS.md`): a panel spelling `code_row_height` here would draw its
    /// rows one height and scroll them by another. The controller is the pane's for the
    /// arrows' sake -- a pick they moved off screen is a row Enter opens unnamed.
    ///
    /// `data` and never a capture: the builder closure is not compared across renders, so
    /// what the rows depend on has to reach them through `new_with_data`.
    pub(crate) fn virtual_rows<D: PartialEq + 'static>(
        &self,
        length: usize,
        data: D,
        row: impl Fn(usize, &D) -> Element + 'static,
    ) -> Element {
        VirtualScrollView::new_with_data(data, row)
            .length(length)
            .item_size(list_row_height())
            .scroll_controller(self.controller)
            .into_element()
    }

    /// All three: the pane on the one ground every panel is drawn on, the bar where there
    /// is one, and the rows under it.
    fn boxed(&self, bar: Option<Bar>, keys: ListKeys, list: impl IntoElement) -> Element {
        // One list's keys, answered in two places: on the rows, and over the box for the
        // keys it hands on. An `Rc` and not two `ListKeys`, the closures being the
        // panel's own rows.
        let keys = Rc::new(keys);
        rect()
            .expanded()
            .content(Content::Flex)
            .background(palette().pane_bg)
            .maybe_child(bar.map(|bar| self.barred(bar, keys.clone())))
            .child(self.rows(keys, list))
            .into()
    }

    /// The bar, in a rect of its own: the keys the box hands on arrive by bubbling out of
    /// it, and only an **ancestor** of the box is reached that way -- the rows are its
    /// sibling (`notes/upstream/freya.md`). It is over the bar alone, so a key answered on
    /// the rows is not answered again here.
    fn barred(&self, bar: Bar, keys: Rc<ListKeys>) -> Element {
        let pane = *self;
        let Bar {
            filter,
            placeholder,
            submits,
            error,
        } = bar;
        rect()
            .width(Size::fill())
            .on_key_down(move |e: Event<KeyboardEventData>| {
                pane.handed(&keys, filter, submits.is_none(), &e);
            })
            .child(FilterBar {
                filter,
                a11y: self.box_id,
                placeholder,
                submits,
                error,
            })
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
    fn rows(&self, keys: Rc<ListKeys>, list: impl IntoElement) -> Rect {
        let pane = *self;
        let (rows, picking) = (self.rows, self.picking);
        let mut measured = self.viewport;
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
                pane.answer(&keys, &e);
            })
            .child(list)
    }

    /// What a focused list does with a key: the chord to its box, the pick moved over
    /// its rows, a tree row folded, Enter on the row the pick was left on, and Escape
    /// back to the tab on screen.
    ///
    /// The scroll follows for the finder's reason: the panel is a screenful of rows and
    /// the keys walk past it, so a pick nobody can see is a row Enter opens unnamed.
    ///
    /// **Each key answers under its own modifiers and no others**, which is the code
    /// panes' rule (`on_listing_key`, `ui/marks.rs`) and is what keeps the window's keys
    /// the window's: Alt+Left is a step back along the tab's trail and must not fold a
    /// row. Enter is the one exception, and the whole of what Ctrl+Enter is -- **a tab
    /// that stays** comes from the Ctrl every row already reads as it opens
    /// (`Reach::outside`), so the key opens the pick exactly as a Ctrl+click on it would.
    fn answer(&self, keys: &ListKeys, e: &Event<KeyboardEventData>) {
        let picking = self.picking;
        if Chord::Find.is(&e.key, e.modifiers) {
            self.box_id.request_focus();
            return;
        }
        let modifiers = chords::held(e.modifiers);
        let plain = modifiers.is_empty();
        let command = modifiers == Modifiers::ctrl_or_meta();
        // A screen is the rows the box shows whole, as a code pane works its page out.
        let page = (*self.viewport.peek() / list_row_height()).floor().max(0.0) as isize;
        let moved = match &e.key {
            // The way out, and the far end of the way in: a chord puts the keyboard in
            // a panel, Escape in its box puts it on the rows, and Escape here hands it
            // back to the tab. The pick stays where it was and goes grey, saying the
            // list is still where the reader left it.
            Key::Named(NamedKey::Escape) if plain => return picking.to_the_tab(),
            Key::Named(NamedKey::Enter) if plain || command => return picking.entered(keys),
            Key::Named(NamedKey::ArrowLeft) if plain => return picking.folded(keys, false),
            Key::Named(NamedKey::ArrowRight) if plain => return picking.folded(keys, true),
            Key::Named(NamedKey::ArrowDown) if plain => picking.stepped(keys, 1),
            Key::Named(NamedKey::ArrowUp) if plain => picking.stepped(keys, -1),
            Key::Named(NamedKey::PageDown) if plain => picking.stepped(keys, page),
            Key::Named(NamedKey::PageUp) if plain => picking.stepped(keys, -page),
            Key::Named(NamedKey::Home) if plain => picking.jumped(keys, 0),
            Key::Named(NamedKey::End) if plain => picking.jumped(keys, usize::MAX),
            _ => return,
        };
        self.followed(keys.length, moved);
    }

    /// What a filter box hands on to the list under it: the arrows and Enter, so a
    /// reader can type, pick and open without a hand leaving the box, Escape to put the
    /// keyboard back on the rows with what was typed still in the box, and the three
    /// chords the toggles beside the box are pressed by.
    ///
    /// Answered here and not on the rows because **a key event reaches the focused
    /// node's own listeners and then its ancestors**, and the rows are the box's sibling
    /// (`notes/upstream/freya.md`). The box declines each of these so that it neither
    /// types them nor cancels them (`box_keys`, `ui/chords.rs`).
    ///
    /// `opens` is whether Enter is the list's here: in a bar that submits -- the Search
    /// panel's -- Enter asks the question and only Ctrl+Enter opens the pick.
    fn handed(
        &self,
        keys: &ListKeys,
        mut filter: State<Filter>,
        opens: bool,
        e: &Event<KeyboardEventData>,
    ) {
        let picking = self.picking;
        if let Some(toggle) = Toggle::pressed(&e.key, e.modifiers) {
            return toggle.flip(&mut filter.write());
        }
        let modifiers = chords::held(e.modifiers);
        let plain = modifiers.is_empty();
        let command = modifiers == Modifiers::ctrl_or_meta();
        let moved = match &e.key {
            Key::Named(NamedKey::Escape) => return self.rows.request_focus(),
            Key::Named(NamedKey::Enter) if command || (plain && opens) => {
                return picking.entered(keys)
            }
            Key::Named(NamedKey::ArrowDown) if plain => picking.stepped(keys, 1),
            Key::Named(NamedKey::ArrowUp) if plain => picking.stepped(keys, -1),
            _ => return,
        };
        self.followed(keys.length, moved);
    }

    /// The list scrolled to the row a key moved the pick to, where one did.
    fn followed(&self, length: usize, at: Option<usize>) {
        let Some(at) = at else {
            return;
        };
        let mut controller = self.controller;
        reveal_caret(
            &mut controller,
            *self.viewport.peek(),
            list_row_height(),
            length,
            at,
        );
    }
}

/// The compiled filter, as the rows of a list are handed it: made once per pattern and
/// shared by every row the list builds, since compiling a regex per row is not free.
///
/// Compared by the pointer, as everything else in the UI with an `Rc` or an `Arc` behind
/// it is. A new one is not equal to the last, so the scroll view builds its rows again --
/// which costs the rows themselves nothing, their own props being what says whether one
/// has to be drawn again.
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

    /// The compiled filter itself: what a panel narrows its list with, and what the bar
    /// over it takes its error from. The one compile per pattern, so the list and the bar
    /// cannot say different things about a half-typed `(`.
    pub(crate) fn matcher(&self) -> &Matcher {
        &self.0
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

/// The one compiled filter a sidebar panel has: what it narrows its list with, what its
/// rows mark with, and what its bar prints when the pattern will not compile.
///
/// A memo, so the regex is compiled when the pattern changes and not when the list does.
/// A panel redraws most while its box is being typed in, which is exactly when compiling
/// is not free. [`use_marking`] is the same hook for a find bar, whose filter is held in
/// a context rather than a state.
///
/// The memo and not the [`Marking`] in it: a panel narrows its list in a memo of its own,
/// and a memo reading a plain value captured from an earlier render would never see the
/// pattern change. The value is one `read` away where a panel wants it.
pub(crate) fn use_list_marking(filter: State<Filter>) -> Memo<Marking> {
    use_memo(move || Marking::new(filter.read().matcher()))
}
