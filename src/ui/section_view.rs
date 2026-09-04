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
use crate::section::{Row, Rows, GAP_BYTES_PER_ROW};
use crate::tabs::Spot;

/// How many screens above and below the viewport are decoded ahead, so that a page up or
/// down lands on rows already there and empty rows are seen only by a reader outrunning
/// the worker.
pub(crate) const BUFFER: f32 = 3.0;

/// At most how many stretches one ask names. The worker takes a chunk of them and the
/// view asks again, so this bounds a message and not the work.
pub(crate) const WINDOW: usize = 64;

/// Where each open code tab was left: the placed address at the top of its pane, and how
/// many rows past that address's row. At the root for the reason [`AsmAt`] is, and an
/// address rather than a row because the rows of a listing being decoded are counted
/// afresh with every answer.
#[derive(Clone, Copy)]
pub(crate) struct CodeAt(pub(crate) State<Positions<Entry, Spot>>);

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
    marks: Option<RowSelection>,
    /// The characters picked out here, for each row to draw its part of.
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
            && self.marks == other.marks
            // The caret and the selection too: a key that moves the caret along a row
            // changes no row of the run, and rows compared without it drew the caret
            // where it had been -- which read, in the unified view alone, as Left, Right,
            // Home and End doing nothing.
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
        Some(AsmData {
            assembly,
            object: self.object.clone(),
            symbol: studied.symbol.data.clone(),
            lanes: studied.lanes.clone(),
            lines: studied.lines.clone(),
            subject: None,
            base: rows.body_start(flat)?,
            bias: rows.bias(flat)?,
            width: lanes::MAX_LANES,
            code_tab: true,
        })
    }
}

/// The text a row copies as: what it draws, one line.
pub(crate) fn row_line(rows: &Rows, reading: &Reading, row: usize) -> String {
    match rows.row(row) {
        Some(Row::Header { section }) => rows
            .code()
            .sections()
            .get(section)
            .map(|placed| format!("section {}", placed.listing.section().name))
            .unwrap_or_default(),
        Some(Row::Label { stretch, index }) => {
            let address = rows.address_of(row).unwrap_or(0);
            let name = label_of(rows, stretch, index)
                .map(|symbol| symbol.display().to_owned())
                .unwrap_or_default();
            format!("{address:016X} {name}:")
        }
        Some(Row::Instruction { stretch, index }) => reading
            .held
            .get(&stretch)
            .and_then(|s| s.code.as_ref())
            .and_then(|studied| studied.assembly.as_ref())
            .and_then(|assembly| assembly.instructions.get(index))
            .map(|instruction| asm_line(instruction, rows.bias(stretch).unwrap_or(0)))
            .unwrap_or_default(),
        Some(Row::Gap { stretch, index }) => gap_bytes(rows, reading, stretch, index)
            .map(|(address, bytes)| {
                let (mark, values) = dump_line(&bytes);
                format!("{address:016X} {mark} {values}")
            })
            .unwrap_or_default(),
        Some(Row::Rule { .. } | Row::Space { .. } | Row::Empty { .. } | Row::Separator { .. })
        | None => String::new(),
    }
}

/// The text row `row` draws after its address, as a character selection copies it:
/// [`row_line`] without the address column, for the rows that have one.
pub(crate) fn code_line(rows: &Rows, reading: &Reading, row: usize) -> Line {
    match rows.row(row) {
        Some(Row::Header { section }) => rows
            .code()
            .sections()
            .get(section)
            .map(|placed| Line::text(format!("section {}", placed.listing.section().name)))
            .unwrap_or_default(),
        Some(Row::Label { stretch, index }) => label_of(rows, stretch, index)
            .map(|symbol| Line::text(format!("{}:", symbol.display())))
            .unwrap_or_default(),
        Some(Row::Instruction { stretch, index }) => reading
            .held
            .get(&stretch)
            .and_then(|s| s.code.as_ref())
            .and_then(|studied| studied.assembly.as_ref())
            .filter(|assembly| index < assembly.instructions.len())
            .map(|assembly| instruction_line(assembly, index))
            .unwrap_or_default(),
        Some(Row::Gap { stretch, index }) => gap_bytes(rows, reading, stretch, index)
            .map(|(_, bytes)| {
                let (mark, values) = dump_line(&bytes);
                text_line(Some(mark), &values)
            })
            .unwrap_or_default(),
        Some(Row::Rule { .. } | Row::Space { .. } | Row::Empty { .. } | Row::Separator { .. })
        | None => Line::default(),
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
fn gap_bytes(rows: &Rows, reading: &Reading, flat: usize, index: usize) -> Option<(u64, Vec<u8>)> {
    let gap = reading.held.get(&flat)?.gap.as_ref()?;
    let start = gap
        .range
        .start
        .checked_add((index as u64).checked_mul(GAP_BYTES_PER_ROW)?)?;
    if start >= gap.range.end {
        return None;
    }
    let end = start.saturating_add(GAP_BYTES_PER_ROW).min(gap.range.end);
    // The section the stretch is in holds the bytes; `gap.range` is in its own addresses.
    let placed = rows.placed_of(flat)?;
    let section = placed.listing.section();
    let offset = start.checked_sub(section.address)?;
    let offset: usize = offset.try_into().ok()?;
    let len: usize = (end - start).try_into().ok()?;
    let bytes = section.data.get(offset..offset + len)?.to_vec();
    Some((placed.place(start), bytes))
}

/// A row that is text and nothing else: a section's header, a symbol's label, a gap's
/// bytes. Takes the mark handlers so a sweep down the listing is not cut at every one.
#[derive(Clone, PartialEq)]
struct TextRow {
    row: usize,
    /// The address column, or none for a row that stands for no address of its own.
    address: Option<u64>,
    text: String,
    color: Color,
    bold: bool,
    wash: Wash,
    /// The symbol a label names, which a **Ctrl**-press on the row opens as a tab of its
    /// own: the door from a function read among its neighbours back to reading it alone.
    /// A plain press is a plain press, and picks the row out like any other.
    opens: Option<Symbol>,
    /// The data directive a row of bytes wears in front of its values, and none for a row
    /// of anything else: the assembler's own word for what the row is, `db` to `dq` by the
    /// unit it is shown in, with the bytes as characters after the values -- a hex dump's
    /// shape, which no instruction row has, so a page of data is not taken for a page of
    /// assembly. Said in the row's shape and not in a colour.
    mark: Option<&'static str>,
    /// The columns of this row inside the pane's character selection (`RowChars`).
    chars: RowChars,
    /// The listing's widest row and its key, as every row of a listing carries them:
    /// a gap's bytes are the widest row in the app, and what the others are floored to.
    widest: Widest,
    listing: u64,
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
        let alt = use_consume::<Alt>().0;
        let mut hovering = use_state(|| false);
        let open = use_open();
        let visits = use_consume::<Visited>().0;
        let opens = self.opens.clone();
        // A label lights as a link only while Ctrl is held, which is when a press is one.
        // The cue is the label's colour, the one the relocation link takes, and the row
        // draws nothing under the pointer: an assembly row's only wash is its pair's,
        // and a listing of an object's code has no pair to light.
        let link = opens.is_some() && ctrl();
        let color = if hovering() && link {
            palette().name_hover_fg
        } else {
            self.color
        };
        let weight = if self.bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        };

        // The text: the data directive, where the row has one, then what the row says --
        // one paragraph, as `row_line` spells it after the address.
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
                .color(color)
                .font_weight(weight)
                .assembly_font(),
        );
        let text = Text {
            line: text_line(self.mark, &self.text),
            head,
            inline: None,
            tail: Vec::new(),
            chars: self.chars,
            door: false,
        };

        // The gutter's width, so the address column starts where it does on an
        // instruction row; then the address, gutter too.
        let before = vec![
            rect()
                .width(Size::px(gutter_width(lanes::MAX_LANES)))
                .into_element(),
            label()
                .text(match self.address {
                    Some(address) => format!("{address:016X} "),
                    None => String::new(),
                })
                .min_width(Size::px(200.0))
                .color(palette().address_fg)
                .max_lines(1)
                .into_element(),
        ];

        // A row of no file: a label or a header is nobody's line.
        code_row(
            Chrome {
                pane: Pane::Assembly,
                row: self.row,
                file: None,
                paired: None,
                wash: self.wash,
                widest: self.widest,
                listing: self.listing,
                measured: true,
            },
            before,
            Some(text),
            None,
        )
        .on_pointer_over(move |_| hovering.set_if_modified(true))
        .on_pointer_out(move |_| hovering.set_if_modified(false))
        .maybe(opens.is_some(), move |el| {
            el.on_press(move |_| {
                // Alt says a press on a link is not a door this time, so the selection
                // the row's own `pointer_down` began stands.
                if !*ctrl.peek() || *alt.peek() {
                    return;
                }
                // A tab of its own, as Ctrl opens one everywhere.
                if let Some(symbol) = opens.clone() {
                    open_document(
                        open,
                        visits,
                        Document::Assembly(Selection::Symbol(symbol)),
                        Reach::NewTab,
                    );
                }
            })
        })
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
    /// The listing's widest row and its key: empty space is washed too.
    widest: Widest,
    listing: u64,
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
        // and swept across.
        code_row(
            Chrome {
                pane: Pane::Assembly,
                row: self.row,
                file: None,
                paired: None,
                wash: self.wash,
                widest: self.widest,
                listing: self.listing,
                measured: false,
            },
            Vec::new(),
            None,
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

/// The listing of one object's code.
#[derive(Clone)]
pub(crate) struct SectionList {
    /// The tab this listing is in, which with `document` is what its place is kept under.
    pub(crate) tab: DocId,
    pub(crate) document: Document,
    pub(crate) object: Arc<Object>,
}

impl PartialEq for SectionList {
    fn eq(&self, other: &Self) -> bool {
        self.tab == other.tab
            && self.document == other.document
            && Arc::ptr_eq(&self.object, &other.object)
    }
}

impl Component for SectionList {
    fn render(&self) -> impl IntoElement {
        let reading_state = use_consume::<Sections>().0;
        let window = use_consume::<Window>().0;
        // Reading it is what redraws the listing as answers land.
        let reading = reading_state.read().clone();
        let marked = use_consume::<Marked>().0;
        let marks = marked_rows(marked, Pane::Assembly);
        let chars = chars_of(marked, Pane::Assembly);
        let pair = pair_of(marked, Pane::Assembly);
        let docs = use_consume::<OpenDocs>().0;
        let code_at = use_consume::<CodeAt>().0;
        let marks_at = use_consume::<MarksAt>().0;
        let plant = use_consume::<Plant>().0;
        let a11y = use_a11y();
        let controller = use_scroll_controller(ScrollConfig::default);
        let mut viewport = use_state(|| 0.0f32);
        // The widest row drawn, under the object's identity and not the rows': `Built`
        // is made afresh as every stretch lands, and the listing is the same one.
        let widest = use_widest();
        let listing = Widest::key(Arc::as_ptr(&self.object).addr());
        // The rows, produced by the place-keeping effect and rendered from here, so that
        // new rows and the offset that keeps the reader's place under them land together.
        let rows = use_consume::<CodeRows>().0;

        let object = self.object.clone();
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
        let stop = docs
            .read()
            .current(self.tab)
            .cloned()
            .unwrap_or_else(|| Stop::whole(self.document.clone()));
        let entry = (self.tab, stop);
        use_kept_place(
            code_at,
            move |(tab, stop): &Entry| docs.peek().contains(*tab, stop),
            // The scroll this pane owes: to the source pane's run, the row of the first
            // instruction compiled from one of its lines, in whichever held stretch has
            // one. Left owed while none does -- the stretch may not be decoded yet, and
            // the answer that decodes it wakes this again.
            move |controller: &mut ScrollController, built: &Built| {
                let row = match owed_reveal(marked, Pane::Assembly) {
                    None => return false,
                    Some(Owing::Own(rows)) => *rows.rows().start(),
                    Some(Owing::Pair(pair)) => {
                        let Some(row) = row_compiled_from(built, &built.reading, &pair) else {
                            return false;
                        };
                        row
                    }
                };
                reveal_made(marked, Pane::Assembly);
                reveal_row(controller, *viewport.peek(), row);
                true
            },
            reading_state,
            marked,
            marks_at,
            plant,
            rows,
            controller,
            &entry,
            generation,
        );
        use_window(
            reading_state,
            window,
            rows,
            controller,
            viewport,
            object.clone(),
        );

        // No skeleton yet means no rows, and a list of none: mounted all the same, see
        // `SectionRows::rows`.
        let built = rows.read().clone().filter(|_| about);
        let length = built.as_ref().map_or(0, |rows| rows.len());

        // The branches touching a picked-out instruction, stretch by held stretch: the
        // run is listing rows and each stretch's lanes speak its own instructions.
        let touching: Vec<(usize, Vec<PlacedEdge>)> = match (&built, marks) {
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
                move |row| reveal_caret(&mut controller, *viewport.peek(), row),
            )
        };

        let nudge = use_nudge();
        let grid = pixel_grid();
        // The list as its rows and a sweep past its edge know it: its scroll, its box,
        // the paragraphs the rows lend it, and its widest row.
        let listing_ctx = use_provide_context(|| Listing::new(controller, widest, listing));
        let bounds = listing_ctx.bounds.clone();

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
                Pane::Assembly,
                listing_ctx.clone(),
                nudge,
                length,
            ))
            // On the grid: see `Nudge`.
            .padding(nudge.padding())
            .child(
                VirtualScrollView::new_with_data_controlled(
                    SectionRows {
                        rows: built,
                        object,
                        pair,
                        touching,
                        marks,
                        chars,
                    },
                    move |i, data: &SectionRows| {
                        build_row(i, data, controller, viewport, widest, listing)
                    },
                    controller,
                )
                .length(length)
                .item_size(code_row_height()),
            )
            .into_element()
    }
}

/// Row `i` of the listing, as what it draws.
fn build_row(
    i: usize,
    data: &SectionRows,
    controller: ScrollController,
    viewport: State<f32>,
    widest: Widest,
    listing: u64,
) -> Element {
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
    let text = |address: Option<u64>,
                text: String,
                color: Color,
                bold: bool,
                opens: Option<Symbol>,
                mark: Option<&'static str>,
                key: RowKey| {
        TextRow {
            row: i,
            address,
            text,
            color,
            bold,
            wash,
            opens,
            mark,
            chars,
            widest,
            listing,
            key: DiffKey::None,
        }
        .key(key)
        .into_element()
    };

    match rows.row(i) {
        Some(Row::Header { section }) => {
            let placed = &rows.code().sections()[section];
            text(
                Some(placed.range().start),
                format!("section {}", placed.listing.section().name),
                palette().text_fg,
                true,
                None,
                None,
                RowKey::Header(section),
            )
        }
        Some(Row::Label { stretch, index }) => {
            let address = rows.address_of(i).unwrap_or(0);
            let symbol = label_of(rows, stretch, index);
            let name = symbol
                .as_ref()
                .map(|symbol| format!("{}:", symbol.display()))
                .unwrap_or_default();
            text(
                Some(address),
                name,
                palette().name_fg,
                true,
                symbol.map(|symbol| Symbol {
                    object: data.object.clone(),
                    data: symbol,
                }),
                None,
                RowKey::Label(address, index),
            )
        }
        // The rule over a stretch, and the two blanks: drawn as an empty row is, washed
        // and swept across. Told apart by their kind, the three of one stretch standing
        // for the one address.
        Some(Row::Rule { stretch }) => EmptyRow {
            row: i,
            wash,
            widest,
            listing,
            rule: true,
            key: DiffKey::None,
        }
        .key(RowKey::Rule(rows.start_of(stretch).unwrap_or(0)))
        .into_element(),
        Some(Row::Space { stretch, under }) => EmptyRow {
            row: i,
            wash,
            widest,
            listing,
            rule: false,
            key: DiffKey::None,
        }
        .key(RowKey::Space(rows.start_of(stretch).unwrap_or(0), under))
        .into_element(),
        Some(Row::Empty { stretch, index }) => EmptyRow {
            row: i,
            wash,
            widest,
            listing,
            rule: false,
            key: DiffKey::None,
        }
        .key(RowKey::Empty(rows.start_of(stretch).unwrap_or(0), index))
        .into_element(),
        Some(Row::Gap { stretch, index }) => {
            let (address, bytes) =
                gap_bytes(rows, &rows.reading, stretch, index).unwrap_or((0, Vec::new()));
            let (mark, values) = dump_line(&bytes);
            text(
                Some(address),
                values,
                palette().operand_fg,
                false,
                None,
                Some(mark),
                // By the row's own address and never the bytes': a row whose bytes could
                // not be found would otherwise share a key with every other such row.
                RowKey::Gap(rows.address_of(i).unwrap_or(address)),
            )
        }
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
                    lanes: asm.lanes.row(index),
                    lit: lanes::lit(touching(stretch), index),
                },
                data: asm,
                index,
                row: i,
                controller,
                viewport,
                widest,
                listing,
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
                    lanes: asm.lanes.boundary(below),
                    lit,
                },
                widest,
                listing,
                key: DiffKey::None,
            }
            .key(RowKey::Sep(address))
            .into_element()
        }
        None => rect().height(Size::px(code_row_height())).into_element(),
    }
}

/// Keep `controller` pointed at the place `tab` was left at, and keep [`CodeAt`] told
/// where it is now -- and produce the rows the place is kept against.
///
/// `use_kept_position`'s shape with an address for a row, since the rows here are counted
/// afresh with every answer: what is written down is the placed address at the top of the
/// pane and how many rows past that address's row, and what is put back is that address's
/// row now plus those rows. The rows are rebuilt **here**, whenever the reading's
/// generation changes, and set into `rows` in the same run that moves the controller, so
/// the pass that first draws new rows draws them at the corrected offset rather than one
/// frame early.
///
/// The picked-out run is kept the same way, in `marks_at` beside the runs `use_land`
/// keeps by rows: the place each of its rows stands for, written whenever the run or the
/// rows change ([`Kept::spots`], stamped with the generation), and read back when the
/// rows are built **for the first time since the reading was reset** -- the run
/// `use_land` put back is by rows of a listing that is gone, and is carried through
/// those places to the rows there are now ([`Kept::carry`]). That run is always
/// `use_land`'s: the reset is a change of the active entry, which `use_land` answers a
/// pass after the memo, and the rows come a pass after the reading follows it. On the
/// run that switches tab the marks on screen are still the last tab's, so nothing is
/// written then unless this run carried a run of its own, or planted one.
///
/// **A door's instruction is planted here** ([`Planting`]), in the first run that has
/// rows and finds a planting naming this document -- over the kept run, a landing
/// winning -- on the row at or below its address (`Rows::body_row_for`), and the address
/// itself is what is kept for the caret's row: a guessed row's own place is its share of
/// an undecoded stretch, and re-placing the caret by that once the stretch decodes would
/// land it on the row nearest the guess rather than on the instruction holding the byte.
/// So a place kept for a row of the run **stays** for as long as it still names the row
/// (`row_of` over the rows on screen), the exact address a planting gave and the derived
/// one alike, and only a row with none is given its own; and the carry across a recount
/// goes through the kept place before the derived one for the same reason. The pane owes
/// the planted caret no scroll: the tab's place is the same address, written by the same
/// door, and is what scrolls the view to it.
fn use_kept_place(
    mut places: State<Positions<Entry, Spot>>,
    is_open: impl Fn(&Entry) -> bool + 'static,
    mut reveal: impl FnMut(&mut ScrollController, &Built) -> bool + 'static,
    reading: State<Reading>,
    marked: State<Marks>,
    mut marks_at: State<Positions<Entry, Kept>>,
    mut plant: State<Option<Planting>>,
    mut rows: State<Option<Arc<Built>>>,
    mut controller: ScrollController,
    tab: &Entry,
    generation: Option<u64>,
) {
    /// What the hook remembers between runs, none of it rendered from.
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
        /// The place a move was issued to and not yet seen, and how often it has been
        /// re-issued.
        moving: Option<Spot>,
        tries: usize,
    }
    /// How often a move is re-issued before the view is taken at its word.
    const MOVE_TRIES: usize = 3;
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
            let rebuilt = state.built != Some(generation);
            let switching = state.tab.as_ref() != Some(tab);
            // The rows drawn until now, which are what the offset was scrolled against.
            let before = rows.peek().clone();
            // Whether this run put the tab's own run back, or planted one, which makes
            // it this tab's run to write down whether or not the tab is being switched
            // to.
            let mut carried = false;
            // The places kept for the run's rows, which a carry goes through first.
            let kept = marks_at.peek().at(tab);
            let built = if rebuilt {
                let reading = reading.peek();
                let Some(code) = reading.code.clone() else {
                    if before.is_some() {
                        rows.set(None);
                    }
                    return;
                };
                let built = Arc::new(Built {
                    rows: Rows::new(code, |flat| reading.body(flat)),
                    reading: (*reading).clone(),
                });
                state.built = Some(generation);
                match before.as_ref() {
                    // The run picked out over the old rows, carried to the new: each of
                    // its rows through the address it stood for, the way the reader's
                    // place is kept across the same recount -- the place kept for the
                    // row where it still names it, which is exact, else the row's own.
                    Some(before) => carry_assembly(marked, |row| {
                        let spot = kept
                            .as_ref()
                            .and_then(|kept| kept.spot_of(row))
                            .filter(|spot| row_of(before, *spot) == Some(row))
                            .or_else(|| spot_at(before, row))?;
                        row_of(&built, spot)
                    }),
                    // The first rows since the reading was reset: the run kept for this
                    // place, carried through the places kept with it, and nothing where
                    // nothing was kept -- a run left over from a listing this tab is not
                    // showing goes.
                    None => {
                        let replanted = kept
                            .as_ref()
                            .and_then(|kept| kept.carry(|spot| row_of(&built, spot)));
                        carried = replanted.is_some();
                        set_assembly(marked, replanted);
                    }
                }
                built
            } else {
                match before.clone() {
                    Some(built) => built,
                    None => return,
                }
            };

            // The caret a door left to be planted, if it is this document's: on the row
            // at or below the address, and spent whether or not there is one -- an
            // address in no stretch is dropped rather than left for ever. Read and not
            // peeked, so a door opened while the tab is on top wakes this. The address
            // goes with the row into the places kept below, exactly.
            let mut planted: Option<(usize, Spot)> = None;
            let planting = plant.read().clone();
            if let Some(planting) = planting.filter(|planting| planting.tab == tab.1.document) {
                plant.set(None);
                if let Some(row) = built.body_row_for(planting.address) {
                    let file = match built.row(row) {
                        Some(Row::Instruction { stretch, index }) => built
                            .reading
                            .held
                            .get(&stretch)
                            .and_then(|stretched| stretched.code.as_ref())
                            .and_then(|studied| studied.position(index))
                            .map(|at| at.file),
                        _ => None,
                    };
                    land_row(marked, file, row, Owed::default());
                    let first = built.row_for(planting.address).unwrap_or(row);
                    planted = Some((
                        row,
                        Spot {
                            address: planting.address,
                            rows: row.saturating_sub(first),
                        },
                    ));
                    carried = true;
                }
            }

            // The places the run's rows stand for, written down as they change and only
            // for a run that is this tab's own -- and for a tab still open, as the place
            // below is. A place already kept that still names the row stays, the exact
            // address a planting gave among them; a row with none gets its own.
            if (!switching || carried) && is_open(tab) {
                let spots = Kept::spots_of(marked.peek().assembly.as_ref(), |row| {
                    planted
                        .filter(|(at, _)| *at == row)
                        .map(|(_, spot)| spot)
                        .or_else(|| {
                            kept.as_ref()?
                                .spots
                                .iter()
                                .map(|(_, spot)| *spot)
                                .find(|spot| row_of(&built, *spot) == Some(row))
                        })
                        .or_else(|| spot_at(&built, row))
                });
                let was = kept.clone();
                let kept = Kept {
                    spots,
                    generation: Some(generation),
                    marks: was
                        .as_ref()
                        .map(|was| was.marks.clone())
                        .unwrap_or_default(),
                };
                if was.as_ref() != Some(&kept) {
                    marks_at.write().remember(tab.clone(), kept);
                }
            }

            let scrolled = (top / height) as usize;
            let row = scrolled.min(built.len().saturating_sub(1));
            let remainder = top - row as f64 * height;
            let derived = spot_at(&built, row);
            // Read and not peeked, unlike `use_kept_position`'s map: a place written from
            // outside while the tab is on top -- an instruction shown among its
            // neighbours -- has to be answered, and the run this wakes on its own write
            // finds nothing moved and writes nothing.
            let known = places.read().at(tab);
            let written = known != state.known;
            state.known = known;

            // A move this hook made and has not seen arrive yet. The view mounts a pass
            // after the rows first exist and resets the offset as it does, and it clamps
            // a target past its content: either would otherwise read as a scroll of the
            // reader's and be written down over the place they asked for. So a move is
            // re-issued until a run finds the view there, a few times and no more.
            if let Some(moving) = state.moving {
                // Arrived when the view's top row is the row the place names -- by row
                // and not by spot, since a place written from outside can be an address
                // inside a row, a call's target in the middle of an instruction, which
                // no spot derived from the offset will ever spell.
                if row_of(&built, moving) == Some(row) || state.tries >= MOVE_TRIES {
                    state.moving = None;
                } else if !switching && !rebuilt {
                    state.tries += 1;
                    if let Some(to) = row_of(&built, moving) {
                        controller.scroll_to_y(to_offset(to as f64));
                    }
                    return;
                }
            }

            // Where this run has to move the view to, if anywhere.
            let target: Option<Spot> = if switching {
                known
            } else if rebuilt {
                // The rows changed under the reader: back to the place they were at --
                // the map's own place where the view was at it, as well as the old rows
                // could tell, since a place written from outside is exact and a row's
                // share of an undecoded stretch is a guess. A target in a stretch the
                // worker had not reached lands on its own row once the stretch is
                // decoded, and not on the row its guess was nearest.
                let exact = known.filter(|known| {
                    before.as_ref().is_some_and(|old| {
                        row_of(old, *known) == Some(scrolled.min(old.len().saturating_sub(1)))
                    })
                });
                exact.or(state.derived).or(known)
            } else if derived != state.derived && known != derived {
                // A scroll: write it down, for a tab that is still open. The run after
                // a close is still holding the tab and would put it straight back.
                if let Some(derived) = derived.filter(|_| is_open(tab)) {
                    places.write().remember(tab.clone(), derived);
                }
                None
            } else if written && known.is_some() && known != derived {
                // Written from outside -- a landing -- while the tab is on top.
                known
            } else {
                None
            };

            if switching {
                state.tab = Some(tab.clone());
            }
            state.derived = derived;
            if rebuilt {
                rows.set(Some(built.clone()));
            }
            // The reveal first, as `use_kept_position` has it: a scroll it makes is where
            // the view goes, and the run it wakes writes the place down.
            if reveal(&mut controller, &built) {
                state.moving = None;
                return;
            }
            if let Some(target) = target {
                let Some(to) = row_of(&built, target) else {
                    return;
                };
                if to != row || (rebuilt && switching) {
                    // Keeping the sub-row remainder, so a chunk landing above does not
                    // snap the view to a row edge.
                    let keep = if switching { 0.0 } else { remainder };
                    controller.scroll_to_y(to_offset(to as f64 + keep / height));
                    state.derived = spot_at(&built, to);
                    state.moving = Some(target);
                    state.tries = 0;
                }
            }
        },
    );
}

/// Show the instruction at `address` -- placed, in `object`'s code -- among its
/// neighbours: the object's code tab, opened in a tab of its own on that address, with
/// the caret on the instruction's row and the line the instruction was compiled from
/// picked out in the source pane where it has one.
///
/// The place is written in the same handler as the open and before any render, so the
/// pane's first run finds it; it comes *after* the open only because the entry it is kept
/// under names the tab, and a new tab has no id until it is opened. When the code tab is
/// already on top the write is what moves the view, `use_kept_place` reading the map for
/// exactly this. The line and the instruction go through `land`, which knows whether
/// the tab is on top; the caret is planted by the pane once it has rows, on the row at
/// or below the address, and moved onto the instruction itself once its stretch decodes.
pub(crate) fn show_in_code(
    open: Open,
    visits: State<Visits>,
    marked: State<Marks>,
    landing: State<Option<Landing>>,
    plant: State<Option<Planting>>,
    mut places: State<Positions<Entry, Spot>>,
    object: Arc<Object>,
    address: u64,
    at: Option<LinePos>,
) {
    let code = Document::Code(object);
    // The stop `land` makes of the landing below, kept for the place written down after
    // it. Moving inside the listing the reader is already in is put on the trail there,
    // so Back comes back to the instruction that was followed and not to where the jump
    // landed, and the place left keeps its own rows and runs, being an entry of its own.
    let stop = Stop::at(code.clone(), address);
    let id = land(
        open,
        visits,
        marked,
        landing,
        plant,
        Landing {
            tab: code.clone(),
            at,
            address: Some(address),
            columns: None,
        },
        Reach::NewTab,
    );
    if let Some(id) = id {
        places
            .write()
            .remember((id, stop), Spot { address, rows: 0 });
    }
}

/// Open `symbol`'s own tab from a row of it read among its neighbours, the caret on that
/// row's instruction -- `address` is the symbol's own, the space its listing draws -- and
/// landing on the line the row was compiled from where it has one: `show_in_code`'s door
/// the other way, and a tab of its own likewise.
pub(crate) fn open_as_symbol(
    open: Open,
    visits: State<Visits>,
    marked: State<Marks>,
    landing: State<Option<Landing>>,
    plant: State<Option<Planting>>,
    symbol: Symbol,
    address: u64,
    at: Option<LinePos>,
) {
    let tab = Document::Assembly(Selection::Symbol(symbol));
    land(
        open,
        visits,
        marked,
        landing,
        plant,
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
        let assembly = studied.assembly.as_ref()?;
        let index = (0..assembly.instructions.len()).find(|&index| {
            studied.position(index).is_some_and(|at| {
                pair.file.as_ref() == Some(&at.file)
                    && (at.line as usize)
                        .checked_sub(1)
                        .is_some_and(|row| pair.rows.contains(row))
            })
        })?;
        Some(rows.body_start(flat)? + studied.lanes.row_of(index))
    })
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
    object: Arc<Object>,
) {
    use_side_effect(move || {
        // The four inputs a scroll, a resize, an answer or a change of reading brings.
        // The reading is **read**, so the effect follows it: the pane mounts a beat
        // before the reading becomes its own -- `Active` is a memo and `use_reading_of`
        // runs off it -- and a run that found the reading about something else asked for
        // nothing, and nothing woke it until the pane was resized. Reading it cannot loop:
        // the one thing written here is the window, and only when it changed.
        let (_, offset) = <(i32, i32)>::from(controller);
        let viewport = *viewport.read();
        let rows = rows.read().clone();
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
