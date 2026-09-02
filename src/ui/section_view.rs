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
pub(crate) struct CodeAt(pub(crate) State<Positions<Document, Spot>>);

/// The rows and the reading they were counted from, **together**: the rows are rebuilt by
/// an effect a pass after an answer lands, so for that one pass the reading the pane can
/// read is newer than the rows on screen -- and a stretch the answer let go of, still
/// drawn from the old rows, would find no bytes and no listing in the new reading. Drawn
/// from this pair, a row always finds what it was counted from; the newer reading is
/// drawn from once it has rows of its own.
struct Built {
    rows: Rows,
    reading: Reading,
}

impl std::ops::Deref for Built {
    type Target = Rows;

    fn deref(&self) -> &Rows {
        &self.rows
    }
}

/// What the rows are built from: the rows themselves with the stretches decoded, and the
/// three things a hover or a click changes.
#[derive(Clone)]
struct SectionRows {
    /// [`None`] until the skeleton has come: the list is mounted all the same, with no
    /// rows, so that the scroll controller is attached before the place-keeping effect
    /// moves it -- a `VirtualScrollView` resets the offset as it mounts.
    rows: Option<Arc<Built>>,
    object: Arc<Object>,
    focus: Option<LinePos>,
    pin: Option<LinePos>,
    /// The listing row under the pointer, and the edges of its instruction, for the
    /// gutter of every row those run through.
    hovered: Option<usize>,
    touching: Vec<PlacedEdge>,
    marks: Option<RowSelection>,
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
            && self.focus == other.focus
            && self.pin == other.pin
            && self.hovered == other.hovered
            && self.touching == other.touching
            && self.marks == other.marks
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
            format!("{address:016X} <{name}>:")
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
        Some(Row::Empty { .. }) | Some(Row::Separator { .. }) | None => String::new(),
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
    selected: bool,
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
        let marked = use_consume::<Marked>().0;
        let shift = use_consume::<Shift>().0;
        let ctrl = use_consume::<Ctrl>().0;
        let mut hovering = use_state(|| false);
        let open = use_open();
        let history = use_consume::<Hist>().0;
        let row = self.row;
        let opens = self.opens.clone();
        // A label lights as a link only while Ctrl is held, which is when a press is one.
        let link = opens.is_some() && ctrl();
        let background = row_background(hovering() && link, false, false, self.selected);

        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .width(Size::fill())
            .height(Size::px(code_row_height()))
            .padding(Gaps::new_symmetric(0.0, 3.0))
            .assembly_font()
            .background(background)
            .on_pointer_down(move |e: Event<PointerEventData>| {
                if e.button() == Some(MouseButton::Left) {
                    mark_press(marked, *shift.peek(), Pane::Assembly, row);
                }
            })
            .on_pointer_over(move |_| {
                hovering.set_if_modified(true);
                mark_drag(marked, Pane::Assembly, row);
            })
            .on_pointer_out(move |_| hovering.set_if_modified(false))
            .maybe(opens.is_some(), move |el| {
                el.on_press(move |_| {
                    if !*ctrl.peek() {
                        return;
                    }
                    if let Some(symbol) = opens.clone() {
                        activate(
                            open,
                            history,
                            Some(Document::Assembly(Selection::Symbol(symbol))),
                            Visit::Went,
                        );
                    }
                })
            })
            // The gutter's width, so the address column starts where it does on an
            // instruction row.
            .child(rect().width(Size::px(gutter_width(lanes::MAX_LANES))))
            .child(
                label()
                    .text(match self.address {
                        Some(address) => format!("{address:016X} "),
                        None => String::new(),
                    })
                    .min_width(Size::px(200.0))
                    .color(palette().address_fg)
                    .max_lines(1),
            )
            .maybe_child(self.mark.map(|mark| {
                label()
                    // Non-breaking: the text engine trims a trailing space off a label.
                    .text(format!("{mark}\u{a0}"))
                    .color(palette().keyword_fg)
                    .font_weight(FontWeight::BOLD)
                    .max_lines(1)
            }))
            .child(
                label()
                    .text(self.text.clone())
                    .color(if hovering() && link {
                        palette().name_hover_fg
                    } else {
                        self.color
                    })
                    .font_weight(if self.bold {
                        FontWeight::BOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .max_lines(1),
            )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// One of the rows a stretch nobody has decoded is guessed to take: empty space, and the
/// mark handlers so a sweep across it is not cut.
#[derive(Clone, PartialEq)]
struct EmptyRow {
    row: usize,
    selected: bool,
    key: DiffKey,
}

impl KeyExt for EmptyRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for EmptyRow {
    fn render(&self) -> impl IntoElement {
        let marked = use_consume::<Marked>().0;
        let shift = use_consume::<Shift>().0;
        let row = self.row;

        rect()
            .width(Size::fill())
            .height(Size::px(code_row_height()))
            .background(row_background(false, false, false, self.selected))
            .on_pointer_down(move |e: Event<PointerEventData>| {
                if e.button() == Some(MouseButton::Left) {
                    mark_press(marked, *shift.peek(), Pane::Assembly, row);
                }
            })
            .on_pointer_over(move |_| mark_drag(marked, Pane::Assembly, row))
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
    Label(u64, usize),
    Empty(u64, usize),
    Insn(u64),
    Sep(u64),
    Gap(u64),
}

/// The listing of one object's code.
#[derive(Clone)]
pub(crate) struct SectionList {
    pub(crate) document: Document,
    pub(crate) object: Arc<Object>,
}

impl PartialEq for SectionList {
    fn eq(&self, other: &Self) -> bool {
        self.document == other.document && Arc::ptr_eq(&self.object, &other.object)
    }
}

impl Component for SectionList {
    fn render(&self) -> impl IntoElement {
        let reading_state = use_consume::<Sections>().0;
        let window = use_consume::<Window>().0;
        // Reading it is what redraws the listing as answers land.
        let reading = reading_state.read().clone();
        let focus = use_consume::<Focused>()
            .0
            .read()
            .as_ref()
            .map(|focus| focus.at.clone());
        let pinned = use_consume::<Pinned>().0;
        let pin = pinned.read().as_ref().map(|pin| pin.at.clone());
        let marked = use_consume::<Marked>().0;
        let marks = marked_rows(marked, Pane::Assembly);
        let docs = use_consume::<OpenDocs>().0;
        let code_at = use_consume::<CodeAt>().0;
        let a11y = use_a11y();
        let controller = use_scroll_controller(ScrollConfig::default);
        let mut viewport = use_state(|| 0.0f32);
        let hover = use_state(|| None::<usize>);
        // The rows, produced by the place-keeping effect and rendered from here, so that
        // new rows and the offset that keeps the reader's place under them land together.
        let rows = use_state(|| None::<Arc<Built>>);

        let object = self.object.clone();
        let about = reading.is_about(&object);
        let generation = if about {
            Some(reading.generation)
        } else {
            None
        };
        use_kept_place(
            code_at,
            move |document: &Document| docs.peek().id_of(document).is_some(),
            // The scroll a pin made on the source side owes this pane: the row of the
            // instruction compiled from that line, in whichever held stretch has one.
            // Left owed while none does -- the stretch may not be decoded yet, and the
            // answer that decodes it wakes this again.
            move |controller: &mut ScrollController, built: &Built| {
                let Some(at) = owed_reveal(pinned, Pane::Assembly) else {
                    return false;
                };
                let Some(row) = row_compiled_from(built, &built.reading, &at) else {
                    return false;
                };
                reveal_made(pinned, Pane::Assembly);
                reveal_row(controller, *viewport.peek(), row);
                true
            },
            reading_state,
            rows,
            controller,
            &self.document,
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

        let touching = hover()
            .and_then(|row| {
                let built = built.as_ref()?;
                match built.row(row)? {
                    Row::Instruction { stretch, index } => {
                        let studied = built.reading.held.get(&stretch)?.code.as_ref()?;
                        Some(studied.lanes.touching(index))
                    }
                    _ => None,
                }
            })
            .unwrap_or_default();

        let on_key_down = {
            let rows = built.clone();
            on_listing_key(marked, Pane::Assembly, length, move |row| {
                rows.as_ref()
                    .map(|built| row_line(built, &built.reading, row))
                    .unwrap_or_default()
            })
        };

        rect()
            .expanded()
            .a11y_id(a11y)
            .a11y_focusable(true)
            .on_pointer_down(move |_| a11y.request_focus())
            .on_key_down(on_key_down)
            .on_sized(move |e: Event<SizedEventData>| viewport.set_if_modified(e.area.height()))
            .child(
                VirtualScrollView::new_with_data_controlled(
                    SectionRows {
                        rows: built,
                        object,
                        focus,
                        pin,
                        hovered: hover(),
                        touching,
                        marks,
                    },
                    move |i, data: &SectionRows| build_row(i, data, hover, controller, viewport),
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
    hover: State<Option<usize>>,
    controller: ScrollController,
    viewport: State<f32>,
) -> Element {
    let Some(rows) = data.rows.as_ref() else {
        return rect().height(Size::px(code_row_height())).into_element();
    };
    let selected = data.marks.is_some_and(|run| run.contains(i));
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
            selected,
            opens,
            mark,
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
                .map(|symbol| format!("<{}>:", symbol.display()))
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
        Some(Row::Empty { stretch, index }) => EmptyRow {
            row: i,
            selected,
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
            let at = asm.position(index);
            let (focused, pinned) = match &at {
                Some(at) => (
                    data.focus.as_ref() == Some(at),
                    data.pin.as_ref() == Some(at),
                ),
                None => (false, false),
            };
            let lit = if data.hovered.and_then(|row| rows.row(row)).is_some_and(
                |hovered| matches!(hovered, Row::Instruction { stretch: s, .. } if s == stretch),
            ) {
                lanes::lit(&data.touching, index)
            } else {
                Lit::default()
            };
            InstructionRow {
                arrows: RowArrows {
                    lanes: asm.lanes.row(index),
                    lit,
                },
                data: asm,
                index,
                row: i,
                hover,
                controller,
                viewport,
                focused,
                pinned,
                selected,
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
            let mut lit = if data.hovered.and_then(|row| rows.row(row)).is_some_and(
                |hovered| matches!(hovered, Row::Instruction { stretch: s, .. } if s == stretch),
            ) {
                lanes::lit(&data.touching, below)
            } else {
                Lit::default()
            };
            lit.corner = false;
            SeparatorRow {
                row: i,
                selected,
                width: lanes::MAX_LANES,
                arrows: RowArrows {
                    lanes: asm.lanes.boundary(below),
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
fn use_kept_place(
    mut places: State<Positions<Document, Spot>>,
    is_open: impl Fn(&Document) -> bool + 'static,
    mut reveal: impl FnMut(&mut ScrollController, &Built) -> bool + 'static,
    reading: State<Reading>,
    mut rows: State<Option<Arc<Built>>>,
    mut controller: ScrollController,
    tab: &Document,
    generation: Option<u64>,
) {
    /// What the hook remembers between runs, none of it rendered from.
    #[derive(Default)]
    struct Held {
        tab: Option<Document>,
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
        move |(tab, generation): &(Document, Option<u64>)| {
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
            let built = if rebuilt {
                let reading = reading.peek();
                let Some(code) = reading.code.clone() else {
                    if rows.peek().is_some() {
                        rows.set(None);
                    }
                    return;
                };
                let built = Arc::new(Built {
                    rows: Rows::new(code, |flat| reading.body(flat)),
                    reading: (*reading).clone(),
                });
                state.built = Some(generation);
                built
            } else {
                match rows.peek().clone() {
                    Some(built) => built,
                    None => return,
                }
            };

            let row = ((top / height) as usize).min(built.len().saturating_sub(1));
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
                if derived == Some(moving) || state.tries >= MOVE_TRIES {
                    state.moving = None;
                } else if !switching && !rebuilt {
                    state.tries += 1;
                    if let Some(to) = built.row_for(moving.address) {
                        let to = (to + moving.rows).min(built.len().saturating_sub(1));
                        controller.scroll_to_y(to_offset(to as f64));
                    }
                    return;
                }
            }

            // Where this run has to move the view to, if anywhere.
            let target: Option<Spot> = if switching {
                known
            } else if rebuilt {
                // The rows changed under the reader: back to the place they were at.
                state.derived.or(known)
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
                let Some(to) = built.row_for(target.address) else {
                    return;
                };
                let to = (to + target.rows).min(built.len().saturating_sub(1));
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
/// neighbours: the object's code tab, opened on that address, with the line the
/// instruction was compiled from pinned in both panes where it has one.
///
/// The place is written **before** the tab is opened, the order a restore uses, so the
/// pane's first run finds it; when the code tab is already on top the write is what moves
/// the view, `use_kept_place` reading the map for exactly this. The pin goes through
/// `land`, which knows whether the tab is on top.
pub(crate) fn show_in_code(
    open: Open,
    history: State<History>,
    pinned: State<Option<Pin>>,
    landing: State<Option<Landing>>,
    mut places: State<Positions<Document, Spot>>,
    object: Arc<Object>,
    address: u64,
    at: Option<LinePos>,
) {
    let code = Document::Code(object);
    places
        .write()
        .remember(code.clone(), Spot { address, rows: 0 });
    match at {
        Some(at) => land(open, history, pinned, landing, code, at),
        None => activate(open, history, Some(code), Visit::Went),
    }
}

/// Open `symbol`'s own tab from a row of it read among its neighbours, landing on the
/// line the row was compiled from where it has one: `show_in_code`'s door the other way.
pub(crate) fn open_as_symbol(
    open: Open,
    history: State<History>,
    pinned: State<Option<Pin>>,
    landing: State<Option<Landing>>,
    symbol: Symbol,
    at: Option<LinePos>,
) {
    let tab = Document::Assembly(Selection::Symbol(symbol));
    match at {
        Some(at) => land(open, history, pinned, landing, tab, at),
        None => activate(open, history, Some(tab), Visit::Went),
    }
}

/// The listing row of the first held instruction compiled from `at`, if any is.
fn row_compiled_from(rows: &Rows, reading: &Reading, at: &LinePos) -> Option<usize> {
    reading.held.iter().find_map(|(&flat, stretched)| {
        let studied = stretched.code.as_ref()?;
        let assembly = studied.assembly.as_ref()?;
        let info = studied.lines.info.as_ref()?;
        let index = assembly.instructions.iter().position(|instruction| {
            info.row_at(instruction.address).is_some_and(|row| {
                row.line == Some(at.line)
                    && row
                        .file
                        .and_then(|file| info.files().get(file))
                        .is_some_and(|file| *file == at.file)
            })
        })?;
        Some(rows.body_start(flat)? + studied.lanes.row_of(index))
    })
}

/// The place row `row` stands for: its address and how many rows past that address's own
/// row it is.
fn spot_at(rows: &Rows, row: usize) -> Option<Spot> {
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
