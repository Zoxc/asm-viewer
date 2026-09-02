//! Every symbol a source line was compiled into, across every open object: the question
//! as the reader asks it, the answer that stands until the next one, and the panel that
//! draws it.
//!
//! The query is [`compiled::compiled_from`], which the worker already runs for a
//! source-driven tab and then keeps one candidate of. This keeps them all. A row of the
//! answer is a **symbol** and not a range inside one: the crate answers symbols by design
//! (where inside a symbol the line's code sits is the forward direction's question), and
//! finding each hit's ranges would be one line-program walk per symbol under the DWARF
//! context's mutex -- seconds for a line that answers with thousands, with every symbol
//! click waiting behind it. Landing on the line inside the symbol is the pin's job instead.

use super::*;

/// The locations the reader last asked for, shared through context. Its own state and
/// not a reading of the active document: an answer stands until replaced, whatever the
/// reader opens meanwhile.
#[derive(Clone, Copy)]
pub(crate) struct Locations(pub(crate) State<Located>);

/// What was asked, and what it came to.
///
/// There is no `pending` field: a line is being looked for exactly while it is `asked`
/// and `found` is not about it, which [`Located::pending`] reads off the two.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Located {
    /// The line whose locations are wanted, or `None` until anything has been asked.
    pub(crate) asked: Option<LinePos>,
    /// The file of the source-driven tab the line was asked from, when it was asked
    /// from one: a row is then **chosen for that tab** -- its assembly side follows the
    /// symbol -- rather than opened as a tab of its own. Asked from an assembly-driven
    /// tab, or once that tab has closed, a row opens the symbol.
    pub(crate) subject: Option<Arc<str>>,
    /// The last answer, whatever it answered with -- an empty list is an answer.
    pub(crate) found: Option<Found>,
}

impl Located {
    /// The line being looked for and not yet found.
    pub(crate) fn pending(&self) -> Option<&LinePos> {
        let asked = self.asked.as_ref()?;
        let found = self.found.as_ref().map(|found| &found.at);
        (found != Some(asked)).then_some(asked)
    }
}

/// The answer to one line: every symbol compiled from it, over the objects that were
/// open when it was asked, in the crate's own order -- object by object and by address
/// within one, which is a tie-break and not a ranking.
#[derive(Clone, PartialEq)]
pub(crate) struct Found {
    pub(crate) at: LinePos,
    /// [`SymbolList`] and not a `Vec`, so handing it to the rows is a pointer compare
    /// rather than a walk of thousands.
    pub(crate) symbols: SymbolList,
}

impl Found {
    pub(crate) fn new(at: LinePos, symbols: Vec<Symbol>) -> Found {
        Found {
            at,
            symbols: SymbolList(Arc::new(symbols)),
        }
    }

    /// Drop every symbol whose object is no longer among `open`, answering whether any
    /// went -- so the caller writes only then.
    ///
    /// `Shown::still_open`'s rule in a second place, for its reason: a [`Symbol`] holds
    /// its `Arc<Object>` holds the whole file's bytes, and this list can hold thousands of
    /// them long after the file was closed. A set of the open objects' addresses rather
    /// than a scan per symbol, since it is thousands against however many are open.
    pub(crate) fn retain_open(&mut self, open: &[Arc<Object>]) -> bool {
        let open: HashSet<usize> = open
            .iter()
            .map(|object| Arc::as_ptr(object).addr())
            .collect();
        let kept: Vec<Symbol> = self
            .symbols
            .0
            .iter()
            .filter(|symbol| open.contains(&Arc::as_ptr(&symbol.object).addr()))
            .cloned()
            .collect();
        if kept.len() == self.symbols.0.len() {
            return false;
        }
        self.symbols = SymbolList(Arc::new(kept));
        true
    }
}

/// Ask for every location of `at`, and bring the panel that will answer to the front.
/// The one writer of [`Located::asked`].
///
/// Asking for the line already answered asks again: the objects may have changed since,
/// and the answer is about the objects that were open when it was asked. Dropping the
/// stale answer is what makes the effect send the question, there being no `pending` to
/// set. The panel is looked for in `dock` and then in the area beside it, since a view
/// may be dragged into either; brought to the front on its own and never on an answer
/// landing, so a reader who moved on meanwhile is not pulled back.
pub(crate) fn find_locations(
    mut located: State<Located>,
    mut dock: State<DockArea>,
    at: LinePos,
    subject: Option<Arc<str>>,
) {
    let mut next = located.peek().clone();
    if next.found.as_ref().is_some_and(|found| found.at == at) {
        next.found = None;
    }
    next.asked = Some(at);
    next.subject = subject;
    located.set(next);

    // Bound before the write below, so the read is gone by then.
    let other = dock.peek().other;
    if !dock.write().show_view(View::Locations) {
        if let Some(mut other) = other {
            other.write().show_view(View::Locations);
        }
    }
}

/// The menu a source row or an instruction row opens on a right-click: one entry, since
/// finding the line's locations is the one thing a row is asked for that a click does
/// not do. Built per press, as `close_menu` is, closing over the row's line; the states
/// come in as arguments because this is called from an event handler, where no hook may
/// run.
pub(crate) fn locate_menu(
    located: State<Located>,
    dock: State<DockArea>,
    at: LinePos,
    subject: Option<Arc<str>>,
) -> Menu {
    Menu::new().child(
        MenuButton::new()
            .on_press(move |_| find_locations(located, dock, at.clone(), subject.clone()))
            .child("Find all locations"),
    )
}

/// A line as the panel names it: the file's own name and the line, the full path being
/// the tooltip's.
fn spell(at: &LinePos) -> String {
    format!("{}:{}", file_name(&at.file), at.line)
}

/// The Locations view: the line that was asked about, over every symbol it answered with.
///
/// `HistoryTab`'s shape with `SymbolsTab`'s list: a filter over a `VirtualScrollView`,
/// through the same `Filtered` memo, because one line answers with thousands. What the
/// pane says is decided in one `match` off [`Located`]'s two fields, so "nothing asked",
/// "being looked for", "found nothing" and the rows cannot disagree about which they are.
#[derive(PartialEq)]
pub(crate) struct LocationsTab;

impl Component for LocationsTab {
    fn render(&self) -> impl IntoElement {
        let located = use_consume::<Locations>().0;
        let filter = use_state(Filter::default);
        let filtered = use_memo(move || {
            let symbols = located
                .read()
                .found
                .as_ref()
                .map(|found| found.symbols.clone())
                .unwrap_or_else(|| SymbolList(Arc::new(Vec::new())));
            Filtered::new(symbols, &filter.read().matcher())
        });
        let filtered = filtered.read().clone();
        let selected = match &*use_consume::<Active>().0.read() {
            Some(Document::Assembly(Selection::Symbol(symbol))) => Some(symbol.clone()),
            _ => None,
        };
        let state = located.read().clone();

        let body: Element = match (&state.asked, state.pending(), &state.found) {
            (None, _, _) => placeholder("Nothing looked for yet"),
            (Some(at), Some(_), _) => {
                placeholder(format!("Finding locations for {}\u{2026}", spell(at)))
            }
            (Some(at), None, Some(found)) if found.symbols.0.is_empty() => {
                placeholder(format!("No code compiled from {}", spell(at)))
            }
            (Some(at), None, Some(found)) => {
                // Said over the list rather than in the tab's title: the rows are not
                // the answer to anything until the question is in view with them.
                let heading = format!(
                    "{} {} for {}",
                    found.symbols.0.len(),
                    if found.symbols.0.len() == 1 {
                        "location"
                    } else {
                        "locations"
                    },
                    spell(at)
                );
                let length = filtered.len();
                rect()
                    .expanded()
                    .content(Content::Flex)
                    .child(row_tooltip(
                        at.file.to_string(),
                        section_heading(&heading, None),
                    ))
                    .child(
                        rect().width(Size::fill()).height(Size::flex(1.0)).child(
                            VirtualScrollView::new_with_data(
                                (filtered, selected),
                                |row, (filtered, selected): &(Filtered, Option<Symbol>)| {
                                    let index = filtered.index(row);
                                    let symbol = &filtered.symbols.0[index];
                                    LocationRow {
                                        symbols: filtered.symbols.clone(),
                                        index,
                                        selected: selected.as_ref() == Some(symbol),
                                        key: DiffKey::None,
                                    }
                                    // The symbol *and* its object: one file parsed
                                    // twice is two rows naming one `SymbolData`.
                                    .key((
                                        Arc::as_ptr(&symbol.object).addr(),
                                        Arc::as_ptr(&symbol.data).addr(),
                                    ))
                                    .into()
                                },
                            )
                            .length(length)
                            .item_size(list_row_height()),
                        ),
                    )
                    .into()
            }
            // Asked and not pending is found, by `pending`'s definition.
            (Some(_), None, None) => placeholder("Nothing looked for yet"),
        };

        filter_pane(filter, palette().symbol_pane_bg, body)
    }
}

/// One symbol a line was compiled into: its name, and the object it is in after it,
/// since the same name in two objects is two rows and the object is what tells them
/// apart.
#[derive(Clone)]
struct LocationRow {
    symbols: SymbolList,
    index: usize,
    /// Whether this is the symbol on screen.
    selected: bool,
    key: DiffKey,
}

impl PartialEq for LocationRow {
    fn eq(&self, other: &Self) -> bool {
        self.symbols == other.symbols
            && self.index == other.index
            && self.selected == other.selected
    }
}

impl KeyExt for LocationRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for LocationRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let open = use_open();
        let history = use_consume::<Hist>().0;
        let pinned = use_consume::<Pinned>().0;
        let landing = use_consume::<Land>().0;
        let driven = use_consume::<Drives>().0;
        let located = use_consume::<Locations>().0.peek().clone();
        let at = located.found.as_ref().map(|found| found.at.clone());
        let subject = located.subject.clone();
        let symbol = self.symbols.0[self.index].clone();
        let name = symbol.data.display().to_owned();
        let object = symbol.object.name.clone();

        let background = if self.selected {
            palette().selected_bg
        } else if hovering() {
            palette().symbol_hover_bg
        } else {
            Color::TRANSPARENT
        };

        row_tooltip(
            format!("{name} \u{2014} {object}"),
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .content(Content::Flex)
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .padding(5.0)
                .spacing(5.0)
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| {
                    // The line is the answer's own, peeked when the row was built: a row
                    // is a row of one answer and cannot outlive it.
                    let Some(at) = at.clone() else {
                        activate(
                            open,
                            history,
                            Some(Document::Assembly(Selection::Symbol(symbol.clone()))),
                            Visit::Went,
                        );
                        return;
                    };
                    // Asked from a source-driven tab that is still open: chosen for it.
                    // The choice is the tab's, and the tab is driven from the line the
                    // locations are of, so its assembly side becomes this symbol.
                    let tab = subject.clone().map(Document::Source);
                    let tab = tab.filter(|tab| open.docs.peek().id_of(tab).is_some());
                    let target = match tab {
                        Some(tab) => {
                            let mut driven = driven;
                            driven.write().remember(tab.clone(), at.line);
                            driven.write().choose(tab.clone(), symbol.clone());
                            tab
                        }
                        None => Document::Assembly(Selection::Symbol(symbol.clone())),
                    };
                    land(open, history, pinned, landing, target, at);
                })
                .child(tree_name(name, false))
                // Capped rather than measured, or a long member name would take the row
                // and leave the symbol it is about with nothing.
                .child(
                    rect()
                        .max_width(Size::percent(45.0))
                        .overflow(Overflow::Clip)
                        .child(
                            label()
                                .text(object)
                                .max_lines(1)
                                .color(palette().address_fg)
                                .text_overflow(TextOverflow::Ellipsis),
                        ),
                ),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}
