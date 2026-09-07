//! The row the Search and Locations panels both draw: a file, or one place found in it.
//!
//! The two ask different questions and hold their answers in different states, but what
//! comes back is the same shape -- places under the file each is in (`src/grouped.rs`) --
//! and so is the drawing of it. One component for both, with [`Folding`] naming the three
//! things they differ in: which state a fold is written to, whether a press may refuse the
//! path it would open, and which panel's pick the row is drawn against (`ui/picks.rs`).

use super::*;
use crate::grouped::Row;
use crate::search::Hit;

/// What a row of a grouped answer draws, and where a press on it goes: one place in a
/// file. Implemented for the two kinds of place the panels find, so both draw one row.
pub(crate) trait Place: Clone + PartialEq + 'static {
    /// The line it is on, numbered from one.
    fn line(&self) -> u32;
    /// That line as the row draws it, and empty where the file would not read.
    fn text(&self) -> &str;
    /// What is marked in the text, as byte ranges into it.
    fn spans(&self) -> &[Range<usize>];
    /// What opening it picks out, over the file's own line and in the UTF-16 units a
    /// pane counts columns in. [`None`] leaves a caret at the start of the line.
    fn columns(&self) -> Option<Range<usize>>;
}

impl Place for Hit {
    fn line(&self) -> u32 {
        self.line
    }

    fn text(&self) -> &str {
        &self.text
    }

    fn spans(&self) -> &[Range<usize>] {
        &self.spans
    }

    fn columns(&self) -> Option<Range<usize>> {
        self.columns.clone()
    }
}

impl Place for references::Reference {
    fn line(&self) -> u32 {
        self.line
    }

    fn text(&self) -> &str {
        &self.text
    }

    fn spans(&self) -> &[Range<usize>] {
        &self.spans
    }

    fn columns(&self) -> Option<Range<usize>> {
        Some(self.columns.clone())
    }
}

/// Which panel's answer a row belongs to: the state its fold is written to, whether a
/// press may refuse the path it would open, and which panel's pick it is drawn against.
#[derive(Clone, Copy)]
pub(crate) enum Folding {
    /// The Search panel's hits.
    Hits(State<Searched>),
    /// The Locations panel's places.
    Places(State<Located>),
}

impl Folding {
    /// Fold the file at `path` in the answer this row is part of, or unfold it.
    fn toggle(self, path: &Path) {
        match self {
            Folding::Hits(mut searched) => {
                searched.write().hits.toggle(path);
            }
            Folding::Places(mut located) => {
                // Bound to a `let` of its own, so the guard the read hands back is gone
                // before the write.
                let mut next = located.peek().clone();
                if next.fold(path) {
                    located.set(next);
                }
            }
        }
    }

    /// Which panel these rows are drawn in, which is whose pick they answer to.
    fn panel(self) -> Panel {
        match self {
            Folding::Hits(_) => Panel::Search,
            Folding::Places(_) => Panel::Locations,
        }
    }

    /// Whether a press may open `path` at all.
    ///
    /// A hit came off a walk of the project's directory, where a file the source pane
    /// would refuse is a row a press should do nothing with. The Locations panel's places
    /// were named by a language server or the debug info, and for those, opening the file
    /// and letting the pane say what is wrong with it is the honest answer -- a `stat` in
    /// the way would silently swallow a move inside a tab already open.
    fn opens(self, path: &Path) -> bool {
        match self {
            Folding::Hits(_) => showable(path),
            Folding::Places(_) => true,
        }
    }
}

/// One row of a grouped answer: a file, or one of the places under it.
#[derive(Clone)]
pub(crate) struct PlaceRow<T> {
    pub(crate) row: Row<T>,
    pub(crate) folding: Folding,
    /// Where this row is in the list as it is drawn, which is what the arrows step and
    /// what a press writes down with the pick (`ui/picks.rs`).
    pub(crate) at: usize,
    pub(crate) key: DiffKey,
}

/// What a row is picked out as: a file row is its path, and a place is what it opens --
/// the file and the line together.
pub(crate) fn place_pick<T: Place>(row: &Row<T>) -> Pick {
    match row {
        Row::File { path, .. } => Pick::Path(path.clone()),
        Row::Item { path, item } => Pick::Place(path.clone(), item.line()),
    }
}

/// What pressing a row does: a file row folds its places away, and a place opens its file
/// as a source-driven tab, landed on the line and with the match or the name picked out.
/// [`open_source_place`] (`agents/Panes.md`) is the arrival every door into a place in a
/// source file makes, so both panels' rows open one the same way, down to the tab's
/// assembly side being driven from that line.
///
/// Shared by the press and by Enter on the row the arrows left the pick on.
pub(crate) fn press_place<T: Place>(
    doors: Doors,
    places: Places,
    ctrl: State<bool>,
    folding: Folding,
    row: &Row<T>,
) -> Pressed {
    match row {
        Row::File { path, .. } => {
            folding.toggle(path);
            Pressed::Folded
        }
        Row::Item { path, item } => {
            if !folding.opens(path) {
                return Pressed::Folded;
            }
            open_source_place(
                doors,
                places,
                path,
                item.line(),
                item.columns(),
                reach(ctrl),
            );
            Pressed::Opened
        }
    }
}

/// The row and where it is are the whole of what is drawn: the states in [`Folding`]
/// compare equal whatever they hold, and a row never moves from one panel to the other. A
/// [`Hit`] and a [`references::Reference`] are both [`Eq`], so two item rows holding the
/// same `Arc` compare equal without reading it.
impl<T: PartialEq> PartialEq for PlaceRow<T> {
    fn eq(&self, other: &Self) -> bool {
        self.row == other.row && self.at == other.at
    }
}

impl<T> KeyExt for PlaceRow<T> {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl<T: Place> Component for PlaceRow<T> {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        // Consumed in the render and peeked in the handler, where no hook may run.
        let doors = use_doors();
        let places = use_places();
        let ctrl = use_consume::<Ctrl>().0;

        let folding = self.folding;
        let picking = use_picking(folding.panel());
        let at = self.at;

        let row = self.row.clone();
        let pressed = row.clone();
        let pick = place_pick(&row);
        let tooltip = match &row {
            Row::File { path, .. } => path.display().to_string(),
            Row::Item { path, item } => format!("{}:{}", path.display(), item.line()),
        };

        extra_tooltip(
            tooltip,
            list_row(hovering, picking.drawn(&pick, false))
                .on_press(move |_| {
                    picking.press(pick.clone(), at, || {
                        press_place(doors, places, ctrl, folding, &pressed)
                    });
                })
                .children(row_children(&row)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// What a row draws: a file row is its fold, its name and its count; a place row is its
/// line number and the line, with the matched or named part of it washed in `match_bg`. A
/// file that would not read leaves the text empty, and the row is the number alone.
fn row_children<T: Place>(row: &Row<T>) -> Vec<Element> {
    match row {
        Row::File {
            name,
            count,
            folded,
            ..
        } => vec![
            disclosure(Some(!*folded)),
            tree_name(name.clone(), false, &[]).into_element(),
            label()
                .text(count.to_string())
                .margin(Gaps::new(0.0, 0.0, 0.0, COUNT_GUTTER))
                .color(palette().address_fg)
                .max_lines(1)
                .into_element(),
        ],
        Row::Item { item, .. } => vec![
            label()
                .text(item.line().to_string())
                .width(Size::px(LINE_NUMBER_WIDTH))
                .text_align(TextAlign::Right)
                .color(palette().address_fg)
                .max_lines(1)
                .into_element(),
            rect()
                .width(Size::flex(1.0))
                .overflow(Overflow::Clip)
                .child(found_line(item.text(), item.spans()))
                .into_element(),
        ],
    }
}
