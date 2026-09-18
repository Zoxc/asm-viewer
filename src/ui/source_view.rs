//! The source half of a document from the list up: the rows of the file it is showing,
//! which file that is, and the pane that decides.
//!
//! [`source_side`] is the one place either pane decides which file is up, so the pane and
//! the effect that drops its picked-out rows cannot disagree about which listing is being
//! shown. Only the symbol's **own** file is ever drawn, never the rest of
//! `LineInfo::files`.
//!
//! A row and everything it draws is `source_row.rs`, the bar over the pane
//! `source_bar.rs`.

use super::*;
use crate::counter;

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
    /// The row this tab opens at the first time it is shown, from
    /// [`SourceSide::opening`], and [`None`] for a tab with nothing better to open at
    /// than the top. The row itself and never one backed off towards the top: the rows
    /// kept above it are the pane's to add. A row remembered for the tab wins over it --
    /// see `use_kept_position`.
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
/// A run of the pane's own is a run of the file it is showing, which [`Owing::row`]
/// answers. The other pane's run is answered here by the line its first placed
/// instruction came from, which `places` reads off the listing: nothing to scroll to when
/// that is a file this pane is not showing -- an inlined header's line 42 is not line 42
/// of the file on screen -- nor when the line is past the end of a file that has moved on
/// since it was compiled.
fn owed_file_row(
    owing: &Owing,
    file: &Arc<str>,
    length: usize,
    places: impl FnOnce(&Picked) -> Vec<LinePos>,
) -> Option<usize> {
    owing
        .row(|pair| places(pair).into_iter().find(|at| at.file == *file)?.row())
        .filter(|index| *index < length)
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
    let at = asked.at.as_ref().filter(|at| at.pos.file == *file)?;
    at.pos.row().filter(|index| *index < length)
}

impl Component for SourceList {
    fn render(&self) -> impl IntoElement {
        let marked = use_consume::<Marked>().0;
        let chars = chars_of(marked, Pane::Source);
        let analysis = use_consume::<Analysis>().0;
        let sectioned = use_sectioned();
        // The assembly pane's run, in a memo of its own: `Marks` holds both panes' runs,
        // so a sweep in *this* pane writes the state the other's is read from. The memo
        // hands the same run back where the write left it alone, which is what spares
        // the walk below.
        let pair = use_memo(move || pair_of(marked, Pane::Source));
        // The lines of this file that run was compiled from, worked out only when the
        // run, the listing or the file changes. The pane renders for a good deal else --
        // every move of a sweep here, every scroll that widens the listing, every answer
        // about the file -- and each of those walked the line info again for a set that
        // came out as it was. The memo keeps the set it has where the lines come out the
        // same, so the rows compare it by pointer.
        //
        // The file and the document go through `use_reactive`: a memo's callback is built
        // once in a `use_hook`, so a captured one would stay the first render's.
        let showing = use_reactive(&(self.document.clone(), self.file.clone()));
        let pairs = use_memo(move || {
            let showing = showing.read();
            let (document, file) = &*showing;
            // The rows only where they are this document's, which is the rule and the
            // subscription both: a memo over a file's lines is not woken by another
            // object's listing.
            let built = document.code().and_then(|object| sectioned.rows_of(object));
            Arc::new(paired_lines(
                document,
                file,
                pair.read().as_ref(),
                &analysis.read(),
                built.as_deref(),
            ))
        });
        let pairs = pairs.read().clone();
        // The gutter's marks: which lines of this file produced code at all, whether
        // anything is picked out and whether or not a listing is up. Asking is the pane's
        // ([`ShowingFile`]); this reads whatever has been answered.
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
        // One value for the claim and for the chord below, which have to name the same
        // listing: an answer is judged by `Searchable::id`.
        let searchable = Searchable::Source(self.source.clone());
        use_searching(at, Some(searchable.clone()));
        let (controller, viewport) = (list.controller, list.viewport());

        // Which of this file's names are links, which is the server's to say and not the
        // pane's to guess. Nothing until it has said so -- so no link is ever drawn that
        // could not be followed, where a pane that lit them as soon as a server *started*
        // drew them through the minute it spends reading the project. Asked for by the
        // pane ([`ShowingFile`]) and read here through `use_try_consume`, a pane mounted
        // without one having no links; the answer is an `Arc` inside, so carrying it to
        // the rows is a pointer compare.
        let links = use_try_consume::<Linking>()
            .and_then(|held| held.0.read().links_in(&self.file).cloned())
            .unwrap_or_default();

        // What a row's names, its menu and the four questions about the caret reach for.
        // Consumed here, in the render, and carried to the rows: the handlers run long
        // after it, where no hook may be called.
        let asking = use_row_states();
        let ctrl = use_consume::<Ctrl>().0;
        let server = try_use_server();
        let hover = use_try_consume::<Hovering>().map(|hovering| hovering.0);

        let length = self.source.0.lines;
        // The tab's entry and not the file: see `SourceList::document`.
        let docs = asking.doors.open.docs;
        // The place the tab is at: two lines of one file reached along one trail are two
        // entries, each with its own scroll. Read and not peeked, so a step between them
        // re-renders this pane and the hook sees the switch.
        let entry = (self.tab, place_at(&docs.read(), self.tab, &self.document));
        use_kept_position(
            asking.doors.places.src_at,
            docs,
            Pane::Source,
            {
                let file = self.file.clone();
                let document = self.document.clone();
                move || {
                    // Asked before anything else: `owed_reveal` reads the marks, and that
                    // read is what wakes this on the next click.
                    let owing = owed_reveal(marked, Pane::Source)?;
                    let built = document
                        .code()
                        .and_then(|object| sectioned.peek_rows_of(object));
                    owed_file_row(&owing, &file, length, |pair| {
                        places_of(&document, pair, &analysis.peek(), built.as_deref())
                    })
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
                move |asked: &Landing| landing_row(asked, &document, &file, length)
            },
            controller,
            viewport,
            &entry,
            length,
            listing,
            self.opening,
        );

        // The tab this file's own line questions are answered for, which is the tab it
        // drives: a companion file beside a symbol drives none.
        let drives = (self.document.driven_from() == Pane::Source).then_some(self.tab);

        // The bar's chords, the step it asks for and the listing's own keys, all of it
        // wired once (`use_listing_keys`).
        let keys = use_listing_keys(
            at,
            marked,
            // Every run of this pane is a run of the file it is showing.
            Some(self.file.clone()),
            &list,
            length,
            Some(searchable),
            ListingText {
                line: Rc::new({
                    let source = self.source.clone();
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
                    }
                }),
                text: Rc::new({
                    let drawn = self.source.clone();
                    move |index| source_line(&drawn, index)
                }),
            },
        );
        // Everything every row of this file shares, built once and handed to them all: the
        // text, what the server placed on it, and the states their names and their menus
        // reach for.
        let common = Rc::new(Common {
            source: self.source.clone(),
            file: self.file.clone(),
            // A source-driven tab's subject is the file its own document names; a
            // companion's tab is a symbol's.
            drives,
            links,
            marking,
            asking,
            ctrl,
            server,
            hover,
        });
        // The four questions about the name under the caret, around the whole of that: the
        // F12 family is neither the bar's chord nor the listing's key, and the three sets
        // of keys are disjoint, so which is asked first settles nothing.
        let on_key_down = caret_questions(marked, common.clone(), keys);

        rect()
            .width(Size::fill())
            .height(Size::flex(1.0))
            .child(listing_inset(list.use_rows(
                marked,
                length,
                on_key_down,
                SourceData {
                    common,
                    pairs,
                    compiled,
                    chars,
                },
                source_row,
            )))
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

    /// The place the rows are kept under: the companion's own, and the file itself for a
    /// subject, which is the tab the reader opened.
    fn document(&self) -> Document {
        match self {
            SourceSide::Subject(file) => Document::Source(file.clone()),
            SourceSide::Companion { document, .. } => document.clone(),
        }
    }

    /// The row the pane opens at the first time it shows this side: the line the
    /// companion named, as a row, which is what selecting a symbol or pressing an
    /// instruction asked to see. The row itself, the margin [`reveal_row`] keeps above
    /// the row it scrolls to being the reveal's to add.
    ///
    /// **The top** for a subject, files being opened at the top, and for a companion that
    /// named no line -- which is what selecting a symbol used to do in every case.
    fn opening(&self) -> Option<usize> {
        match self {
            SourceSide::Subject(_) => None,
            SourceSide::Companion { line, .. } => LinePos::row_of((*line)?),
        }
    }

    /// Whether the bar's name is a door: a companion's is, a subject being that tab
    /// already.
    pub(crate) fn opens(&self) -> bool {
        matches!(self, SourceSide::Companion { .. })
    }

    /// The file as a source-driven tab: what the bar draws its glyph from. The same as
    /// [`SourceSide::document`] for a subject, which is why a subject's name is not a
    /// door.
    pub(crate) fn as_source(&self) -> Document {
        Document::Source(self.file().clone())
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
        Document::Object(_) | Document::Symbol(_) => {
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
                        .is_some_and(|info| info.files().any(|named| named == *file))
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
    #[cfg(test)]
    PAIRINGS.set(PAIRINGS.get() + 1);
    let Some(pair) = pair else {
        return HashSet::new();
    };
    places_of(document, pair, analysis, built)
        .into_iter()
        .filter(|at| at.file == *file)
        .map(|at| at.line)
        .collect()
}

counter!(
    /// Test-only: how many times this thread has worked out a source pane's paired
    /// lines, which is what says a render made no such walk.
    pub(crate) fn pairings() = PAIRINGS
);

/// The file the Source pane is showing, shared through context, and nothing else: the one
/// fact the reader, the gutter's marks and the links are each asked about.
///
/// **Written by the pane and read beside each of the three states an answer lands in.**
/// One file, because one is drawn; a pane that moves to another asks again, and `None`
/// where it draws none, so a pane that has stopped drawing a file stops asking about one.
/// Its own state rather than a field of each: three copies of one fact agree only while
/// one writer keeps them in step, and each write woke everything reading the state it
/// went into -- the pane itself among them, which reads [`Sourced`] for what to draw.
#[derive(Clone, Copy)]
pub(crate) struct ShowingFile(pub(crate) State<Option<Arc<str>>>);

/// The Source pane: the tab's source side, whichever of the two sides that is.
#[derive(Clone, PartialEq)]
pub(crate) struct SourcePane {
    /// The tab this pane is in, for the positions its rows keep.
    pub(crate) tab: DocId,
    pub(crate) document: Document,
}

impl Component for SourcePane {
    fn render(&self) -> impl IntoElement {
        let marked = use_consume::<Marked>().0;
        // Whether a sweep is under way, for the header not to answer the pointer during one.
        let sweeping = use_sweeping();
        let doors = use_doors();
        let (open, visits) = (doors.open, doors.visits);
        let ctrl = use_consume::<Ctrl>().0;
        let analysis = use_consume::<Analysis>().0;
        let sectioned = use_sectioned();
        // **Borrowed and not cloned.** This pane is drawn again on every move of a sweep
        // in either pane and on every word from the worker, and both states are whole
        // answers -- a listing and its question, and the two runs with their files. So
        // the guards are bound here, spent, and dropped; the side that comes out of them
        // is owned, and every later read of the analysis is a scope of its own. Reading
        // them is also what subscribes this tab to the two, so the pane fills in when a
        // newly selected symbol's line info is worked out.
        //
        // The tab's own document and not `Active`, which is a memo and a beat behind:
        // this pane is only ever mounted for the tab it belongs to.
        let side = {
            let (analysis, marks) = (analysis.read(), marked.read());
            // Peeked and not read: the line a code tab opens at is read out of the rows,
            // and a window of them decoding must not draw the pane again.
            let built = self
                .document
                .code()
                .and_then(|object| sectioned.peek_rows_of(object));
            source_side(Some(&self.document), &analysis, &marks, built.as_deref())
        };

        // **The one fact three questions are asked about:** which file this pane is
        // showing. Its text, the lines of it anything open has code from, and which of
        // its names the language server calls links -- none is the other's to wait for,
        // and they are answered by the reader, the analysis worker and the server, which
        // are three threads. Each has an effect of its own that reads this beside the
        // state its answer lands in, so what is written here is the file and nothing
        // about any of them. Written here rather than by the rows, which are drawn out of
        // the first answer and so could ask for the other two only once it had landed;
        // and **before the early return below**, a hook having to run on every render --
        // which is why the reader's state is consumed here too, the pane reading it past
        // that return for what to draw.
        let showing = use_consume::<ShowingFile>().0;
        let sourced = use_consume::<Sourcing>().0;
        let file = side.as_ref().map(|side| side.file().clone());
        use_side_effect_with_deps(&file, move |file: &Option<Arc<str>>| {
            // Written only where it moved: a pane redrawn for any of the dozen other
            // reasons must not wake a worker.
            let mut showing = showing;
            showing.set_if_modified(file.clone());
        });

        let Some(side) = side else {
            // The same answer the assembly pane gives, from the same place, plus one case
            // of its own: a symbol can be analysed and still name no file.
            let analysis = analysis.read();
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
        let (document, opening) = (side.document(), side.opening());

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
            .read()
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
            .child(find_bar_over((Placing::Tab(self.tab), Pane::Source)))
            .into()
    }
}

#[cfg(test)]
mod tests;
