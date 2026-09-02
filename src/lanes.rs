//! Where each branch of a symbol is drawn in the arrow gutter left of its listing.
//! Framework-free: a `VirtualScrollView` builds row *n* knowing nothing but *n*, so every
//! row has to be *told* which lines pass through it, and [`Lanes`] is that answer for a
//! whole symbol, computed once beside the disassembly.
//!
//! A **lane** is a column of the gutter, numbered from the listing outwards. Lanes are
//! assigned greedily in order of span length, shortest first, which is what makes a
//! branch nested inside another come out nearer the code without anything enforcing it
//! afterwards. The gutter is at most [`MAX_LANES`] wide; edges that find every lane taken
//! share the outermost one rather than being dropped, since the corner and the arrowhead
//! survive sharing and only the joining line goes ambiguous -- and it is the longest
//! edges, whose ends are rarely on screen together, that are pushed out there.

use analysis::BranchEdge;

/// How many lanes the gutter is ever drawn with.
const MAX_LANES: usize = 5;

/// The vertical strokes one lane of one row carries. Two halves rather than one flag
/// because a row where a branch starts or ends is a *corner*: the line runs from the
/// middle of the row to one edge only.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Vertical {
    pub top: bool,
    pub bottom: bool,
}

/// What one row draws in the gutter.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct RowLanes {
    /// The vertical strokes, indexed by lane — 0 nearest the code.
    pub lanes: [Vertical; MAX_LANES],
    /// The outermost lane with a corner in this row, and so where the horizontal run to
    /// the listing starts; `None` when no branch starts or ends here. One per row and not
    /// one per corner, since they would all be drawn at the same height and merge anyway.
    pub stub: Option<usize>,
    /// Whether a branch *lands* on this row, which is what the arrowhead says. A row that
    /// is the target of several branches has one arrowhead: it points at the row, not at
    /// any one line.
    pub arrow: bool,
}

/// One edge after it has been given a lane, as the two rows it is drawn between rather
/// than as the two it runs between — [`BranchEdge`] keeps execution order and this is
/// listing order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlacedEdge {
    lane: usize,
    first: usize,
    last: usize,
}

/// Every branch of one symbol, laid out in lanes, with the answer for each row worked out
/// in advance.
pub struct Lanes {
    rows: Vec<RowLanes>,
    placed: Vec<PlacedEdge>,
    /// The instructions a separator row is drawn above, ascending: every row a branch
    /// lands on except the symbol's first. See [`Lanes::listing_rows`].
    separators: Vec<usize>,
    /// How many lanes the gutter is drawn with: the deepest nesting this symbol reaches,
    /// capped at [`MAX_LANES`], and 0 for a symbol whose gutter is not drawn at all.
    pub(crate) width: usize,
}

impl Lanes {
    /// Lay out `edges`, a symbol's [`Assembly::edges`](analysis::Assembly::edges), over a
    /// listing of `instructions` rows.
    pub fn new(edges: &[BranchEdge], instructions: usize) -> Self {
        // Both ends of an edge index a real instruction — `analysis` says so — but the
        // check costs one comparison per edge and the alternative to having it is an
        // index-out-of-bounds panic in a gutter, on a file the user merely opened.
        let mut sorted: Vec<&BranchEdge> = edges
            .iter()
            .filter(|edge| edge.last() < instructions)
            .collect();

        if sorted.is_empty() {
            return Lanes {
                rows: Vec::new(),
                placed: Vec::new(),
                width: 0,
                separators: Vec::new(),
            };
        }

        // Shortest span first, which is what makes the nesting fall out of the greedy
        // assignment below rather than having to be repaired afterwards.
        sorted.sort_by_key(|edge| (edge.last() - edge.first(), edge.first()));

        // What each lane already holds, as spans kept sorted by where they start, so that
        // the overlap test looks at the one span that could overlap rather than walking
        // the lane.
        let mut occupied: Vec<Vec<(usize, usize)>> = vec![Vec::new(); MAX_LANES];
        let mut placed: Vec<PlacedEdge> = Vec::with_capacity(sorted.len());
        let mut width = 0;

        for edge in &sorted {
            let (first, last) = (edge.first(), edge.last());
            let lane = (0..MAX_LANES)
                .find(|&lane| free(&occupied[lane], first, last))
                // Every lane is taken, so this edge shares the outermost one.
                .unwrap_or(MAX_LANES - 1);

            let at = occupied[lane].partition_point(|span| span.0 < first);
            occupied[lane].insert(at, (first, last));
            width = width.max(lane + 1);
            placed.push(PlacedEdge { lane, first, last });
        }

        let mut rows = vec![RowLanes::default(); instructions];

        // The vertical strokes, as a difference array over the *gaps* between rows: an
        // edge crossing the gap above row `r` gives row `r` its top half and row `r - 1`
        // its bottom half. Walking each span instead costs the sum of every span, which a
        // function full of long branches makes quadratic.
        let mut crossings = vec![[0i32; MAX_LANES]; instructions + 1];
        for edge in &placed {
            crossings[edge.first + 1][edge.lane] += 1;
            crossings[edge.last + 1][edge.lane] -= 1;
        }

        // The gap above the first row is never crossed — an edge's line starts at its own
        // topmost row — so the sweep starts at row 1 and `row - 1` is always a row.
        let mut open = [0i32; MAX_LANES];
        for row in 1..instructions {
            for lane in 0..width {
                open[lane] += crossings[row][lane];
                if open[lane] > 0 {
                    rows[row].lanes[lane].top = true;
                    rows[row - 1].lanes[lane].bottom = true;
                }
            }
        }

        for edge in &placed {
            corner(&mut rows[edge.first], edge.lane);
            corner(&mut rows[edge.last], edge.lane);
        }

        // The arrowhead is at the branch's *target*, which is the only place execution
        // order still matters: `first`/`last` have forgotten which end that is.
        for edge in edges {
            if edge.last() < instructions {
                rows[edge.to].arrow = true;
            }
        }

        // Every instruction a branch lands on begins a basic block, and the listing draws
        // a separator above each. Never above the first: a boundary over the top of the
        // symbol says nothing, and an empty row there would be a gap the listing opens
        // with. Sorted by construction, which is what lets the two index spaces below be
        // a binary search rather than a scan.
        let separators = (1..instructions).filter(|&row| rows[row].arrow).collect();

        Lanes {
            rows,
            placed,
            width,
            separators,
        }
    }

    /// What the row at `index` draws. Total, so a row past the end is nothing rather than
    /// a panic.
    pub fn row(&self, index: usize) -> RowLanes {
        self.rows.get(index).copied().unwrap_or_default()
    }

    /// What a separator above the instruction at `index` draws in the gutter: the lanes
    /// that carry on across the boundary, and nothing else.
    ///
    /// A lane is drawn through it exactly when the row below has that lane coming *down*
    /// into it (`top`), so a branch's line is unbroken where the listing opens a gap
    /// under it. No stub and no arrowhead: the corner and the arrowhead belong to the row
    /// the branch actually lands on, and drawing either here would be a second one.
    pub fn boundary(&self, index: usize) -> RowLanes {
        let below = self.row(index);
        let mut lanes = [Vertical::default(); MAX_LANES];
        for lane in 0..MAX_LANES {
            let through = below.lanes[lane].top;
            lanes[lane] = Vertical {
                top: through,
                bottom: through,
            };
        }

        RowLanes {
            lanes,
            stub: None,
            arrow: false,
        }
    }

    /// How many rows the listing draws for `instructions` instructions: one each, plus the
    /// separators.
    ///
    /// The two index spaces this and the next two convert between are the whole cost of
    /// the separator being a row rather than a border: **an instruction index is what the
    /// gutter, the line info and the branch edges speak**, and a listing row is what the
    /// `VirtualScrollView`, the scroll and the picked-out run speak. Nothing else may
    /// confuse them.
    pub fn listing_rows(&self, instructions: usize) -> usize {
        instructions + self.separators.len()
    }

    /// The listing row the instruction at `index` is drawn in: itself, plus every
    /// separator that comes at or before it.
    pub fn row_of(&self, index: usize) -> usize {
        index + self.separators.partition_point(|&at| at <= index)
    }

    /// The instruction the listing's `row` draws, or [`None`] where it is a separator.
    ///
    /// `row_of` climbs by one or two and never falls, so the answer is the last
    /// instruction drawn at or above `row` -- and it is a separator exactly when that
    /// instruction is drawn higher up than `row`.
    pub fn instruction_at(&self, row: usize) -> Option<usize> {
        // The count of separators at or above `row` in *listing* terms: a separator sits
        // at `row_of(at) - 1`, so it is at or above `row` when `row_of(at) <= row + 1`.
        let above = self
            .separators
            .partition_point(|&at| self.row_of(at) <= row + 1);
        let index = row.checked_sub(above)?;
        (self.row_of(index) == row).then_some(index)
    }

    /// The edges that start or end at `row`. An edge merely passing through is not one of
    /// them: it has nothing to do with the row it crosses.
    pub fn touching(&self, row: usize) -> Vec<PlacedEdge> {
        self.placed
            .iter()
            .copied()
            .filter(|edge| edge.first == row || edge.last == row)
            .collect()
    }
}

/// How much of one row belongs to a branch of the row the pointer is on.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Lit {
    /// The lanes such a branch is drawn in, at this row. A lane rather than an edge,
    /// because a lane is what is drawn: where the cap has put two edges in the outermost
    /// lane, lighting one lights the other with it.
    pub lanes: [bool; MAX_LANES],
    /// Whether one of them starts or ends *here*, which is what lights the row's
    /// horizontal run and its arrowhead. Asked of the edges rather than read off the
    /// strokes, since in the shared outermost lane a corner and a line passing through
    /// are drawn the same way.
    pub corner: bool,
}

/// What the edges in `touching` (from [`Lanes::touching`]) light up at `row`.
pub fn lit(touching: &[PlacedEdge], row: usize) -> Lit {
    let mut lit = Lit::default();
    for edge in touching {
        if edge.first <= row && row <= edge.last {
            lit.lanes[edge.lane] = true;
            lit.corner |= edge.first == row || edge.last == row;
        }
    }
    lit
}

/// Whether a lane holding `spans` has room for one from `first` to `last`. Ends count as
/// an overlap: one branch ending where another begins would otherwise put a top half and
/// a bottom half in one lane of one row, which reads as a single line passing through.
fn free(spans: &[(usize, usize)], first: usize, last: usize) -> bool {
    // The spans in a lane never overlap each other, so the only ones that can overlap this
    // one are the last that starts at or before it and the first that starts after it.
    let at = spans.partition_point(|span| span.0 < first);
    !spans[at.saturating_sub(1)..spans.len().min(at + 1)]
        .iter()
        .any(|&(start, end)| start <= last && first <= end)
}

/// Record that a branch starts or ends in this row, in `lane`.
fn corner(row: &mut RowLanes, lane: usize) {
    row.stub = Some(row.stub.map_or(lane, |outer| outer.max(lane)));
}

#[cfg(test)]
mod tests;
