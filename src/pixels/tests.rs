use super::*;

/// Every scale a display is likely to hand us, plus a couple nobody sane would.
const SCALES: [f64; 6] = [1.0, 1.25, 1.5, 2.0, 2.5, 3.0];

/// Whether a logical coordinate falls on a device pixel boundary. The tolerance is for
/// `f32` alone: a coordinate is computed as a whole number of device pixels divided by the
/// scale, and dividing and multiplying by 1.25 does not always come back exactly.
fn on_grid(logical: f32, scale: f64) -> bool {
    let device = logical * scale as f32;
    (device - device.round()).abs() < 1e-3
}

/// The point of the whole module: both edges of a stroke land on the grid, at every scale
/// and wherever the line it is drawn along happens to fall.
#[test]
fn both_edges_of_a_stroke_land_on_the_device_pixel_grid() {
    for scale in SCALES {
        let grid = Grid::new(scale);
        for step in 0..40 {
            let centre = step as f32 * 0.35;
            let stroke = grid.stroke(centre, 1.0);
            assert!(
                on_grid(stroke.near, scale) && on_grid(stroke.far(), scale),
                "at {scale}x a stroke down {centre} is {stroke:?}"
            );
        }
    }
}

/// And it is put on the device pixel the line runs through, not on whichever one the
/// rounding of a centre would have reached.
///
/// This is the fault it was written for: a row whose height is even puts the gutter's
/// horizontal run at a whole number, so placing its centre there left the stroke spanning
/// 12.5 to 13.5 -- one pixel's worth of ink shared between two, drawn as two grey rows.
#[test]
fn a_hairline_takes_the_pixel_its_line_runs_through() {
    let grid = Grid::new(1.0);
    assert_eq!(
        grid.stroke(13.0, 1.0),
        Stroke {
            near: 13.0,
            thick: 1.0
        }
    );
    assert_eq!(
        grid.stroke(12.5, 1.0),
        Stroke {
            near: 12.0,
            thick: 1.0
        }
    );
    // A lane's centre sits half a logical pixel off a multiple of the lane width, which
    // is exactly the case that already came out right and must go on doing so.
    assert_eq!(
        grid.stroke(3.5, 1.0),
        Stroke {
            near: 3.0,
            thick: 1.0
        }
    );
}

/// A hairline is one device pixel however few logical pixels that is, and a scale that
/// would round it away rounds it up instead: a branch line that vanishes on one display
/// and not another is a worse fault than one drawn a third too thick.
#[test]
fn a_stroke_is_never_thinner_than_one_device_pixel() {
    for scale in [0.4, 0.75, 1.0, 1.5] {
        let grid = Grid::new(scale);
        let stroke = grid.stroke(10.0, 1.0);
        assert!(
            (stroke.thick * scale as f32) >= 1.0 - 1e-3,
            "at {scale}x a hairline came out {stroke:?}"
        );
    }
}

/// A span rounds its two ends and keeps them apart. Both ends, because a run from a
/// snapped edge to an unsnapped one is half aligned, which looks the same as not aligned.
#[test]
fn a_span_rounds_both_ends_and_never_closes() {
    for scale in SCALES {
        let grid = Grid::new(scale);
        let span = grid.span(3.2, 17.9);
        assert!(
            on_grid(span.near, scale) && on_grid(span.far(), scale),
            "at {scale}x the span is {span:?}"
        );
        assert!((span.thick * scale as f32) >= 1.0 - 1e-3);
        // Two ends the grid rounds together still leave a stroke behind.
        let pinched = grid.span(5.0, 5.0);
        assert!(
            (pinched.thick * scale as f32) >= 1.0 - 1e-3,
            "at {scale}x a pinched span is {pinched:?}"
        );
    }
}

/// The far edge is what a line meeting this one end-on has to reach, and the centre is
/// what something drawn at an angle to it pivots on -- which is *not* on the grid when
/// the stroke is an odd number of device pixels thick, and is the whole reason the pivot
/// is asked for separately rather than snapped like everything else here.
#[test]
fn a_stroke_offers_its_far_edge_and_its_ink_centre() {
    // One device pixel thick, so its middle is half of one.
    let hairline = Grid::new(1.0).stroke(6.1, 1.0);
    assert_eq!(hairline.far(), hairline.near + hairline.thick);
    assert_eq!(hairline.centre(), 6.5);
    assert!(
        !on_grid(hairline.centre(), 1.0),
        "{hairline:?} pivots on the grid"
    );

    // Two of them, and the middle is back on it. Either way the pivot is the middle of
    // the ink and not of the line that was asked for.
    let doubled = Grid::new(2.0).stroke(6.1, 1.0);
    assert_eq!(doubled.centre(), doubled.near + doubled.thick / 2.0);
    assert!(on_grid(doubled.centre(), 2.0), "{doubled:?}");
}

/// A diagonal is not aligned but weighted: half a device pixel more ink than the stroke
/// it joins, so the two rows the antialiasing spreads it over do not read lighter.
#[test]
fn a_diagonal_is_half_a_device_pixel_thicker_than_a_stroke() {
    for scale in SCALES {
        let grid = Grid::new(scale);
        let straight = grid.stroke(10.0, 1.0).thick;
        let diagonal = grid.diagonal(1.0);
        assert!(
            ((diagonal - straight) * scale as f32 - 0.5).abs() < 1e-3,
            "at {scale}x a {straight} stroke has a {diagonal} diagonal"
        );
    }
}

/// A scale factor the platform could not have meant is taken as one, so that a lie from
/// the window manager costs a slightly wrong line and not a gutter of zero-width rects or
/// of NaNs.
#[test]
fn a_nonsense_scale_factor_is_taken_as_one() {
    for scale in [0.0, -2.0, f64::NAN, f64::INFINITY] {
        let grid = Grid::new(scale);
        assert_eq!(
            grid.stroke(12.5, 1.0),
            Stroke {
                near: 12.0,
                thick: 1.0
            }
        );
        assert_eq!(grid.edge(4.4), 4.0);
    }
}

/// Rounding a bare coordinate is the same grid the strokes are on: it is what the ends of
/// the gutter -- the arrow's tip, the width the address column starts after -- are put on.
#[test]
fn an_edge_is_the_nearest_device_pixel() {
    assert_eq!(Grid::new(1.0).edge(10.4), 10.0);
    assert_eq!(Grid::new(2.0).edge(10.4), 10.5);
    assert_eq!(Grid::new(2.0).edge(10.3), 10.5);
    assert_eq!(Grid::new(2.0).edge(10.1), 10.0);
}
