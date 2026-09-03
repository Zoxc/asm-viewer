use super::*;

fn edges(pairs: &[(usize, usize)]) -> Vec<BranchEdge> {
    pairs
        .iter()
        .map(|&(from, to)| BranchEdge { from, to })
        .collect()
}

/// The gutter as it is drawn, one string per row, outermost lane on the left.
///
/// A lane is `|` where the line runs the whole row, `'` where only its top half is
/// drawn, `,` where only its bottom half is, and a space where the lane is empty. The
/// horizontal run out to the listing is `-`, and the character past the last lane is
/// `>` where a branch lands. Trailing blanks are cut.
fn picture(lanes: &Lanes, rows: usize) -> Vec<String> {
    (0..rows)
        .map(|index| {
            let row = lanes.row(index);
            let mut drawn: String = (0..lanes.width)
                .rev()
                .map(|lane| {
                    match (row.lanes[lane].top, row.lanes[lane].bottom) {
                        (true, true) => '|',
                        (true, false) => '\'',
                        (false, true) => ',',
                        // Inside the horizontal run, an empty lane is the run itself.
                        (false, false) if row.stub.is_some_and(|outer| lane < outer) => '-',
                        (false, false) => ' ',
                    }
                })
                .collect();
            drawn.push(match (row.arrow, row.stub) {
                (true, _) => '>',
                (false, Some(_)) => '-',
                (false, None) => ' ',
            });
            drawn.trim_end().to_owned()
        })
        .collect()
}

#[test]
fn a_symbol_that_branches_nowhere_has_no_gutter() {
    let lanes = Lanes::new(&[], 4);
    assert_eq!(lanes.width, 0);
    assert_eq!(lanes.row(2), RowLanes::default());
    assert!(lanes.touching(2).is_empty());
}

/// The smallest honest picture, and the one `line_fixture.o`'s `sum_to` draws: a jump
/// forward over the body to the condition, and a conditional jump back up into it.
#[test]
fn a_loop_is_a_forward_edge_and_a_backward_one() {
    let lanes = Lanes::new(&edges(&[(0, 4), (5, 1)]), 7);

    assert_eq!(lanes.width, 2);
    assert_eq!(
        picture(&lanes, 7),
        [" ,-", ",|>", "||", "||", "|'>", "'--", ""].map(str::to_owned)
    );
}

/// The nesting rule: the shorter branch is drawn nearer the code whichever order the
/// edges arrive in, and whichever way round they run.
#[test]
fn a_branch_inside_another_is_drawn_inside_it() {
    for pairs in [[(0, 9), (3, 6)], [(3, 6), (0, 9)], [(9, 0), (6, 3)]] {
        let lanes = Lanes::new(&edges(&pairs), 10);
        assert_eq!(lanes.width, 2);

        let inner = lanes.touching(3);
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0].lane, 0);

        let outer = lanes.touching(0);
        assert_eq!(outer.len(), 1);
        assert_eq!(outer[0].lane, 1);
    }
}

/// Two branches that overlap without either containing the other still get a lane
/// each — they are drawn side by side down the rows they share.
#[test]
fn overlapping_branches_that_do_not_nest_take_two_lanes() {
    let lanes = Lanes::new(&edges(&[(0, 5), (3, 8)]), 9);

    assert_eq!(lanes.width, 2);
    assert_eq!(lanes.touching(0)[0].lane, lanes.touching(5)[0].lane);
    assert_eq!(lanes.touching(3)[0].lane, lanes.touching(8)[0].lane);
    assert_ne!(lanes.touching(0)[0].lane, lanes.touching(3)[0].lane);
}

/// Branches that share nothing but a row would read as one line passing through, so
/// they do not share a lane.
#[test]
fn a_branch_ending_where_another_begins_takes_a_second_lane() {
    let lanes = Lanes::new(&edges(&[(0, 3), (3, 6)]), 7);

    assert_eq!(lanes.width, 2);
    assert_eq!(
        picture(&lanes, 7),
        [" ,-", " |", " |", ",'>", "|", "|", "'->"].map(str::to_owned)
    );
}

/// Branches that share no row at all share a lane, however many of them there are.
#[test]
fn branches_that_never_overlap_all_take_the_innermost_lane() {
    let lanes = Lanes::new(&edges(&[(0, 1), (2, 3), (4, 5), (6, 7), (8, 9)]), 10);

    assert_eq!(lanes.width, 1);
    assert!(lanes.touching(4).iter().all(|edge| edge.lane == 0));
}

/// Past the cap the outermost lane is shared, and it is the longest branches that end up
/// sharing it: six branches nested one inside the next put their two outermost in lane 4,
/// and every one of them keeps its corners and its arrowhead.
#[test]
fn more_branches_than_lanes_share_the_outermost_one() {
    let pairs: Vec<(usize, usize)> = (0..6).map(|i| (i, 11 - i)).collect();
    let lanes = Lanes::new(&edges(&pairs), 12);

    assert_eq!(lanes.width, MAX_LANES);

    let assigned: Vec<usize> = (0..6).map(|row| lanes.touching(row)[0].lane).collect();
    assert_eq!(assigned, [4, 4, 3, 2, 1, 0]);

    for (row, lane) in assigned.iter().enumerate() {
        assert_eq!(lanes.row(row).stub, Some(*lane));
        assert!(lanes.row(11 - row).arrow);
    }
}

/// The lit lanes are the ones a branch of the hovered row runs down, and only over the
/// rows it actually spans; its two ends are lit as corners and nothing between them is.
#[test]
fn lighting_a_row_lights_the_lanes_of_its_own_branches() {
    let lanes = Lanes::new(&edges(&[(1, 7), (3, 5)]), 9);
    let touching = lanes.touching(1);
    let lane = [false, true, false, false, false];

    assert_eq!(lit(&touching, 0), Lit::default());
    assert_eq!(
        lit(&touching, 1),
        Lit {
            lanes: lane,
            corner: true
        }
    );
    assert_eq!(
        lit(&touching, 4),
        Lit {
            lanes: lane,
            corner: false
        }
    );
    assert_eq!(
        lit(&touching, 7),
        Lit {
            lanes: lane,
            corner: true
        }
    );
    assert_eq!(lit(&touching, 8), Lit::default());

    // The inner branch is not one of row 1's, even though its lane is lit at row 4.
    assert_eq!(
        lit(&lanes.touching(3), 4).lanes,
        [true, false, false, false, false]
    );
}

/// A run of listing rows is converted back to the instructions it holds before the
/// edges are asked about: a separator at either end belongs to the instruction below it,
/// so one opening the run is inside it and one closing the run is not, and a run that is
/// one separator holds nothing. The edges lit are the ones with an end in the run, in
/// one pass, so a run over the whole listing costs the edges and not the rows.
#[test]
fn a_run_of_rows_lights_the_branches_of_the_instructions_it_holds() {
    // Landing rows 5 and 7 each get a separator above them: instruction 5 is drawn at
    // row 6 and instruction 7 at row 9.
    let lanes = Lanes::new(&edges(&[(1, 7), (3, 5)]), 9);
    assert_eq!(lanes.row_of(5), 6);
    assert_eq!(lanes.row_of(7), 9);

    assert_eq!(lanes.instructions_in(0..=3), Some(0..=3));
    // Past the end is no instruction, which a row asked about its neighbour below asks:
    // the arithmetic alone answered the next index, and the row drawn from it panicked.
    assert_eq!(lanes.instruction_at(lanes.listing_rows(9)), None);
    assert_eq!(lanes.instruction_at(lanes.listing_rows(9) + 5), None);
    assert_eq!(lanes.instructions_in(10..=12), None);
    // The separator at row 5 opens the run: instruction 5 is inside.
    assert_eq!(lanes.instructions_in(5..=6), Some(5..=5));
    // And closes it: the instruction below is outside.
    assert_eq!(lanes.instructions_in(3..=5), Some(3..=4));
    assert_eq!(lanes.instructions_in(5..=5), None);
    assert_eq!(lanes.instructions_in(4..=2), None);

    // The branch from 1 to 7 is lit by a run holding either end and by none between.
    assert_eq!(lanes.touching_any(0..=1), lanes.touching(1));
    assert_eq!(lanes.touching_any(7..=8), lanes.touching(7));
    assert!(lanes.touching_any(2..=2).is_empty());
    assert_eq!(lanes.touching_any(0..=8).len(), 2);
}

/// An edge naming a row that is not there is dropped rather than panicking. `analysis`
/// does not produce one; a corrupted object it has not seen might.
#[test]
fn an_edge_past_the_end_is_not_drawn() {
    let lanes = Lanes::new(&edges(&[(0, 9), (1, 2)]), 4);

    assert_eq!(lanes.width, 1);
    assert_eq!(lanes.touching(0).len(), 0);
    assert!(!lanes.row(0).arrow);
}

/// The listing's rows against the symbol's instructions. Every row a branch lands on gets
/// a separator above it, so the two spaces drift apart by one per block, and the round
/// trip has to hold for every row of both: `instruction_at` is the inverse of `row_of`
/// where there is one, and `None` exactly where there is not.
#[test]
fn a_separator_row_sits_above_every_row_a_branch_lands_on() {
    // The `sum_to` shape: a jump forward to row 4 and one back up to row 1, so rows 1 and
    // 4 begin a block and rows 0, 2, 3, 5 and 6 do not.
    let lanes = Lanes::new(&edges(&[(0, 4), (5, 1)]), 7);
    assert_eq!(lanes.listing_rows(7), 9);

    // Instruction -> row, and the drift is one per separator already passed.
    let rows: Vec<usize> = (0..7).map(|index| lanes.row_of(index)).collect();
    assert_eq!(rows, vec![0, 2, 3, 4, 6, 7, 8]);

    // Row -> instruction, which must be the exact inverse: the two separator rows answer
    // nothing and every other row answers the instruction that named it.
    let drawn: Vec<Option<usize>> = (0..9).map(|row| lanes.instruction_at(row)).collect();
    assert_eq!(
        drawn,
        vec![
            Some(0),
            None,
            Some(1),
            Some(2),
            Some(3),
            None,
            Some(4),
            Some(5),
            Some(6),
        ]
    );
}

/// A symbol whose first instruction is branched to gets no separator over its head: a
/// boundary above the top of a listing says nothing, and the row would be a gap the
/// symbol opens with.
#[test]
fn the_first_row_never_gets_a_separator() {
    let lanes = Lanes::new(&edges(&[(3, 0)]), 5);
    assert_eq!(lanes.row(0).arrow, true, "the branch still lands there");
    assert_eq!(lanes.listing_rows(5), 5);
    assert_eq!(lanes.row_of(0), 0);
    assert_eq!(
        (0..5)
            .map(|row| lanes.instruction_at(row))
            .collect::<Vec<_>>(),
        (0..5).map(Some).collect::<Vec<_>>()
    );
}

/// A symbol with no branches at all is one row per instruction, and a separator drawn
/// over nothing carries no lanes.
#[test]
fn a_symbol_that_branches_nowhere_is_one_row_per_instruction() {
    let lanes = Lanes::new(&[], 4);
    assert_eq!(lanes.listing_rows(4), 4);
    assert_eq!(lanes.row_of(3), 3);
    assert_eq!(lanes.instruction_at(3), Some(3));
    // And nothing past the last: with no branches there is no row table to bound the
    // index, and the row asked about its neighbour below panicked in the listing.
    assert_eq!(lanes.instruction_at(4), None);
    assert_eq!(lanes.boundary(3), RowLanes::default());
}

/// The separator carries the lanes that cross it and neither of the row's own marks: the
/// line a branch is drawn with must not break where the listing opens a gap under it, and
/// the arrowhead belongs to the row the branch lands on.
#[test]
fn a_separator_carries_the_lanes_that_cross_it() {
    // Row 4 is the target of the forward branch drawn in lane 0 and is crossed by the
    // backward one, which reaches from row 1 to row 5.
    let lanes = Lanes::new(&edges(&[(0, 4), (5, 1)]), 7);
    let boundary = lanes.boundary(4);
    let below = lanes.row(4);

    assert!(!boundary.arrow, "the separator drew a second arrowhead");
    assert!(
        boundary.stub.is_none(),
        "the separator drew a second corner"
    );
    for lane in 0..MAX_LANES {
        let through = below.lanes[lane].top;
        assert_eq!(
            boundary.lanes[lane],
            Vertical {
                top: through,
                bottom: through,
            },
            "lane {lane} does not carry on across the boundary"
        );
    }
    // And at least one lane really does cross, or the assertion above holds vacuously.
    assert!(boundary.lanes.iter().any(|lane| lane.top));
}
