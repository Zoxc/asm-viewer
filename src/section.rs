//! The rows a listing of an object's whole code is made of, before and after any of it is
//! decoded. Framework-free: a `VirtualScrollView` builds row *n* knowing nothing but *n*
//! and has to be told its length up front, so this is the one place that says how many
//! rows there are and what row *n* is.
//!
//! The listing is [`CodeListing`]'s stretches, one after another: a **header** row where a
//! section starts, a **label** row per symbol at a stretch's address, then the stretch's
//! body. A body that has been decoded is the symbol's instruction rows and block
//! separators -- exactly the rows its own listing draws, `Lanes` and all -- followed by
//! its gap as rows of hex bytes. A body nobody has decoded yet is a run of **empty** rows,
//! as many as its bytes suggest, so the listing has its whole length from the first frame
//! and the reader scrolls over empty space that fills in as the worker reaches it. The
//! length therefore starts estimated and settles; keeping the reader's row still while it
//! does is the view's job, and [`Rows::address_of`] / [`Rows::row_for`] are what it does
//! it with, an address being the one name for a row that survives the rows around it
//! changing.
//!
//! Every address here is **placed** (`Placed::place`): the section's own plus where the
//! object's layout put it, so two functions of a relocatable object, both at 0 in the file,
//! draw at two addresses and the listing reads as one.

use crate::lanes::Lanes;
use analysis::{Assembly, CodeListing, Place};
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
    pub gap: Option<Range<u64>>,
}

impl Body {
    fn instructions(&self) -> usize {
        self.assembly
            .as_ref()
            .map_or(0, |assembly| assembly.instructions.len())
    }

    /// The instruction rows and the separators between them.
    fn listing_rows(&self) -> usize {
        self.lanes.listing_rows(self.instructions())
    }

    fn gap_rows(&self) -> usize {
        self.gap.as_ref().map_or(0, |gap| gap_rows(gap))
    }
}

/// How many rows `gap` takes: sixteen bytes each, the last one short.
fn gap_rows(gap: &Range<u64>) -> usize {
    let bytes = gap.end.saturating_sub(gap.start);
    bytes
        .div_ceil(GAP_BYTES_PER_ROW)
        .try_into()
        .unwrap_or(usize::MAX)
}

/// One row of the listing, as what it draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    /// Where a placed section starts: its name.
    Header { section: usize },
    /// The `index`th symbol at the stretch's address.
    Label { stretch: usize, index: usize },
    /// One of the rows a stretch nobody has decoded is guessed to take.
    Empty { stretch: usize, index: usize },
    /// The `index`th instruction of the stretch's symbol.
    Instruction { stretch: usize, index: usize },
    /// The block separator above the instruction `below`.
    Separator { stretch: usize, below: usize },
    /// The `index`th row of sixteen bytes of the stretch's gap.
    Gap { stretch: usize, index: usize },
}

/// One stretch's share of the rows.
struct StretchRows {
    place: Place,
    /// The placed address the stretch starts at, and how many bytes it covers.
    start: u64,
    bytes: u64,
    bias: u64,
    header: bool,
    labels: usize,
    body: BodyRows,
}

enum BodyRows {
    Estimated(usize),
    Decoded(Body),
}

impl StretchRows {
    fn body_rows(&self) -> usize {
        match &self.body {
            BodyRows::Estimated(rows) => *rows,
            BodyRows::Decoded(body) => body.listing_rows() + body.gap_rows(),
        }
    }

    fn rows(&self) -> usize {
        usize::from(self.header) + self.labels + self.body_rows()
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

/// The stretch at flat index `flat` -- sections concatenated in placed order -- as the
/// crate names it. What a window of stretches is asked for by, since one number is what a
/// list of them wants.
pub fn place_of(code: &CodeListing, flat: usize) -> Option<Place> {
    let mut first = 0;
    for (section, placed) in code.sections().iter().enumerate() {
        let count = placed.listing.stretches().len();
        if flat < first + count {
            return Some(Place {
                section,
                stretch: flat - first,
            });
        }
        first += count;
    }
    None
}

/// Every row of one object's code listing, worked out from the skeleton and whichever
/// stretches have been decoded.
pub struct Rows {
    code: Arc<CodeListing>,
    stretches: Vec<StretchRows>,
    /// `starts[i]` is the first row of stretch `i`; one more entry holds the total.
    starts: Vec<usize>,
    /// `sections[s]` is the flat index of section `s`'s first stretch; one more entry
    /// holds the stretch count.
    sections: Vec<usize>,
}

impl Rows {
    /// The rows for `code`, with `decoded` answering for the stretches -- by flat index --
    /// that have been.
    pub fn new(code: Arc<CodeListing>, decoded: impl Fn(usize) -> Option<Body>) -> Self {
        let mut stretches = Vec::new();
        let mut sections = Vec::with_capacity(code.sections().len() + 1);
        for (section, placed) in code.sections().iter().enumerate() {
            sections.push(stretches.len());
            let bias = placed.bias();
            for (index, stretch) in placed.listing.stretches().iter().enumerate() {
                let flat = stretches.len();
                let bytes = stretch.range.end.saturating_sub(stretch.range.start);
                let labels = stretch.symbols.len();
                let body = match decoded(flat) {
                    Some(body) => BodyRows::Decoded(body),
                    None => BodyRows::Estimated(StretchRows::estimate(bytes, labels > 0)),
                };
                stretches.push(StretchRows {
                    place: Place {
                        section,
                        stretch: index,
                    },
                    start: placed.place(stretch.range.start),
                    bytes,
                    bias,
                    header: index == 0,
                    labels,
                    body,
                });
            }
        }
        sections.push(stretches.len());

        let mut starts = Vec::with_capacity(stretches.len() + 1);
        let mut total = 0;
        for stretch in &stretches {
            starts.push(total);
            total += stretch.rows();
        }
        starts.push(total);

        Self {
            code,
            stretches,
            starts,
            sections,
        }
    }

    pub fn code(&self) -> &Arc<CodeListing> {
        &self.code
    }

    /// The placed section stretch `flat` is in.
    pub fn placed_of(&self, flat: usize) -> Option<&analysis::Placed> {
        self.code
            .sections()
            .get(self.stretches.get(flat)?.place.section)
    }

    /// How many rows the listing has, estimates included.
    pub fn len(&self) -> usize {
        *self.starts.last().unwrap_or(&0)
    }

    /// The stretch at flat index `flat`, as the crate names it.
    pub fn place(&self, flat: usize) -> Option<Place> {
        Some(self.stretches.get(flat)?.place)
    }

    /// The flat index of a place, if the listing has it.
    pub fn flat(&self, place: Place) -> Option<usize> {
        let first = *self.sections.get(place.section)?;
        let end = *self.sections.get(place.section + 1)?;
        let flat = first.checked_add(place.stretch)?;
        (flat < end).then_some(flat)
    }

    /// The row a stretch's body starts at, after its header and labels: what its lanes'
    /// rows are relative to.
    pub fn body_start(&self, flat: usize) -> Option<usize> {
        let stretch = self.stretches.get(flat)?;
        Some(self.starts[flat] + usize::from(stretch.header) + stretch.labels)
    }

    /// What was decoded for stretch `flat`, if anything was.
    pub fn body(&self, flat: usize) -> Option<&Body> {
        match &self.stretches.get(flat)?.body {
            BodyRows::Decoded(body) => Some(body),
            BodyRows::Estimated(_) => None,
        }
    }

    /// What the stretch adds to its symbol's own addresses.
    pub fn bias(&self, flat: usize) -> Option<u64> {
        Some(self.stretches.get(flat)?.bias)
    }

    /// The placed address stretch `flat` starts at.
    pub fn start_of(&self, flat: usize) -> Option<u64> {
        Some(self.stretches.get(flat)?.start)
    }

    /// Which stretch row `row` is in.
    fn stretch_of(&self, row: usize) -> Option<usize> {
        if row >= self.len() {
            return None;
        }
        // The last start at or before `row`; `starts` has one entry past the stretches.
        let after = self.starts.partition_point(|&start| start <= row);
        after
            .checked_sub(1)
            .filter(|&flat| flat < self.stretches.len())
    }

    /// What row `row` draws.
    pub fn row(&self, row: usize) -> Option<Row> {
        let flat = self.stretch_of(row)?;
        let stretch = &self.stretches[flat];
        let mut local = row - self.starts[flat];
        if stretch.header {
            if local == 0 {
                return Some(Row::Header {
                    section: stretch.place.section,
                });
            }
            local -= 1;
        }
        if local < stretch.labels {
            return Some(Row::Label {
                stretch: flat,
                index: local,
            });
        }
        local -= stretch.labels;
        match &stretch.body {
            BodyRows::Estimated(_) => Some(Row::Empty {
                stretch: flat,
                index: local,
            }),
            BodyRows::Decoded(body) => {
                let listing = body.listing_rows();
                if local < listing {
                    Some(match body.lanes.instruction_at(local) {
                        Some(index) => Row::Instruction {
                            stretch: flat,
                            index,
                        },
                        None => Row::Separator {
                            stretch: flat,
                            below: body.lanes.instruction_at(local + 1).unwrap_or(0),
                        },
                    })
                } else {
                    Some(Row::Gap {
                        stretch: flat,
                        index: local - listing,
                    })
                }
            }
        }
    }

    /// The placed address row `row` stands for: what names it once the rows around it
    /// have changed. A header is its section's start, a label its stretch's, an empty row
    /// its share of the stretch's bytes, an instruction its own, a separator the
    /// instruction below it, and a gap row its first byte.
    pub fn address_of(&self, row: usize) -> Option<u64> {
        let flat = self.stretch_of(row)?;
        let stretch = &self.stretches[flat];
        Some(match self.row(row)? {
            Row::Header { section } => self.code.sections().get(section)?.range().start,
            Row::Label { .. } => stretch.start,
            Row::Empty { index, .. } => {
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
            Row::Instruction { index, .. } | Row::Separator { below: index, .. } => {
                let assembly = self.body(flat)?.assembly.as_ref()?;
                assembly
                    .instructions
                    .get(index)?
                    .address
                    .wrapping_add(stretch.bias)
            }
            Row::Gap { index, .. } => {
                let gap = self.body(flat)?.gap.as_ref()?;
                gap.start
                    .wrapping_add(stretch.bias)
                    .saturating_add((index as u64).saturating_mul(GAP_BYTES_PER_ROW))
            }
        })
    }

    /// The row a placed address is drawn in: a stretch's **first** row for its start --
    /// the header or the label, since the first instruction shares that address with them
    /// and a reader landing on an address is better shown the label over it -- else the
    /// instruction, gap row or empty row covering it. [`None`] for an address in no
    /// stretch: between two sections, or outside every one. A view keeping its place by
    /// an address keeps how many rows past this it was, so the top row being a stretch's
    /// first instruction comes back as that row and not as the label two rows up.
    pub fn row_for(&self, address: u64) -> Option<usize> {
        let flat = self.flat(self.code.at(address)?)?;
        let stretch = &self.stretches[flat];
        let first = self.starts[flat];
        if address <= stretch.start {
            return Some(first);
        }
        let body = self.body_start(flat)?;
        let into = address - stretch.start;
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
                let local = address.wrapping_sub(stretch.bias);
                if let Some(gap) = decoded.gap.as_ref().filter(|gap| gap.contains(&local)) {
                    let index = ((local - gap.start) / GAP_BYTES_PER_ROW) as usize;
                    return Some(body + decoded.listing_rows() + index);
                }
                let assembly = decoded.assembly.as_ref()?;
                // The last instruction starting at or before the address.
                let after = assembly
                    .instructions
                    .partition_point(|instruction| instruction.address <= local);
                let index = after.checked_sub(1)?;
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
    pub fn body_row_for(&self, address: u64) -> Option<usize> {
        let row = self.row_for(address)?;
        let flat = self.stretch_of(row)?;
        let body = self.body_start(flat)?;
        if row >= body {
            return Some(row);
        }
        (body < self.starts[flat + 1]).then_some(body)
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
                let rows = self.starts[flat]..self.starts[flat + 1];
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
