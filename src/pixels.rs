//! The device pixel grid a window is drawn on, and the strokes laid onto it.
//!
//! freya lays a window out in **logical** pixels and multiplies the whole tree by the
//! window's scale factor on the way to Skia -- `TreeAdapterFreya::get_node` calls torin's
//! `Scaled::scale` on every node it hands the layout, and nothing rounds afterwards. So a
//! hairline whose edges come out at 12.5 and 13.5 device pixels is not drawn as one lit
//! row of pixels but as two half-lit ones, which beside crisp text reads as a blurred
//! line rather than a thin one.
//!
//! What fixes that is rounding the stroke's **edges** to the grid, never placing its
//! centre on a fraction and hoping: a stroke is asked for by the line it runs along and
//! the ink it should have, and comes back as the run of whole device pixels nearest to
//! that. Logical pixels go in and logical pixels come out, because logical pixels are
//! what freya is given; the scale factor only decides which of them are grid points.
//!
//! The answers are relative to whatever the caller positions them in, so they land on the
//! grid only if that element's own origin does. At 1x and 2x that is every ancestor whose
//! offsets are whole numbers, which is all of them here; at 1.5x the pane's own origin
//! decides, and nothing inside a row can see it.

/// The device pixel grid: one scale factor, and the rounding a stroke needs to sit on it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Grid {
    /// Device pixels per logical pixel. Always finite and positive, whatever the platform
    /// said: a scale of zero or a NaN would otherwise turn every coordinate below into
    /// one, and a gutter is not the place to find out that the window manager lied.
    scale: f32,
}

/// A stroke laid on the grid: the edge nearer the origin and the thickness, both in
/// logical pixels, and both landing on a device pixel boundary.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Stroke {
    pub near: f32,
    pub thick: f32,
}

impl Stroke {
    /// The far edge -- where a line meeting this one end-on has to reach to fill the
    /// corner rather than leave a notch in it.
    pub fn far(self) -> f32 {
        self.near + self.thick
    }

    /// The middle of the ink, which is where something drawn at an angle to this stroke
    /// has to pivot. Not on the grid, and not meant to be: it is half a device pixel off
    /// it whenever the stroke is an odd number of them thick.
    pub fn centre(self) -> f32 {
        self.near + self.thick / 2.0
    }
}

impl Grid {
    pub fn new(scale: f64) -> Self {
        let scale = scale as f32;
        Grid {
            scale: if scale.is_finite() && scale > 0.0 {
                scale
            } else {
                1.0
            },
        }
    }

    /// A coordinate moved to the nearest device pixel boundary.
    pub fn edge(self, logical: f32) -> f32 {
        (logical * self.scale).round() / self.scale
    }

    /// How far down from `top` the next device pixel boundary is: the padding that puts
    /// whatever is laid out under a box at `top` on the grid, whatever fraction the boxes
    /// above it added up to. Never negative, and less than one device pixel.
    pub fn nudge(self, top: f32) -> f32 {
        let device = top * self.scale;
        let below = device.ceil() - device;
        if below.is_finite() {
            below / self.scale
        } else {
            0.0
        }
    }

    /// The stroke of `thick` logical pixels that best covers the line at `centre`: whole
    /// device pixels, never fewer than one, placed so the line runs down their middle.
    ///
    /// Never nothing: a hairline asked for at a scale that rounds it away would vanish,
    /// and a branch line that disappears on one display and not another is worse than one
    /// drawn a third too thick.
    pub fn stroke(self, centre: f32, thick: f32) -> Stroke {
        let thick = (thick * self.scale).round().max(1.0);
        let near = (centre * self.scale - thick / 2.0).round();
        Stroke {
            near: near / self.scale,
            thick: thick / self.scale,
        }
    }

    /// The stroke reaching from `from` to `to`, both ends rounded to the grid. At least
    /// one device pixel long, for the reason [`Grid::stroke`] is at least one thick.
    pub fn span(self, from: f32, to: f32) -> Stroke {
        let near = (from * self.scale).round();
        let thick = ((to * self.scale).round() - near).max(1.0);
        Stroke {
            near: near / self.scale,
            thick: thick / self.scale,
        }
    }

    /// How thick to draw a stroke that is *not* axis-aligned: half a device pixel more
    /// than [`Grid::stroke`] would give it.
    ///
    /// A diagonal cannot be put on the grid at all -- at any angle that is not a multiple
    /// of a right angle it crosses into a new row of pixels every so often, wherever it
    /// is placed -- so the choice is between aligning nothing and weighting it. One
    /// device pixel of ink spread by the antialiasing over two rows reads *lighter* than
    /// the crisp one-pixel line it joins; half a pixel more brings its weight back.
    pub fn diagonal(self, thick: f32) -> f32 {
        ((thick * self.scale).round().max(1.0) + 0.5) / self.scale
    }
}

#[cfg(test)]
mod tests;
