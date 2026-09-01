//! Where each branch of a symbol is drawn in the arrow gutter left of its listing.
//!
//! This module is **framework-free** — no freya types appear here — for the reason
//! `filter.rs`, `history.rs`, `tabs.rs` and `tree.rs` are: the rules below are decisions
//! with cases and no pixels, and they can be asserted without mounting a UI.
//!
//! What 7a left is a flat `Vec<BranchEdge>` per symbol, both ends an index into the
//! instructions. What a gutter needs is geometry, and the one thing a row cannot work out
//! for itself: a `VirtualScrollView` builds row *n* knowing nothing but *n*, so every row
//! has to be *told* which lines pass through it, which start in it and which end in it.
//! [`Lanes`] is that answer for a whole symbol, computed once beside the disassembly and
//! asked one row at a time.
//!
//! A **lane** is a column of the gutter, numbered from the listing outwards: lane 0 is the
//! one nearest the code and the last one is furthest from it. Two rules decide which lane
//! an edge takes.
//!
//! **Nesting.** A branch whose span sits inside another's is drawn *inside* it — nearer
//! the code — so that the picture reads the way the control flow nests: the inner loop
//! inside the outer one, the `jne` over three instructions inside the `jmp` over the
//! whole body. That is not enforced after the fact; it falls out of assigning lanes
//! greedily in order of **span length, shortest first**. When the longer edge is placed,
//! every lane inside the one it gets is already blocked by an edge overlapping it, and an
//! edge nested inside it is one of those — so it can only land further out. Ties go to
//! the edge that starts higher up, which is nothing but determinism.
//!
//! **The cap.** Optimised code has branches everywhere and nothing bounds how many of
//! them overlap one row, so a gutter that grew a lane per overlap would be eating the
//! instruction text on exactly the functions that are hardest to read. The gutter is
//! therefore at most [`MAX_LANES`] wide, and it is exactly as wide as the deepest nesting
//! the symbol actually reaches, which for almost every function is one or two lanes.
//! Edges that find every lane taken share the outermost one rather than being dropped:
//! what a row most needs to say is *this row branches* and *something lands here*, which
//! is the corner and the arrowhead, and those survive sharing. Only the line joining the
//! two ends becomes ambiguous — and it is the longest edges that are pushed out there,
//! whose two ends are rarely on screen together anyway, so the line between them was the
//! least useful thing in the gutter.

use analysis::BranchEdge;

/// How many lanes the gutter is ever drawn with. See the module comment on the cap.
pub const MAX_LANES: usize = 5;

/// The vertical strokes one lane of one row carries.
///
/// Two halves rather than one flag because a row where a branch starts or ends is a
/// *corner*: the line runs from the middle of the row, where the horizontal run to the
/// listing leaves it, to one edge only. Both halves set is a line passing straight
/// through.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Vertical {
    /// A stroke from the row's top edge to its middle.
    pub top: bool,
    /// A stroke from the row's middle to its bottom edge.
    pub bottom: bool,
}

/// What one row draws in the gutter.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct RowLanes {
    /// The vertical strokes, indexed by lane — 0 nearest the code.
    pub lanes: [Vertical; MAX_LANES],
    /// The outermost lane with a corner in this row, and so where the horizontal run to
    /// the listing starts; `None` when no branch starts or ends here.
    ///
    /// One horizontal per row and not one per corner, because they would all be drawn at
    /// the same height and merge into the one line anyway: a row that is the target of a
    /// jump *and* the source of another has one run out to the code, starting at whichever
    /// of the two corners is further out.
    pub stub: Option<usize>,
    /// Whether a branch *lands* on this row, which is what the arrowhead says. A row that
    /// is the target of several branches has one arrowhead: it points at the row, not at
    /// any one line.
    pub arrow: bool,
}

/// One edge after it has been given a lane, as the two rows it is drawn between rather
/// than as the two it runs between — [`BranchEdge`] keeps execution order and this is
/// listing order, which is all the geometry cares about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlacedEdge {
    pub lane: usize,
    pub first: usize,
    pub last: usize,
}

impl PlacedEdge {
    /// Whether this edge is drawn in `row`, ends included.
    fn covers(&self, row: usize) -> bool {
        self.first <= row && row <= self.last
    }
}

/// Every branch of one symbol, laid out in lanes, with the answer for each row worked out
/// in advance.
pub struct Lanes {
    rows: Vec<RowLanes>,
    placed: Vec<PlacedEdge>,
    width: usize,
}

impl Lanes {
    /// Lay out `edges`, a symbol's [`Assembly::edges`](analysis::Assembly::edges), over a
    /// listing of `instructions` rows.
    ///
    /// The two are passed separately rather than as the `Assembly` they both come from so
    /// that a test is a list of `(from, to)` pairs and a row count, which is all this is
    /// about; nothing here reads an instruction.
    pub fn new(edges: &[BranchEdge], instructions: usize) -> Self {
        // Both ends of an edge index a real instruction — `analysis` says so and its
        // robustness sweeps assert it on every corrupted input they can produce — but the
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
            };
        }

        // Shortest span first, which is what makes the nesting fall out of the greedy
        // assignment below rather than having to be repaired afterwards. See the module
        // comment.
        sorted.sort_by_key(|edge| (edge.last() - edge.first(), edge.first()));

        // What each lane already holds, as spans kept sorted by where they start. Sorted
        // so that the overlap test is a look at the one span that could overlap rather
        // than a walk of the lane, which on a function with thousands of branches is the
        // difference between a linear pass and a quadratic one.
        let mut occupied: Vec<Vec<(usize, usize)>> = vec![Vec::new(); MAX_LANES];
        let mut placed: Vec<PlacedEdge> = Vec::with_capacity(sorted.len());
        let mut width = 0;

        for edge in &sorted {
            let (first, last) = (edge.first(), edge.last());
            let lane = (0..MAX_LANES)
                .find(|&lane| free(&occupied[lane], first, last))
                // Every lane is taken, so this edge shares the outermost one. See the
                // module comment on the cap.
                .unwrap_or(MAX_LANES - 1);

            let at = occupied[lane].partition_point(|span| span.0 < first);
            occupied[lane].insert(at, (first, last));
            width = width.max(lane + 1);
            placed.push(PlacedEdge { lane, first, last });
        }

        let mut rows = vec![RowLanes::default(); instructions];

        // The vertical strokes, as a difference array over the *gaps* between rows rather
        // than as a walk down each edge's span: an edge crossing the gap above row `r`
        // gives row `r` its top half and row `r - 1` its bottom half. A walk would be the
        // obvious way to write it and costs the sum of every span, which a function full
        // of long branches makes quadratic; this costs one pass over the rows whatever
        // the edges do.
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

        Lanes {
            rows,
            placed,
            width,
        }
    }

    /// How many lanes the gutter is drawn with: the deepest nesting this symbol reaches,
    /// capped at [`MAX_LANES`], and 0 for a symbol that branches nowhere within itself —
    /// which is a symbol whose gutter is not drawn at all.
    pub fn width(&self) -> usize {
        self.width
    }

    /// What the row at `index` draws. Total, so that a row of a symbol with no gutter and
    /// a row past the end are both simply nothing rather than a panic.
    pub fn row(&self, index: usize) -> RowLanes {
        self.rows.get(index).copied().unwrap_or_default()
    }

    /// The edges that start or end at `row`, which is what "where does this row's branch
    /// go, and what jumps here" means. An edge merely passing through the row is not one
    /// of them: it has nothing to do with the row it crosses.
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
    /// The lanes such a branch is drawn in, at this row.
    ///
    /// A lane rather than an edge, because a lane is what is drawn: where the cap has put
    /// two edges in the outermost lane, lighting one of them lights that stretch of the
    /// lane and so the other one with it. That is the same ambiguity the sharing already
    /// is, and not a second one.
    pub lanes: [bool; MAX_LANES],
    /// Whether one of them starts or ends *here*, which is what lights the row's
    /// horizontal run and its arrowhead — the two ends of the gesture, the row the
    /// pointer is on and the row its branch goes to.
    ///
    /// Asked of the edges and not read off the strokes, though a corner is otherwise
    /// exactly a lane whose line stops in the middle of the row: in the shared outermost
    /// lane a corner and a line passing through are drawn in the same lane of the same
    /// row, and the strokes no longer remember which is which.
    pub corner: bool,
}

/// What the edges in `touching` (from [`Lanes::touching`]) light up at `row`.
pub fn lit(touching: &[PlacedEdge], row: usize) -> Lit {
    let mut lit = Lit::default();
    for edge in touching {
        if edge.covers(row) {
            lit.lanes[edge.lane] = true;
            lit.corner |= edge.first == row || edge.last == row;
        }
    }
    lit
}

/// Whether a lane holding `spans` has room for one from `first` to `last`.
fn free(spans: &[(usize, usize)], first: usize, last: usize) -> bool {
    // The spans in a lane never overlap each other, so the only ones that can overlap this
    // one are the last that starts at or before it and the first that starts after it.
    let at = spans.partition_point(|span| span.0 < first);
    !spans[at.saturating_sub(1)..spans.len().min(at + 1)]
        .iter()
        .any(|&span| overlaps(span, first, last))
}

/// Whether two spans may not share a lane. Ends count as an overlap: one branch ending
/// where another begins would otherwise put a top half and a bottom half in one lane of
/// one row, which is drawn — and read — as a single line passing straight through it.
fn overlaps(span: (usize, usize), first: usize, last: usize) -> bool {
    span.0 <= last && first <= span.1
}

/// Record that a branch starts or ends in this row, in `lane`.
fn corner(row: &mut RowLanes, lane: usize) {
    row.stub = Some(row.stub.map_or(lane, |outer| outer.max(lane)));
}

#[cfg(test)]
mod tests;
