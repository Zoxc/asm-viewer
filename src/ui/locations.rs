//! Every symbol a source line -- or the function around it -- was compiled into, across
//! every open object: the question as the reader asks it, the answer that stands until
//! the next one, and the panel that draws it.
//!
//! The query is [`compiled::compiled_from`], which the worker already runs for a
//! source-driven tab and then keeps one candidate of. This keeps them all. Asked of a
//! function's lines it is the **instance picker**: a generic function compiles into one
//! symbol per instantiation times one per object, and this is where a reader says which
//! of them the source is read against, the row's press being the same choice either way.
//! Every symbol holding code from those lines is listed, an inlined caller included, in
//! the crate's own order; the filter over the rows is how a name is narrowed to. A row of the
//! answer is a **symbol** and not a range inside one: the crate answers symbols by design
//! (where inside a symbol the line's code sits is the forward direction's question), and
//! finding each hit's ranges would be one line-program walk per symbol under the DWARF
//! context's mutex -- seconds for a line that answers with thousands, with every symbol
//! click waiting behind it. Landing on the line inside the symbol is the picked-out run's job
//! instead.
//!
//! The panel answers one more question, which is not the crate's at all: where a name is
//! **used**, which only a language server can say (`ui::language`). It is the same panel
//! because it is the same act -- a reader asking where else to look -- and a second list
//! beside this one would be two panels showing one thing at a time. What comes back is a
//! place in a file and not a symbol, so those rows are grouped under the file each is in
//! (`src/references.rs`) and open a source-driven tab; the symbols that line was compiled into
//! are then one right-click away, which is the question above.

use super::*;

/// The locations the reader last asked for, shared through context. Its own state and
/// not a reading of the active document: an answer stands until replaced, whatever the
/// reader opens meanwhile.
#[derive(Clone, Copy)]
pub(crate) struct Locations(pub(crate) State<Located>);

/// What the panel is asked for: a line, or the function around one.
///
/// `at` is the row the question was asked from in either case -- what a row of the
/// answer lands on, and what a source-driven tab is driven from when a row is chosen
/// for it -- and the scope says which lines the symbols are wanted for.
#[derive(Clone, PartialEq)]
pub(crate) struct Query {
    pub(crate) at: LinePos,
    pub(crate) scope: Scope,
}

/// The lines a [`Query`] is about.
#[derive(Clone, PartialEq)]
pub(crate) enum Scope {
    /// The one line at `Query::at`.
    Line,
    /// The whole of the function around it, as the source spells it
    /// ([`functions::enclosing`]).
    Function {
        name: String,
        lines: RangeInclusive<u32>,
    },
    /// Every reference to the name at [`Query::at`], as the language server answers it.
    ///
    /// `column` is where the name was asked about, in the UTF-16 units the protocol
    /// takes, and `run` is the server run it was asked in: an answer from a server
    /// started since is not an answer to this question.
    References { name: String, column: u32, run: u64 },
    /// What implements the name at [`Query::at`], as the language server answers it, with
    /// `column` and `run` meaning what they mean above.
    Implementations { name: String, column: u32, run: u64 },
}

impl Query {
    /// The question about one line.
    pub(crate) fn line(at: LinePos) -> Query {
        Query {
            at,
            scope: Scope::Line,
        }
    }

    /// The question about the whole of `function`, asked from `at`.
    pub(crate) fn function(at: LinePos, function: &Function) -> Query {
        Query {
            at,
            scope: Scope::Function {
                name: function.name.clone(),
                lines: function.lines.clone(),
            },
        }
    }

    /// The question about every reference to `name`, asked at `column` of `at` in the server
    /// run `run`.
    pub(crate) fn references(at: LinePos, name: String, column: u32, run: u64) -> Query {
        Query {
            at,
            scope: Scope::References { name, column, run },
        }
    }

    /// The question about what implements `name`, asked the same way.
    pub(crate) fn implementations(at: LinePos, name: String, column: u32, run: u64) -> Query {
        Query {
            at,
            scope: Scope::Implementations { name, column, run },
        }
    }

    /// The server run this was asked in, and `None` where it is not a question for a
    /// server at all. What an answer is matched against, so that neither the run nor the
    /// question has to be named twice.
    pub(crate) fn run(&self) -> Option<u64> {
        match &self.scope {
            Scope::Line | Scope::Function { .. } => None,
            Scope::References { run, .. } | Scope::Implementations { run, .. } => Some(*run),
        }
    }

    /// What the rows are, singular and plural: the one place the wording of a question
    /// lives, so the heading, the wait and the empty answer cannot drift apart.
    fn words(&self) -> (&'static str, &'static str) {
        match self.scope {
            Scope::Line => ("location for", "locations for"),
            Scope::Function { .. } => ("instance of", "instances of"),
            Scope::References { .. } => ("reference to", "references to"),
            Scope::Implementations { .. } => ("implementation of", "implementations of"),
        }
    }

    /// The lines the symbols are wanted for, and `None` where symbols are not what is
    /// wanted: a question about references is the language server's and never the worker's.
    pub(crate) fn symbols_wanted(&self) -> Option<RangeInclusive<u32>> {
        match &self.scope {
            Scope::Line => Some(self.at.line..=self.at.line),
            Scope::Function { lines, .. } => Some(lines.clone()),
            Scope::References { .. } | Scope::Implementations { .. } => None,
        }
    }

    /// What the panel calls the question: `file:line`, or the function's name.
    fn spell(&self) -> String {
        match &self.scope {
            Scope::Line => spell(&self.at),
            Scope::Function { name, .. }
            | Scope::References { name, .. }
            | Scope::Implementations { name, .. } => name.clone(),
        }
    }

    /// The whole of it, for the heading's tooltip: the file's path, and for a function
    /// the lines of it that were asked about.
    fn tooltip(&self) -> String {
        match &self.scope {
            Scope::Line => self.at.file.to_string(),
            Scope::Function { lines, .. } => {
                format!("{}:{}\u{2013}{}", self.at.file, lines.start(), lines.end())
            }
            // Where it was asked about, which is the one thing a name alone does not say.
            Scope::References { .. } | Scope::Implementations { .. } => {
                format!("{}:{}", self.at.file, self.at.line)
            }
        }
    }

    /// The heading over `count` rows: what they are, and what they are of.
    fn heading(&self, count: usize) -> String {
        let (one, many) = self.words();
        format!(
            "{count} {} {}",
            if count == 1 { one } else { many },
            self.spell()
        )
    }
}

/// What was asked, and what it came to.
///
/// There is no `pending` field: a question is being looked for exactly while it is `asked`
/// and `found` is not about it, which [`Located::pending`] reads off the two.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Located {
    /// The question whose symbols are wanted, or `None` until anything has been asked.
    pub(crate) asked: Option<Query>,
    /// The source-driven tab the line was asked from and the file it was showing, when
    /// it was asked from one: a row is then **chosen for that tab** -- its assembly side
    /// follows the symbol -- rather than opened as a tab of its own. Asked from an
    /// assembly-driven tab, or once that tab has closed or moved off the file, a row
    /// opens the symbol.
    pub(crate) subject: Option<(DocId, Arc<str>)>,
    /// The last answer, whatever it answered with -- an empty list is an answer.
    pub(crate) found: Option<Found>,
}

impl Located {
    /// The question being looked for and not yet found.
    pub(crate) fn pending(&self) -> Option<&Query> {
        let asked = self.asked.as_ref()?;
        let found = self.found.as_ref().map(|found| &found.of);
        (found != Some(asked)).then_some(asked)
    }

    /// Take `found` as the answer to the question this is waiting for, `run` being the
    /// server run it came back under. Whether anything changed, so the caller writes only
    /// then.
    ///
    /// Which question it was is not asked: only a question for a server has a run at all
    /// (`Query::run`), and there is one of those pending at a time.
    ///
    /// An answer under a run this did not ask in is an answer to nobody, and so is one to
    /// a question already answered. **Every way of not answering is an empty answer**: a
    /// server that refused the question or stopped answering it leaves a question that
    /// would otherwise be looked for for ever.
    pub(crate) fn answer_places(&mut self, run: u64, found: references::References) -> bool {
        let asked = self.pending().filter(|query| query.run() == Some(run));
        let Some(of) = asked.cloned() else {
            return false;
        };
        self.found = Some(Found {
            of,
            what: What::Places(found),
        });
        true
    }

    /// Fold the file at `path` in a list of places, or unfold it. Whether anything changed.
    pub(crate) fn fold(&mut self, path: &Path) -> bool {
        let Some(found) = self.found.as_mut() else {
            return false;
        };
        match &mut found.what {
            What::Places(places) => places.toggle(path),
            What::Symbols(_) => false,
        }
    }
}

/// The answer to one question: what it was, and what came of it.
#[derive(Clone, PartialEq)]
pub(crate) struct Found {
    pub(crate) of: Query,
    pub(crate) what: What,
}

/// What an answer holds, which is what was asked for.
#[derive(Clone, PartialEq)]
pub(crate) enum What {
    /// Every symbol compiled from the question's lines, over the objects that were open
    /// when it was asked, in the crate's own order -- object by object and by address
    /// within one, which is a tie-break and not a ranking.
    ///
    /// [`SymbolList`] and not a `Vec`, so handing it to the rows is a pointer compare
    /// rather than a walk of thousands.
    Symbols(SymbolList),
    /// The places one of the server's two list questions answered with, under the file
    /// each is in. Both are the same shape, and the panel draws one at a time, so which
    /// question it was is the `Query`'s to say and not this.
    Places(references::References),
}

impl Found {
    pub(crate) fn new(of: Query, symbols: Vec<Symbol>) -> Found {
        Found {
            of,
            what: What::Symbols(SymbolList(Arc::new(symbols))),
        }
    }

    /// The symbols it answered with, and `None` where it was a question for the server.
    pub(crate) fn symbols(&self) -> Option<&SymbolList> {
        match &self.what {
            What::Symbols(symbols) => Some(symbols),
            What::Places(_) => None,
        }
    }

    /// The places it answered with, and `None` where it was a question about symbols.
    pub(crate) fn places(&self) -> Option<&references::References> {
        match &self.what {
            What::Places(places) => Some(places),
            What::Symbols(_) => None,
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
        // A use is a place in a file and holds no object, so there is nothing here for a
        // closed binary to take.
        let What::Symbols(symbols) = &self.what else {
            return false;
        };
        let open: HashSet<usize> = open
            .iter()
            .map(|object| Arc::as_ptr(object).addr())
            .collect();
        let kept: Vec<Symbol> = symbols
            .0
            .iter()
            .filter(|symbol| open.contains(&Arc::as_ptr(&symbol.object).addr()))
            .cloned()
            .collect();
        if kept.len() == symbols.0.len() {
            return false;
        }
        self.what = What::Symbols(SymbolList(Arc::new(kept)));
        true
    }
}

/// Ask `query`, and bring the panel that will answer to the front. The one writer of
/// [`Located::asked`].
///
/// Asking the question already answered asks again: the objects may have changed since,
/// and the answer is about the objects that were open when it was asked. Dropping the
/// stale answer is what makes the effect send the question, there being no `pending` to
/// set. The panel is looked for in `dock` and then in the area beside it, since a view
/// may be dragged into either; brought to the front on its own and never on an answer
/// landing, so a reader who moved on meanwhile is not pulled back.
pub(crate) fn find_locations(
    mut located: State<Located>,
    dock: State<DockArea>,
    query: Query,
    subject: Option<(DocId, Arc<str>)>,
) {
    let mut next = located.peek().clone();
    if next.found.as_ref().is_some_and(|found| found.of == query) {
        next.found = None;
    }
    next.asked = Some(query);
    next.subject = subject;
    located.set(next);

    raise_view(dock, View::Locations);
}

/// Ask where the name at `column` of `at` is used, and bring the panel to the front.
pub(crate) fn find_references(
    located: State<Located>,
    dock: State<DockArea>,
    language: State<Language>,
    jobs: &LspJobs,
    at: LinePos,
    name: String,
    column: u32,
) {
    find_places(
        located,
        dock,
        language,
        jobs,
        at,
        name,
        column,
        Wanted::References,
        Query::references,
    );
}

/// Ask what implements the name at `column` of `at`, and bring the panel to the front.
pub(crate) fn find_implementations(
    located: State<Located>,
    dock: State<DockArea>,
    language: State<Language>,
    jobs: &LspJobs,
    at: LinePos,
    name: String,
    column: u32,
) {
    find_places(
        located,
        dock,
        language,
        jobs,
        at,
        name,
        column,
        Wanted::Implementations,
        Query::implementations,
    );
}

/// The half the panel's two questions for the server share: ask it, hold the question,
/// and raise the panel.
///
/// The question is the server's, so it is sent here rather than from the effect that
/// sends the worker's: what it is asked in is a server run, and there is nothing to ask
/// with no server -- a question is not what starts one, that being the control the reader
/// presses (`follow_name`'s rule).
#[allow(clippy::too_many_arguments)]
fn find_places(
    mut located: State<Located>,
    dock: State<DockArea>,
    language: State<Language>,
    jobs: &LspJobs,
    at: LinePos,
    name: String,
    column: u32,
    want: Wanted,
    query: fn(LinePos, String, u32, u64) -> Query,
) {
    let lookup = Lookup {
        file: PathBuf::from(&*at.file),
        // The protocol counts lines from zero, where a `LinePos` is 1-based; the column
        // is already what it takes.
        line: at.line.saturating_sub(1),
        column,
    };
    let Some(run) = ask_where(language, jobs, lookup, want) else {
        return;
    };
    let query = query(at, name, column, run);

    // Asking again drops the answer that stands, which is what makes this question
    // pending; `find_locations`' rule, and here it cannot even be the same question,
    // since the run it was asked in is part of it.
    let mut next = located.peek().clone();
    if next.found.as_ref().is_some_and(|found| found.of == query) {
        next.found = None;
    }
    next.asked = Some(query);
    // These answers are places in files: no row of one chooses a symbol for a tab.
    next.subject = None;
    located.set(next);

    raise_view(dock, View::Locations);
}

/// The menu a source row or an instruction row opens on a right-click: the line's
/// locations, -- for a source row inside a function -- the function's instances, and
/// and `named` where the press was on a name a server can be asked about, the things a
/// row is asked for that a click does not do. Built per press, as `close_menu` is, closing
/// over the row's line; the states come in as arguments because this is called from an
/// event handler, where no hook may run.
pub(crate) fn locate_menu(
    located: State<Located>,
    dock: State<DockArea>,
    at: LinePos,
    subject: Option<(DocId, Arc<str>)>,
    function: Option<Function>,
    named: Vec<MenuButton>,
) -> Menu {
    let line = Query::line(at.clone());
    let instances = function.map(|function| {
        let query = Query::function(at, &function);
        let subject = subject.clone();
        MenuButton::new()
            .on_press(move |_| find_locations(located, dock, query.clone(), subject.clone()))
            .child(format!("Find instances of {}", function.name))
    });

    // The name's questions first, where the press was on one: they are about what is
    // under the pointer, where the two below are about the line it is on.
    Menu::new()
        .children(named.into_iter().map(MenuButton::into_element))
        .child(
            MenuButton::new()
                .on_press(move |_| find_locations(located, dock, line.clone(), subject.clone()))
                .child("Find all locations"),
        )
        .maybe_child(instances)
}

/// A line as the panel names it: the file's own name and the line, the full path being
/// the tooltip's.
fn spell(at: &LinePos) -> String {
    format!("{}:{}", file_name(&at.file), at.line)
}

/// The Locations view: what was asked about, over every symbol it answered with.
///
/// `HistoryTab`'s shape with `SymbolsTab`'s list: a filter over a `VirtualScrollView`,
/// through the same `Filtered` memo, because one line answers with thousands. What the
/// pane says is decided in one `match` off [`Located`]'s two fields, so "nothing asked",
/// "being looked for", "found nothing" and the rows cannot disagree about which they are.
///
/// The row lit is the symbol the panes are **drawing** -- `Analysis`, not `Active` --
/// because for a source-driven tab the active document is a file, and the whole point of
/// choosing a row for one is that its assembly side changes; the lit row is the one
/// answer the panel gives to which instance is up. Reading `Analysis` wakes the tab on
/// the worker's `pending` and `slow` flips too, which the rows' data compares equal
/// across, so nothing below re-renders for them.
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
                .and_then(Found::symbols)
                .cloned()
                .unwrap_or_else(|| SymbolList(Arc::new(Vec::new())));
            Filtered::new(symbols, &filter.read().matcher())
        });
        let filtered = filtered.read().clone();
        // A references answer is tens of rows where a line's symbols are thousands, so the
        // filter is applied where the rows are built (`filter_bar.rs`) -- but through a
        // memo all the same, since the rows are compared by the pointer they are shared
        // under and a fresh one every render would redraw every row.
        let used = use_memo(move || {
            let state = located.read();
            let found = state.found.as_ref().and_then(Found::places);
            found
                .map(|found| found.rows_matching(&filter.read().matcher()))
                .unwrap_or_default()
        });
        let used = used.read().clone();
        let selected = use_consume::<Analysis>()
            .0
            .read()
            .shown
            .as_ref()
            .map(|shown| shown.studied.symbol.clone());
        let state = located.read().clone();

        let body: Element = match (&state.asked, state.pending(), &state.found) {
            (None, _, _) => placeholder("Nothing looked for yet"),
            (Some(_), Some(query), _) => placeholder(format!(
                "Finding {} {}\u{2026}",
                query.words().1,
                query.spell()
            )),
            (Some(query), None, Some(found))
                if found.places().is_some_and(|found| found.count() == 0) =>
            {
                placeholder(format!("No {} {}", query.words().1, query.spell()))
            }
            (Some(query), None, Some(found)) if found.places().is_some() => {
                let heading =
                    query.heading(found.places().map_or(0, references::References::count));
                let length = used.len();
                rect()
                    .expanded()
                    .content(Content::Flex)
                    .child(row_tooltip(
                        query.tooltip(),
                        section_heading(&heading, None),
                    ))
                    .child(
                        rect().width(Size::fill()).height(Size::flex(1.0)).child(
                            VirtualScrollView::new_with_data(used, |row, used: &ReferenceRows| {
                                ReferencesRow {
                                    rows: used.clone(),
                                    index: row,
                                    key: DiffKey::None,
                                }
                                .into()
                            })
                            .length(length)
                            .item_size(list_row_height()),
                        ),
                    )
                    .into()
            }
            (Some(query), None, Some(found))
                if found.symbols().is_some_and(|symbols| symbols.0.is_empty()) =>
            {
                placeholder(format!("No code compiled from {}", query.spell()))
            }
            (Some(query), None, Some(found)) => {
                // Said over the list rather than in the tab's title: the rows are not
                // the answer to anything until the question is in view with them.
                let heading = query.heading(found.symbols().map_or(0, |symbols| symbols.0.len()));
                let length = filtered.len();
                rect()
                    .expanded()
                    .content(Content::Flex)
                    .child(row_tooltip(
                        query.tooltip(),
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

        use_filter_pane(filter, palette().symbol_pane_bg, body)
    }
}

/// One row of a references answer: a file, or one of the references under it.
///
/// `HitRow`'s shape (`ui::search_view`), for the same reason -- a flattened tree drawn by
/// a scroll view -- and not its rows: a hit carries the line's text, which a search read
/// off the disk and a language server never says.
#[derive(Clone)]
struct ReferencesRow {
    rows: ReferenceRows,
    index: usize,
    key: DiffKey,
}

impl PartialEq for ReferencesRow {
    fn eq(&self, other: &Self) -> bool {
        self.rows == other.rows && self.index == other.index
    }
}

impl KeyExt for ReferencesRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for ReferencesRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut located = use_consume::<Locations>().0;
        let open = use_open();
        let visits = use_consume::<Visited>().0;
        let ctrl = use_consume::<Ctrl>().0;
        let marked = use_consume::<Marked>().0;
        let landing = use_consume::<Land>().0;
        let plant = use_consume::<Plant>().0;
        let driven = use_consume::<Drives>().0;

        let row = self.rows.row(self.index).clone();
        let pressed = row.clone();
        let tooltip = match &row {
            ReferenceRow::File { path, .. } => path.display().to_string(),
            ReferenceRow::Reference { path, reference } => {
                format!("{}:{}", path.display(), reference.line)
            }
        };

        let background = if hovering() {
            palette().object_hover_bg
        } else {
            Color::TRANSPARENT
        };

        row_tooltip(
            tooltip,
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .content(Content::Flex)
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .padding(Gaps::new_symmetric(0.0, 5.0))
                .spacing(5.0)
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| match &pressed {
                    // Bound to a `let` of its own, so the guard the read hands back is
                    // gone before the write.
                    ReferenceRow::File { path, .. } => {
                        let mut next = located.peek().clone();
                        if next.fold(path) {
                            located.set(next);
                        }
                    }
                    ReferenceRow::Reference { path, reference } => open_source_place(
                        open,
                        visits,
                        marked,
                        landing,
                        plant,
                        driven,
                        path,
                        reference.line,
                        Some(reference.columns.start as usize..reference.columns.end as usize),
                        reach(ctrl),
                    ),
                })
                .children(reference_row_children(&row)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// What a references row draws: a file row is its fold, its name and its count, and a
/// reference row its line number and the line, both as the Search panel's rows draw
/// theirs.
fn reference_row_children(row: &ReferenceRow) -> Vec<Element> {
    match row {
        ReferenceRow::File {
            name,
            count,
            folded,
            ..
        } => vec![
            label()
                .text(if *folded { "\u{25b8}" } else { "\u{25be}" })
                .width(Size::px(CHEVRON_WIDTH))
                .color(palette().icon_fg)
                .into_element(),
            tree_name(name.clone(), false).into_element(),
            label()
                .text(count.to_string())
                .margin(Gaps::new(0.0, 0.0, 0.0, COUNT_GUTTER))
                .color(palette().address_fg)
                .max_lines(1)
                .into_element(),
        ],
        // The Search panel's match row: the line's number, and the line with the name
        // marked in it. A file that would not read leaves the text empty, and the row is
        // the number alone.
        ReferenceRow::Reference { reference, .. } => vec![
            label()
                .text(reference.line.to_string())
                .width(Size::px(LINE_NUMBER_WIDTH))
                .text_align(TextAlign::Right)
                .color(palette().address_fg)
                .max_lines(1)
                .into_element(),
            rect()
                .width(Size::flex(1.0))
                .overflow(Overflow::Clip)
                .child(
                    paragraph()
                        .width(Size::fill())
                        .max_lines(1)
                        .text_overflow(TextOverflow::Ellipsis)
                        .spans_iter(marked_spans(&reference.text, &reference.spans).into_iter()),
                )
                .into_element(),
        ],
    }
}

/// One symbol a line was compiled into: its name, and the object it is in after it,
/// since the same name in two objects is two rows and the object is what tells them
/// apart.
#[derive(Clone)]
struct LocationRow {
    symbols: SymbolList,
    index: usize,
    /// Whether this is the symbol the panes are drawing.
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
        let visits = use_consume::<Visited>().0;
        let ctrl = use_consume::<Ctrl>().0;
        let marked = use_consume::<Marked>().0;
        let landing = use_consume::<Land>().0;
        let plant = use_consume::<Plant>().0;
        let driven = use_consume::<Drives>().0;
        let located = use_consume::<Locations>().0.peek().clone();
        let at = located.found.as_ref().map(|found| found.of.at.clone());
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
                    let symbol_tab = Document::Assembly(Selection::Symbol(symbol.clone()));
                    // The line is the answer's own, peeked when the row was built: a row
                    // is a row of one answer and cannot outlive it.
                    let Some(at) = at.clone() else {
                        open_document(open, visits, symbol_tab, reach(ctrl));
                        return;
                    };
                    // Asked from a source-driven tab that is still open and still on the
                    // file: chosen for it. The choice is that entry's, and the entry is
                    // driven from the line the question was asked from, so the tab's
                    // assembly side becomes this symbol -- for an instance, provided the
                    // instance holds code from that line, which `compiled::pick` falls
                    // back from where it does not. Bound to a `let` so the table's guard
                    // is gone before `driven` is written.
                    let subject = subject.clone().filter(|(id, file)| {
                        open.docs.peek().get(*id) == Some(&Document::Source(file.clone()))
                    });
                    match subject {
                        Some((id, file)) => {
                            // The place that tab is at, not the file: a drive written
                            // under a stop the trail does not hold is a drive nothing
                            // reads. The guard is gone before `driven` is written.
                            let at_place = place_at(&open.docs.peek(), id, &Document::Source(file));
                            let entry = (id, at_place);
                            {
                                let mut driven = driven;
                                let mut driven = driven.write();
                                driven.remember(entry.clone(), at.line);
                                driven.choose(entry, symbol.clone());
                            }
                            land_on(open, marked, landing, id, at);
                        }
                        None => {
                            // A line and no instruction: the row names a place in a
                            // file, and the assembly pane's caret is the pair's.
                            land(
                                open,
                                visits,
                                marked,
                                landing,
                                plant,
                                Landing {
                                    tab: symbol_tab,
                                    at: Some(at),
                                    address: None,
                                    columns: None,
                                },
                                reach(ctrl),
                            );
                        }
                    }
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
