//! The section view: the assembly side of an object's code document, all of its code as
//! one listing, read in windows.
//!
//! The rows are `section::Rows` -- counted from the skeleton, estimated where nothing is
//! decoded yet -- drawn into one `VirtualScrollView` of `code_row_height()` rows, the
//! instruction rows being the assembly pane's own `InstructionRow` told where its symbol
//! starts and what its section adds to every address. Two effects do the rest. One keeps
//! the reader's **place** as an address and how many rows past it, re-derives it on every
//! scroll and re-applies it whenever the rows change under the view, producing the rows it
//! applies it against in the same run so that a chunk landing above the viewport never
//! draws one frame in the wrong place. The other works out the **window** -- the stretches
//! within a buffer of screens of the viewport that are not held, nearest the reader first
//! -- and asks for it through `Window`, which the analysis worker's sender reads.

use super::*;
use crate::positions::Spot;
use crate::section::{Row, Rows, GAP_BYTES_PER_ROW};

/// How many screens above and below the viewport are decoded ahead, so that a page up or
/// down lands on rows already there and empty rows are seen only by a reader outrunning
/// the worker.
pub(crate) const BUFFER: f32 = 3.0;

/// At most how many stretches one ask names. The worker takes a chunk of them and the
/// view asks again, so this bounds a message and not the work.
pub(crate) const WINDOW: usize = 64;

/// The rows and the reading they were counted from, **together**: the rows are rebuilt by
/// an effect a pass after an answer lands, so for that one pass the reading the pane can
/// read is newer than the rows on screen -- and a stretch the answer let go of, still
/// drawn from the old rows, would find no bytes and no listing in the new reading. Drawn
/// from this pair, a row always finds what it was counted from; the newer reading is
/// drawn from once it has rows of its own.
pub(crate) struct Built {
    pub(crate) rows: Rows,
    pub(crate) reading: Reading,
}

/// The rows the section view is drawing, shared through context: [`None`] until the
/// skeleton has come, and rebuilt by the view's place-keeping effect with every answer.
/// At the root and not in the view because the Source pane beside an object's code reads
/// them too, to find the lines the picked-out instructions were compiled from.
#[derive(Clone, Copy)]
pub(crate) struct CodeRows(pub(crate) State<Option<Arc<Built>>>);

/// The positions the instructions drawn in the listing rows `rows` of an object's code
/// were compiled from, in listing order, over the stretches held -- which is the window
/// around the reader, so a run over the whole listing costs what is decoded and no more.
pub(crate) fn code_places(built: Option<&Built>, rows: RangeInclusive<usize>) -> Vec<LinePos> {
    let Some(built) = built else {
        return Vec::new();
    };
    built
        .reading
        .held
        .iter()
        .filter_map(|(&flat, stretched)| {
            let studied = stretched.code.as_ref()?;
            let base = built.rows.body_start(flat)?;
            Some(studied.places(rows.clone(), base))
        })
        .flatten()
        .collect()
}

impl std::ops::Deref for Built {
    type Target = Rows;

    fn deref(&self) -> &Rows {
        &self.rows
    }
}

/// What the rows are built from: the rows themselves with the stretches decoded, and the
/// three things a click changes.
#[derive(Clone)]
struct SectionRows {
    /// [`None`] until the skeleton has come: the list is mounted all the same, with no
    /// rows, so that the scroll controller is attached before the place-keeping effect
    /// moves it -- a `VirtualScrollView` resets the offset as it mounts.
    rows: Option<Arc<Built>>,
    object: Arc<Object>,
    /// The source pane's picked-out run, whose pair the instruction rows light.
    pair: Option<Picked>,
    /// The edges starting or ending at a picked-out instruction, by the stretch they
    /// are in, for the gutter of every row those run through.
    touching: Vec<(usize, Vec<PlacedEdge>)>,
    /// The run picked out here -- the caret, the characters, and so the rows -- for each
    /// row to draw its part of, or `None` when there is none.
    chars: Option<CharSelection>,
}

impl PartialEq for SectionRows {
    fn eq(&self, other: &Self) -> bool {
        let same_rows = match (&self.rows, &other.rows) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        same_rows
            && Arc::ptr_eq(&self.object, &other.object)
            && self.pair == other.pair
            && self.touching == other.touching
            // The caret and the columns, and not only the rows the run touches: a key
            // that moves the caret along a row changes no row of it, and rows compared
            // without the caret drew it where it had been -- which read, in the unified
            // view alone, as Left, Right, Home and End doing nothing.
            && self.chars == other.chars
    }
}

impl SectionRows {
    /// The assembly pane's own data for stretch `flat`, if it is decoded and has code.
    fn asm_data(&self, flat: usize) -> Option<AsmData> {
        let rows = self.rows.as_ref()?;
        let stretched = rows.reading.held.get(&flat)?;
        let studied = stretched.code.as_ref()?;
        let assembly = studied.assembly.clone()?;
        // A listing of the object's code: no source-driven tab behind it, this symbol's
        // rows starting where its stretch does, its addresses placed where the layout put
        // its section, and one gutter width for every symbol so the addresses start at one
        // x.
        Some(AsmData::of(
            studied.clone(),
            assembly,
            None,
            rows.body_start(flat)?,
            rows.bias(flat)?,
            lanes::MAX_LANES,
            true,
        ))
    }
}

/// A row that is text and nothing else: the header, a label, a gap's bytes. **One
/// answer for the three of them**, so that what the row draws, what a run of rows copies
/// and what a sweep of characters copies cannot drift apart: [`build_row`] draws this,
/// [`code_line`] is this as a line, and [`row_line`] is that after the address column.
///
/// The colour and the weight are in it because the row draws them from nothing else.
/// They come from [`palette`], and asking it is what subscribes to the theme: the render
/// that draws the row, and nothing at all in the copying paths, which run under no
/// reactive context.
struct TextOf {
    /// The address column, or none for a row that stands for no address of its own.
    address: Option<u64>,
    /// The data directive a row of bytes wears in front of its values, and none for
    /// anything else ([`dump_line`]).
    mark: Option<&'static str>,
    text: String,
    color: Color,
    bold: bool,
    /// The symbol a label names, which a **Ctrl**-press on the label opens as a tab of
    /// its own.
    opens: Option<Arc<SymbolData>>,
}

/// What text row `row` says -- and [`None`] for a row that is not text: an instruction, a
/// blank, a separator, or one whose section or bytes are not there to be read, which
/// draws and copies nothing.
///
/// The rows alone answer this: an instruction is the one row that is read out of the
/// reading.
fn text_of(rows: &Rows, row: usize) -> Option<TextOf> {
    match rows.row(row)? {
        Row::Header { section } => {
            let placed = rows.code().sections().get(section)?;
            Some(TextOf {
                address: Some(placed.range().start),
                mark: None,
                text: format!("section {}", placed.listing.section().name),
                color: palette().text_fg,
                bold: true,
                opens: None,
            })
        }
        Row::Label { stretch, index } => {
            let symbol = label_of(rows, stretch, index)?;
            Some(TextOf {
                address: Some(rows.address_of(row).unwrap_or(0)),
                mark: None,
                text: format!("{}:", symbol.display()),
                color: palette().name_fg,
                bold: true,
                opens: Some(symbol),
            })
        }
        Row::Gap { stretch, index } => {
            let (address, bytes) = gap_bytes(rows, stretch, index)?;
            let (mark, values) = dump_line(&bytes);
            Some(TextOf {
                address: Some(address),
                mark: Some(mark),
                text: values,
                color: palette().operand_fg,
                bold: false,
                opens: None,
            })
        }
        Row::Instruction { .. }
        | Row::Rule { .. }
        | Row::Space { .. }
        | Row::Empty { .. }
        | Row::Separator { .. } => None,
    }
}

/// The text a row copies as: what it draws, one line -- the address column, then
/// [`code_line`]. A row that draws nothing copies nothing, address or no address.
pub(crate) fn row_line(rows: &Rows, reading: &Reading, row: usize) -> String {
    let line = code_line(rows, reading, row).to_string();
    match rows.address_of(row) {
        Some(address) if !line.is_empty() => format!("{address:016X} {line}"),
        _ => line,
    }
}

/// The text row `row` draws after its address, as a character selection copies it:
/// [`row_line`] without the address column.
pub(crate) fn code_line(rows: &Rows, reading: &Reading, row: usize) -> Line {
    match rows.row(row) {
        Some(Row::Instruction { stretch, index }) => reading
            .held
            .get(&stretch)
            .and_then(|s| s.code.as_ref())
            .and_then(|studied| studied.assembly.as_ref())
            .filter(|assembly| index < assembly.instructions.len())
            .map(|assembly| instruction_line(assembly, index))
            .unwrap_or_default(),
        _ => text_of(rows, row)
            .map(|text| text_line(text.mark, &text.text))
            .unwrap_or_default(),
    }
}

/// The `index`th symbol at stretch `flat`'s address.
fn label_of(rows: &Rows, flat: usize, index: usize) -> Option<Arc<SymbolData>> {
    let placed = rows.placed_of(flat)?;
    let place = rows.place(flat)?;
    placed
        .listing
        .stretches()
        .get(place.stretch)?
        .symbols
        .get(index)
        .cloned()
}

/// The bytes gap row `index` of stretch `flat` draws, and the placed address they start
/// at.
fn gap_bytes(rows: &Rows, flat: usize, index: usize) -> Option<(u64, Vec<u8>)> {
    // The rows' own gap and not the reading's: they are counted from it, and it is the
    // whole stretch where nothing was decoded.
    let gap = rows.body(flat)?.gap.as_ref()?;
    let start = gap
        .start
        .checked_add((index as u64).checked_mul(GAP_BYTES_PER_ROW)?)?;
    if start >= gap.end {
        return None;
    }
    let end = start.saturating_add(GAP_BYTES_PER_ROW).min(gap.end);
    // The section the stretch is in holds the bytes; `gap` is in its own addresses.
    let placed = rows.placed_of(flat)?;
    let section = placed.listing.section();
    let offset = start.checked_sub(section.address)?;
    let offset: usize = offset.try_into().ok()?;
    let len: usize = (end - start).try_into().ok()?;
    let bytes = section.data.get(offset..offset + len)?.to_vec();
    Some((placed.place(start), bytes))
}

/// A row that is text and nothing else -- a section's header, a symbol's label, a gap's
/// bytes -- drawn as [`text_of`] says it. Takes the mark handlers so a sweep down the
/// listing is not cut at every one.
#[derive(Clone, PartialEq)]
struct TextRow {
    row: usize,
    /// The address column, or none for a row that stands for no address of its own.
    address: Option<u64>,
    text: String,
    color: Color,
    bold: bool,
    wash: Wash,
    /// The symbol a label names, which a **Ctrl**-press on the label opens as a tab of
    /// its own: the door from a function read among its neighbours back to reading it
    /// alone. A plain press is a plain press, and picks the row out like any other, which
    /// is why the label is drawn as a link only while Ctrl is held (`Door::Label`).
    opens: Option<Symbol>,
    /// The data directive a row of bytes wears in front of its values, and none for a row
    /// of anything else: the assembler's own word for what the row is, `db` to `dq` by the
    /// unit it is shown in, with the bytes as characters after the values -- a hex dump's
    /// shape, which no instruction row has, so a page of data is not taken for a page of
    /// assembly. Said in the row's shape and not in a colour.
    mark: Option<&'static str>,
    /// The columns of this row inside the pane's character selection (`RowChars`).
    chars: RowChars,
    key: DiffKey,
}

/// A gap row as data: the directive for the largest unit that divides the row's bytes --
/// `dq` for quadwords down to `db` for bytes -- and the row's text: the values in that
/// unit, little-endian as x86 reads them, padded to the width a row of bytes would take,
/// then the same bytes as characters between bars, a dot for anything unprintable.
fn dump_line(bytes: &[u8]) -> (&'static str, String) {
    let (mark, unit) = [("dq", 8), ("dd", 4), ("dw", 2), ("db", 1)]
        .into_iter()
        .find(|&(_, unit)| !bytes.is_empty() && bytes.len() % unit == 0)
        .unwrap_or(("db", 1));
    let values: Vec<String> = bytes
        .chunks(unit)
        .map(|chunk| {
            let value = chunk
                .iter()
                .rev()
                .fold(0u64, |value, &byte| (value << 8) | u64::from(byte));
            format!("{value:0width$X}", width = unit * 2)
        })
        .collect();
    let width = GAP_BYTES_PER_ROW as usize * 3 - 1;
    let ascii: String = bytes
        .iter()
        .map(|&byte| {
            if byte.is_ascii_graphic() || byte == b' ' {
                byte as char
            } else {
                '.'
            }
        })
        .collect();
    (mark, format!("{:<width$} |{ascii}|", values.join(", ")))
}

impl KeyExt for TextRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for TextRow {
    fn render(&self) -> impl IntoElement {
        let ctrl = use_consume::<Ctrl>().0;
        let doors = use_doors();
        let weight = if self.bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        };

        // The text: the data directive, where the row has one, then what the row says --
        // one paragraph, and the same one `code_line` copies.
        let mut head = Vec::new();
        if let Some(mark) = self.mark {
            // Non-breaking, so the engine cannot trim it: it is one unit of the text
            // either way.
            head.push(
                Span::new(format!("{mark}\u{a0}"))
                    .color(palette().keyword_fg)
                    .font_weight(FontWeight::BOLD)
                    .assembly_font(),
            );
        }
        head.push(
            Span::new(self.text.clone())
                .color(self.color)
                .font_weight(weight)
                .assembly_font(),
        );
        // The label as the link it is: a run of the row's own text, which for a label is
        // the whole of it -- the symbol's name and the colon after it. The row lights it,
        // shows the hand over it and follows it exactly while `Door::open_now` says the
        // door is open, which for a label is while Ctrl is held; without Ctrl the press
        // is the row's, picking it out like any other.
        let line = text_line(self.mark, &self.text);
        let whole = 0..line.units();
        let links = self.opens.clone().map(|symbol| {
            let door = Door::Label {
                symbol: symbol.clone(),
            };
            TextLinks {
                columns: vec![whole],
                is_link: Rc::new(move || door.open_now(|| ctrl())),
                // A tab of its own, as Ctrl opens one everywhere.
                follow: Rc::new(move |_| {
                    open_document(
                        doors.open,
                        doors.visits,
                        Document::Assembly(Selection::Symbol(symbol.clone())),
                        Reach::NewTab,
                    );
                }),
            }
        });
        let text = Text {
            line,
            head,
            tail: Vec::new(),
            chars: self.chars,
            // As in the assembly pane: an instruction is in no file.
            names: Vec::new(),
            on_hover: None,
            links,
        };

        // The mark's column and the gutter's, which this row gives up rather than draws,
        // so both the address column and the arrows start where they do on an instruction
        // row; then the address, gutter too. A row that is nobody's line is never marked.
        let before = std::iter::once(code_mark(false))
            .chain(gutter_column(lanes::MAX_LANES, None))
            .chain([address_label(self.address)])
            .collect();

        // A row of no file: a label or a header is nobody's line. Nothing is chained onto
        // what comes back: freya keeps an element's handlers in a map by event name, so a
        // handler put on here would replace the row's own of that name and say nothing
        // (`ui/code_row.rs`).
        code_row(
            Chrome {
                pane: Pane::Assembly,
                row: self.row,
                file: None,
                paired: None,
                wash: self.wash,
                measured: true,
            },
            before,
            Some(text),
            None,
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The text a [`TextRow`] draws after its address, as the clipboard sees it: the data
/// directive and a space where the row has one, then the text.
fn text_line(mark: Option<&str>, text: &str) -> Line {
    match mark {
        Some(mark) => Line::text(format!("{mark} {text}")),
        None => Line::text(text),
    }
}

/// One of the rows a stretch nobody has decoded is guessed to take: empty space, and the
/// mark handlers so a sweep across it is not cut.
#[derive(Clone, PartialEq)]
struct EmptyRow {
    row: usize,
    wash: Wash,
    /// Whether the row carries the rule: the space over a stretch does, so one function
    /// is told from the next the way one basic block is told from the one above it, and
    /// the guessed rows of a stretch nobody has decoded do not.
    rule: bool,
    key: DiffKey,
}

impl KeyExt for EmptyRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for EmptyRow {
    fn render(&self) -> impl IntoElement {
        // Nothing to measure and nothing to press but the row: empty space is washed too,
        // and swept across. The mark's column all the same, so this row's rule and a
        // separator's stay the distance apart they were.
        code_row(
            Chrome {
                pane: Pane::Assembly,
                row: self.row,
                file: None,
                paired: None,
                wash: self.wash,
                measured: false,
            },
            vec![code_mark(false)],
            None::<Text<NoLinks>>,
            None,
        )
        .maybe(self.rule, |row| row.child(block_rule()))
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// Which row is which, for the diff: every kind in a key space of its own, over the
/// placed address the row stands for.
#[derive(Hash)]
enum RowKey {
    Header(usize),
    Rule(u64),
    Space(u64, bool),
    Label(u64, usize),
    Empty(u64, usize),
    Insn(u64),
    Sep(u64),
    Gap(u64),
}

/// Where a listing of an object's code keeps its place, its runs and its caret -- and
/// whether that place is one anything is filed under at all.
///
/// **A tab's is an entry on its trail**, and [`Places`] is forgotten with the tab by the
/// three closers. A listing that is **no tab** has no `DocId` to be filed under,
/// and an entry under a made-up one would hold the `Arc<Object>` its document points into
/// with nothing that would ever forget it -- so it names an entry nothing is ever written
/// under, and what keeps the reader's place across a recount is the place derived from the
/// offset, which is the hook's own and not the map's.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placing {
    Tab(DocId),
    /// The Scratchpad's listing of the program its pad built.
    Pad,
}

/// The listing of one object's code.
#[derive(Clone)]
pub(crate) struct SectionList {
    /// Where this listing is drawn, which is what its place is kept under -- or not kept.
    pub(crate) place: Placing,
    pub(crate) object: Arc<Object>,
}

impl PartialEq for SectionList {
    fn eq(&self, other: &Self) -> bool {
        self.place == other.place && Arc::ptr_eq(&self.object, &other.object)
    }
}

impl Component for SectionList {
    fn render(&self) -> impl IntoElement {
        let reading_state = use_consume::<Sections>().0;
        let window = use_consume::<Window>().0;
        // Reading it is what redraws the listing as answers land.
        let reading = reading_state.read().clone();
        let marked = use_consume::<Marked>().0;
        let chars = chars_of(marked, Pane::Assembly);
        let pair = pair_of(marked, Pane::Assembly);
        // The two bundles the place-keeping hook below is given, and the id table this
        // listing's entry is read out of.
        let doors = use_doors();
        let places = use_places();
        let docs = doors.open.docs;
        // The listing these rows are of, held under the object's identity and not the
        // rows': `Built` is made afresh as every stretch lands, and the listing is the
        // same one.
        let listing = Widest::key(Arc::as_ptr(&self.object).addr());
        // The box the rows are drawn in, and the scroll and the measurement that come
        // with it.
        let list = use_list_box(Pane::Assembly, listing);
        let (controller, viewport) = (list.controller, list.viewport);
        // The rows, produced by the place-keeping effect and rendered from here, so that
        // new rows and the offset that keeps the reader's place under them land together.
        let rows = use_consume::<CodeRows>().0;

        let object = self.object.clone();
        // The object as `use_window`'s effect reads it. That effect's closure is built on
        // the first render and never again, and the strip moving between two objects'
        // code tabs **re-renders this scope rather than remounting it**: freya keeps a
        // same-key component's hooks and only replaces its props. An object captured in
        // the closure would stay the first one, every run after the switch would find the
        // reading about something else, and the second tab would draw an empty listing
        // for as long as it was open. Written here by pointer identity, and read in the
        // effect, which is what wakes it on another object.
        let mut current = use_state(|| object.clone());
        let moved = !Arc::ptr_eq(&current.peek(), &object);
        if moved {
            current.set(object.clone());
        }
        let about = reading.is_about(&object);
        let generation = if about {
            Some(reading.generation)
        } else {
            None
        };
        // The place on the trail this listing is showing, which is what its position and
        // its runs are kept under: two stops in one object's code are two places, and
        // stepping between them is what Back does inside a listing. Read and not peeked,
        // so a step re-renders this pane and the hook sees the switch.
        let document = Document::Code(self.object.clone());
        let place = self.place;
        let entry = match place {
            Placing::Tab(tab) => (
                tab,
                docs.read()
                    .current(tab)
                    .cloned()
                    .unwrap_or_else(|| Stop::whole(document.clone())),
            ),
            // An entry nothing is filed under, so nothing has to forget it.
            Placing::Pad => (DocId::unfiled(), Stop::whole(document.clone())),
        };
        use_kept_place(
            doors,
            places,
            move |(tab, stop): &Entry| match place {
                Placing::Tab(_) => docs.peek().contains(*tab, stop),
                Placing::Pad => false,
            },
            // The scroll this pane owes: to the source pane's run, the row of the first
            // instruction compiled from one of its lines, in whichever held stretch has
            // one. Left owed while none does -- the stretch may not be decoded yet, and
            // the answer that decodes it wakes this again.
            move |controller: &mut ScrollController, built: &Built| {
                let row = match owed_reveal(marked, Pane::Assembly) {
                    None => return false,
                    Some(Owing::Own(rows)) => *rows.start(),
                    Some(Owing::Pair(pair)) => {
                        let Some(row) = row_compiled_from(built, &built.reading, &pair) else {
                            return false;
                        };
                        row
                    }
                };
                if !reveal_row(controller, *viewport.read(), built.len(), row) {
                    return false;
                }
                reveal_made(marked, Pane::Assembly);
                true
            },
            reading_state,
            rows,
            controller,
            &entry,
            generation,
        );
        use_window(reading_state, window, rows, controller, viewport, current);

        // No skeleton yet means no rows, and a list of none: mounted all the same, see
        // `SectionRows::rows`.
        let built = rows.read().clone().filter(|_| about);
        let length = built.as_ref().map_or(0, |rows| rows.len());

        // The branches touching a picked-out instruction, stretch by held stretch: the
        // run is listing rows and each stretch's lanes speak its own instructions.
        let touching: Vec<(usize, Vec<PlacedEdge>)> = match (&built, chars) {
            (Some(built), Some(run)) => built
                .reading
                .held
                .iter()
                .filter_map(|(&flat, stretched)| {
                    let studied = stretched.code.as_ref()?;
                    let base = built.body_start(flat)?;
                    let first = run.rows().start().saturating_sub(base);
                    let last = run.rows().end().checked_sub(base)?;
                    let indices = studied.lanes.instructions_in(first..=last)?;
                    let edges = studied.lanes.touching_any(indices);
                    (!edges.is_empty()).then_some((flat, edges))
                })
                .collect(),
            _ => Vec::new(),
        };

        let on_key_down = {
            let rows = built.clone();
            let drawn = built.clone();
            let mut controller = controller;
            on_listing_key(
                marked,
                Pane::Assembly,
                // An assembly run's file is the row's own, so a run of the whole
                // listing is a run of no one file.
                None,
                length,
                viewport,
                move |row| {
                    rows.as_ref()
                        .map(|built| row_line(built, &built.reading, row))
                        .unwrap_or_default()
                },
                move |row| {
                    drawn
                        .as_ref()
                        .map(|built| code_line(built, &built.reading, row))
                        .unwrap_or_default()
                },
                // The caret's row, brought on screen after a key has moved it.
                move |row| {
                    reveal_caret(
                        &mut controller,
                        *viewport.peek(),
                        code_row_height(),
                        length,
                        row,
                    )
                },
            )
        };

        list.render(
            marked,
            length,
            on_key_down,
            SectionRows {
                rows: built,
                object,
                pair,
                touching,
                chars,
            },
            build_row,
        )
    }
}

/// Row `i` of the listing, as what it draws.
fn build_row(i: usize, data: &SectionRows) -> Element {
    let Some(rows) = data.rows.as_ref() else {
        return rect().height(Size::px(code_row_height())).into_element();
    };
    let wash = wash_of(data.chars, i);
    let chars = RowChars::of(data.chars, i);
    // The edges lit in `stretch`, which is the stretch's own entry and nothing when it
    // has none.
    let touching = |stretch: usize| -> &[PlacedEdge] {
        data.touching
            .iter()
            .find(|(flat, _)| *flat == stretch)
            .map_or(&[][..], |(_, edges)| edges.as_slice())
    };
    match rows.row(i) {
        // The three rows that are text and nothing else, drawn from the one answer they
        // are copied from ([`text_of`]); a row whose section or bytes could not be read
        // draws the blank it copies as.
        Some(row @ (Row::Header { .. } | Row::Label { .. } | Row::Gap { .. })) => {
            let Some(text) = text_of(rows, i) else {
                return rect().height(Size::px(code_row_height())).into_element();
            };
            let key = match row {
                Row::Header { section } => RowKey::Header(section),
                Row::Label { index, .. } => RowKey::Label(rows.address_of(i).unwrap_or(0), index),
                // By the row's own address and never the bytes': a row whose bytes could
                // not be found would otherwise share a key with every other such row.
                _ => RowKey::Gap(rows.address_of(i).or(text.address).unwrap_or(0)),
            };
            TextRow {
                row: i,
                address: text.address,
                text: text.text,
                color: text.color,
                bold: text.bold,
                wash,
                opens: text.opens.map(|symbol| Symbol {
                    object: data.object.clone(),
                    data: symbol,
                }),
                mark: text.mark,
                chars,
                key: DiffKey::None,
            }
            .key(key)
            .into_element()
        }
        // The rule over a stretch, and the two blanks: drawn as an empty row is, washed
        // and swept across. Told apart by their kind, the three of one stretch standing
        // for the one address.
        Some(Row::Rule { stretch }) => EmptyRow {
            row: i,
            wash,
            rule: true,
            key: DiffKey::None,
        }
        .key(RowKey::Rule(rows.start_of(stretch).unwrap_or(0)))
        .into_element(),
        Some(Row::Space { stretch, under }) => EmptyRow {
            row: i,
            wash,
            rule: false,
            key: DiffKey::None,
        }
        .key(RowKey::Space(rows.start_of(stretch).unwrap_or(0), under))
        .into_element(),
        Some(Row::Empty { stretch, index }) => EmptyRow {
            row: i,
            wash,
            rule: false,
            key: DiffKey::None,
        }
        .key(RowKey::Empty(rows.start_of(stretch).unwrap_or(0), index))
        .into_element(),
        Some(Row::Instruction { stretch, index }) => {
            let Some(asm) = data.asm_data(stretch) else {
                return rect().height(Size::px(code_row_height())).into_element();
            };
            let address = asm.assembly.instructions[index]
                .address
                .wrapping_add(asm.bias);
            // The rows either side, where they are instructions of this same stretch:
            // a label, a header or a separator is nobody's pair.
            let paired_at = |row: usize| match rows.row(row) {
                Some(Row::Instruction {
                    stretch: other,
                    index,
                }) if other == stretch => asm.paired(index, data.pair.as_ref()),
                _ => false,
            };
            let paired = paired_at(i).then(|| Edges::of(i, paired_at));
            InstructionRow {
                arrows: RowArrows {
                    lanes: asm.lanes().row(index),
                    lit: lanes::lit(touching(stretch), index),
                },
                data: asm,
                index,
                row: i,
                paired,
                wash,
                chars,
                key: DiffKey::None,
            }
            .key(RowKey::Insn(address))
            .into_element()
        }
        Some(Row::Separator { stretch, below }) => {
            let Some(asm) = data.asm_data(stretch) else {
                return rect().height(Size::px(code_row_height())).into_element();
            };
            let address = asm.assembly.instructions[below]
                .address
                .wrapping_add(asm.bias);
            let mut lit = lanes::lit(touching(stretch), below);
            lit.corner = false;
            SeparatorRow {
                row: i,
                wash,
                width: lanes::MAX_LANES,
                arrows: RowArrows {
                    lanes: asm.lanes().boundary(below),
                    lit,
                },
                key: DiffKey::None,
            }
            .key(RowKey::Sep(address))
            .into_element()
        }
        None => rect().height(Size::px(code_row_height())).into_element(),
    }
}

/// What the place-keeping effect remembers between runs, none of it rendered from.
#[derive(Default)]
struct Held {
    tab: Option<Entry>,
    built: Option<u64>,
    /// The place last derived from the offset, to tell a scroll from a write made
    /// from outside.
    derived: Option<Spot>,
    /// The map's value as this last saw it: a write from outside is a *change* of
    /// it, and answered once. Not "the map disagrees with the view", which it does
    /// for good whenever the view cannot be put exactly where the map says -- a
    /// listing of millions of rows sits past where an `f32` offset is exact -- and
    /// which answered every time was a move that woke this into another move for
    /// ever.
    known: Option<Spot>,
    /// The move issued and not yet seen arrive.
    moving: Option<Move>,
}

/// A move the hook made and has not seen the view arrive at.
///
/// The view mounts a pass after the rows first exist and resets the offset as it does,
/// and it clamps a target past its content: either would otherwise read as a scroll of
/// the reader's and be written down over the place they asked for. So a move is
/// re-issued until a run finds the view there, a few times and no more.
#[derive(Clone, Copy)]
struct Move {
    /// The place the move was to.
    to: Spot,
    /// How often it has been issued again.
    tries: usize,
}

impl Move {
    /// How often a move is re-issued before the view is taken at its word.
    const TRIES: usize = 3;

    /// Whether the view is where the move was to, or has been told often enough.
    ///
    /// By row and not by spot, since a place written from outside can be an address
    /// inside a row -- a call's target in the middle of an instruction -- which no spot
    /// derived from the offset will ever spell.
    fn arrived(&self, built: &Built, row: usize) -> bool {
        row_of(built, self.to) == Some(row) || self.tries >= Self::TRIES
    }

    /// The row to issue the move at again, counting the try. [`None`] where the place
    /// has no row in the rows there are now; the run stops there all the same, the move
    /// still owed.
    fn retry(&mut self, built: &Built) -> Option<usize> {
        self.tries += 1;
        row_of(built, self.to)
    }
}

/// One run of the effect as its stages share it: what the run is about, and what the
/// stages before have found.
struct Step<'a> {
    tab: &'a Entry,
    /// The reading generation the rows are counted at.
    generation: u64,
    /// The rows are counted afresh this run, the generation having changed.
    rebuilt: bool,
    /// The tab is not the one the last run was for.
    switching: bool,
    /// The rows drawn until now, which are what the offset was scrolled against.
    before: Option<Arc<Built>>,
    /// The places kept for the tab's run, which a carry goes through first.
    kept: Option<Kept>,
    /// Whether this run put the tab's own run back, or planted one, which makes it this
    /// tab's run to write down whether or not the tab is being switched to.
    carried: bool,
}

/// Where the view is, worked out once from the offset for the stages that need it.
struct At {
    /// The row at the top of the pane, before it is clamped: the offset was scrolled
    /// against the rows there were, and a rebuild clamps it against those instead.
    scrolled: usize,
    /// That row among the rows this run has.
    row: usize,
    /// How far past the top row's edge the view is.
    remainder: f64,
    /// The place the top row stands for.
    derived: Option<Spot>,
    /// The place the map holds for the tab.
    known: Option<Spot>,
    /// `known` has changed since the last run, which is a write from outside.
    written: bool,
}

impl At {
    /// Where the view is against the rows this run has, with the map's place for the tab
    /// beside it and whether that has changed since `was`.
    ///
    /// The map is **read** and not peeked, unlike `use_kept_position`'s: a place written
    /// from outside while the tab is on top -- an instruction shown among its neighbours
    /// -- has to be answered, and the run this wakes on its own write finds nothing moved
    /// and writes nothing.
    fn of(
        step: &Step,
        built: &Built,
        places: State<Positions<Entry, Spot>>,
        was: Option<Spot>,
        top: f64,
        height: f64,
    ) -> At {
        let scrolled = (top / height) as usize;
        let row = scrolled.min(built.len().saturating_sub(1));
        let known = places.read().at(step.tab);
        At {
            scrolled,
            row,
            remainder: top - row as f64 * height,
            derived: spot_at(built, row),
            known,
            written: known != was,
        }
    }
}

/// Keep `controller` pointed at the place `tab` was left at, and keep [`Places::code_at`]
/// told where it is now -- and produce the rows the place is kept against.
///
/// `use_kept_position`'s shape with an address for a row, since the rows here are counted
/// afresh with every answer: what is written down is the placed address at the top of the
/// pane and how many rows past that address's row, and what is put back is that address's
/// row now plus those rows. The rows are rebuilt **here**, whenever the reading's
/// generation changes, and set into `rows` in the same run that moves the controller, so
/// the pass that first draws new rows draws them at the corrected offset rather than one
/// frame early.
///
/// **One effect, because the order is the whole of it**: [`rebuild`] the rows and carry
/// the run across, plant the caret a door left ([`plant_caret`]), name the run's file
/// ([`name_run`]), write down the places its rows stand for ([`keep_spots`]), work out
/// where the view is ([`At`]), re-issue a move the view has not made ([`Move`]), choose a
/// target ([`target_of`]), publish the rows, and then pay the reveal or scroll. Each
/// stage is a function over the [`Step`] they share -- what one stage tells the next is a
/// field of it -- and the rule a stage keeps is written on the stage.
fn use_kept_place(
    doors: Doors,
    places: Places,
    is_open: impl Fn(&Entry) -> bool + 'static,
    mut reveal: impl FnMut(&mut ScrollController, &Built) -> bool + 'static,
    reading: State<Reading>,
    mut rows: State<Option<Arc<Built>>>,
    mut controller: ScrollController,
    tab: &Entry,
    generation: Option<u64>,
) {
    let (marked, plant) = (doors.marked, doors.plant);
    let (code_at, marks_at) = (places.code_at, places.marks_at);
    let held = use_hook(|| Rc::new(RefCell::new(Held::default())));

    use_side_effect_with_deps(
        &(tab.clone(), generation),
        move |(tab, generation): &(Entry, Option<u64>)| {
            // Subscribes this effect to the pane's scroll, so it comes before any return.
            let (_, offset) = <(i32, i32)>::from(controller);
            // In `f64`: a listing of a large binary is millions of rows, tens of millions
            // of pixels down, past where an `f32` holds a pixel, and a row worked out
            // and read back through one would not agree with itself.
            let height = code_row_height() as f64;
            let top = f64::from((-offset).max(0));
            let to_offset =
                |rows: f64| -> i32 { -((rows * height).round().min(i32::MAX as f64) as i32) };

            let Some(generation) = *generation else {
                if rows.peek().is_some() {
                    rows.set(None);
                }
                *held.borrow_mut() = Held::default();
                return;
            };

            let mut state = held.borrow_mut();
            let mut step = Step {
                tab,
                generation,
                rebuilt: state.built != Some(generation),
                switching: state.tab.as_ref() != Some(tab),
                before: rows.peek().clone(),
                kept: marks_at.peek().at(tab),
                carried: false,
            };

            let Some(built) = rebuild(&mut step, &mut state, reading, marked, rows) else {
                return;
            };
            let planted = plant_caret(&mut step, &built, plant, marked);
            name_run(&built, marked);
            keep_spots(&step, &built, planted, marked, marks_at, &is_open);

            let at = At::of(&step, &built, code_at, state.known, top, height);
            state.known = at.known;

            // A move made and not seen arrive: issued again until a run finds the view
            // there, and left where it is on a run that switches tab or counts the rows
            // afresh, which chooses its own target below.
            if let Some(mut moving) = state.moving {
                if moving.arrived(&built, at.row) {
                    state.moving = None;
                } else if !step.switching && !step.rebuilt {
                    let to = moving.retry(&built);
                    state.moving = Some(moving);
                    if let Some(to) = to {
                        controller.scroll_to_y(to_offset(to as f64));
                    }
                    return;
                }
            }

            let target = target_of(&state, &step, &at, code_at, &is_open);

            if step.switching {
                state.tab = Some(tab.clone());
            }
            state.derived = at.derived;
            if step.rebuilt {
                rows.set(Some(built.clone()));
            }
            // The reveal first, as `use_kept_position` has it: a scroll it makes is where
            // the view goes, and the run it wakes writes the place down.
            if reveal(&mut controller, &built) {
                state.moving = None;
                return;
            }
            let Some(target) = target else {
                return;
            };
            let Some(to) = row_of(&built, target) else {
                return;
            };
            if to != at.row || (step.rebuilt && step.switching) {
                // Keeping the sub-row remainder, so a chunk landing above does not snap
                // the view to a row edge.
                let keep = if step.switching { 0.0 } else { at.remainder };
                controller.scroll_to_y(to_offset(to as f64 + keep / height));
                state.derived = spot_at(&built, to);
                state.moving = Some(Move {
                    to: target,
                    tries: 0,
                });
            }
        },
    );
}

/// The rows this run keeps a place against: the ones on screen, or, where the reading's
/// generation has changed, counted afresh -- and, where they are, the picked-out run
/// carried over to them.
///
/// The run is carried the way the reader's place is: each of its rows through the address
/// that row stood for, the place kept for the row where it still names the row, which is
/// exact, and the row's own place otherwise. The kept place goes first because the exact
/// address a door planted is kept there ([`plant_caret`]).
///
/// On the first rows since the reading was reset there are no old rows to carry from, and
/// the run comes back through the places kept for the tab instead ([`Kept::carry`]); a
/// run left over from a listing this tab is not showing goes. That run is always
/// `use_land`'s: the reset is a change of the active entry, which `use_land` answers a
/// pass after the memo, and the rows come a pass after the reading follows it.
///
/// [`None`] ends the run: there is no code to count rows from, or there are no rows at
/// all and nothing to keep a place against.
fn rebuild(
    step: &mut Step,
    held: &mut Held,
    reading: State<Reading>,
    marked: State<Marks>,
    mut rows: State<Option<Arc<Built>>>,
) -> Option<Arc<Built>> {
    if !step.rebuilt {
        return step.before.clone();
    }
    let reading = reading.peek();
    let Some(code) = reading.code.clone() else {
        if step.before.is_some() {
            rows.set(None);
        }
        return None;
    };
    let built = Arc::new(Built {
        rows: Rows::new(code, |flat| reading.body(flat)),
        reading: (*reading).clone(),
    });
    held.built = Some(step.generation);
    if let Some(before) = step.before.as_ref() {
        carry_assembly(marked, |row| {
            let spot = step
                .kept
                .as_ref()
                .and_then(|kept| kept.spot_of(row))
                .filter(|spot| row_of(before, *spot) == Some(row))
                .or_else(|| spot_at(before, row))?;
            row_of(&built, spot)
        });
    } else {
        let replanted = step
            .kept
            .as_ref()
            .and_then(|kept| kept.carry(|spot| row_of(&built, spot)));
        step.carried = replanted.is_some();
        set_assembly(marked, replanted);
    }
    Some(built)
}

/// Plant the caret a door left for this document ([`Planting`]), in the first run that
/// has rows to plant it in -- over the kept run, a landing winning -- on the row at or
/// below its address (`Rows::body_row_for`), and spend it whether or not there was a row:
/// an address in no stretch is dropped rather than left for ever. The planting is read
/// and not peeked, so a door opened while the tab is on top wakes the effect.
///
/// What comes back is that row and the **exact** address for it, which is what
/// [`keep_spots`] keeps for the caret's row: a guessed row's own place is its share of an
/// undecoded stretch, and re-placing the caret by that once the stretch decodes would
/// land it on the row nearest the guess rather than on the instruction holding the byte.
///
/// The pane owes the caret its reveal, as the symbol pane owes its own planting one: the
/// reveal wins over the place the door wrote, and keeps `CONTEXT_ROWS` above the row
/// where the place alone put the instruction against the top of the pane with nothing
/// before it. The place is still the exact address, so a stretch decoding under the view
/// re-places it on the instruction itself; what the reveal gives is where the view sits
/// when the door opens, which is the only moment the reader is reading it.
fn plant_caret(
    step: &mut Step,
    built: &Built,
    mut plant: State<Option<Planting>>,
    marked: State<Marks>,
) -> Option<(usize, Spot)> {
    let planting = plant.read().clone();
    let planting = planting.filter(|planting| planting.tab == step.tab.1.document)?;
    plant.set(None);
    let row = built.body_row_for(planting.address)?;
    land_row(marked, file_at(built, row), row, Owed::by(Pane::Assembly));
    let first = built.row_for(planting.address).unwrap_or(row);
    step.carried = true;
    Some((
        row,
        Spot {
            address: planting.address,
            rows: row.saturating_sub(first),
        },
    ))
}

/// The file the run is a run of, worked out again while it has none.
///
/// A run is planted the moment there are rows, which is the skeleton, where the row it
/// lands on is a guess and names no file -- and a carry maps row indices and keeps the
/// rest of the run as it was, so nothing else would ever fill it in. The unified view
/// draws no symbol of its own, so this run's file is the whole of what the Source pane
/// beside it has to show (`source_side`): left unfilled, a tab the reader opened *at* an
/// instruction says "Click an instruction" for as long as it is open.
///
/// **Filled in, never cleared.** A row that has gone back to a guess keeps the file it
/// was decoded with, and a run that names one is left alone, so this is one write on the
/// pass a stretch decodes under the run and none after.
fn name_run(built: &Built, marked: State<Marks>) {
    let unnamed = marked
        .peek()
        .assembly
        .as_ref()
        .filter(|picked| picked.file.is_none())
        .map(|picked| picked.chars.anchor().row);
    let Some(anchor) = unnamed else {
        return;
    };
    let Some(file) = file_at(built, anchor) else {
        return;
    };
    let named = marked.peek().assembly.as_ref().map(|picked| Picked {
        file: Some(file),
        ..picked.clone()
    });
    set_assembly(marked, named);
}

/// Write down the places the run's rows stand for ([`Kept::spots`], stamped with the
/// generation), which is how the run is carried across a recount and put back after a
/// switch -- `marks_at` beside the runs `use_land` keeps by rows.
///
/// Only for a run that is this tab's own, and for a tab still open, as the place is. On
/// the run that switches tab the marks on screen are still the last tab's, so nothing is
/// written then unless this run carried a run of its own, or planted one.
///
/// A place already kept **stays** for as long as it still names the row (`row_of` over
/// the rows on screen), the exact address a planting gave and a derived one alike, and
/// only a row with none is given its own.
fn keep_spots(
    step: &Step,
    built: &Built,
    planted: Option<(usize, Spot)>,
    marked: State<Marks>,
    mut marks_at: State<Positions<Entry, Kept>>,
    is_open: &dyn Fn(&Entry) -> bool,
) {
    if (step.switching && !step.carried) || !is_open(step.tab) {
        return;
    }
    let spots = Kept::spots_of(marked.peek().assembly.as_ref(), |row| {
        planted
            .filter(|(at, _)| *at == row)
            .map(|(_, spot)| spot)
            .or_else(|| {
                step.kept
                    .as_ref()?
                    .spots
                    .iter()
                    .map(|(_, spot)| *spot)
                    .find(|spot| row_of(built, *spot) == Some(row))
            })
            .or_else(|| spot_at(built, row))
    });
    let was = step.kept.as_ref();
    let kept = Kept {
        spots,
        generation: Some(step.generation),
        marks: was.map(|was| was.marks.clone()).unwrap_or_default(),
    };
    if was != Some(&kept) {
        marks_at.write().remember(step.tab.clone(), kept);
    }
}

/// Where this run has to move the view to, if anywhere. Four answers in order: a switch
/// goes to the map's place; a recount goes back to where the rows were; a scroll is the
/// reader's own, written down here and moving nothing; and a place written from outside
/// while the tab is on top is gone to.
fn target_of(
    held: &Held,
    step: &Step,
    at: &At,
    mut places: State<Positions<Entry, Spot>>,
    is_open: &dyn Fn(&Entry) -> bool,
) -> Option<Spot> {
    if step.switching {
        return at.known;
    }
    if step.rebuilt {
        // The rows changed under the reader: back to the place they were at -- the map's
        // own place where the view was at it, as well as the old rows could tell, since a
        // place written from outside is exact and a row's share of an undecoded stretch
        // is a guess. A target in a stretch the worker had not reached lands on its own
        // row once the stretch is decoded, and not on the row its guess was nearest.
        let exact = at.known.filter(|known| {
            step.before.as_ref().is_some_and(|old| {
                row_of(old, *known) == Some(at.scrolled.min(old.len().saturating_sub(1)))
            })
        });
        return exact.or(held.derived).or(at.known);
    }
    if at.derived != held.derived && at.known != at.derived {
        // A scroll: write it down, for a tab that is still open. The run after a close is
        // still holding the tab and would put it straight back.
        if let Some(derived) = at.derived.filter(|_| is_open(step.tab)) {
            places.write().remember(step.tab.clone(), derived);
        }
        return None;
    }
    // Written from outside -- a landing -- while the tab is on top.
    if at.written && at.known != at.derived {
        at.known
    } else {
        None
    }
}

/// Show the instruction at `address` -- placed, in `object`'s code -- among its
/// neighbours: the object's code tab, opened the way `reach` says on that address, with
/// the caret on the instruction's row and the line the instruction was compiled from
/// picked out in the source pane where it has one.
///
/// `reach` is the press's to say: a menu item asks for a tab of its own, a bare address
/// pressed in a symbol's listing for the code in place and, with Ctrl, for a tab of its
/// own (`reach_inside`). Where the listing is that code already the reach never comes up:
/// `land` finds the document on top and moves inside it.
///
/// The place is written in the same handler as the open and before any render, so the
/// pane's first run finds it; it comes *after* the open only because the entry it is kept
/// under names the tab, and a new tab has no id until it is opened. When the code tab is
/// already on top the write is what moves the view, `use_kept_place` reading the map for
/// exactly this. The line and the instruction go through `land`, which knows whether
/// the tab is on top; the caret is planted by the pane once it has rows, on the row at
/// or below the address, and moved onto the instruction itself once its stretch decodes.
pub(crate) fn show_in_code(
    doors: Doors,
    places: Places,
    object: Arc<Object>,
    address: u64,
    at: Option<LinePos>,
    reach: Reach,
) {
    let code = Document::Code(object.clone());
    // The stop `land` makes of the landing below, kept for the place written down after
    // it. Moving inside the listing the reader is already in is put on the trail there,
    // so Back comes back to the instruction that was followed and not to where the jump
    // landed, and the place left keeps its own rows and runs, being an entry of its own.
    let stop = Stop::at(object, address);
    let id = land(
        doors,
        Landing {
            tab: code.clone(),
            at,
            address: Some(address),
            columns: None,
        },
        reach,
    );
    if let Some(id) = id {
        let mut code_at = places.code_at;
        code_at
            .write()
            .remember((id, stop), Spot { address, rows: 0 });
    }
}

/// Open `symbol`'s own tab from a row of it read among its neighbours, the caret on that
/// row's instruction -- `address` is the symbol's own, the space its listing draws -- and
/// landing on the line the row was compiled from where it has one: `show_in_code`'s door
/// the other way, and a tab of its own likewise.
pub(crate) fn open_as_symbol(doors: Doors, symbol: Symbol, address: u64, at: Option<LinePos>) {
    let tab = Document::Assembly(Selection::Symbol(symbol));
    land(
        doors,
        Landing {
            tab,
            at,
            address: Some(address),
            columns: None,
        },
        Reach::NewTab,
    );
}

/// The listing row of the first held instruction compiled from a line of the source
/// pane's run `pair`, if any is.
fn row_compiled_from(rows: &Rows, reading: &Reading, pair: &Picked) -> Option<usize> {
    reading.held.iter().find_map(|(&flat, stretched)| {
        let studied = stretched.code.as_ref()?;
        let index = studied.first_paired(pair)?;
        Some(rows.body_start(flat)? + studied.lanes.row_of(index))
    })
}

/// The file row `row` was compiled from, where it is an instruction of a stretch that has
/// decoded. [`None`] for every other row -- a label, a header, the bytes no symbol claims,
/// and any row of a stretch still guessed, none of which is anybody's line yet.
fn file_at(built: &Built, row: usize) -> Option<Arc<str>> {
    match built.row(row) {
        Some(Row::Instruction { stretch, index }) => built
            .reading
            .held
            .get(&stretch)
            .and_then(|stretched| stretched.code.as_ref())
            .and_then(|studied| studied.position(index))
            .map(|at| at.file),
        _ => None,
    }
}

/// The row `spot` names now: its address's row -- the row at or below the address, where
/// the address is inside one -- and the rows past it, clamped to the listing. [`None`]
/// for an address in no stretch.
pub(crate) fn row_of(rows: &Rows, spot: Spot) -> Option<usize> {
    let first = rows.row_for(spot.address)?;
    Some((first + spot.rows).min(rows.len().saturating_sub(1)))
}

/// The place row `row` stands for: its address and how many rows past that address's own
/// row it is.
pub(crate) fn spot_at(rows: &Rows, row: usize) -> Option<Spot> {
    let address = rows.address_of(row)?;
    let first = rows.row_for(address)?;
    Some(Spot {
        address,
        rows: row.saturating_sub(first),
    })
}

/// Ask for the stretches within [`BUFFER`] screens of the viewport that are not held,
/// nearest the reader first; and, before there is a skeleton, ask for that.
fn use_window(
    reading: State<Reading>,
    mut window: State<Option<CodeAsk>>,
    rows: State<Option<Arc<Built>>>,
    controller: ScrollController,
    viewport: State<f32>,
    object: State<Arc<Object>>,
) {
    use_side_effect(move || {
        // The five inputs a scroll, a resize, an answer, a change of reading or another
        // object brings. The reading is **read**, so the effect follows it: the pane
        // mounts a beat before the reading becomes its own -- `Active` is a memo and
        // `use_reading_of` runs off it -- and a run that found the reading about something
        // else asked for nothing, and nothing woke it until the pane was resized. The
        // object is read and never captured, for the reason in the pane. Reading them
        // cannot loop: the one thing written here is the window, and only when it changed.
        let (_, offset) = <(i32, i32)>::from(controller);
        let viewport = *viewport.read();
        let rows = rows.read().clone();
        let object = object.read().clone();
        let reading = reading.read();
        if !reading.is_about(&object) {
            return;
        }
        let Some(rows) = rows else {
            // The skeleton, and nothing decoded with it yet.
            let ask = reading.code.is_none().then(|| CodeAsk {
                object: object.clone(),
                code: None,
                window: Vec::new(),
            });
            window.set_if_modified(ask);
            return;
        };
        let height = code_row_height();
        let top = ((-offset).max(0) as f32 / height) as usize;
        let screen = (viewport / height).ceil().max(1.0) as usize;
        let view = top..top.saturating_add(screen);
        let buffer = (BUFFER * screen as f32) as usize;
        let wanted = rows.window(
            view,
            buffer,
            |flat| reading.held.contains_key(&flat),
            WINDOW,
        );
        let ask = (!wanted.is_empty()).then(|| CodeAsk {
            object: object.clone(),
            code: Some(rows.code().clone()),
            window: wanted,
        });
        window.set_if_modified(ask);
    });
}
