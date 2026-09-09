//! The source half of a document, from the row up: one line of the file drawn with the
//! spans a parse resolved, the list of them, and the pane deciding which file that is.
//!
//! [`source_side`] is the one place either pane decides which file is up, so the pane and
//! the effect that drops its picked-out rows cannot disagree about which listing is being
//! shown. Only the symbol's **own** file is ever drawn, never the rest of
//! `LineInfo::files`. The rows are the app's own and not freya's `CodeEditor`, which paints
//! a background only for the cursor's row and keeps its scroll state private.
//!
//! Each row's gutter marks whether anything open has code from that line ([`Coded`]), so
//! a reader can tell what was compiled from what produced nothing without picking a line
//! out, and before they have picked one out at all.

use super::*;

/// What the source rows are built from: the file's text and highlighting, which file it is
/// -- a row picked out is a line of a file, and a line number is not a place on its own --
/// and which of its lines the assembly pane's picked-out run was compiled from.
///
/// Those are line numbers rather than positions because the file has already been
/// matched here rather than per visible row.
#[derive(Clone)]
struct SourceData {
    source: SourceText,
    file: Arc<str>,
    /// The lines of this file the assembly pane's run was compiled from: its pair here.
    pairs: Arc<HashSet<u32>>,
    /// The lines of this file the listing beside it has instructions for at all: what the
    /// gutter marks.
    compiled: Arc<HashSet<u32>>,
    /// The run picked out here -- the caret, the characters, and so the rows -- for each
    /// row to draw its part of, or `None` when there is none.
    chars: Option<CharSelection>,
    /// The tab these rows *drive*, for a source-driven tab, where a click also says which
    /// assembly the other side shows -- and `None` for the companion file beside a
    /// symbol, where the click picks the line out and no more.
    ///
    /// It travels here and through `new_with_data` rather than being captured by the
    /// builder closure, which is never compared across renders.
    drives: Option<DocId>,
    /// Which of this file's names the language server placed, and so which are links.
    /// Read once for the list and carried, never asked per row: what a server is doing
    /// changes with every word it says about its progress, and every mounted row would be
    /// drawn again for it.
    links: links::Links,
    /// What the find bar is looking for, compiled once for the list; `None` where no bar
    /// is open (`find_bar.rs`).
    marking: Option<Marking>,
}

impl PartialEq for SourceData {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.file, &other.file)
            // By contents: the set is rebuilt whenever a run in either pane changes, and
            // most of those leave it as it was.
            && self.pairs == other.pairs
            && self.compiled == other.compiled
            && self.chars == other.chars
            && self.drives == other.drives
            && self.links == other.links
            && self.marking == other.marking
    }
}

/// One line of a source file: its number in a gutter, then its text. `file` is carried to
/// be picked out rather than drawn: a line number without the file it is a line of is no
/// place for the assembly pane to light up.
#[derive(Clone)]
struct SourceRow {
    source: SourceText,
    file: Arc<str>,
    index: usize,
    /// Whether an instruction of the assembly pane's picked-out run was compiled from
    /// this line, and if so which of its edges end the run of such lines.
    paired: Option<Edges>,
    /// Whether the listing beside this pane has an instruction from this line: what the
    /// mark in the gutter says.
    compiled: bool,
    /// The wash of its pane's selection, told to it by the list for the reason
    /// `InstructionRow`'s is.
    wash: Wash,
    /// The columns of this row inside the pane's character selection, likewise.
    chars: RowChars,
    /// The tab a click here also drives the assembly side of, if any. See [`SourceData`].
    drives: Option<DocId>,
    /// Which of the file's names the server placed. See [`SourceData::links`].
    links: links::Links,
    /// What the find bar is looking for. See [`SourceData::marking`].
    marking: Option<Marking>,
    key: DiffKey,
}

impl PartialEq for SourceRow {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.file, &other.file)
            && self.index == other.index
            && self.paired == other.paired
            && self.compiled == other.compiled
            && self.wash == other.wash
            && self.chars == other.chars
            && self.drives == other.drives
            && self.links == other.links
            && self.marking == other.marking
    }
}

impl KeyExt for SourceRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

/// The pieces row `index` of `source` draws, each in its colour: the spans a parse
/// resolved, with leading indentation as spaces. What the row draws and what a character
/// selection copies, so a column into one is a column into the other.
///
/// In range because the list's length is the file's own `lines`, which is at most
/// `blocks.len()` -- and `SyntaxBlocks::get_line` unwraps rather than answering `None`,
/// so being in range is checked here.
fn source_pieces(source: &SourceText, index: usize) -> Vec<(Color, String)> {
    let source = &source.0;
    if index >= source.lines {
        return Vec::new();
    }
    source
        .blocks
        .get_line(index)
        .iter()
        .map(|(color, node)| {
            let text = match node {
                TextNode::Range(range) => source.rope.slice(range.clone()).to_string(),
                // Leading indentation, handed over as a length so an editor can draw it
                // as dots. Plain spaces here, this pane showing a file and not editing
                // one.
                TextNode::LineOfChars { len, .. } => " ".repeat(*len),
            };
            (*color, text)
        })
        .collect()
}

/// The text of `columns` on row `index`, or `None` where they name nothing of it.
///
/// The columns are the byte offsets a language server counts in (`src/links.rs`), counted
/// back into the units the row draws in and cut out of the row's own text -- which is
/// what a menu built from a press calls the name, so what the reader right-clicked is
/// what the menu says.
pub(crate) fn name_at(source: &SourceText, index: usize, columns: &Range<u32>) -> Option<String> {
    let line = source_line(source, index);
    let drawn = drawn_columns(&line.to_string(), columns);
    let name = line.slice(drawn.start, drawn.end);
    (!name.is_empty()).then_some(name)
}

/// A run of a name's byte columns as the UTF-16 units the row `text` is drawn in.
fn drawn_columns(text: &str, columns: &Range<u32>) -> Range<usize> {
    chars::columns_of(text, columns.start as usize..columns.end as usize)
}

/// A column of the row's `text` as the byte offset a language server is asked at.
fn byte_column(text: &str, column: usize) -> u32 {
    u32::try_from(chars::bytes_of(text, column..column).start).unwrap_or(u32::MAX)
}

/// The text row `index` draws, as the clipboard sees a character selection of it.
pub(crate) fn source_line(source: &SourceText, index: usize) -> Line {
    let mut line = Line::default();
    for (_, text) in source_pieces(source, index) {
        line.push_text(text);
    }
    line
}

/// What a press, a right-click or the pointer on one of a row's names is answered from:
/// where the row is, the text its columns are counted through, which of the file's names
/// the language server placed, and whom to ask about one.
///
/// Cloned once into each closure a row's names need, rather than the same four values
/// cloned one by one into every one. The rules those closures carry out are the methods
/// below, so a closure is the call and nothing else.
#[derive(Clone)]
struct Named {
    /// The position this row is, and so the one its questions are about. Lines are
    /// 1-based, as DWARF's are.
    at: LinePos,
    /// The row's text as it is drawn, which the columns here are counted through: a
    /// link's are byte offsets into the file's line (`src/links.rs`) and a row's are the
    /// UTF-16 units the text engine answers in. The drawn text and not the rope's, and
    /// the two agree wherever a column can land: what the row draws differently is the
    /// indentation, one space per character of it.
    text: Rc<str>,
    /// Which of the file's names the server placed. See [`SourceData::links`].
    links: links::Links,
    /// Whom a press on a link or a name asks, and [`None`] where there is nobody: a pane
    /// mounted without a server draws its text and no links (`links_in`).
    server: Option<Server>,
}

impl Named {
    /// A run of a name's byte columns as the units this row is drawn in.
    fn drawn(&self, columns: &Range<u32>) -> Range<usize> {
        drawn_columns(&self.text, columns)
    }

    /// A column of this row as the place a language server is asked about.
    fn lookup(&self, column: usize) -> Lookup {
        Lookup::at(&self.at, byte_column(&self.text, column))
    }

    /// The names in this row the server placed, and what a press on one does: ask it
    /// where that name is, and go to what it answers. [`None`] with nobody to ask, so no
    /// link is ever drawn that could not be followed.
    ///
    /// A press with Ctrl held opens what it names in a tab of its own, the rule every
    /// door inside a pane follows.
    fn linked(&self, open: Open, ctrl: State<bool>) -> Option<TextLinks> {
        let server = self.server.clone()?;
        let named = self.clone();
        Some(TextLinks {
            columns: self
                .links
                .followed_on(self.at.line)
                .map(|columns| self.drawn(columns))
                .collect(),
            // Always a door: nothing here is a link until the server has said the name is
            // one, so there is nothing to hold a modifier back for.
            is_link: Rc::new(|| true),
            follow: Rc::new(move |columns: Range<usize>| {
                let column = byte_column(&named.text, columns.start);
                follow_link(
                    &server,
                    &named.links,
                    open,
                    &named.at,
                    column,
                    Reach::inside(ctrl),
                );
            }),
        })
    }

    /// Every name the server placed on this row, links and the places where one is
    /// defined alike: what a reader hovers is a name and not a door, and a hover over the
    /// name where a function is defined is where its own signature and doc comment are.
    fn names(&self) -> Vec<Range<usize>> {
        self.links
            .on_line(self.at.line)
            .iter()
            .map(|link| self.drawn(&link.columns))
            .collect()
    }

    /// What the pointer moving onto one of those names, off them all, or out from under
    /// the row itself says: `waiting` as the move leaves it, and whether it moved at all,
    /// so the caller writes only then.
    fn pointed(&self, waiting: &mut Hover, under: Under) -> bool {
        match under {
            Under::Name(columns, drawn) => waiting.enter(Pointed {
                at: self.lookup(columns.start),
                drawn,
            }),
            Under::Off => waiting.left_name(),
            Under::Moved => waiting.gone(),
        }
    }

    /// The name at `column` of row `index`, which the three questions a menu offers are
    /// about: [`None`] over no name, and over one this row draws nothing of.
    fn at_column(&self, source: &SourceText, index: usize, column: usize) -> Option<NameAt> {
        let link = self
            .links
            .at(self.at.line, byte_column(&self.text, column))?;
        Some(NameAt {
            at: self.at.clone(),
            name: name_at(source, index, &link.columns)?,
            column: link.columns.start,
        })
    }
}

/// What the right button offers on row `index`: the three questions for the server where
/// the press was on a name, then the line's locations and, inside a function as the
/// file's parse says, the function's instances. A location found from the file a
/// source-driven tab is about is chosen for that tab; from a companion it opens the
/// symbol, and the menu offers the file itself as a tab of its own.
///
/// Both the name under the pointer and the function this row is a line of are looked for
/// on the press and not per render -- the function being a walk of the file's own -- since
/// a row is rendered far more often than it is right-clicked.
///
/// Every state is handed in, because reaching for a context is a hook and the handler runs
/// long after the render that built it.
#[allow(clippy::too_many_arguments)]
fn source_menu(
    doors: Doors,
    places: Places,
    located: State<Located>,
    dock: State<DockArea>,
    named: Named,
    source: SourceText,
    index: usize,
    drives: Option<DocId>,
    file: Arc<str>,
) -> Rc<dyn Fn(Event<PressEventData>, Option<usize>)> {
    let open = doors.open;
    // The file this row is in, where the pane is showing it beside somebody else's tab. A
    // subject is that tab already and has nothing to open.
    let opens = drives.is_none().then(|| file.clone());
    let subject = drives.map(|tab| (tab, file));
    // Whom to ask about a name, where this row's names are links at all. A row drawing
    // none is a row over no server, and a question nobody could answer is not offered.
    let asking = (!named.links.is_empty())
        .then(|| named.server.clone())
        .flatten();

    Rc::new(move |e: Event<PressEventData>, column| {
        let at = named.at.clone();
        let function = functions::enclosing(&source.0.functions, at.line).cloned();
        // The name the press was on, which the three questions are about.
        let name = column
            .and_then(|column| named.at_column(&source, index, column))
            .zip(asking.clone())
            .map(|(name, server)| name_menu(&server, located, dock, open, name))
            .unwrap_or_default();
        let menu = locate_menu(located, dock, at.clone(), subject.clone(), function, name);
        // The door into the file itself, which the tab has only beside it: the same
        // arrival every other door into a source file makes, so the assembly side follows
        // this line as it follows a clicked one.
        let menu = menu.maybe_child(opens.clone().map(|file| {
            let (line, name) = (at.line, source::name_of(Path::new(&*file)));
            MenuButton::new()
                .on_press(move |_| {
                    open_source_place(doors, places, Path::new(&*file), line, None, Reach::NewTab)
                })
                .child(format!("Open {name}"))
        }));
        ContextMenu::open_from_event(&e, menu);
    })
}

/// A press in the file a source-driven tab is about, which is what says which assembly
/// its other side shows.
///
/// **The only writer of `Driven` inside the panes.** A click in a companion file picks
/// the line out and no more, and a click in the assembly pane never comes here at all, so
/// there is no way for the listing to re-drive itself.
fn drive(docs: State<Docs>, mut driven: State<Driven>, tab: DocId, at: &LinePos) {
    // The place the tab is at and not the file: two lines of one file reached along one
    // trail are two entries, and a drive written under the wrong one is a drive nothing
    // reads. Bound before the write, the guard being live until the end of the statement.
    let entry = place_at(&docs.peek(), tab, &Document::Source(at.file.clone()));
    driven.write().remember((tab, entry), at.line);
}

impl Component for SourceRow {
    fn render(&self) -> impl IntoElement {
        let places = use_places();
        // Consumed here, in the render, because the menu handler may not run a hook.
        let located = use_consume::<Locations>().0;
        // Which tab a press on a link is made in: where its answer opens, and the two
        // halves of the landing a companion's door leaves.
        let doors = use_doors();
        let docs = doors.open.docs;
        let ctrl = use_consume::<Ctrl>().0;
        let server = try_use_server();
        // Where the name under the pointer is written, and `None` where there is nobody
        // to write it: a pane mounted without it draws its text and says nothing about a
        // name.
        let hover = try_consume_context::<Hovering>().map(|hovering| hovering.0);
        let dock = use_consume::<SidebarDock>().0;
        let index = self.index;

        let pieces = source_pieces(&self.source, index);
        let line = {
            let mut line = Line::default();
            for (_, text) in &pieces {
                line.push_text(text.clone());
            }
            line
        };
        let named = Named {
            at: LinePos {
                file: self.file.clone(),
                line: index as u32 + 1,
            },
            text: pieces
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<String>()
                .into(),
            links: self.links.clone(),
            server,
        };

        let text = Text {
            finds: self
                .marking
                .as_ref()
                .map(|marking| marking.hits(&line))
                .unwrap_or_default(),
            line,
            head: pieces
                .into_iter()
                .map(|(color, text)| Span::new(text).color(color).assembly_font())
                .collect(),
            tail: Vec::new(),
            chars: self.chars,
            links: named.linked(doors.open, ctrl),
            names: named.names(),
            // What the pointer on one of them says. Consumed in the render, as everything
            // a handler here reaches for is: a handler may not run a hook.
            on_hover: hover.map(|hover| {
                let named = named.clone();
                Rc::new(move |under: Under| {
                    let mut hover = hover;
                    let mut waiting = hover.peek().clone();
                    if named.pointed(&mut waiting, under) {
                        hover.set(waiting);
                    }
                }) as Rc<dyn Fn(Under)>
            }),
        };

        let menu = source_menu(
            doors,
            places,
            located,
            dock,
            named.clone(),
            self.source.clone(),
            index,
            self.drives,
            self.file.clone(),
        );

        // The line number, which is gutter: a press on it picks the row out and no
        // characters. A fixed width and not a minimum: skia lays a paragraph out to the
        // width it is given and aligns within *that*, so a label free to be wider puts
        // its number at the far right of the row, on top of the text. The gap is
        // non-breaking because skia trims trailing whitespace when it measures.
        let number = label()
            .text(format!("{}\u{a0}", self.index + 1))
            .width(Size::px(60.0))
            .text_align(TextAlign::Right)
            .color(palette().address_fg)
            .max_lines(1)
            .into_element();

        let mark = code_mark(self.compiled);

        // The same gesture as the assembly pane's, from the same chrome. The run is a
        // run of this file.
        code_row(
            Chrome {
                pane: Pane::Source,
                row: index,
                file: Some(self.file.clone()),
                paired: self.paired,
                wash: self.wash,
                measured: true,
            },
            vec![mark, number],
            Some(text),
            Some(menu),
        )
        // A press in a source-driven tab's own file also says which listing the
        // other side shows; the row is picked out by `pointer_down` either way.
        .maybe(self.drives.is_some(), |el| {
            let (drives, at) = (self.drives, named.at.clone());
            el.on_press(move |_| {
                if let Some(tab) = drives {
                    drive(docs, places.driven, tab, &at);
                }
            })
        })
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The source rows themselves, split out of the pane because that has several early
/// returns before it knows which file it is showing, and a hook has to run on every render.
#[derive(Clone)]
struct SourceList {
    source: SourceText,
    file: Arc<str>,
    /// The tab these rows are in.
    tab: DocId,
    /// The place on that tab's trail these rows belong to, which with the tab is what the
    /// viewing position is kept under and is **not** the same as the file being shown:
    /// two functions compiled from one file are two places, and keying by the file would
    /// have them share a position.
    document: Document,
    /// The row this tab opens at the first time it is shown, from [`opening_row`], and
    /// [`None`] for a tab with nothing better to open at than the top. The row itself and
    /// never one backed off towards the top: the rows kept above it are the pane's to
    /// add. A row remembered for the tab wins over it -- see `use_kept_position`.
    opening: Option<usize>,
}

impl PartialEq for SourceList {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.file, &other.file)
            && self.tab == other.tab
            && self.document == other.document
            && self.opening == other.opening
    }
}

/// The row of `file` the reveal `owing` asks for goes to, and [`None`] where the pane
/// showing that file cannot answer it -- which leaves the request owed rather than spent
/// on a guess.
///
/// A run of the pane's own is a run of the file it is showing, so the run's first row is
/// the answer. The other pane's run is answered by the line its first placed instruction
/// came from, which `places` reads off the listing: nothing to scroll to when that is a
/// file this pane is not showing -- an inlined header's line 42 is not line 42 of the
/// file on screen -- nor when the line is past the end of a file that has moved on since
/// it was compiled.
fn owed_row(
    owing: &Owing,
    file: &Arc<str>,
    length: usize,
    places: impl FnOnce(&Picked) -> Vec<LinePos>,
) -> Option<usize> {
    let index = match owing {
        Owing::Own(rows) => *rows.start(),
        Owing::Pair(pair) => {
            let line = places(pair).into_iter().find(|at| at.file == *file)?.line;
            (line as usize).checked_sub(1)?
        }
    };
    (index < length).then_some(index)
}

/// The row of `file` a landing names for the pane drawing `document`, and [`None`] where
/// it names another place, another file, or a line the file does not have.
fn landing_row(
    asked: &Landing,
    document: &Document,
    file: &Arc<str>,
    length: usize,
) -> Option<usize> {
    if asked.tab != *document {
        return None;
    }
    let at = asked.at.as_ref().filter(|at| at.file == *file)?;
    (at.line as usize)
        .checked_sub(1)
        .filter(|index| *index < length)
}

impl Component for SourceList {
    fn render(&self) -> impl IntoElement {
        let marked = use_consume::<Marked>().0;
        let chars = chars_of(marked, Pane::Source);
        // The assembly pane's run, and the lines of this file it was compiled from.
        let pair = pair_of(marked, Pane::Source);
        let analysis = use_consume::<Analysis>().0;
        let code_rows = use_consume::<CodeRows>().0;
        let pairs = Arc::new(paired_lines(
            &self.document,
            &self.file,
            pair.as_ref(),
            &analysis.read(),
            code_rows.read().as_deref(),
        ));
        // The gutter's marks: which lines of this file produced code at all, whether
        // anything is picked out and whether or not a listing is up. Asking is the pane's
        // ([`asks_for`]); this reads whatever has been answered.
        let coded = use_consume::<Coding>().0;
        let compiled = coded
            .read()
            .lines_in(&self.file)
            .cloned()
            .unwrap_or_default();
        // The listing these rows are of, which is the highlighted file: what its widest
        // row and its kept position are held under.
        let listing = Widest::key(Arc::as_ptr(&self.source.0).addr());
        // The box the rows are drawn in, and the scroll and the measurement that come
        // with it.
        let list = use_list_box(Pane::Source, listing);
        // What the find bar over this pane is looking for, for every row to wash, and
        // what it searches, claimed for as long as these rows are drawn.
        let at = (Placing::Tab(self.tab), Pane::Source);
        let marking = use_marking(at);
        use_searching(at, Searchable::Source(self.source.clone()));
        let (controller, viewport) = (list.controller, list.viewport);

        // Which of this file's names are links, which is the server's to say and not the
        // pane's to guess. Nothing until it has said so -- so no link is ever drawn that
        // could not be followed, where a pane that lit them as soon as a server *started*
        // drew them through the minute it spends reading the project. Asked for by the
        // pane ([`asks_for`]) and read here through `try_consume_context`, a pane mounted
        // without one having no links; the answer is an `Arc` inside, so carrying it to
        // the rows is a pointer compare.
        let links = try_consume_context::<Linking>()
            .and_then(|held| held.0.read().links_in(&self.file).cloned())
            .unwrap_or_default();

        let length = self.source.0.lines;
        // The tab's entry and not the file: see `SourceList::document`.
        let docs = use_open().docs;
        // The place the tab is at: two lines of one file reached along one trail are two
        // entries, each with its own scroll. Read and not peeked, so a step between them
        // re-renders this pane and the hook sees the switch.
        let entry = (self.tab, place_at(&docs.read(), self.tab, &self.document));
        use_kept_position(
            use_places().src_at,
            docs,
            {
                let file = self.file.clone();
                let document = self.document.clone();
                move |controller: &mut ScrollController| {
                    // Asked before anything else: `owed_reveal` reads the marks, and that
                    // read is what wakes this on the next click.
                    let Some(owing) = owed_reveal(marked, Pane::Source) else {
                        return false;
                    };
                    let owed = owed_row(&owing, &file, length, |pair| {
                        places_of(
                            &document,
                            pair,
                            &analysis.peek(),
                            code_rows.peek().as_deref(),
                        )
                    });
                    let Some(index) = owed else {
                        return false;
                    };
                    if !reveal_row(controller, *viewport.read(), length, index) {
                        return false;
                    }
                    reveal_made(marked, Pane::Source);
                    true
                }
            },
            {
                // The landing a door has left and `use_land` has yet to turn into this
                // pane's run: taken here when it names a line of the file on screen, so
                // the pane draws the arriving document on that line rather than at the
                // offset the outgoing place left. Nothing is marked -- the run is
                // `use_land`'s to plant -- and the reveal it plants finds the row here.
                let file = self.file.clone();
                let document = self.document.clone();
                move |asked: &Landing, controller: &mut ScrollController| {
                    let Some(index) = landing_row(asked, &document, &file, length) else {
                        return false;
                    };
                    // Answered only where the pane could go there: a landing is gone
                    // to once, so one taken by a pane with no measurement yet would be
                    // remembered as answered and never made good.
                    reveal_row(controller, *viewport.read(), length, index)
                }
            },
            controller,
            viewport,
            &entry,
            length,
            listing,
            self.opening,
        );

        // The step the bar asked for, made here: the hits are rows, and only the list
        // knows how far to scroll to reach one.
        {
            let mut controller = controller;
            use_find_steps(at, marked, Some(self.file.clone()), move |row| {
                reveal_caret(
                    &mut controller,
                    *viewport.peek(),
                    code_row_height(),
                    length,
                    row,
                )
            });
        }

        let on_key_down = {
            let source = self.source.clone();
            let drawn = self.source.clone();
            let seeded = self.source.clone();
            let mut controller = controller;
            find_chord(
                at,
                marked,
                Searchable::Source(self.source.clone()),
                // What Ctrl+F seeds the box with is what a copy would take: the columns
                // are columns of the line as drawn.
                move |index| source_line(&seeded, index),
                on_listing_key(
                    marked,
                    Pane::Source,
                    // Every run of this pane is a run of the file it is showing.
                    Some(self.file.clone()),
                    length,
                    viewport,
                    move |index| {
                        // The file's own text and not the row's spans: what is pasted is the
                        // line as it is on disk, tabs and all. The newline is the join's
                        // business.
                        source
                            .0
                            .rope
                            .get_line(index)
                            .map(|line| {
                                let line = line.to_string();
                                line.trim_end_matches(|c| c == '\n' || c == '\r').to_owned()
                            })
                            .unwrap_or_default()
                    },
                    // The characters are columns of the line as drawn, so that is what they
                    // copy: an indentation as the spaces the row draws it as.
                    move |index| source_line(&drawn, index),
                    // The caret's row, brought on screen after a key has moved it.
                    move |index| {
                        reveal_caret(
                            &mut controller,
                            *viewport.peek(),
                            code_row_height(),
                            length,
                            index,
                        )
                    },
                ),
            )
        };

        rect()
            .width(Size::fill())
            .height(Size::flex(1.0))
            .padding(5.0)
            .child(list.render(
                marked,
                length,
                on_key_down,
                SourceData {
                    source: self.source.clone(),
                    file: self.file.clone(),
                    pairs,
                    compiled,
                    chars,
                    // A source-driven tab's subject is the file its own document names;
                    // a companion's tab is a symbol's.
                    drives: (self.document.driven_from() == Pane::Source).then_some(self.tab),
                    links,
                    marking,
                },
                |i, data: &SourceData| {
                    let paired_at = |row: usize| data.pairs.contains(&(row as u32 + 1));
                    SourceRow {
                        source: data.source.clone(),
                        file: data.file.clone(),
                        index: i,
                        paired: paired_at(i).then(|| Edges::of(i, paired_at)),
                        compiled: data.compiled.contains(&(i as u32 + 1)),
                        wash: wash_of(data.chars, i),
                        chars: RowChars::of(data.chars, i),
                        drives: data.drives,
                        links: data.links.clone(),
                        marking: data.marking.clone(),
                        key: DiffKey::None,
                    }
                    .key(i)
                    .into()
                },
            ))
    }
}

/// Which file the Source pane is drawing, and whose side of the tab it is: a **subject** is
/// a source-driven tab's own file, a **companion** the file the drawn symbol, or the
/// pressed instruction, was compiled from.
///
/// The companion comes out of the *analysis* and not out of `Active`, because the two
/// disagree for as long as the worker takes and it is the analysis that says which symbol
/// is actually drawn. A subject opens at the top of its file and is keyed under the file
/// itself, so it says the file and no more.
pub(crate) enum SourceSide {
    Subject(Arc<str>),
    Companion {
        file: Arc<str>,
        /// The place the rows are kept under: the drawn symbol's tab, or the object's
        /// code. **Not the file**: two functions compiled from one file are two places,
        /// and keying by the file would have them share a viewing position.
        document: Document,
        /// The line the pane opens at the first time it shows this tab, which is the
        /// symbol's own line or the pressed instruction's.
        ///
        /// [`None`] where there is nothing better to say than the top of the file: an
        /// object with no line info, a prologue DWARF places on no line, an instruction
        /// row that is still a guess, and a companion that is not the symbol's own file
        /// -- the last being a landing's doing, which comes with a reveal of its own and
        /// would otherwise be sent to a line of the wrong file.
        line: Option<u32>,
    },
}

impl SourceSide {
    pub(crate) fn file(&self) -> &Arc<str> {
        match self {
            SourceSide::Subject(file) | SourceSide::Companion { file, .. } => file,
        }
    }
}

/// The whole of what the Source pane draws for `active`: which file, which place its rows
/// are kept under, and which line it opens at.
///
/// The companion is the symbol's own file -- the one its first instruction was compiled
/// from -- except when the source pane's picked-out run is in another file the listing's
/// line info knows: a row in the Locations panel opens a symbol on a line of the file the
/// line is in, and a symbol whose prologue was inlined from elsewhere would otherwise open
/// on that elsewhere, with the line the reader asked for in a file that is not up. A run
/// picked out inside the pane is in the file already shown, so a click there changes no
/// file, and a click on an inlined instruction never picks anything out on this side at
/// all.
///
/// `code_rows` are an object's code as the section view has counted it. Only a code tab's
/// line is read out of them, so a caller wanting the file alone passes [`None`].
pub(crate) fn source_side(
    active: Option<&Document>,
    analysis: &Analyzed,
    marks: &Marks,
    code_rows: Option<&Built>,
) -> Option<SourceSide> {
    match active? {
        Document::Source(file) => Some(SourceSide::Subject(file.clone())),
        // An object's code draws no symbol of its own, so its companion is the file of
        // whatever the reader picked out in it -- an instruction row's run is a run of
        // the file the pressed row was compiled from -- and nothing until they have. The
        // tab opens on that row's line, read off the same run.
        document @ Document::Code(_) => {
            let picked = marks.assembly.as_ref()?;
            let anchor = picked.chars.anchor().row;
            Some(SourceSide::Companion {
                file: picked.file.clone()?,
                document: document.clone(),
                line: code_places(code_rows, anchor..=anchor)
                    .into_iter()
                    .next()
                    .map(|at| at.line),
            })
        }
        Document::Assembly(_) => {
            let shown = analysis.shown.as_ref()?;
            let lines = &shown.studied.lines;
            let picked = marks
                .source
                .as_ref()
                .and_then(|picked| picked.file.as_ref())
                .filter(|file| {
                    lines
                        .info
                        .as_ref()
                        .is_some_and(|info| info.files().iter().any(|named| named == *file))
                });
            let file = picked.cloned().or_else(|| lines.file.clone())?;
            // The symbol's line only where the file it is a line of is the one drawn.
            let line = lines.line.filter(|_| lines.file.as_ref() == Some(&file));
            Some(SourceSide::Companion {
                file,
                // The *drawn* symbol's tab and not the active one: a row written down
                // against the tab that is arriving would be a row of the listing that is
                // leaving.
                document: asked_of(&shown.ask),
                line,
            })
        }
    }
}

/// The positions the assembly pane's picked-out run `pair` was compiled from, for the
/// listing the pane draws beside `document`: the object's code for a code tab, read
/// through the reading's rows, and the drawn symbol's listing otherwise.
fn places_of(
    document: &Document,
    pair: &Picked,
    analysis: &Analyzed,
    built: Option<&Built>,
) -> Vec<LinePos> {
    match document {
        Document::Code(_) => code_places(built, pair.chars.rows()),
        _ => analysis
            .shown
            .as_ref()
            .map(|shown| shown.studied.places(pair.chars.rows(), 0))
            .unwrap_or_default(),
    }
}

/// The lines of `file` the assembly pane's run `pair` was compiled from: what the source
/// rows light as its pair. Bounded by the run and not by the file.
fn paired_lines(
    document: &Document,
    file: &Arc<str>,
    pair: Option<&Picked>,
    analysis: &Analyzed,
    built: Option<&Built>,
) -> HashSet<u32> {
    let Some(pair) = pair else {
        return HashSet::new();
    };
    places_of(document, pair, analysis, built)
        .into_iter()
        .filter(|at| at.file == *file)
        .map(|at| at.line)
        .collect()
}

/// Write `file` into the `wanted` of one of the three states a pane asks through, and
/// only when it is not what is already being asked for: a pane redrawn for any of the
/// dozen other reasons must not wake a worker.
///
/// `None` is written too, so that a pane which has stopped drawing a file stops asking
/// about one.
fn asks_for<T: Clone + PartialEq + 'static>(
    mut state: State<T>,
    wanted: impl Fn(&mut T) -> &mut Option<Arc<str>>,
    file: &Option<Arc<str>>,
) {
    let mut next = state.peek().clone();
    if wanted(&mut next) == file {
        return;
    }
    *wanted(&mut next) = file.clone();
    state.set(next);
}

/// The gutter's marks, shared through context: the Source pane writes the file it is
/// showing and the analysis worker writes the lines back.
#[derive(Clone, Copy)]
pub(crate) struct Coding(pub(crate) State<Coded>);

/// The lines of one source file the open objects have code for: what the gutter marks,
/// and the answer to a [`Question::Marks`] the pane asks by writing the file it is
/// showing into [`Coded::wanted`].
///
/// **Every line that produced code, not the drawn symbol's own.** A source-driven tab has
/// no drawn symbol until a line is clicked, so a mark bounded by one would be a gutter
/// that stayed bare until the reader guessed where to click -- which is the thing the
/// mark exists to save them. So it says the file's own fact: this line produced code, in
/// something. Which symbol is the pair's question and the Locations panel's.
///
/// There is no `pending` field, for [`Located`]'s reason: a file is being looked for
/// exactly while it is wanted and the answer is not about it.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Coded {
    /// The file the Source pane is showing, written by it. One file, because one is
    /// drawn; a pane that moves to another asks again.
    pub(crate) wanted: Option<Arc<str>>,
    /// The file the lines below are of, and the lines. An empty set is an answer.
    pub(crate) found: Option<(Arc<str>, Arc<HashSet<u32>>)>,
    /// The objects the answer was worked out over, by pointer, which is what identity is
    /// here. Held as addresses and not as `Arc`s: a set of line numbers has nothing in it
    /// to sweep for a binary that has since closed, so the way this stays true is to be
    /// asked again when what is open changes -- and a state keeping the objects alive to
    /// notice that would be the state stopping them from closing.
    pub(crate) over: Vec<usize>,
}

/// The objects `open` are, by pointer, in their own order.
pub(crate) fn object_ids(open: &[Arc<Object>]) -> Vec<usize> {
    open.iter()
        .map(|object| Arc::as_ptr(object).addr())
        .collect()
}

impl Coded {
    /// The file a question is owed for: one is wanted, and the answer is about another
    /// file or was worked out over other objects than `open`.
    pub(crate) fn pending(&self, open: &[Arc<Object>]) -> Option<&Arc<str>> {
        let wanted = self.wanted.as_ref()?;
        match &self.found {
            Some((file, _)) if file == wanted && self.over == object_ids(open) => None,
            _ => Some(wanted),
        }
    }

    /// Take `lines` as the answer about `file`, worked out `over` those objects. Whether
    /// anything changed, so the caller writes only then ([`write_if`]).
    ///
    /// The locate's rule, against the file the pane is showing *now*: a reader who moved
    /// on while the index built is not given the file they left. There is no per-object
    /// sweep, the answer being lines and not symbols -- what keeps it true as binaries
    /// come and go is `over` and the effect that reads it.
    pub(crate) fn take(
        &mut self,
        file: Arc<str>,
        lines: Arc<HashSet<u32>>,
        over: Vec<usize>,
    ) -> bool {
        if self.wanted.as_ref() != Some(&file) {
            return false;
        }
        self.found = Some((file, lines));
        self.over = over;
        true
    }

    /// The lines of `file` that have code, and nothing where the answer is about another
    /// file -- which is what a pane draws in the beat between moving and being answered.
    fn lines_in(&self, file: &str) -> Option<&Arc<HashSet<u32>>> {
        match &self.found {
            Some((of, lines)) if &**of == file => Some(lines),
            _ => None,
        }
    }
}

/// The row the Source pane opens a tab it has never shown at: the line
/// [`SourceSide::Companion`] named, as a row, which is what selecting a symbol or pressing
/// an instruction asked to see. The row itself, the margin [`reveal_row`] keeps above the
/// row it scrolls to being the reveal's to add.
///
/// **The top of the file where the side named no line**, which is what selecting a symbol
/// used to do in every case.
fn opening_row(line: Option<u32>) -> Option<usize> {
    (line? as usize).checked_sub(1)
}

/// What the Source pane says over a file whose bytes are not the ones the debug info's
/// checksum was taken of: the file is shown, since it is still the best thing to show, but
/// its line numbers are the compiler's and not necessarily this file's.
pub(crate) const STALE_SOURCE: &str = "This file differs from the one the binary was built from";

/// The bar over the Source pane, naming the file the pane is showing and carrying the
/// control that puts the pane beside it away.
///
/// **A companion's name is a door**: pressing it opens that file as a source-driven tab,
/// as pressing a source file's row in the Files view does, and until the source search
/// lands those are the two ways into one. A **subject** is that tab already, so its name
/// is a name and nothing to press. Either way the bar says which file is up, which the
/// tab's own chip only has room for the last part of.
///
/// The states come in as arguments because this is a function and not a component: a hook
/// written here would be the pane's own.
fn source_bar(
    side: &SourceSide,
    tab: DocId,
    open: Open,
    visits: State<Visits>,
    ctrl: State<bool>,
    sweeping: bool,
) -> Element {
    let file = side.file().clone();
    let opens = matches!(side, SourceSide::Companion { .. });

    rect()
        .width(Size::fill())
        .horizontal()
        // The name takes what the toggle leaves, which torin only works out for a `flex`
        // child of a `Content::Flex` parent.
        .content(Content::Flex)
        .padding(Gaps::new_symmetric(0.0, 8.0))
        .background(palette().header_bg)
        .border(bottom_hairline())
        .child(
            // A box of its own and not the name as the `flex` child directly: a flex child
            // is measured from its content first, so a label placed there takes the width
            // of the whole path and the ellipsis never happens.
            rect()
                .width(Size::flex(1.0))
                .overflow(Overflow::Clip)
                // Not hit while a sweep is under way: the pointer dragging a selection up
                // past the bar would otherwise arm its tooltip, and light it.
                .interactive(!sweeping)
                .child(extra_tooltip(
                    file.to_string(),
                    rect()
                        .horizontal()
                        .cross_align(Alignment::Center)
                        .width(Size::fill())
                        .height(Size::px(list_row_height()))
                        .spacing(6.0)
                        .maybe(opens, |bar| {
                            let document = Document::Source(file.clone());
                            bar.on_press(move |_| {
                                open_document(open, visits, document.clone(), Reach::inside(ctrl));
                            })
                        })
                        .child(entry_icon(&Document::Source(file.clone())))
                        .child(
                            label()
                                .text(source::name_of(Path::new(&*file)))
                                .width(Size::fill())
                                .max_lines(1)
                                .text_overflow(TextOverflow::Ellipsis),
                        ),
                )),
        )
        // Only where this side leads, and outside the name rather than inside it, so a
        // press on it is a press on the toggle and never a door into the file.
        .maybe(!opens, |bar| {
            bar.child(PaneToggle {
                of: Placing::Tab(tab),
            })
        })
        .into_element()
}

/// The Source pane: the tab's source side, whichever of the two sides that is.
#[derive(Clone)]
pub(crate) struct SourcePane {
    /// The tab this pane is in, for the positions its rows keep.
    pub(crate) tab: DocId,
    pub(crate) document: Document,
}

impl PartialEq for SourcePane {
    fn eq(&self, other: &Self) -> bool {
        self.tab == other.tab && self.document == other.document
    }
}

impl Component for SourcePane {
    fn render(&self) -> impl IntoElement {
        // Whether a sweep is under way, for the header not to answer the pointer during one.
        let sweeping = sweeping(use_consume::<Marked>().0);
        let doors = use_doors();
        let (open, visits) = (doors.open, doors.visits);
        let ctrl = use_consume::<Ctrl>().0;
        // Reading it is what subscribes this tab to the analysis, so the pane fills in when
        // a newly selected symbol's line info is worked out.
        let analysis = use_consume::<Analysis>().0.read().clone();
        // The tab's own document and not `Active`, which is a memo and a beat behind: this
        // pane is only ever mounted for the tab it belongs to.
        let marks = use_consume::<Marked>().0.read().clone();
        let code_rows = use_consume::<CodeRows>().0;
        // Peeked and not read: the line a code tab opens at is read out of the rows, and
        // a window of them decoding must not draw the pane again. In a scope of its own,
        // so the guard is let go before anything below writes.
        let side = {
            let built = code_rows.peek();
            source_side(Some(&self.document), &analysis, &marks, built.as_deref())
        };

        // **The three questions about the file this pane is showing, asked together.**
        // Its text, the lines of it anything open has code from, and which of its names
        // the language server calls links: none is the other's to wait for, and they are
        // answered by the reader, the analysis worker and the server, which are three
        // threads. Asked here rather than by the rows, which are drawn out of the first
        // answer and so could ask for the other two only once it had landed; and
        // **before the early return below**, a hook having to run on every render.
        let sourced = use_consume::<Sourcing>().0;
        let coded = use_consume::<Coding>().0;
        let linking = try_consume_context::<Linking>().map(|linking| linking.0);
        let showing = side.as_ref().map(|side| side.file().clone());
        use_side_effect_with_deps(&showing, move |file: &Option<Arc<str>>| {
            asks_for(sourced, |sourced| &mut sourced.wanted, file);
            asks_for(coded, |coded| &mut coded.wanted, file);
            // Unconditional, as every hook is: a pane mounted with no server context
            // writes nothing, inside the closure and not around it.
            if let Some(linking) = linking {
                asks_for(linking, |linked| &mut linked.wanted, file);
            }
        });

        let Some(side) = side else {
            // The same answer the assembly pane gives, from the same place, plus one case
            // of its own: a symbol can be analysed and still name no file.
            return match analysis.showing(&self.document) {
                Showing::Message(text) => placeholder(text),
                Showing::Nothing => blank_pane(palette().pane_bg),
                Showing::Listing(shown) if shown.studied.lines.info.is_some() => {
                    placeholder("No source file for this symbol")
                }
                Showing::Listing(_) => placeholder("No line info"),
            };
        };

        let file = side.file().clone();
        // The tab, and the row it opens at the first time it is shown. A source-driven
        // tab is a *file* the reader opened, so it opens where a file does, at the top;
        // a companion opens on the line [`source_side`] named, which is the symbol's own
        // or the pressed instruction's.
        let (document, opening) = match &side {
            SourceSide::Subject(file) => (Document::Source(file.clone()), None),
            SourceSide::Companion { document, line, .. } => (document.clone(), opening_row(*line)),
        };

        // The file itself, out of what the reader has answered -- and nothing until it
        // has, which is what keeps the read off this thread.
        let drawing = sourced.read().drawing(Path::new(&*file));
        let text = match &drawing {
            Drawing::Text(text) => Some(text.0.clone()),
            Drawing::Missing | Drawing::Waiting => None,
        };

        // Whether the file on disk is the one the binary was built from, by the checksum
        // the debug info recorded for it — where it recorded one, and where the file
        // opened at all. Compared against the *drawn* symbol's line info, for a subject
        // and a companion alike: it is the one place a recorded checksum comes from. The
        // bytes are the ones the parse was made from, so nothing here reads a file to
        // find out.
        let stale = analysis
            .shown
            .as_ref()
            .and_then(|shown| shown.studied.lines.hash_for(&file))
            .zip(text)
            .is_some_and(|(recorded, opened)| !opened.file.matches(recorded));

        rect()
            .expanded()
            // The header takes its own height and the list is given the rest, which torin
            // only works out for a `flex` child of a `Content::Flex` parent.
            .content(Content::Flex)
            .background(palette().pane_bg)
            .child(source_bar(&side, self.tab, open, visits, ctrl, sweeping))
            .maybe_child(stale.then(|| stale_banner(STALE_SOURCE)))
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    // The path is named in the message because it is the only clue to
                    // *why*: built elsewhere, moved and deleted all look alike from here.
                    .child(match drawing {
                        Drawing::Text(source) => SourceList {
                            source,
                            file,
                            tab: self.tab,
                            document,
                            opening,
                        }
                        .into_element(),
                        Drawing::Missing => placeholder(format!("Source file not found: {file}")),
                        Drawing::Waiting => rect().expanded().into(),
                    }),
            )
            // Last, so the rows above are given what is left: the code makes room for the
            // bar rather than being covered by it.
            .maybe_child(find_bar_over((Placing::Tab(self.tab), Pane::Source)))
            .into()
    }
}

#[cfg(test)]
mod tests;
