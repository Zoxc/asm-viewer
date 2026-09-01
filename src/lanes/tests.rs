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

/// An edge naming a row that is not there is dropped rather than panicking. `analysis`
/// does not produce one; a corrupted object it has not seen might.
#[test]
fn an_edge_past_the_end_is_not_drawn() {
    let lanes = Lanes::new(&edges(&[(0, 9), (1, 2)]), 4);

    assert_eq!(lanes.width, 1);
    assert_eq!(lanes.touching(0).len(), 0);
    assert!(!lanes.row(0).arrow);
}
