//! The rows a listing of an object's whole code is made of, before and after any of it is
//! decoded. Framework-free: a `VirtualScrollView` builds row *n* knowing nothing but *n*
//! and has to be told its length up front, so this is the one place that says how many
//! rows there are and what row *n* is.
//!
//! The listing is [`CodeListing`]'s stretches, one after another: a **rule** row over each
//! stretch but the first with a blank under it, a **header** row where a section starts
//! with a blank under that, a **label** row per symbol at a stretch's address, then the
//! stretch's body. A body that has been decoded is the symbol's instruction rows and block
//! separators -- exactly the rows its own listing draws, `Lanes` and all -- followed by
//! its gap as rows of hex bytes, under a **cut** row where the gap is the rest of a
//! symbol whose extent was capped, so the listing does not read as the function ending
//! there; where the decode found no instructions, no backend reading the architecture,
//! the whole stretch is those bytes. A body nobody has decoded yet is a run of **empty**
//! rows, as many as its bytes suggest, so the listing has its whole length from the first
//! frame and the reader scrolls over empty space that fills in as the worker reaches it. The length therefore starts estimated and settles;
//! keeping the reader's row still while it does is the view's job, and
//! [`Rows::address_of`] / [`Rows::row_for`] are what it does it with, an address being
//! the one name for a row that survives the rows around it changing.
//!
//! Every address here is **placed** (`Placed::place`): the section's own plus where the
//! object's layout put it, so two functions of a relocatable object, both at 0 in the file,
//! draw at two addresses and the listing reads as one.

use crate::counter;
use crate::lanes::Lanes;
use analysis::{Assembly, Bias, CodeListing, Gap, GapKind, Place, Placed, PlacedAddress, Stretch};
use std::{ops::Range, sync::Arc};

/// How many of a gap's bytes one row draws.
pub const GAP_BYTES_PER_ROW: u64 = 16;

/// How many bytes an undecoded symbol is guessed to spend per row: x86's mean instruction
/// length, near enough. Only the estimate rides on it; nothing is drawn by it.
pub const ESTIMATED_BYTES_PER_ROW: u64 = 4;

/// What a decoded stretch draws: the symbol's listing, its lanes, and the bytes left
/// between its extent and the next label. Addresses in `gap` are the section's own, as the
/// crate states them; the bias is the stretch's ([`Rows::bias`]).
#[derive(Clone)]
pub struct Body {
    /// [`None`] for the leading stretch of a section, which has no symbol, and for a symbol
    /// with no bytes.
    pub assembly: Option<Arc<Assembly>>,
    pub lanes: Arc<Lanes>,
    /// Widened to the whole stretch where nothing was decoded ([`BodyRows::of`]).
    pub gap: Option<Gap>,
}

impl Body {
    /// A stretch's body from what the crate's decode gave for it: the lanes laid out over
    /// the listing, as the worker lays them out for the pane ([`Lanes::over`]).
    pub fn of(assembly: Option<Arc<Assembly>>, gap: Option<Gap>) -> Body {
        let lanes = Lanes::over(assembly.as_deref());
        Body {
            assembly,
            lanes,
            gap,
        }
    }

    /// The instruction rows and the separators between them. The lanes were laid out over
    /// this body's own assembly, so the count is theirs.
    fn listing_rows(&self) -> usize {
        self.lanes.listing_rows()
    }

    fn gap_rows(&self) -> usize {
        self.gap.as_ref().map_or(0, gap_rows)
    }

    /// The rows over the gap's bytes: the cut row, where the gap is a cut, and none else.
    fn cut_rows(&self) -> usize {
        self.gap.as_ref().map_or(0, cut_rows)
    }
}

/// How many rows `gap` takes: its cut row, if it has one, then sixteen bytes each, the last
/// one short.
fn gap_rows(gap: &Gap) -> usize {
    let bytes = gap.range.start.bytes_to_saturating(gap.range.end);
    let rows: usize = bytes
        .div_ceil(GAP_BYTES_PER_ROW)
        .try_into()
        .unwrap_or(usize::MAX);
    rows.saturating_add(cut_rows(gap))
}

/// One for a gap that is the rest of a capped symbol, which the cut row stands over.
fn cut_rows(gap: &Gap) -> usize {
    usize::from(gap.kind == GapKind::Cut)
}

/// One row of the listing: the stretch it belongs to, and what it draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Row {
    /// The stretch the row is in, by flat index.
    pub stretch: usize,
    pub kind: Kind,
}

/// What a row draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Where the stretch's section starts: its name.
    Header,
    /// The rule over a stretch: what a reader tells one function from the next by, drawn
    /// as the rule between two basic blocks is.
    Rule,
    /// A blank row: the one under the rule, and -- `under` -- the one between a section
    /// header and the first label beneath it. The rule is not drawn against the name it
    /// separates, and a header is not drawn against its first label.
    Space { under: bool },
    /// The `index`th symbol at the stretch's address.
    Label(usize),
    /// One of the rows a stretch nobody has decoded is guessed to take.
    Empty(usize),
    /// The `index`th instruction of the stretch's symbol.
    Instruction(usize),
    /// The block separator above the instruction `below`.
    Separator { below: usize },
    /// The row over a gap that is the rest of a capped symbol ([`GapKind::Cut`]): what
    /// says the listing stopped at the cap and not where the function ends.
    Cut,
    /// The `index`th row of sixteen bytes of the stretch's gap.
    Gap(usize),
}

/// One stretch's share of the rows: what it draws, and where in the listing it draws it.
///
/// [`Layout`] holds one per stretch, and [`Rows`] one per decoded stretch laid over it.
/// It is also the way in for a reader holding **one** stretch and no listing: the search
/// that walks an object's code decodes a stretch, reads it and lets it go, and a
/// [`Layout`] per stretch would count the whole skeleton each time
/// (`ui/section_view.rs`).
pub struct StretchRows {
    /// The placed address the stretch starts at, and how many bytes it covers.
    start: PlacedAddress,
    bytes: u64,
    bias: Bias,
    /// The rule over the stretch and the blank under it, which every stretch has but the
    /// listing's first.
    space: bool,
    header: bool,
    labels: usize,
    body: BodyRows,
}

enum BodyRows {
    Estimated(usize),
    Decoded(Body),
}

impl BodyRows {
    /// What a stretch's body is made of: what was decoded for it, or a guess at how many
    /// rows it will take where nothing has been.
    ///
    /// A decode that found **no instructions** -- an architecture no backend reads --
    /// leaves no byte of the stretch an instruction's, so the gap is widened to the whole
    /// stretch and the body is those bytes. Without that the body would be no rows at
    /// all. Nothing was decoded, so nothing was cut: the widened gap is plain bytes.
    fn of(stretch: &Stretch, decoded: Option<Body>) -> Self {
        let Some(mut body) = decoded else {
            let rows = Self::estimate(stretch_bytes(stretch), !stretch.symbols.is_empty());
            return BodyRows::Estimated(rows);
        };
        if body.listing_rows() == 0 {
            let end = body
                .gap
                .as_ref()
                .map_or(stretch.range.end, |gap| gap.range.end);
            body.gap = Some(Gap {
                range: stretch.range.start..end,
                kind: GapKind::Bytes,
            });
        }
        BodyRows::Decoded(body)
    }

    /// How many rows to guess for a body nobody has decoded: never none, so every label
    /// has something under it and an address inside the stretch has a row.
    fn estimate(bytes: u64, labelled: bool) -> usize {
        let per_row = if labelled {
            ESTIMATED_BYTES_PER_ROW
        } else {
            // No symbol, so no instructions either: the whole stretch is a gap and will
            // be drawn as one.
            GAP_BYTES_PER_ROW
        };
        bytes
            .div_ceil(per_row)
            .max(1)
            .try_into()
            .unwrap_or(usize::MAX)
    }
}

counter!(
    /// Test-only: how many stretches this thread has counted the rows of. An answer
    /// landing counts the stretches held and not the listing, which pins that.
    pub fn stretches_counted() = STRETCHES_COUNTED
);

impl StretchRows {
    /// The rows stretch `flat` of the listing draws, given whatever was decoded for it.
    /// `place` says whether it opens its section, which is a header row, and `flat`
    /// whether anything is drawn above it -- every stretch but the listing's first wears
    /// a rule.
    pub fn of(
        placed: &Placed,
        stretch: &Stretch,
        place: Place,
        flat: usize,
        decoded: Option<Body>,
    ) -> StretchRows {
        #[cfg(test)]
        STRETCHES_COUNTED.set(STRETCHES_COUNTED.get() + 1);
        StretchRows {
            start: placed.place(stretch.range.start),
            bytes: stretch_bytes(stretch),
            bias: placed.bias(),
            space: flat > 0,
            header: place.stretch == 0,
            labels: stretch.symbols.len(),
            body: BodyRows::of(stretch, decoded),
        }
    }

    /// What the rows are drawn from: what was decoded, with its gap as the rows draw it
    /// -- widened to the whole stretch where the decode found no instructions
    /// ([`BodyRows::of`]). [`None`] where nothing has been decoded.
    pub fn body(&self) -> Option<&Body> {
        match &self.body {
            BodyRows::Decoded(body) => Some(body),
            BodyRows::Estimated(_) => None,
        }
    }

    /// Every row the stretch draws, in the order it draws them. [`Rows::row`] answers
    /// this one row at a time for the listing as a whole, out of this very layout, so a
    /// row kind added to [`Kind`] reaches both readers together.
    pub fn kinds(&self) -> impl Iterator<Item = Kind> + '_ {
        self.heading()
            .chain((0..self.body_rows()).filter_map(|local| self.body_kind(local)))
    }

    fn body_rows(&self) -> usize {
        match &self.body {
            BodyRows::Estimated(rows) => *rows,
            BodyRows::Decoded(body) => body.listing_rows() + body.gap_rows(),
        }
    }

    /// The rows above the body, in the order they are drawn: the rule over the stretch
    /// and its blank, a section's header and the blank under it, and one row per label.
    /// The one place they are laid out, so counting them and drawing them cannot
    /// disagree.
    fn heading(&self) -> impl Iterator<Item = Kind> + '_ {
        let space = self
            .space
            .then_some([Kind::Rule, Kind::Space { under: false }]);
        let header = self
            .header
            .then_some([Kind::Header, Kind::Space { under: true }]);
        space
            .into_iter()
            .flatten()
            .chain(header.into_iter().flatten())
            .chain((0..self.labels).map(Kind::Label))
    }

    /// How many rows stand above the body: four at most, plus one per label.
    fn above(&self) -> usize {
        self.heading().count()
    }

    fn rows(&self) -> usize {
        self.above() + self.body_rows()
    }

    /// What row `local` of the body draws, counted from the body's first row. [`None`]
    /// for a separator with no instruction under it: [`Lanes`] lays out none, and no row
    /// is better than one labelled with the wrong instruction.
    fn body_kind(&self, local: usize) -> Option<Kind> {
        Some(match &self.body {
            BodyRows::Estimated(_) => Kind::Empty(local),
            BodyRows::Decoded(body) => {
                let listing = body.listing_rows();
                if local < listing {
                    match body.lanes.instruction_at(local) {
                        Some(index) => Kind::Instruction(index),
                        None => Kind::Separator {
                            below: body.lanes.instruction_at(local + 1)?,
                        },
                    }
                } else {
                    let into = local - listing;
                    match into.checked_sub(body.cut_rows()) {
                        Some(index) => Kind::Gap(index),
                        None => Kind::Cut,
                    }
                }
            }
        })
    }
}

/// How many bytes a stretch covers. Saturating: a range stated backwards is no bytes,
/// not a panic.
fn stretch_bytes(stretch: &Stretch) -> u64 {
    stretch.range.start.bytes_to_saturating(stretch.range.end)
}

/// Where each of `counts` starts once they are laid end to end, with one more entry
/// holding the total. A count of nought shares the start of whatever follows it, which is
/// what the `partition_point` over the result has to step over ([`Flat::place`],
/// [`Rows::stretch_of`]).
fn starts(counts: impl ExactSizeIterator<Item = usize>) -> Vec<usize> {
    let mut starts = Vec::with_capacity(counts.len() + 1);
    let mut total = 0;
    for count in counts {
        starts.push(total);
        total += count;
    }
    starts.push(total);
    starts
}

/// A listing's stretches numbered end to end: every section's, in placed order. One
/// number is what a list of stretches wants, so a flat index is the currency between the
/// view's window, the worker and [`Rows`], and this is the one place that maps it to the
/// crate's [`Place`] and back. Both ways are questions about the listing's shape and
/// nothing else.
pub struct Flat {
    code: Arc<CodeListing>,
    /// `sections[s]` is the flat index of section `s`'s first stretch; one more entry
    /// holds the stretch count.
    sections: Vec<usize>,
}

impl Flat {
    pub fn new(code: Arc<CodeListing>) -> Self {
        let sections = starts(
            code.sections()
                .iter()
                .map(|placed| placed.listing.stretches().len()),
        );
        Self { code, sections }
    }

    pub fn code(&self) -> &Arc<CodeListing> {
        &self.code
    }

    /// How many stretches the listing has. Not a row count: [`Rows::len`] is that.
    pub fn count(&self) -> usize {
        *self.sections.last().unwrap_or(&0)
    }

    /// The stretch at flat index `flat`, as the crate names it.
    pub fn place(&self, flat: usize) -> Option<Place> {
        if flat >= self.count() {
            return None;
        }
        // The last section starting at or before `flat`, which steps over a section with
        // no stretches; `sections` has one entry past the sections.
        let after = self.sections.partition_point(|&first| first <= flat);
        let section = after.checked_sub(1)?;
        Some(Place {
            section,
            stretch: flat - self.sections[section],
        })
    }

    /// The stretch at flat index `flat`: where the crate names it, and the stretch
    /// itself. One call, so nothing indexes the listing with a place it was handed a
    /// line earlier.
    pub fn stretch(&self, flat: usize) -> Option<(Place, &Stretch)> {
        let place = self.place(flat)?;
        let stretch = self
            .code
            .sections()
            .get(place.section)?
            .listing
            .stretches()
            .get(place.stretch)?;
        Some((place, stretch))
    }

    /// The flat index of a place, if the listing has it.
    pub fn index(&self, place: Place) -> Option<usize> {
        let first = *self.sections.get(place.section)?;
        let end = *self.sections.get(place.section + 1)?;
        let flat = first.checked_add(place.stretch)?;
        (flat < end).then_some(flat)
    }
}

/// The rows of one object's code listing with nothing decoded: every stretch's estimate,
/// counted once, on the worker, and the skeleton the view and the worker share. [`Rows`] lays the stretches that have been decoded over it,
/// so an answer landing costs the stretches held and not every stretch of the object --
/// which in the app's own binary is some 190k.
pub struct Layout {
    /// The listing, and its stretches numbered: what a row's `stretch` indexes.
    flat: Flat,
    /// One per stretch, in flat order, none of them decoded.
    stretches: Vec<StretchRows>,
    /// `starts[i]` is the first row of stretch `i`; one more entry holds the total.
    starts: Vec<usize>,
}

impl Layout {
    pub fn new(code: Arc<CodeListing>) -> Self {
        let flat = Flat::new(code);
        let mut stretches = Vec::with_capacity(flat.count());
        for (section, placed) in flat.code().sections().iter().enumerate() {
            for (index, stretch) in placed.listing.stretches().iter().enumerate() {
                let place = Place {
                    section,
                    stretch: index,
                };
                let at = stretches.len();
                stretches.push(StretchRows::of(placed, stretch, place, at, None));
            }
        }
        let starts = starts(stretches.iter().map(StretchRows::rows));
        Self {
            flat,
            stretches,
            starts,
        }
    }

    pub fn code(&self) -> &Arc<CodeListing> {
        self.flat.code()
    }

    pub fn flat(&self) -> &Flat {
        &self.flat
    }
}

/// A decoded stretch laid over the [`Layout`]: its rows, and the row after its last.
struct Decoded {
    flat: usize,
    rows: StretchRows,
    end: usize,
}

/// Every row of one object's code listing, worked out from the skeleton and whichever
/// stretches have been decoded.
pub struct Rows {
    layout: Arc<Layout>,
    /// The decoded stretches, in flat order.
    decoded: Vec<Decoded>,
}

impl Rows {
    /// The rows for `code`, with `decoded` answering for the stretches -- by flat index --
    /// that have been. Asks about every stretch: the app lays what it holds over a
    /// [`Layout`] it keeps instead ([`Rows::over`]).
    #[cfg(test)]
    pub fn new(code: Arc<CodeListing>, decoded: impl Fn(usize) -> Option<Body>) -> Self {
        let layout = Arc::new(Layout::new(code));
        let bodies: Vec<(usize, Body)> = (0..layout.flat.count())
            .filter_map(|flat| Some((flat, decoded(flat)?)))
            .collect();
        Self::over(layout, bodies)
    }

    /// `layout`'s rows with `decoded` -- bodies by flat index -- laid over it. Costs the
    /// stretches decoded, not the listing.
    pub fn over(layout: Arc<Layout>, decoded: impl IntoIterator<Item = (usize, Body)>) -> Self {
        let mut bodies: Vec<(usize, Body)> = decoded.into_iter().collect();
        bodies.sort_by_key(|(flat, _)| *flat);
        bodies.dedup_by_key(|(flat, _)| *flat);
        let mut rows = Self {
            layout,
            decoded: Vec::with_capacity(bodies.len()),
        };
        for (flat, body) in bodies {
            let Some((place, stretch)) = rows.layout.flat.stretch(flat) else {
                continue;
            };
            let Some(placed) = rows.code().sections().get(place.section) else {
                continue;
            };
            let stretch_rows = StretchRows::of(placed, stretch, place, flat, Some(body));
            // Every stretch decoded so far is before this one, so its start is final.
            let end = rows.start(flat).saturating_add(stretch_rows.rows());
            rows.decoded.push(Decoded {
                flat,
                rows: stretch_rows,
                end,
            });
        }
        rows
    }

    /// The layout the rows were laid over, for the next answer's rows to be laid over too.
    pub fn layout(&self) -> &Arc<Layout> {
        &self.layout
    }

    /// The first row of stretch `flat`, or the listing's length for the flat index one
    /// past the last: the layout's, moved by what the decoded stretches before it changed.
    fn start(&self, flat: usize) -> usize {
        let base = self
            .layout
            .starts
            .get(flat)
            .copied()
            .unwrap_or(self.layout_len());
        let before = self.decoded.partition_point(|decoded| decoded.flat < flat);
        let Some(last) = before.checked_sub(1).map(|at| &self.decoded[at]) else {
            return base;
        };
        // What the layout counts from the end of that stretch to this one's start, which
        // no decoded stretch between the two has changed.
        let end_then = self.layout.starts[last.flat + 1];
        last.end.saturating_add(base.saturating_sub(end_then))
    }

    fn layout_len(&self) -> usize {
        *self.layout.starts.last().unwrap_or(&0)
    }

    /// Stretch `flat`'s rows: the decoded ones where it has been, else the layout's.
    fn stretch_rows(&self, flat: usize) -> Option<&StretchRows> {
        match self
            .decoded
            .binary_search_by_key(&flat, |decoded| decoded.flat)
        {
            Ok(at) => Some(&self.decoded[at].rows),
            Err(_) => self.layout.stretches.get(flat),
        }
    }

    pub fn code(&self) -> &Arc<CodeListing> {
        self.layout.code()
    }

    /// The placed section stretch `flat` is in.
    pub fn placed_of(&self, flat: usize) -> Option<&analysis::Placed> {
        self.code().sections().get(self.place(flat)?.section)
    }

    /// How many rows the listing has, estimates included.
    pub fn len(&self) -> usize {
        self.start(self.layout.flat.count())
    }

    /// The stretch at flat index `flat`, as the crate names it.
    pub fn place(&self, flat: usize) -> Option<Place> {
        self.layout.flat.place(flat)
    }

    /// The stretch at flat index `flat`.
    pub fn stretch(&self, flat: usize) -> Option<&Stretch> {
        Some(self.layout.flat.stretch(flat)?.1)
    }

    /// The row a stretch's body starts at, after its header and labels: what its lanes'
    /// rows are relative to.
    pub fn body_start(&self, flat: usize) -> Option<usize> {
        let stretch = self.stretch_rows(flat)?;
        Some(self.start(flat) + stretch.above())
    }

    /// What was decoded for stretch `flat`, if anything was.
    pub fn body(&self, flat: usize) -> Option<&Body> {
        self.stretch_rows(flat)?.body()
    }

    /// What the stretch adds to its symbol's own addresses.
    pub fn bias(&self, flat: usize) -> Option<Bias> {
        Some(self.stretch_rows(flat)?.bias)
    }

    /// The placed address stretch `flat` starts at.
    pub fn start_of(&self, flat: usize) -> Option<PlacedAddress> {
        Some(self.stretch_rows(flat)?.start)
    }

    /// Which stretch row `row` is in.
    fn stretch_of(&self, row: usize) -> Option<usize> {
        if row >= self.len() {
            return None;
        }
        // The last start at or before `row`; `starts` has one entry past the stretches.
        // The last stretch starting at or before `row`, which steps over a stretch of no
        // rows; `start` answers for one flat index past the stretches.
        let count = self.layout.flat.count();
        let (mut low, mut high) = (0, count + 1);
        while low < high {
            let middle = low + (high - low) / 2;
            if self.start(middle) <= row {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low.checked_sub(1).filter(|&flat| flat < count)
    }

    /// What row `row` draws.
    pub fn row(&self, row: usize) -> Option<Row> {
        let flat = self.stretch_of(row)?;
        let stretch = self.stretch_rows(flat)?;
        let local = row - self.start(flat);
        let kind = match stretch.heading().nth(local) {
            Some(kind) => kind,
            None => stretch.body_kind(local - stretch.above())?,
        };
        Some(Row {
            stretch: flat,
            kind,
        })
    }

    /// The placed address row `row` stands for: what names it once the rows around it
    /// have changed. A header is its section's start, a rule, a blank row and a label its
    /// stretch's, an empty row
    /// its share of the stretch's bytes, an instruction its own, a separator the
    /// instruction below it, a cut row the gap row below it, and a gap row its first byte.
    pub fn address_of(&self, row: usize) -> Option<PlacedAddress> {
        let Row {
            stretch: flat,
            kind,
        } = self.row(row)?;
        let stretch = self.stretch_rows(flat)?;
        Some(match kind {
            Kind::Header => self.placed_of(flat)?.range().start,
            Kind::Rule | Kind::Space { .. } | Kind::Label(_) => stretch.start,
            Kind::Empty(index) => {
                let BodyRows::Estimated(rows) = &stretch.body else {
                    return None;
                };
                // Rounded up, so that `row_for` lands back on this row: the row an
                // address falls in is worked out by rounding down.
                let share = (index as u64)
                    .saturating_mul(stretch.bytes)
                    .div_ceil(*rows as u64);
                stretch.start.saturating_add(share)
            }
            Kind::Instruction(index) | Kind::Separator { below: index } => {
                let assembly = self.body(flat)?.assembly.as_ref()?;
                let address = assembly.instructions.get(index)?.address;
                self.placed_of(flat)?.place(address)
            }
            Kind::Cut => {
                let gap = self.body(flat)?.gap.as_ref()?;
                self.placed_of(flat)?.place(gap.range.start)
            }
            Kind::Gap(index) => {
                let gap = self.body(flat)?.gap.as_ref()?;
                self.placed_of(flat)?
                    .place(gap.range.start)
                    .saturating_add((index as u64).saturating_mul(GAP_BYTES_PER_ROW))
            }
        })
    }

    /// The row a placed address is drawn in: a stretch's **first** row for its start --
    /// the rule over it, or its header or label where it has none, since the first
    /// instruction shares that address with them and a reader landing on an address is
    /// better shown the label over it -- else the
    /// instruction, gap row or empty row covering it. [`None`] for an address in no
    /// stretch: between two sections, or outside every one. A view keeping its place by
    /// an address keeps how many rows past this it was, so the top row being a stretch's
    /// first instruction comes back as that row and not as the label two rows up.
    pub fn row_for(&self, address: PlacedAddress) -> Option<usize> {
        let flat = self.layout.flat.index(self.code().at(address)?)?;
        let stretch = self.stretch_rows(flat)?;
        let first = self.start(flat);
        if address <= stretch.start {
            return Some(first);
        }
        let body = self.body_start(flat)?;
        let into = stretch.start.bytes_to(address)?;
        match &stretch.body {
            BodyRows::Estimated(rows) => {
                let share = into
                    .saturating_mul(*rows as u64)
                    .checked_div(stretch.bytes)
                    .unwrap_or(0);
                let index = usize::try_from(share).unwrap_or(usize::MAX).min(rows - 1);
                Some(body + index)
            }
            BodyRows::Decoded(decoded) => {
                let local = self.placed_of(flat)?.local(address);
                if let Some(gap) = decoded
                    .gap
                    .as_ref()
                    .filter(|gap| gap.range.contains(&local))
                {
                    let into = gap.range.start.bytes_to(local)?;
                    let index = (into / GAP_BYTES_PER_ROW) as usize;
                    // Under the cut row, where it has one: the row of bytes holding the
                    // address, as a separator's instruction is found and not the separator.
                    return Some(body + decoded.listing_rows() + cut_rows(gap) + index);
                }
                let assembly = decoded.assembly.as_ref()?;
                let index = assembly.instruction_at(local)?;
                Some(body + decoded.lanes.row_of(index))
            }
        }
    }

    /// The row **holding** the byte at `address`: [`row_for`](Self::row_for)'s answer,
    /// except past the header and the labels where the address is a stretch's start --
    /// the first instruction, or the first guessed row -- since what a caret is put on is
    /// a row of code and not the name over it, which `row_for` answers for a view that
    /// is better shown the label. [`None`] where `row_for` is, and for a stretch with no
    /// body at all.
    pub fn body_row_for(&self, address: PlacedAddress) -> Option<usize> {
        let row = self.row_for(address)?;
        let flat = self.stretch_of(row)?;
        let body = self.body_start(flat)?;
        if row >= body {
            return Some(row);
        }
        (body < self.start(flat + 1)).then_some(body)
    }

    /// The stretches whose rows intersect `rows`, as a range of flat indices.
    pub fn stretches_in(&self, rows: Range<usize>) -> Range<usize> {
        if rows.start >= rows.end || rows.start >= self.len() {
            return 0..0;
        }
        let first = self.stretch_of(rows.start);
        let last = self.stretch_of((rows.end - 1).min(self.len() - 1));
        match (first, last) {
            (Some(first), Some(last)) => first..last + 1,
            _ => 0..0,
        }
    }

    /// The stretches to ask for next: those within `buffer` rows of the rows in `view`,
    /// not yet `held`, nearest the middle of the view first, at most `cap` of them.
    pub fn window(
        &self,
        view: Range<usize>,
        buffer: usize,
        held: impl Fn(usize) -> bool,
        cap: usize,
    ) -> Vec<usize> {
        let wanted =
            self.stretches_in(view.start.saturating_sub(buffer)..view.end.saturating_add(buffer));
        let centre = view.start.saturating_add(view.end) / 2;
        let mut wanted: Vec<(usize, usize)> = wanted
            .filter(|&flat| !held(flat))
            .map(|flat| {
                let rows = self.start(flat)..self.start(flat + 1);
                // How far the stretch is from the middle of the view, and none if the
                // middle is inside it.
                let distance = if rows.contains(&centre) {
                    0
                } else if rows.start > centre {
                    rows.start - centre
                } else {
                    centre - (rows.end - 1)
                };
                (distance, flat)
            })
            .collect();
        wanted.sort_unstable();
        wanted.into_iter().take(cap).map(|(_, flat)| flat).collect()
    }
}

#[cfg(test)]
mod tests;
