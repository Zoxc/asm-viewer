//! The source half of a document, from the row up: one line of the file drawn with the
//! spans a parse resolved, the list of them, and the pane deciding which file that is.
//!
//! [`source_side`] is the one place either pane decides which file is up, so the pane and
//! the effect that drops its picked-out rows cannot disagree about which listing is being
//! shown. Only the symbol's **own** file is ever drawn, never the rest of
//! `LineInfo::files`. The rows are the app's own and not freya's `CodeEditor`, which paints
//! a background only for the cursor's row and keeps its scroll state private.

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
    /// The run of rows picked out here, or `None` when there is none.
    rows: Option<RowSelection>,
    /// The characters picked out here, for each row to draw its part of.
    chars: Option<CharSelection>,
    /// The tab these rows *drive*, for a source-driven tab, where a click also says which
    /// assembly the other side shows -- and `None` for the companion file beside a
    /// symbol, where the click picks the line out and no more.
    ///
    /// It travels here and through `new_with_data` rather than being captured by the
    /// builder closure, which is never compared across renders.
    drives: Option<DocId>,
    /// The widest row drawn, under the highlighted file's identity, and that key: what
    /// every row is at least as wide as. Handles, so out of the `PartialEq` below.
    widest: Widest,
    listing: u64,
}

impl PartialEq for SourceData {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.file, &other.file)
            // By contents: the set is rebuilt whenever a run in either pane changes, and
            // most of those leave it as it was.
            && self.pairs == other.pairs
            && self.rows == other.rows
            && self.chars == other.chars
            && self.drives == other.drives
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
    /// The wash of its pane's selection, told to it by the list for the reason
    /// `InstructionRow`'s is.
    wash: Wash,
    /// The columns of this row inside the pane's character selection, likewise.
    chars: RowChars,
    /// The tab a click here also drives the assembly side of, if any. See [`SourceData`].
    drives: Option<DocId>,
    /// The listing's widest row and its key, as an `InstructionRow` carries them.
    widest: Widest,
    listing: u64,
    key: DiffKey,
}

impl PartialEq for SourceRow {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.file, &other.file)
            && self.index == other.index
            && self.paired == other.paired
            && self.wash == other.wash
            && self.chars == other.chars
            && self.drives == other.drives
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

/// The text row `index` draws, as the clipboard sees a character selection of it.
pub(crate) fn source_line(source: &SourceText, index: usize) -> Line {
    let mut line = Line::default();
    for (_, text) in source_pieces(source, index) {
        line.push_text(text);
    }
    line
}

impl Component for SourceRow {
    fn render(&self) -> impl IntoElement {
        let mut driven = use_consume::<Drives>().0;
        // Consumed here, in the render, because the menu handler may not run a hook.
        let located = use_consume::<Locations>().0;
        let dock = use_consume::<ContentDock>().0;
        let index = self.index;

        // The position this row is, and so the one its menu asks about. Lines are
        // 1-based, as DWARF's are.
        let at = LinePos {
            file: self.file.clone(),
            line: self.index as u32 + 1,
        };

        let pieces = source_pieces(&self.source, index);
        let text = Text {
            line: {
                let mut line = Line::default();
                for (_, text) in &pieces {
                    line.push_text(text.clone());
                }
                line
            },
            head: pieces
                .into_iter()
                .map(|(color, text)| Span::new(text).color(color).assembly_font())
                .collect(),
            inline: None,
            tail: Vec::new(),
            chars: self.chars,
            door: false,
        };

        // The menu: the line's locations and, inside a function as the file's parse
        // says, the function's instances. A location found from the file a
        // source-driven tab is about is chosen for that tab; from a companion it opens
        // the symbol. The function this row is a line of is looked for on the press and
        // not per render: it is a walk of the file's functions, and a row is rendered
        // far more often than it is right-clicked.
        let menu: Rc<dyn Fn(Event<PressEventData>)> = Rc::new({
            let at = at.clone();
            let subject = self.drives.map(|tab| (tab, self.file.clone()));
            let source = self.source.clone();
            move |e: Event<PressEventData>| {
                let function = functions::enclosing(&source.0.functions, at.line).cloned();
                ContextMenu::open_from_event(
                    &e,
                    locate_menu(located, dock, at.clone(), subject.clone(), function),
                );
            }
        });

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

        // The same gesture as the assembly pane's, from the same chrome. The run is a
        // run of this file.
        code_row(
            Chrome {
                pane: Pane::Source,
                row: index,
                file: Some(self.file.clone()),
                paired: self.paired,
                wash: self.wash,
                widest: self.widest,
                listing: self.listing,
                measured: true,
            },
            vec![number],
            Some(text),
            Some(menu),
        )
        // A press in a source-driven tab's own file also says which listing the
        // other side shows; the row is picked out by `pointer_down` either way.
        .maybe(self.drives.is_some(), |el| {
            let tab = self.drives;
            el.on_press(move |_| {
                // **The only writer of `Driven` inside the panes.** A click in the
                // file a source-driven tab is about is what says which assembly its
                // other side shows; a click in a companion file picks the line out
                // and no more, and a click in the assembly pane never comes here at
                // all, so there is no way for the listing to re-drive itself.
                if let Some(tab) = tab {
                    driven
                        .write()
                        .remember((tab, Document::Source(at.file.clone())), at.line);
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
    /// The row this tab opens at the first time it is shown, from [`opening_row`]. A row
    /// remembered for the tab wins over it -- see `use_kept_position`.
    opening: usize,
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

impl Component for SourceList {
    fn render(&self) -> impl IntoElement {
        let marked = use_consume::<Marked>().0;
        let rows = marked_rows(marked, Pane::Source);
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
        let a11y = use_a11y();

        let controller = use_scroll_controller(ScrollConfig::default);
        let mut viewport = use_state(|| 0.0f32);
        // The widest row drawn, under the highlighted file's identity.
        let widest = use_widest();
        let listing = Widest::key(Arc::as_ptr(&self.source.0).addr());

        let length = self.source.0.lines;
        // The tab's entry and not the file: see `SourceList::document`.
        let docs = use_consume::<OpenDocs>().0;
        let entry = (self.tab, self.document.clone());
        use_kept_position(
            use_consume::<SrcAt>().0,
            move |(tab, document): &Entry| docs.peek().contains(*tab, document),
            {
                let file = self.file.clone();
                let document = self.document.clone();
                move |controller: &mut ScrollController| {
                    let index = match owed_reveal(marked, Pane::Source) {
                        None => return false,
                        Some(Owing::Own(rows)) => *rows.rows().start(),
                        // The line the run's first placed instruction came from. Nothing
                        // to scroll to when that is a file this pane is not showing --
                        // an inlined header's line 42 is not line 42 of the file on
                        // screen -- nor when the line is past the end of a file that
                        // has moved on since it was compiled.
                        Some(Owing::Pair(pair)) => {
                            let places = places_of(
                                &document,
                                &pair,
                                &analysis.peek(),
                                code_rows.peek().as_deref(),
                            );
                            let Some(line) =
                                places.iter().find(|at| at.file == file).map(|at| at.line)
                            else {
                                return false;
                            };
                            let Some(index) = (line as usize).checked_sub(1) else {
                                return false;
                            };
                            index
                        }
                    };
                    if index >= length {
                        return false;
                    }
                    reveal_made(marked, Pane::Source);
                    reveal_row(controller, *viewport.peek(), index);
                    true
                }
            },
            controller,
            &entry,
            length,
            self.opening,
        );

        let nudge = use_nudge();
        let grid = pixel_grid();
        // The list as its rows and a sweep past its edge know it: its scroll, its box,
        // the paragraphs the rows lend it, and its widest row.
        let listing_ctx = use_provide_context(|| Listing::new(controller, widest, listing));
        let bounds = listing_ctx.bounds.clone();
        let on_key_down = {
            let source = self.source.clone();
            let drawn = self.source.clone();
            let mut controller = controller;
            on_listing_key(
                marked,
                Pane::Source,
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
                move |index| reveal_caret(&mut controller, *viewport.peek(), index),
            )
        };

        rect()
            .width(Size::fill())
            .height(Size::flex(1.0))
            .padding(5.0)
            .child(
                rect()
                    .expanded()
                    .a11y_id(a11y)
                    .a11y_focusable(true)
                    .on_pointer_down(move |_| a11y.request_focus())
                    .on_key_down(on_key_down)
                    .on_sized({
                        let bounds = bounds.clone();
                        move |e: Event<SizedEventData>| {
                            viewport.set_if_modified(e.area.height());
                            nudge.measured(grid, e.area.min_y());
                            bounds.set(e.area);
                        }
                    })
                    .on_global_pointer_move(use_sweep_beyond(
                        marked,
                        Pane::Source,
                        listing_ctx.clone(),
                        nudge,
                        length,
                    ))
                    // On the grid: see `Nudge`.
                    .padding(nudge.padding())
                    .child(
                        VirtualScrollView::new_with_data_controlled(
                            SourceData {
                                source: self.source.clone(),
                                file: self.file.clone(),
                                pairs,
                                rows,
                                chars,
                                // A source-driven tab's subject is the file its own
                                // document names; a companion's tab is a symbol's.
                                drives: matches!(self.document, Document::Source(_))
                                    .then_some(self.tab),
                                widest,
                                listing,
                            },
                            |i, data: &SourceData| {
                                let paired_at = |row: usize| data.pairs.contains(&(row as u32 + 1));
                                SourceRow {
                                    source: data.source.clone(),
                                    file: data.file.clone(),
                                    index: i,
                                    paired: paired_at(i).then(|| Edges::of(i, paired_at)),
                                    wash: wash_of(data.chars, i),
                                    chars: RowChars::of(data.chars, i),
                                    drives: data.drives,
                                    widest: data.widest,
                                    listing: data.listing,
                                    key: DiffKey::None,
                                }
                                .key(i)
                                .into()
                            },
                            controller,
                        )
                        .length(length)
                        .item_size(code_row_height()),
                    ),
            )
    }
}

/// Which file the Source pane is drawing, and whose side of the tab it is: a **subject** is
/// a source-driven tab's own file, a **companion** the file the drawn symbol was compiled
/// from.
///
/// The companion comes out of the *analysis* and not out of `Active`, because the two
/// disagree for as long as the worker takes and it is the analysis that says which symbol
/// is actually drawn.
pub(crate) enum SourceSide {
    Subject(Arc<str>),
    Companion(Arc<str>),
}

impl SourceSide {
    pub(crate) fn file(&self) -> &Arc<str> {
        match self {
            SourceSide::Subject(file) | SourceSide::Companion(file) => file,
        }
    }
}

/// Which file the Source pane draws for `active`: a source-driven tab's own file, or the
/// drawn symbol's companion. The companion is the symbol's own file -- the one its first
/// instruction was compiled from -- except when the source pane's picked-out run is in
/// another file the listing's line info knows: a row in the Locations panel opens a
/// symbol on a line of the file the line is in, and a symbol whose prologue was inlined
/// from elsewhere would otherwise open on that elsewhere, with the line the reader asked
/// for in a file that is not up. A run picked out inside the pane is in the file already
/// shown, so a click there changes no file, and a click on an inlined instruction never
/// picks anything out on this side at all.
pub(crate) fn source_side(
    active: Option<&Document>,
    analysis: &Analyzed,
    marks: &Marks,
) -> Option<SourceSide> {
    match active? {
        Document::Source(file) => Some(SourceSide::Subject(file.clone())),
        // An object's code draws no symbol of its own, so its companion is the file of
        // whatever the reader picked out in it -- an instruction row's run is a run of
        // the file the pressed row was compiled from -- and nothing until they have.
        Document::Code(_) => marks
            .assembly
            .as_ref()
            .and_then(|picked| picked.file.clone())
            .map(SourceSide::Companion),
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
            picked
                .cloned()
                .or_else(|| lines.file.clone())
                .map(SourceSide::Companion)
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
        Document::Code(_) => code_places(built, pair.rows.rows()),
        _ => analysis
            .shown
            .as_ref()
            .map(|shown| shown.studied.places(pair.rows.rows(), 0))
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

/// The row the Source pane opens a tab it has never shown at: the line the symbol itself
/// opens at, backed off by the margin [`reveal_row`] keeps above the row it scrolls to, so
/// a function's signature is not flush against the top of the pane.
///
/// **The top of the file where there is nothing better to say**, which is what selecting a
/// symbol used to do in every case: an object with no line info, a symbol whose opening row
/// DWARF places on no line, and a companion that is not the symbol's own file -- the last
/// being a landing's doing, which comes with a reveal of its own and would otherwise be
/// sent to a line of the wrong file.
fn opening_row(lines: &SymbolLines, file: &Arc<str>) -> usize {
    let line = lines
        .line
        .filter(|_| lines.file.as_ref() == Some(file))
        .unwrap_or(0);
    (line as usize)
        .saturating_sub(1)
        .saturating_sub(CONTEXT_ROWS as usize)
}

/// What the Source pane says over a file whose bytes are not the ones the debug info's
/// checksum was taken of: the file is shown, since it is still the best thing to show, but
/// its line numbers are the compiler's and not necessarily this file's.
pub(crate) const STALE_SOURCE: &str = "This file differs from the one the binary was built from";

/// One row over the source rows saying [`STALE_SOURCE`], drawn only when it is so. In the
/// header's own colours: a notice about the file, in the place the file is named.
fn stale_banner() -> Element {
    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .width(Size::fill())
        .height(Size::px(list_row_height()))
        .padding(Gaps::new_symmetric(0.0, 8.0))
        .background(palette().header_bg)
        .child(label().text(STALE_SOURCE).color(palette().text_fg))
        .into()
}

/// The bar over the Source pane naming the file it is showing as a **companion** -- a
/// subject gets none, being named by its own tab -- and opening that file as a
/// source-driven tab when it is pressed, the one door into one that is not a Files row.
///
/// The states come in as arguments: this is called from inside a `match`, and a hook may
/// only run unconditionally in a component's body.
fn companion_header(
    open: Open,
    visits: State<Visits>,
    ctrl: State<bool>,
    file: Arc<str>,
    sweeping: bool,
) -> Element {
    let document = Document::Source(file.clone());

    // Not hit while a sweep is under way: the pointer dragging a selection up past the
    // header would otherwise arm its tooltip, and light it.
    rect()
        .width(Size::fill())
        .interactive(!sweeping)
        .child(row_tooltip(
            file.to_string(),
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .padding(Gaps::new_symmetric(0.0, 8.0))
                .spacing(6.0)
                .background(palette().header_bg)
                .border(bottom_hairline())
                // A click inside the tab: in place, or a tab of its own with Ctrl.
                .on_press(move |_| {
                    let reach = if *ctrl.peek() {
                        Reach::NewTab
                    } else {
                        Reach::InPlace
                    };
                    open_document(open, visits, document.clone(), reach);
                })
                .child(entry_icon(&Document::Source(file.clone())))
                .child(label().text(file_name(&file)).max_lines(1)),
        ))
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
        let open = use_open();
        let visits = use_consume::<Visited>().0;
        let ctrl = use_consume::<Ctrl>().0;
        // Reading it is what subscribes this tab to the analysis, so the pane fills in when
        // a newly selected symbol's line info is worked out.
        let analysis = use_consume::<Analysis>().0.read().clone();
        // The tab's own document and not `Active`, which is a memo and a beat behind: this
        // pane is only ever mounted for the tab it belongs to.
        let marks = use_consume::<Marked>().0.read().clone();
        let code_rows = use_consume::<CodeRows>().0;
        let side = source_side(Some(&self.document), &analysis, &marks);

        let Some(side) = side else {
            // The same answer the assembly pane gives, from the same place, plus one case
            // of its own: a symbol can be analysed and still name no file.
            return match analysis.showing(&self.document) {
                Showing::Message(text) => placeholder(text),
                Showing::Nothing => rect().expanded().background(palette().pane_bg).into(),
                Showing::Listing(shown) if shown.studied.lines.info.is_some() => {
                    placeholder("No source file for this symbol")
                }
                Showing::Listing(_) => placeholder("No line info"),
            };
        };

        let file = side.file().clone();
        // The tab, and the row it opens at the first time it is shown. A source-driven
        // tab is a *file* the reader opened, so it opens where a file does, at the top;
        // an assembly tab is a symbol, and the symbol's own lines are what asking for it
        // asked to see.
        let (document, opening) = match &side {
            SourceSide::Subject(file) => (Document::Source(file.clone()), 0),
            // In an object's code the companion is the file of the row the reader
            // pressed, and the tab opens on that row's line.
            SourceSide::Companion(_) if matches!(self.document, Document::Code(_)) => {
                let line = marks
                    .assembly
                    .as_ref()
                    .and_then(|picked| {
                        let anchor = picked.rows.anchor;
                        code_places(code_rows.peek().as_deref(), anchor..=anchor)
                            .into_iter()
                            .next()
                    })
                    .map_or(0, |at| at.line as usize);
                (
                    self.document.clone(),
                    line.saturating_sub(1).saturating_sub(CONTEXT_ROWS as usize),
                )
            }
            // The *drawn* symbol's tab and not the active one: a row written down against
            // the tab that is arriving would be a row of the listing that is leaving.
            SourceSide::Companion(_) => match analysis.shown.as_ref() {
                Some(shown) => (
                    asked_of(&shown.ask),
                    opening_row(&shown.studied.lines, &file),
                ),
                None => return rect().expanded().background(palette().pane_bg).into(),
            },
        };

        // Whether the file on disk is the one the binary was built from, by the checksum
        // the debug info recorded for it — where it recorded one, and where the file
        // opened at all. Compared against the *drawn* symbol's line info, for a subject
        // and a companion alike: it is the one place a recorded checksum comes from.
        let stale = analysis
            .shown
            .as_ref()
            .and_then(|shown| shown.studied.lines.hash_for(&file))
            .zip(source::load(Path::new(&*file)))
            .is_some_and(|(recorded, opened)| !opened.matches(recorded));

        rect()
            .expanded()
            // The header takes its own height and the list is given the rest, which torin
            // only works out for a `flex` child of a `Content::Flex` parent.
            .content(Content::Flex)
            .background(palette().pane_bg)
            .maybe_child(match &side {
                SourceSide::Companion(file) => {
                    Some(companion_header(open, visits, ctrl, file.clone(), sweeping))
                }
                SourceSide::Subject(_) => None,
            })
            .maybe_child(stale.then(stale_banner))
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    // The path is named in the message because it is the only clue to
                    // *why*: built elsewhere, moved and deleted all look alike from here.
                    .child(match source_text(Path::new(&*file)) {
                        Some(source) => SourceList {
                            source,
                            file,
                            tab: self.tab,
                            document,
                            opening,
                        }
                        .into_element(),
                        None => placeholder(format!("Source file not found: {file}")),
                    }),
            )
            .into()
    }
}
