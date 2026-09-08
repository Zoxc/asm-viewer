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
    /// One of the language server's two list questions about the name at [`Query::at`]:
    /// everywhere it is used, or what implements it. One variant because the two are one
    /// shape and the panel draws one of them at a time; `of` is which.
    ///
    /// `column` is where the name was asked about, as a byte offset into its line
    /// (`src/lsp.rs`); `run` is the server run it was asked in, since an answer from a server
    /// started since is not an answer to this question; and `id` is the question's own,
    /// since neither is a run when two questions are asked in one -- which is the
    /// ordinary case, a run lasting as long as the server.
    Listed {
        of: lsp::Listed,
        name: String,
        column: u32,
        run: u64,
        id: u64,
    },
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

    /// The question `of` about `name`, asked at `column` of `at` as the question `id` of
    /// the server run `run`.
    pub(crate) fn listed(
        of: lsp::Listed,
        at: LinePos,
        name: String,
        column: u32,
        run: u64,
        id: u64,
    ) -> Query {
        Query {
            at,
            scope: Scope::Listed {
                of,
                name,
                column,
                run,
                id,
            },
        }
    }

    /// The server run this was asked in and which question of it this is, and `None`
    /// where it is not a question for a server at all. What an answer is matched against,
    /// so that neither the question nor what it was asked under has to be named twice.
    pub(crate) fn asked(&self) -> Option<(u64, u64)> {
        match &self.scope {
            Scope::Line | Scope::Function { .. } => None,
            Scope::Listed { run, id, .. } => Some((*run, *id)),
        }
    }

    /// What the rows are, singular and plural: the one place the wording of a question
    /// lives, so the heading, the wait and the empty answer cannot drift apart.
    fn words(&self) -> (&'static str, &'static str) {
        match self.scope {
            Scope::Line => ("location for", "locations for"),
            Scope::Function { .. } => ("instance of", "instances of"),
            Scope::Listed {
                of: lsp::Listed::References,
                ..
            } => ("reference to", "references to"),
            Scope::Listed {
                of: lsp::Listed::Implementations,
                ..
            } => ("implementation of", "implementations of"),
        }
    }

    /// The lines the symbols are wanted for, and `None` where symbols are not what is
    /// wanted: a question about references is the language server's and never the worker's.
    pub(crate) fn symbols_wanted(&self) -> Option<RangeInclusive<u32>> {
        match &self.scope {
            Scope::Line => Some(self.at.line..=self.at.line),
            Scope::Function { lines, .. } => Some(lines.clone()),
            Scope::Listed { .. } => None,
        }
    }

    /// What the panel calls the question: `file:line`, or the function's name.
    fn spell(&self) -> String {
        match &self.scope {
            Scope::Line => spell(&self.at),
            Scope::Function { name, .. } | Scope::Listed { name, .. } => name.clone(),
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
            Scope::Listed { .. } => format!("{}:{}", self.at.file, self.at.line),
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

    /// Take `found` as the answer to the question this is waiting for, `run` and `id`
    /// being the server run and the question it came back under. Whether anything
    /// changed, so the caller writes only then.
    ///
    /// An answer under a run this did not ask in is an answer to nobody; so is one to
    /// another question of that run, which is what a reader asking a second thing before
    /// the first came back leaves behind; and so is one to a question already answered.
    /// **Every way of not answering is an empty answer**: a server that refused the
    /// question or stopped answering it leaves a question that would otherwise be looked
    /// for for ever.
    pub(crate) fn answer_places(
        &mut self,
        run: u64,
        id: u64,
        found: references::References,
    ) -> bool {
        let asked = self
            .pending()
            .filter(|query| query.asked() == Some((run, id)));
        let Some(of) = asked.cloned() else {
            return false;
        };
        self.found = Some(Found {
            of,
            what: What::Places(found),
        });
        true
    }

    /// Take `symbols` as the answer to `query`, over the binaries `open`. Whether
    /// anything changed, so the caller writes only then ([`write_if`]).
    ///
    /// [`Analyzed::take`]'s rule against the question the panel is asking *now*: a reader
    /// who asked for something else while the worker ran is not given what they left. And
    /// [`Shown::still_open`]'s rule applied per symbol, so a binary closed while the
    /// worker ran is not put back by its answer.
    pub(crate) fn take(
        &mut self,
        query: Query,
        symbols: Vec<Symbol>,
        open: &[Arc<Object>],
    ) -> bool {
        if self.asked.as_ref() != Some(&query) {
            return false;
        }
        let mut found = Found::new(query, symbols);
        found.retain_open(open);
        self.found = Some(found);
        true
    }

    /// **A closed binary takes its locations with it**: drop every symbol whose object is
    /// no longer among `open`, answering whether any went -- so a load that only added an
    /// object writes nothing.
    pub(crate) fn retain_open(&mut self, open: &[Arc<Object>]) -> bool {
        self.found
            .as_mut()
            .is_some_and(|found| found.retain_open(open))
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
    /// A [`Shared`] and not a `Vec`, so handing it to the rows is a pointer compare
    /// rather than a walk of thousands.
    Symbols(Shared<Symbol>),
    /// The places one of the server's two list questions answered with, under the file
    /// each is in. Both are the same shape, and the panel draws one at a time, so which
    /// question it was is the `Query`'s to say and not this.
    Places(references::References),
}

impl Found {
    pub(crate) fn new(of: Query, symbols: Vec<Symbol>) -> Found {
        Found {
            of,
            what: What::Symbols(symbols.into()),
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
            .iter()
            .filter(|symbol| open.contains(&Arc::as_ptr(&symbol.object).addr()))
            .cloned()
            .collect();
        if kept.len() == symbols.len() {
            return false;
        }
        self.what = What::Symbols(kept.into());
        true
    }
}

/// Ask `query`, and bring the panel that will answer to the front. The one writer of
/// [`Located::asked`].
///
/// Asking the question already answered asks again: the objects may have changed since,
/// and the answer is about the objects that were open when it was asked. Dropping the
/// stale answer is what makes the effect send the question, there being no `pending` to
/// set. The panel is brought to the top of whichever group of the sidebar holds it, since
/// it may have been dragged into any of them -- and only when the question is asked, never
/// when the answer lands, so a reader who moved on meanwhile is not pulled back.
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

    raise_panel(dock, Panel::Locations);
}

/// The name a question for the server is about: the row it is on, what it is called, and
/// which column of that row it starts at.
#[derive(Clone, PartialEq)]
pub(crate) struct NameAt {
    pub(crate) at: LinePos,
    pub(crate) name: String,
    pub(crate) column: u32,
}

/// Ask the server question `of` about `named`, hold the question, and bring the panel to
/// the front.
///
/// The question is the server's, so it is sent here rather than from the effect that
/// sends the worker's: what it is asked in is a server run, and there is nothing to ask
/// with no server -- a question is not what starts one, that being the control the reader
/// presses (`follow_name`'s rule).
pub(crate) fn find_listed(
    server: &Server,
    located: State<Located>,
    dock: State<DockArea>,
    named: NameAt,
    of: lsp::Listed,
) {
    let NameAt { at, name, column } = named;
    let asked = ask_where(
        server.language,
        &server.jobs,
        Lookup::at(&at, column),
        lsp::Question::Listed(of),
    );
    let Some((run, id)) = asked else {
        return;
    };
    // These answers are places in files: no row of one chooses a symbol for a tab.
    find_locations(
        located,
        dock,
        Query::listed(of, at, name, column, run, id),
        None,
    );
}

/// The three questions a server can be asked about `named`, as the rows a name's menu
/// begins with: where it is defined, where it is used, and what implements it.
///
/// "Go to definition" is a link's own door and lands in place. It is offered all the same,
/// because the menu is offered over a name where one is **defined** too -- no link there,
/// and where a reader asks what refers to it. The other two a click cannot ask at all.
///
/// Built per press, as [`locate_menu`] is: the states come in as arguments because a menu
/// handler may run no hook.
pub(crate) fn name_menu(
    server: &Server,
    located: State<Located>,
    dock: State<DockArea>,
    open: Open,
    named: NameAt,
) -> Vec<MenuButton> {
    let definition = {
        let (server, at, column) = (server.clone(), named.at.clone(), named.column);
        MenuButton::new()
            .on_press(move |_| {
                follow_name(
                    &server,
                    open,
                    Lookup::at(&at, column),
                    lsp::Followed::Definition,
                    Reach::InPlace,
                )
            })
            .child("Go to definition")
    };
    // The two list questions are one shape; only which one differs.
    let listed = |of| {
        let (server, named) = (server.clone(), named.clone());
        MenuButton::new().on_press(move |_| find_listed(&server, located, dock, named.clone(), of))
    };
    vec![
        definition,
        listed(lsp::Listed::References).child(format!("Find references to {}", named.name)),
        listed(lsp::Listed::Implementations).child("Find implementations"),
    ]
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
    format!("{}:{}", source::name_of(Path::new(&*at.file)), at.line)
}

/// The shell both of the panel's lists are drawn in: the question over the rows, with
/// `count` of them in it.
///
/// The count is said over the list rather than in the tab's title: the rows are not the
/// answer to anything until the question is in view with them.
fn headed(query: &Query, count: usize, list: Element) -> Element {
    rect()
        .expanded()
        .content(Content::Flex)
        .child(extra_tooltip(
            query.tooltip(),
            section_heading(&query.heading(count), None),
        ))
        .child(
            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .child(list),
        )
        .into()
}

/// The Locations view: what was asked about, over every symbol it answered with.
///
/// `HistoryPanel`'s shape with `SymbolsPanel`'s list: a filter over a `VirtualScrollView`,
/// ranked by the same [`Filtered`], because one line answers with thousands. What the
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
pub(crate) struct LocationsPanel;

impl Component for LocationsPanel {
    fn render(&self) -> impl IntoElement {
        let located = use_consume::<Locations>().0;
        let filter = use_state(Filter::default);
        let pane = use_list_pane(Panel::Locations);
        let filtered = use_memo(move || {
            let symbols = match &located.read().found {
                Some(Found {
                    what: What::Symbols(symbols),
                    ..
                }) => symbols.clone(),
                _ => Shared::default(),
            };
            Filtered::new(symbols, &filter.read().matcher(), |symbol| {
                symbol.data.display()
            })
        });
        let filtered = filtered.read().clone();
        // A references answer is tens of rows where a line's symbols are thousands, so the
        // filter is applied where the rows are built (`filter_bar.rs`) -- but through a
        // memo all the same, since the rows are compared by the pointer they are shared
        // under and a fresh one every render would redraw every row.
        let used = use_memo(move || match &located.read().found {
            Some(Found {
                what: What::Places(places),
                ..
            }) => places.rows(&filter.read().matcher()),
            _ => ReferenceRows::default(),
        });
        let used = used.read().clone();
        let selected = use_consume::<Analysis>()
            .0
            .read()
            .shown
            .as_ref()
            .map(|shown| shown.studied.symbol.clone());
        let state = located.read().clone();
        // What Enter on a row reaches through, and the two facts a location row opens
        // with: the line the question was asked from, and the tab it was asked in.
        let to = use_landings();
        // What Enter on a reference row reaches through beside those: a place in a file
        // is opened the way both grouped panels open one (`ui/place_row.rs`).
        let places = use_places();
        let asked_at = state.found.as_ref().map(|found| found.of.at.clone());
        let subject = state.subject.clone();

        // The filter compiled once for the rows to mark what it matched in them, beside
        // the memos above which compile one of their own to narrow the lists with.
        let marking = Marking::new(filter.read().matcher());

        let mut keys = ListKeys::none();
        let body: Element = match (&state.asked, state.pending(), &state.found) {
            // Asked and not pending is found, by `pending`'s definition, so the second
            // of these never comes up.
            (None, _, _) | (Some(_), None, None) => placeholder("Nothing looked for yet"),
            (Some(_), Some(query), _) => placeholder(format!(
                "Finding {} {}\u{2026}",
                query.words().1,
                query.spell()
            )),
            (
                Some(query),
                None,
                Some(Found {
                    what: What::Places(found),
                    ..
                }),
            ) if found.count() == 0 => {
                placeholder(format!("No {} {}", query.words().1, query.spell()))
            }
            (
                Some(query),
                None,
                Some(Found {
                    what: What::Places(found),
                    ..
                }),
            ) => {
                let count = found.count();
                let length = used.len();
                // The rows the arrows step and Enter presses: a `ReferenceRows` is the
                // rows behind an `Arc`, so this is a pointer each.
                let listed = used.clone();
                let stepped = listed.clone();
                keys = ListKeys {
                    length,
                    at: Box::new(move |at| stepped.get(at).map(place_pick)),
                    open: Box::new(move |at| match listed.get(at) {
                        Some(row) => {
                            press_place(to.doors, places, to.ctrl, Folding::Places(located), row)
                        }
                        None => Pressed::Folded,
                    }),
                };
                headed(
                    query,
                    count,
                    VirtualScrollView::new_with_data(
                        (used, located),
                        |row, (used, located): &(ReferenceRows, State<Located>)| {
                            PlaceRow {
                                row: used[row].clone(),
                                folding: Folding::Places(*located),
                                at: row,
                                key: DiffKey::None,
                            }
                            .into()
                        },
                    )
                    .length(length)
                    .item_size(list_row_height())
                    .scroll_controller(pane.controller)
                    .into(),
                )
            }
            (
                Some(query),
                None,
                Some(Found {
                    what: What::Symbols(symbols),
                    ..
                }),
            ) if symbols.is_empty() => {
                placeholder(format!("No code compiled from {}", query.spell()))
            }
            (
                Some(query),
                None,
                Some(Found {
                    what: What::Symbols(symbols),
                    ..
                }),
            ) => {
                let count = symbols.len();
                let length = filtered.len();
                // The rows the arrows step and Enter presses: a `Filtered` is the list
                // behind an `Arc` and the indices the filter kept.
                let rows = Rc::new(filtered.clone());
                let symbol_at = move |rows: &Filtered<Symbol>, at: usize| rows.at(at).cloned();
                let stepped = rows.clone();
                let (at_asked, subject) = (asked_at.clone(), subject.clone());
                keys = ListKeys {
                    length,
                    at: Box::new(move |at| symbol_at(&stepped, at).map(Pick::Symbol)),
                    open: Box::new(move |at| match symbol_at(&rows, at) {
                        Some(symbol) => {
                            press_location(to, at_asked.clone(), subject.clone(), symbol)
                        }
                        None => Pressed::Folded,
                    }),
                };
                headed(
                    query,
                    count,
                    VirtualScrollView::new_with_data(
                        (filtered, selected, marking),
                        |row,
                         (filtered, selected, marking): &(
                            Filtered<Symbol>,
                            Option<Symbol>,
                            Marking,
                        )| {
                            let index = filtered.index(row);
                            let symbol = &filtered.list()[index];
                            LocationRow {
                                symbols: filtered.list().clone(),
                                index,
                                selected: selected.as_ref() == Some(symbol),
                                at: row,
                                marks: marking.marks(symbol.data.display()),
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
                    .item_size(list_row_height())
                    .scroll_controller(pane.controller)
                    .into(),
                )
            }
        };

        pane.filtered(filter, keys, body)
    }
}

/// One symbol a line was compiled into: its name, and the object it is in after it,
/// since the same name in two objects is two rows and the object is what tells them
/// apart.
#[derive(Clone)]
struct LocationRow {
    symbols: Shared<Symbol>,
    /// Which symbol this is, in the list the filter narrowed.
    index: usize,
    /// Whether this is the symbol the panes are drawing.
    selected: bool,
    /// Where this row is in the list as it is drawn, which under a filter is not `index`.
    at: usize,
    /// Where the filter matched in the name, for the row to mark.
    marks: Vec<Range<usize>>,
    key: DiffKey,
}

impl PartialEq for LocationRow {
    fn eq(&self, other: &Self) -> bool {
        self.symbols == other.symbols
            && self.index == other.index
            && self.selected == other.selected
            && self.at == other.at
            && self.marks == other.marks
    }
}

impl KeyExt for LocationRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

/// Everything a location row's press reaches through: what a door is given, and the two
/// states beyond it a chosen symbol is written to. A struct because both the row and the
/// panel's Enter hand over the same set.
#[derive(Clone, Copy)]
struct Landings {
    doors: Doors,
    driven: State<Driven>,
    ctrl: State<bool>,
}

/// The three, consumed in the render as every context-consuming function must be.
fn use_landings() -> Landings {
    Landings {
        doors: use_doors(),
        driven: use_places().driven,
        ctrl: use_consume::<Ctrl>().0,
    }
}

/// What pressing a location row does: open the symbol, on the line the question was asked
/// from where there was one. Shared by the press and by Enter on the row the arrows left
/// the pick on.
///
/// `at` is the answer's own line, peeked when the row was built: a row is a row of one
/// answer and cannot outlive it. `subject` is the source-driven tab the question was asked
/// from, where it is still open and still on the file.
fn press_location(
    to: Landings,
    at: Option<LinePos>,
    subject: Option<(DocId, Arc<str>)>,
    symbol: Symbol,
) -> Pressed {
    let Landings {
        doors,
        driven,
        ctrl,
    } = to;
    let open = doors.open;
    let symbol_tab = Document::Assembly(Selection::Symbol(symbol.clone()));
    let Some(at) = at else {
        open_document(open, doors.visits, symbol_tab, Reach::outside(ctrl));
        return Pressed::Opened;
    };
    // Chosen for the tab the question was asked from. The choice is that entry's, and the
    // entry is driven from the line the question was asked from, so the tab's assembly
    // side becomes this symbol -- for an instance, provided the instance holds code from
    // that line, which `compiled::pick` falls back from where it does not. Bound to a
    // `let` so the table's guard is gone before `driven` is written.
    let subject = subject
        .filter(|(id, file)| open.docs.peek().get(*id) == Some(&Document::Source(file.clone())));
    match subject {
        Some((id, file)) => {
            // The place that tab is at, not the file: a drive written under a stop the
            // trail does not hold is a drive nothing reads. The guard is gone before
            // `driven` is written.
            let at_place = place_at(&open.docs.peek(), id, &Document::Source(file));
            let entry = (id, at_place);
            {
                let mut driven = driven;
                let mut driven = driven.write();
                driven.remember(entry.clone(), at.line);
                driven.choose(entry, symbol);
            }
            land_on(doors, id, at);
        }
        None => {
            // A line and no instruction: the row names a place in a file, and the
            // assembly pane's caret is the pair's.
            land(
                doors,
                Landing {
                    tab: symbol_tab,
                    at: Some(at),
                    address: None,
                    columns: None,
                },
                Reach::outside(ctrl),
            );
        }
    }
    Pressed::Opened
}

impl Component for LocationRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        // The two texts a row draws, each measured: the symbol's name and the object it
        // is in.
        let (named, about) = (use_fitted(), use_fitted());
        let to = use_landings();
        let located = use_consume::<Locations>().0.peek().clone();
        let at = located.found.as_ref().map(|found| found.of.at.clone());
        let subject = located.subject.clone();
        let picking = use_picking(Panel::Locations);
        let row = self.at;
        let symbol = self.symbols[self.index].clone();
        let pick = Pick::Symbol(symbol.clone());
        let name = symbol.data.display().to_owned();
        let object = symbol.object.name.clone();

        // One tooltip over two texts, so it is shown where either of them was cut.
        cut_tooltip(
            named.cut() || about.cut(),
            format!("{name} \u{2014} {object}"),
            list_row(hovering, picking.drawn(&pick, self.selected))
                .on_press(move |_| {
                    picking.press(pick.clone(), row, || {
                        press_location(to, at.clone(), subject.clone(), symbol.clone())
                    });
                })
                .child(tree_name_fitted(named, name, false, &self.marks))
                // Capped rather than measured, or a long member name would take the row
                // and leave the symbol it is about with nothing.
                .child(
                    rect()
                        .max_width(Size::percent(45.0))
                        .overflow(Overflow::Clip)
                        .child(one_line_fitted(about, object).color(palette().address_fg)),
                ),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

#[cfg(test)]
mod tests;
