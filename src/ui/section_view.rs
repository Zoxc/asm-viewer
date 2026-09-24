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
use crate::counter;
use crate::find::Direction;
use crate::positions::Spot;
use crate::section::{Body, Kind, Row, Rows, StretchRows, GAP_BYTES_PER_ROW};
use analysis::Stretch;
use std::sync::Weak;

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
    /// What the find bar is looking for, compiled once for the list; `None` where no bar
    /// is open (`find_bar.rs`).
    marking: Option<Marking>,
    /// What a row's menu writes, consumed once by the list: see [`RowStates`].
    asking: RowStates,
}

impl PartialEq for SectionRows {
    fn eq(&self, other: &Self) -> bool {
        same_arc(&self.rows, &other.rows)
            && Arc::ptr_eq(&self.object, &other.object)
            && self.pair == other.pair
            && self.touching == other.touching
            // The caret and the columns, and not only the rows the run touches: a key
            // that moves the caret along a row changes no row of it, and rows compared
            // without the caret drew it where it had been -- which read, in the unified
            // view alone, as Left, Right, Home and End doing nothing.
            && self.chars == other.chars
            && self.marking == other.marking
        // `asking` and `links` compare equal always -- handles the root never replaces
        // -- so they are left out.
    }
}

impl SectionRows {
    /// The assembly pane's own data for stretch `flat`, if it is decoded and has code.
    fn asm_data(&self, flat: usize) -> Option<AsmData> {
        let rows = self.rows.as_ref()?;
        let stretched = rows.reading.held.get(&flat)?;
        let studied = stretched.code.as_ref()?;
        // A listing of the object's code: no source-driven tab behind it, this symbol's
        // rows starting where its stretch does, its addresses placed where the layout put
        // its section, and one gutter width for every symbol so the addresses start at one
        // x.
        AsmData::of(
            studied.clone(),
            In::Code {
                base: rows.body_start(flat)?,
                bias: rows.bias(flat)?,
            },
        )
    }
}

/// Which of the four text rows a row is, which is what picks its colour and its weight.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    Header,
    Label,
    Cut,
    Gap,
}

/// What the row over a cut gap says ([`Kind::Cut`]): that the bytes under it are where the
/// decode stopped, not where the function ends.
pub(crate) const CUT_TEXT: &str =
    "; listing cut at the decode cap: the function very likely goes on";

/// A row that is text and nothing else: the header, a label, a cut gap's note, a gap's
/// bytes. **One answer for the four of them**, so that what the row draws, what a run of
/// rows copies and what a sweep of characters copies cannot drift apart: [`build_row`]
/// draws this, [`code_line`] is this as a line, and [`row_line`] is that after the address
/// column.
///
/// It says what the row **is** and never what it looks like. A colour resolved here would
/// be a colour asked for wherever this is asked for, and this is asked for in three places
/// that draw nothing: the sweep, the run of rows, and the walk over an object's code
/// ([`stretch_texts`]), which runs on the find worker's own thread, where [`palette`]
/// answers out of a thread-local of that thread's own and never the window's. The row
/// asks for its own colour where it is drawn, as every other row does.
#[derive(Clone)]
struct TextOf {
    /// The address column, or none for a row that stands for no address of its own.
    address: Option<PlacedAddress>,
    /// The data directive a row of bytes wears in front of its values, and none for
    /// anything else ([`dump_line`]).
    mark: Option<&'static str>,
    text: String,
    role: Role,
    /// The symbol a label names, which a **Ctrl**-press on the label opens as a tab of
    /// its own; `None` for the other two.
    opens: Option<Arc<SymbolData>>,
}

impl PartialEq for TextOf {
    fn eq(&self, other: &Self) -> bool {
        self.address == other.address
            && self.mark == other.mark
            && self.text == other.text
            && self.role == other.role
            // By pointer, as every identity in the UI is: `SymbolData` has no `PartialEq`.
            && same_arc(&self.opens, &other.opens)
    }
}

/// What the row of kind `kind` says in stretch `stretch` of section `placed`, `body`
/// being what its rows are drawn from ([`StretchRows::body`]).
///
/// [`None`] for a row that is not text -- an instruction, a rule, a blank, an empty row,
/// a separator -- or one whose bytes are not there to be read, which draws and copies
/// nothing.
fn text_at(
    placed: &analysis::Placed,
    stretch: &Stretch,
    body: Option<&Body>,
    kind: Kind,
) -> Option<TextOf> {
    match kind {
        Kind::Header => Some(TextOf {
            address: Some(placed.range().start),
            mark: None,
            text: header_text(placed),
            role: Role::Header,
            opens: None,
        }),
        Kind::Label(index) => {
            let symbol = stretch.symbols.get(index)?.clone();
            Some(TextOf {
                address: Some(placed.place(stretch.range.start)),
                mark: None,
                text: label_text(&symbol),
                role: Role::Label,
                opens: Some(symbol),
            })
        }
        Kind::Cut => Some(TextOf {
            address: Some(placed.place(body?.gap.as_ref()?.range.start)),
            mark: None,
            text: CUT_TEXT.to_owned(),
            role: Role::Cut,
            opens: None,
        }),
        Kind::Gap(index) => {
            let (address, bytes) = gap_row_bytes(placed, &body?.gap.as_ref()?.range, index)?;
            let (mark, values) = dump_line(&bytes);
            Some(TextOf {
                address: Some(address),
                mark: Some(mark),
                text: values,
                role: Role::Gap,
                opens: None,
            })
        }
        Kind::Instruction(_)
        | Kind::Rule
        | Kind::Space { .. }
        | Kind::Empty(_)
        | Kind::Separator { .. } => None,
    }
}

/// The same asked of the rows the pane is drawing: what text row `row` says.
fn text_of(rows: &Rows, row: usize) -> Option<TextOf> {
    let Row { stretch, kind } = rows.row(row)?;
    text_at(
        rows.placed_of(stretch)?,
        rows.stretch(stretch)?,
        rows.body(stretch),
        kind,
    )
}

/// The placed address the row of kind `kind` stands at and the line it says, or [`None`]
/// where it says nothing. The one answer for both readers of it: what a row draws and
/// copies, and what the walk searches ([`stretch_texts`]).
fn line_at(
    placed: &analysis::Placed,
    stretch: &Stretch,
    body: Option<&Body>,
    kind: Kind,
) -> Option<(PlacedAddress, Line)> {
    if let Kind::Instruction(index) = kind {
        let assembly = body?.assembly.as_ref()?;
        let address = placed.place(assembly.instructions.get(index)?.address);
        return Some((address, instruction_line(assembly, index)));
    }
    let text = text_at(placed, stretch, body, kind)?;
    Some((text.address?, text_line(text.mark, &text.text)))
}

/// Every line stretch `flat` of `object`'s code draws, in listing order: the placed
/// address each sits at, the kind of row it is and its text. The kind is what tells apart
/// the rows at one address: a header, the labels and the first instruction share theirs.
///
/// **The one statement of what a stretch says.** The pane draws it a row at a time
/// through [`Rows`], which counts its rows out of the very same [`StretchRows`]. The
/// search that walks an object's code (`hunt.rs`) has no [`Rows`] -- it decodes a
/// stretch, reads it and lets it go, and a [`Rows`] per stretch would build the whole
/// skeleton each time -- so it takes a whole stretch from here. A row kind added to
/// [`Kind`] therefore reaches the walk with the pane, and a search can neither find what
/// the reader cannot see nor miss what they can.
///
/// The decode is the crate's own, thrown away again with the lines. The lanes over it are
/// laid out because the rows are counted from them, which is what tells an instruction
/// row from a separator.
pub(crate) fn stretch_texts(
    object: &Object,
    code: &CodeListing,
    flat: usize,
) -> Vec<(PlacedAddress, Kind, Line)> {
    let Some((placed, stretch)) = code.stretch(flat) else {
        return Vec::new();
    };
    let decoded = code
        .decode(object, flat)
        .map(|decoded| Body::of(decoded.code, decoded.gap));
    let rows = StretchRows::of(placed, stretch, code.opens_section(flat), flat, decoded);
    rows.kinds()
        .filter_map(|kind| {
            let (address, line) = line_at(placed, stretch, rows.body(), kind)?;
            Some((address, kind, line))
        })
        .collect()
}

/// The text a row copies as: what it draws, one line -- the address column, then
/// [`code_line`]. A row that draws nothing copies nothing, address or no address.
pub(crate) fn row_line(rows: &Rows, row: usize) -> String {
    let line = code_line(rows, row).to_string();
    match rows.address_of(row) {
        Some(address) if !line.is_empty() => format!("{}{line}", address_column(address)),
        _ => line,
    }
}

/// The text row `row` draws after its address, as a character selection copies it:
/// [`row_line`] without the address column.
pub(crate) fn code_line(rows: &Rows, row: usize) -> Line {
    line_of(rows, row).map(|(_, line)| line).unwrap_or_default()
}

/// The same asked of the rows the pane is drawing: where row `row` stands and what it
/// says, and [`None`] where it says nothing.
fn line_of(rows: &Rows, row: usize) -> Option<(PlacedAddress, Line)> {
    let Row { stretch, kind } = rows.row(row)?;
    line_at(
        rows.placed_of(stretch)?,
        rows.stretch(stretch)?,
        rows.body(stretch),
        kind,
    )
}

/// The text a section's header row draws.
fn header_text(placed: &analysis::Placed) -> String {
    format!("section {}", placed.listing.section().name)
}

/// The text a symbol's label row draws.
fn label_text(symbol: &SymbolData) -> String {
    format!("{}:", symbol.display())
}

/// The bytes gap row `index` of `gap` draws, and the placed address they start at.
fn gap_row_bytes(
    placed: &analysis::Placed,
    gap: &Range<SectionAddress>,
    index: usize,
) -> Option<(PlacedAddress, Vec<u8>)> {
    let start = gap
        .start
        .checked_add((index as u64).checked_mul(GAP_BYTES_PER_ROW)?)?;
    if start >= gap.end {
        return None;
    }
    let end = start.saturating_add(GAP_BYTES_PER_ROW).min(gap.end);
    // The section the stretch is in holds the bytes; `gap` is in its own addresses.
    let bytes = placed.listing.section().bytes_in(start..end)?.to_vec();
    Some((placed.place(start), bytes))
}

/// A row that is text and nothing else -- a section's header, a symbol's label, a cut
/// gap's note, a gap's bytes -- drawn as [`text_of`] says it. Takes the mark handlers so a
/// sweep down the listing is not cut at every one.
///
/// It carries that answer **whole** rather than copying its fields out, so a field added
/// to [`TextOf`] reaches the row without a line here. A row of bytes wears its data
/// directive in front of its values -- the assembler's own word for what the row is, `db`
/// to `dq` by the unit it is shown in, with the bytes as characters after the values --
/// which is a hex dump's shape, and no instruction row has one: a page of data is told
/// from a page of assembly in the row's shape and not in a colour.
#[derive(Clone)]
struct TextRow {
    row: usize,
    /// What the row says, and what a sweep and a run of rows copy out of it.
    text: TextOf,
    /// The object the label's symbol is in, for the door a **Ctrl**-press opens: the way
    /// from a function read among its neighbours back to reading it alone. A plain press
    /// is a plain press and picks the row out like any other, which is why the label is
    /// drawn as a link only while Ctrl is held (`Door::Label`).
    object: Arc<Object>,
    wash: Wash,
    /// The columns of this row inside the pane's character selection (`RowChars`).
    chars: RowChars,
    /// What the find bar is looking for. See [`SectionRows::marking`].
    marking: Option<Marking>,
    key: DiffKey,
}

impl PartialEq for TextRow {
    fn eq(&self, other: &Self) -> bool {
        self.row == other.row
            && self.text == other.text
            && Arc::ptr_eq(&self.object, &other.object)
            && self.wash == other.wash
            && self.chars == other.chars
            && self.marking == other.marking
    }
}

/// A gap row as data: the directive for the largest unit that divides the row's bytes --
/// `dq` for quadwords down to `db` for bytes -- and the row's text: the values in that
/// unit, little-endian as x86 reads them, padded to the width a row of bytes would take,
/// then the same bytes as characters between bars, a dot for anything unprintable.
fn dump_line(bytes: &[u8]) -> (&'static str, String) {
    let (mark, unit) = [("dq", 8), ("dd", 4), ("dw", 2), ("db", 1)]
        .into_iter()
        .find(|&(_, unit)| !bytes.is_empty() && bytes.len().is_multiple_of(unit))
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

keyed!(TextRow);

impl Component for TextRow {
    fn render(&self) -> impl IntoElement {
        // What the label's link reaches for, gathered here for the press to hold.
        let link_states = use_link_states(use_doors());
        // What the row is, drawn: the colour and the weight the four kinds differ in.
        // Asked for here, in the row's own render, because asking is what subscribes a
        // scope to the theme, and it is this scope a switch has to draw again.
        let (color, weight) = match self.text.role {
            Role::Header => (palette().text_fg, FontWeight::BOLD),
            Role::Label => (palette().name_fg, FontWeight::BOLD),
            Role::Cut => (palette().comment_fg, FontWeight::NORMAL),
            Role::Gap => (palette().operand_fg, FontWeight::NORMAL),
        };

        // The text: the data directive, where the row has one, then what the row says --
        // one paragraph, and the same one `code_line` copies.
        let mut spans = Vec::new();
        if let Some(mark) = self.text.mark {
            // The same text `text_line` copies, a plain space included: the spans add
            // up to the row's text, byte for byte, which is what its columns count.
            spans.push(
                Span::new(format!("{mark} "))
                    .color(palette().keyword_fg)
                    .font_weight(FontWeight::BOLD)
                    .assembly_font(),
            );
        }
        spans.push(
            Span::new(self.text.text.clone())
                .color(color)
                .font_weight(weight)
                .assembly_font(),
        );
        // The label as the link it is: a run of the row's own text, which for a label is
        // the whole of it -- the symbol's name and the colon after it. The row lights it,
        // shows the hand over it and follows it exactly while `Door::open_now` says the
        // door is open, which for a label is while Ctrl is held; without Ctrl the press
        // is the row's, picking it out like any other.
        let line = text_line(self.text.mark, &self.text.text);
        let whole = 0..line.len();
        let links = self.text.opens.clone().and_then(|data| {
            let symbol = Symbol {
                object: self.object.clone(),
                data,
            };
            link_states.links(vec![(whole, Door::Label { symbol })])
        });
        let text = Text {
            marking: self.marking.clone(),
            line,
            spans,
            chars: self.chars,
            links,
        };

        // The mark's column and the gutter's, which this row gives up rather than draws,
        // so both the address column and the arrows start where they do on an instruction
        // row; then the address, gutter too. A row that is nobody's line is never marked.
        let before = std::iter::once(code_mark(false))
            .chain(gutter_column(CODE_LANES, None))
            .chain([address_label(self.text.address)])
            .collect();

        // A row of no file: a label or a header is nobody's line. Nothing is chained onto
        // what comes back: freya keeps an element's handlers in a map by event name, so a
        // handler put on here would replace the row's own of that name and say nothing
        // (`ui/code_row.rs`).
        use_code_row(
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
        self.keyed()
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

keyed!(EmptyRow);

impl Component for EmptyRow {
    fn render(&self) -> impl IntoElement {
        // Nothing to measure and nothing to press but the row: empty space is washed too,
        // and swept across. The mark's column all the same, so this row's rule and a
        // separator's stay the distance apart they were.
        use_code_row(
            Chrome {
                pane: Pane::Assembly,
                row: self.row,
                file: None,
                paired: None,
                wash: self.wash,
                measured: false,
            },
            vec![code_mark(false)],
            None,
            None,
        )
        .maybe(self.rule, |row| row.child(block_rule()))
    }

    fn render_key(&self) -> DiffKey {
        self.keyed()
    }
}

/// Which row is which, for the diff: every kind in a key space of its own, over the
/// placed address the row stands for.
#[derive(Hash)]
enum RowKey {
    Header(PlacedAddress),
    Rule(PlacedAddress),
    Space(PlacedAddress, bool),
    Label(PlacedAddress, usize),
    Empty(PlacedAddress, usize),
    Insn(PlacedAddress),
    Sep(PlacedAddress),
    Cut(PlacedAddress),
    Gap(PlacedAddress),
}

/// What a row with no address of its own is keyed by, which is only ever a row whose
/// address could not be worked out: a key space no real address shares is not worth the
/// trouble, since two such rows keying alike is a row redrawn and not a wrong row.
const NO_ADDRESS: PlacedAddress = PlacedAddress::ZERO;

impl RowKey {
    /// The key row `row` draws under, `at` being what [`Rows::row`] said it is. **The one
    /// place a [`Kind`] becomes a key**, and a total match: a tenth kind is a key space of
    /// its own or a compile error, never a row that quietly keys as another kind's.
    ///
    /// `stated` is the address the caller has already worked out for the row -- a text
    /// row's own ([`text_of`]), an instruction's placed one -- so that neither is worked
    /// out twice and an instruction is keyed by the address the row is drawn at. [`None`]
    /// where the caller has none.
    fn of(rows: &Rows, row: usize, at: Row, stated: Option<PlacedAddress>) -> Self {
        // The stretch's start, which the rule over it, the blank under it and its guessed
        // rows all stand for, and the row's own address.
        let start = || rows.start_of(at.stretch).unwrap_or(NO_ADDRESS);
        let address = || rows.address_of(row);
        match at.kind {
            // The section's own start, which is the only address a header stands for and
            // is one section's alone: the sections are placed in order and do not overlap.
            Kind::Header => Self::Header(address().unwrap_or(NO_ADDRESS)),
            Kind::Rule => Self::Rule(start()),
            Kind::Space { under } => Self::Space(start(), under),
            Kind::Label(index) => Self::Label(address().unwrap_or(NO_ADDRESS), index),
            Kind::Empty(index) => Self::Empty(start(), index),
            Kind::Instruction(_) => Self::Insn(stated.or_else(address).unwrap_or(NO_ADDRESS)),
            Kind::Separator { .. } => Self::Sep(stated.or_else(address).unwrap_or(NO_ADDRESS)),
            Kind::Cut => Self::Cut(address().or(stated).unwrap_or(NO_ADDRESS)),
            // By the row's own address and never the bytes': a row whose bytes could not
            // be found would otherwise share a key with every other such row.
            Kind::Gap(_) => Self::Gap(address().or(stated).unwrap_or(NO_ADDRESS)),
        }
    }
}

/// The listing of one object's code.
#[derive(Clone)]
pub(crate) struct SectionList {
    /// Where this listing is drawn, and so what its place is kept under -- or not kept:
    /// the Scratchpad's is carried across a recount by the place derived from the
    /// offset, which is the hook's own and not the map's.
    pub(crate) place: Placing,
    pub(crate) object: Arc<Object>,
}

impl PartialEq for SectionList {
    fn eq(&self, other: &Self) -> bool {
        self.place == other.place && Arc::ptr_eq(&self.object, &other.object)
    }
}

counter!(
    /// Test-only: how many times this thread has drawn an object's code listing, the
    /// heaviest component in the pane, so what wakes it is worth pinning.
    pub(crate) fn listings_drawn() = LISTINGS_DRAWN
);

impl Component for SectionList {
    fn render(&self) -> impl IntoElement {
        #[cfg(test)]
        LISTINGS_DRAWN.set(LISTINGS_DRAWN.get() + 1);
        let sectioned = use_sectioned();
        let reading = sectioned.reading;
        let marked = use_consume::<Marked>().0;
        let chars = chars_of(marked, Pane::Assembly);
        let pair = pair_of(marked, Pane::Assembly);
        // What the rows' menus write, consumed here and carried to them: a handler may not
        // run a hook. One of its fields is the bundle the place-keeping hook below is
        // given, and the id table this listing's entry is read out of comes with it.
        let asking = use_row_states();
        let doors = asking.doors;
        let docs = doors.open.docs;
        // The listing these rows are of, held under the object's identity and not the
        // rows': `Built` is made afresh as every stretch lands, and the listing is the
        // same one.
        let listing = Widest::key(Arc::as_ptr(&self.object).addr());
        // The box the rows are drawn in, and the scroll and the measurement that come
        // with it.
        let list = use_list_box(Pane::Assembly, listing);
        let (controller, viewport) = (list.controller, list.viewport());
        // What the find bar over this pane is looking for, for every row to wash. It
        // searches no listing: an object's code is decoded a stretch at a time, so a step
        // over it walks on for the next match (`hunt.rs`) and the bar has no count. That
        // claim is made all the same, so a bar left over a symbol's listing by the pane
        // this one replaced gives that listing up.
        let at = (self.place, Pane::Assembly);
        let marking = use_marking(at);
        use_searching(at, None);
        // The rows, produced by the place-keeping effect and rendered from here, so that
        // new rows and the offset that keeps the reader's place under them land together.
        let rows = sectioned.rows;

        let object = self.object.clone();
        // The place on the trail this listing is showing, which is what its position and
        // its runs are kept under: two stops in one object's code are two places, and
        // stepping between them is what Back does inside a listing. Read and not peeked,
        // so a step re-renders this pane and the hook sees the switch.
        let document = Document::Code(self.object.clone());
        let place = self.place;
        // The tab, where this listing is on one at all: the Scratchpad's is no tab, has
        // no id, and so is filed nowhere and forgotten by nobody.
        let tab = match place {
            Placing::Tab(tab) => Some(tab),
            Placing::Pad => None,
        };
        let stop = match tab {
            Some(tab) => place_at(&docs.read(), tab, &document),
            None => Stop::whole(document.clone()),
        };
        use_kept_place(
            doors,
            sectioned,
            controller,
            tab,
            &stop,
            &object,
            // The scroll this pane owes: to its own run's first row, or to the source
            // pane's run, the row of the first instruction compiled from one of its
            // lines, in whichever held stretch has one. Left owed while none does -- the
            // stretch may not be decoded yet, and the answer that decodes it wakes this
            // again.
            move |controller: &mut ScrollController, built: &Built| {
                let owed = owed_reveal(marked, Pane::Assembly)
                    .and_then(|owing| owing.row(|pair| row_compiled_from(built, pair)));
                let Some(row) = owed else {
                    return false;
                };
                if !reveal_row(controller, *viewport.read(), built.len(), row) {
                    return false;
                }
                reveal_made(marked, Pane::Assembly);
                true
            },
        );
        use_window(sectioned, controller, viewport, &object);

        // No skeleton yet means no rows, and a list of none: mounted all the same, see
        // `SectionRows::rows`. Reading them is also what redraws this as a window lands.
        let built = sectioned.rows_of(&self.object);
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
                    let edges = studied.touching(run.rows(), base);
                    (!edges.is_empty()).then_some((flat, edges))
                })
                .collect(),
            _ => Vec::new(),
        };

        // The walk a step over this listing asks for, and where the match it finds lands:
        // an object's code is read a piece at a time, so a step reads on rather than
        // stepping through an answer the pane holds (`hunt.rs`).
        {
            let (caret, held) = (marked, rows);
            let mut controller = controller;
            use_code_hunt(
                at,
                self.object.clone(),
                reading,
                move |direction| {
                    // Where the pane is, as a line and a column: the run's far end for a
                    // walk forward and its near end for one back, so a match picked out
                    // is behind the walk either way. None where there is no run in it
                    // yet, and the walk starts at the top.
                    let (near, far) = caret.peek().assembly.as_ref()?.chars.ends();
                    let at = match direction {
                        Direction::Forward => far,
                        Direction::Back => near,
                    };
                    let built = held.peek().clone()?;
                    let line = CodeLine {
                        address: built.address_of(at.row)?,
                        kind: built.row(at.row)?.kind,
                    };
                    Some((line, at.col))
                },
                move |object: &Arc<Object>, line: CodeLine, columns, landed: HuntLanding| {
                    // The row the line is in **now**: the rows are counted afresh as
                    // stretches decode, so the walk answers a line and the row is worked
                    // out here, where there are rows to work it out against.
                    //
                    // The rows are **read**, not peeked: a walk can answer before there
                    // are rows, and the read is what wakes the landing when they come.
                    // Only `object`'s: for a pass after a switch the slot holds the last
                    // listing's.
                    let rows = held.read().clone();
                    let Some(built) = rows.filter(|built| built.reading.is_about(object)) else {
                        return landed;
                    };
                    let mut reveal = |row| {
                        reveal_caret(
                            &mut controller,
                            *viewport.peek(),
                            code_row_height(),
                            built.len(),
                            row,
                        )
                    };
                    let own = built
                        .code()
                        .at(line.address)
                        .and_then(|flat| built.row_of_kind(flat, line.kind));
                    if let Some(row) = own {
                        mark_columns(caret, Pane::Assembly, file_at(&built, row), row, columns);
                        reveal(row);
                        return HuntLanding::Yes;
                    }
                    // In a stretch not decoded yet: the view goes, once, to the row holding
                    // the address, which is where the instruction is guessed to be, and the
                    // window it asks for decodes the stretch. The caret waits for that.
                    match (landed, built.body_row_for(line.address)) {
                        (HuntLanding::No, Some(row)) => {
                            reveal(row);
                            HuntLanding::Guessed
                        }
                        _ => landed,
                    }
                },
            );
        }

        // The bar's chords, the step it asks for and the listing's own keys, all of it
        // wired once (`use_listing_keys`). The step over an object's code is the walk
        // above's and not the hook's: `use_find_steps` leaves a bar with no listing alone.
        let on_key_down = use_listing_keys(
            at,
            marked,
            &list,
            length,
            None,
            ListingText {
                line: Rc::new({
                    let rows = built.clone();
                    move |row| {
                        rows.as_ref()
                            .map(|built| row_line(built, row))
                            .unwrap_or_default()
                    }
                }),
                text: Rc::new({
                    let rows = built.clone();
                    move |row| {
                        rows.as_ref()
                            .map(|built| code_line(built, row))
                            .unwrap_or_default()
                    }
                }),
                file: Rc::new({
                    let rows = built.clone();
                    move |row| rows.as_ref().and_then(|built| file_at(built, row))
                }),
            },
        );

        list.use_rows(
            marked,
            length,
            on_key_down,
            SectionRows {
                rows: built,
                object,
                pair,
                touching,
                chars,
                marking,
                asking,
            },
            build_row,
        )
    }
}

/// Row `i` of the listing, as what it draws.
fn build_row(i: usize, data: &SectionRows) -> Element {
    // A row with nothing to draw, at the height every row of the listing is: what a row
    // the rows cannot answer for comes to -- past the end, in a stretch nobody has
    // decoded, or with a section or bytes that could not be read.
    let blank = || rect().height(Size::px(code_row_height())).into_element();
    let Some(rows) = data.rows.as_ref() else {
        return blank();
    };
    let wash = wash_of(data.chars, i);
    let chars = RowChars::of(data.chars, i);
    // The rule over a stretch and the two blanks: drawn as an empty row is, washed and
    // swept across, and differing in the rule alone. The key is what tells them apart,
    // the three of one stretch standing for the one address.
    let empty = |rule: bool, key: RowKey| {
        EmptyRow {
            row: i,
            wash,
            rule,
            key: DiffKey::None,
        }
        .key(key)
        .into_element()
    };
    // The edges lit in `stretch`, which is the stretch's own entry and nothing when it
    // has none.
    let touching = |stretch: usize| -> &[PlacedEdge] {
        data.touching
            .iter()
            .find(|(flat, _)| *flat == stretch)
            .map_or(&[][..], |(_, edges)| edges.as_slice())
    };
    let Some(at @ Row { stretch, kind }) = rows.row(i) else {
        return blank();
    };
    match kind {
        // The four rows that are text and nothing else, drawn from the one answer they
        // are copied from ([`text_of`]); a row whose section or bytes could not be read
        // draws the blank it copies as.
        Kind::Header | Kind::Label(_) | Kind::Cut | Kind::Gap(_) => {
            let Some(text) = text_of(rows, i) else {
                return blank();
            };
            let key = RowKey::of(rows, i, at, text.address);
            TextRow {
                row: i,
                text,
                object: data.object.clone(),
                wash,
                chars,
                marking: data.marking.clone(),
                key: DiffKey::None,
            }
            .key(key)
            .into_element()
        }
        Kind::Rule => empty(true, RowKey::of(rows, i, at, None)),
        Kind::Space { .. } => empty(false, RowKey::of(rows, i, at, None)),
        Kind::Empty(_) => empty(false, RowKey::of(rows, i, at, None)),
        Kind::Instruction(index) => {
            let Some(asm) = data.asm_data(stretch) else {
                return blank();
            };
            let address = asm.drawn_address(index);
            // The rows either side, where they are instructions of this same stretch:
            // a label, a header or a separator is nobody's pair.
            let paired_at = |row: usize| match rows.row(row) {
                Some(Row {
                    stretch: other,
                    kind: Kind::Instruction(index),
                }) if other == stretch => asm.paired(index, data.pair.as_ref()),
                _ => false,
            };
            let paired = paired_at(i).then(|| Edges::of(i, paired_at));
            InstructionRow::at(
                asm,
                data.asking,
                index,
                i,
                paired,
                data.chars,
                touching(stretch),
                data.marking.clone(),
            )
            .key(RowKey::of(rows, i, at, address.placed()))
            .into_element()
        }
        Kind::Separator { below } => {
            let Some(asm) = data.asm_data(stretch) else {
                return blank();
            };
            let address = asm.drawn_address(below);
            SeparatorRow::over(&asm, below, i, data.chars, touching(stretch))
                .key(RowKey::of(rows, i, at, address.placed()))
                .into_element()
        }
    }
}

/// What the place-keeping effect remembers between runs, none of it rendered from.
#[derive(Default)]
struct Held {
    /// What the last run was for: the tab, where there was one, and the stop it was
    /// showing. Both, since two tabs can show one stop.
    tab: Option<DocId>,
    stop: Option<Stop>,
    /// The object and the reading generation the rows were counted at. The object too,
    /// since every object's reading counts from nought: a pane moved in place to another
    /// object's code can see that one's first answer at the generation the old rows were
    /// counted at. By `Weak`, which holds no bytes and keeps any other object off the
    /// address.
    built: Option<(Weak<Object>, u64)>,
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
    /// The place the move was to, with how far into its row: what it is issued at, and
    /// issued again at.
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

    /// Where to issue the move again, counting the try. [`None`] where the place has no
    /// row in the rows there are now; the run stops there all the same, the move still
    /// owed.
    fn retry(&mut self, built: &Built) -> Option<TopRow> {
        self.tries += 1;
        top_of(built, self.to)
    }
}

/// One run of the effect as its stages share it: what the run is about, and what the
/// stages before have found.
struct Step<'a> {
    /// The place this run keeps things under: the tab and the stop it is showing.
    /// [`None`] for the Scratchpad's listing, which is no tab and has nothing to file a
    /// place, a run or a driven line under.
    tab: Option<Entry>,
    /// What the listing is showing, tab or no tab: what a planting names.
    stop: &'a Stop,
    /// The object and the reading generation the rows are counted at.
    object: &'a Arc<Object>,
    generation: u64,
    /// The rows are counted afresh this run, the object or the generation having changed.
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
    /// That row among the rows this run has, and how far into it the view is.
    top: TopRow,
    /// The place the top of the pane stands for.
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
        let scrolled = TopRow::of_offset(top, height);
        // Past the last row the top of it, with no part of a row to keep.
        let top = scrolled.within(built.len());
        let known = step.tab.as_ref().and_then(|tab| places.read().at(tab));
        At {
            scrolled: scrolled.row,
            top,
            derived: spot_of(built, top),
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
///
/// `tab` is [`None`] for a listing that is no tab -- the Scratchpad's -- which has
/// nothing to file a place or a run under and so has none written for it; `stop` is what
/// the listing is showing either way. `doors.open.docs` says whether the place is still
/// open, which is what a write down here is allowed for, and is asked of the state itself
/// for the reason [`use_kept_position`] gives. `object` is the pane's, and a dep for the reason
/// [`use_window`] gives: the reading is read in the effect, which is what wakes it as an
/// answer lands, and only a reading of `object` has rows for it.
fn use_kept_place(
    doors: Doors,
    sectioned: Sectioned,
    mut controller: ScrollController,
    tab: Option<DocId>,
    stop: &Stop,
    object: &Arc<Object>,
    mut reveal: impl FnMut(&mut ScrollController, &Built) -> bool + 'static,
) {
    let (marked, plant, docs) = (doors.marked, doors.plant, doors.open.docs);
    let (reading, mut rows) = (sectioned.reading, sectioned.rows);
    let (code_at, marks_at) = (doors.places.code_at, doors.places.marks_at);
    let held = use_hook(|| Rc::new(RefCell::new(Held::default())));

    use_side_effect_with_deps(
        &(tab, stop.clone(), ByPtr(object.clone())),
        move |(tab, stop, ByPtr(object)): &(Option<DocId>, Stop, ByPtr<Object>)| {
            // Subscribes this effect to the pane's scroll, so it comes before any return.
            let (_, offset) = <(i32, i32)>::from(controller);
            // And to the reading, so an answer wakes this effect and not the listing: the
            // listing draws the rows this publishes.
            let generation = {
                let reading = reading.read();
                reading.is_about(object).then_some(reading.generation)
            };
            // In `f64`: a listing of a large binary is millions of rows, tens of millions
            // of pixels down, past where an `f32` holds a pixel, and a row worked out
            // and read back through one would not agree with itself.
            let height = code_row_height() as f64;
            let top = f64::from((-offset).max(0));
            // Nowhere past the rows to hold a move to: the view clamps it as it draws.
            let scroll_to = |top: TopRow| scroll_y(top, height, f64::INFINITY);

            let Some(generation) = generation else {
                if rows.peek().is_some() {
                    rows.set(None);
                }
                *held.borrow_mut() = Held::default();
                return;
            };

            let mut state = held.borrow_mut();
            // The place the maps are keyed by, where this listing is a tab's at all.
            let entry = tab.map(|tab| (tab, stop.clone()));
            let mut step = Step {
                object,
                generation,
                rebuilt: !state.built.as_ref().is_some_and(|(of, at)| {
                    *at == generation && std::ptr::eq(of.as_ptr(), Arc::as_ptr(object))
                }),
                switching: state.tab != *tab || state.stop.as_ref() != Some(stop),
                before: rows.peek().clone(),
                kept: entry.as_ref().and_then(|tab| marks_at.peek().at(tab)),
                tab: entry,
                stop,
                carried: false,
            };

            let Some(built) = rebuild(&mut step, &mut state, reading, marked, rows) else {
                return;
            };
            let planted = plant_caret(&mut step, &built, plant, marked);
            name_run(&built, marked);
            // Whether the entry is still on an open tab's trail, which is the one
            // question it answers: a listing that is no tab has no entry to ask it of.
            let is_open = |(id, stop): &Entry| docs.peek().contains(*id, stop);
            keep_spots(&step, &built, planted, marked, marks_at, &is_open);

            let at = At::of(&step, &built, code_at, state.known, top, height);
            state.known = at.known;

            // A move made and not seen arrive: issued again until a run finds the view
            // there, and left where it is on a run that switches tab or counts the rows
            // afresh, which chooses its own target below.
            if let Some(mut moving) = state.moving {
                if moving.arrived(&built, at.top.row) {
                    state.moving = None;
                } else if !step.switching && !step.rebuilt {
                    let to = moving.retry(&built);
                    state.moving = Some(moving);
                    if let Some(to) = to {
                        controller.scroll_to_y(scroll_to(to));
                    }
                    return;
                }
            }

            let target = target_of(&state, &step, &at, code_at, &is_open);

            if step.switching {
                state.tab = *tab;
                state.stop = Some(stop.clone());
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
            // A place arriving brings how far into its row it was left; otherwise the view
            // keeps its own, so a chunk landing above does not snap it to a row edge.
            let into = if step.switching {
                target.past.into
            } else {
                at.top.into
            };
            let target = Spot {
                past: TopRow {
                    into,
                    ..target.past
                },
                ..target
            };
            let Some(to) = top_of(&built, target) else {
                return;
            };
            if to != at.top || (step.rebuilt && step.switching) {
                controller.scroll_to_y(scroll_to(to));
                state.derived = spot_of(&built, to);
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
    // The layout is counted with the skeleton, on the worker; an answer lays only what is
    // held over it.
    let Some(layout) = reading.code.clone() else {
        if step.before.is_some() {
            rows.set(None);
        }
        return None;
    };
    let decoded = reading
        .held
        .iter()
        .map(|(&flat, stretched)| (flat, stretched.body()));
    let built = Arc::new(Built {
        rows: Rows::over(layout, decoded),
        reading: (*reading).clone(),
    });
    held.built = Some((Arc::downgrade(step.object), step.generation));
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
/// below its address (`Rows::body_row_for`). [`take_planting`] spends it before the row
/// is looked for, so an address in no stretch is dropped rather than left for ever.
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
    plant: State<Option<Planting>>,
    marked: State<Marks>,
) -> Option<(usize, Spot)> {
    // A planting for this listing, which draws an object's whole code and so places
    // every address it draws; a symbol's own is another listing's.
    let address = take_planting(plant, &step.stop.document)?.placed()?;
    let row = built.body_row_for(address)?;
    land_row(marked, file_at(built, row), row, Owed::by(Pane::Assembly));
    let first = built.row_for(address).unwrap_or(row);
    step.carried = true;
    Some((
        row,
        Spot {
            address,
            past: TopRow::at(row.saturating_sub(first)),
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
    if step.switching && !step.carried {
        return;
    }
    // A listing that is no tab keeps nothing, and neither does a tab this place has
    // already left.
    let Some(tab) = step.tab.as_ref().filter(|tab| is_open(tab)) else {
        return;
    };
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
        marks_at.write().remember(tab.clone(), kept);
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
        // still holding the tab and would put it straight back, and a listing that is no
        // tab has nowhere to put it.
        let tab = step.tab.as_ref().filter(|tab| is_open(tab));
        if let (Some(tab), Some(derived)) = (tab, at.derived) {
            places.write().remember(tab.clone(), derived);
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

/// The listing row of the first held instruction compiled from a line of the source
/// pane's run `pair`, if any is.
///
/// The pair whole, as [`file_at`] takes it: it reads both halves, and rows counted from
/// one reading against the stretches of another is exactly what [`Built`] exists to make
/// impossible.
fn row_compiled_from(built: &Built, pair: &Picked) -> Option<usize> {
    built.reading.held.iter().find_map(|(&flat, stretched)| {
        let studied = stretched.code.as_ref()?;
        let index = studied.first_paired(pair)?;
        Some(built.body_start(flat)? + studied.lanes.row_of(index))
    })
}

/// The file row `row` was compiled from, where it is an instruction of a stretch that has
/// decoded. [`None`] for every other row -- a label, a header, the bytes no symbol claims,
/// and any row of a stretch still guessed, none of which is anybody's line yet.
fn file_at(built: &Built, row: usize) -> Option<Arc<Path>> {
    match built.row(row) {
        Some(Row {
            stretch,
            kind: Kind::Instruction(index),
        }) => built
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
    top_of(rows, spot).map(|top| top.row)
}

/// [`row_of`] with how far into the row: where the top of a pane goes to put `spot` there.
fn top_of(rows: &Rows, spot: Spot) -> Option<TopRow> {
    let first = rows.row_for(spot.address)?;
    let past = spot.past;
    Some(
        TopRow {
            row: first + past.row,
            ..past
        }
        .within(rows.len()),
    )
}

/// The place row `row` stands for: its address and how many rows past that address's own
/// row it is.
pub(crate) fn spot_at(rows: &Rows, row: usize) -> Option<Spot> {
    spot_of(rows, TopRow::at(row))
}

/// [`spot_at`] with how far into the row: the place the top of a pane at `top` stands for.
fn spot_of(rows: &Rows, top: TopRow) -> Option<Spot> {
    let address = rows.address_of(top.row)?;
    let first = rows.row_for(address)?;
    Some(Spot {
        address,
        past: TopRow {
            row: top.row.saturating_sub(first),
            ..top
        },
    })
}

/// Ask for the stretches within [`BUFFER`] screens of the viewport that are not held,
/// nearest the reader first; and, before there is a skeleton, ask for that.
fn use_window(
    sectioned: Sectioned,
    controller: ScrollController,
    viewport: State<f32>,
    object: &Arc<Object>,
) {
    let (reading, mut window, rows) = (sectioned.reading, sectioned.window, sectioned.rows);
    // The object is a **dep** and never captured: the effect's closure is built on the
    // first render and never again, while the strip moving between two objects' code tabs
    // re-renders the pane rather than remounting it (freya keeps a same-key component's
    // hooks and replaces its props). A closure holding the first object went on asking for
    // nothing after the switch, and the second tab drew an empty listing for as long as it
    // was open. Through [`ByPtr`]: an `Object` has no `PartialEq`, and pointer identity is
    // what one is told from another by everywhere else in the UI.
    use_side_effect_with_deps(
        &ByPtr(object.clone()),
        move |ByPtr(object): &ByPtr<Object>| {
            // The four other inputs a scroll, a resize, an answer or a change of reading
            // brings. The reading is **read**, so the effect follows it: the pane mounts a
            // beat before the reading becomes its own -- `Active` is a memo and
            // `use_reading_of` runs off it -- and a run that found the reading about
            // something else asked for nothing, and nothing woke it until the pane was
            // resized. Reading them cannot loop: the one thing written here is the window,
            // and only when it changed.
            let (_, offset) = <(i32, i32)>::from(controller);
            let viewport = *viewport.read();
            let rows = rows.read().clone();
            let reading = reading.read();
            if !reading.is_about(object) {
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
                code: Some(rows.rows.layout().clone()),
                window: wanted,
            });
            window.set_if_modified(ask);
        },
    );
}
