//! The row the Search and Locations panels both draw: a file, or one place found in it.
//!
//! The two ask different questions and hold their answers in different states, but what
//! comes back is the same shape -- places under the file each is in (`src/grouped.rs`) --
//! and so is the drawing of it. One component for both, with [`Folding`] naming the two
//! things they differ in: which state a fold is written to, and whether a press may
//! refuse the path it would open.

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
        Some(self.columns.start as usize..self.columns.end as usize)
    }
}

/// Which panel's answer a row belongs to: the state its fold is written to, and, the one
/// other thing the two differ in, whether a press may refuse the path it would open.
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

    /// Whether a press may open `path` at all.
    ///
    /// A hit came off a walk of the project's directory, where a file the source pane
    /// would refuse is a row a press should do nothing with. The Locations panel's places
    /// were named by a language server or the debug info, and for those, opening the file
    /// and letting the pane say what is wrong with it is the honest answer -- a `stat` in
    /// the way would silently swallow a move inside a tab already open.
    fn opens(self, path: &Path) -> bool {
        match self {
            Folding::Hits(_) => shows_as_source(path),
            Folding::Places(_) => true,
        }
    }
}

/// One row of a grouped answer: a file, or one of the places under it.
#[derive(Clone)]
pub(crate) struct PlaceRow<T> {
    pub(crate) row: Row<T>,
    pub(crate) folding: Folding,
    pub(crate) key: DiffKey,
}

/// The row is the whole of what is drawn: the states in [`Folding`] compare equal
/// whatever they hold, and a row never moves from one panel to the other.
impl<T: PartialEq> PartialEq for PlaceRow<T> {
    fn eq(&self, other: &Self) -> bool {
        self.row == other.row
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
        let open = use_open();
        let visits = use_consume::<Visited>().0;
        let ctrl = use_consume::<Ctrl>().0;
        let marked = use_consume::<Marked>().0;
        let landing = use_consume::<Land>().0;
        let plant = use_consume::<Plant>().0;
        let driven = use_consume::<Drives>().0;

        let folding = self.folding;
        let row = self.row.clone();
        let pressed = row.clone();
        let tooltip = match &row {
            Row::File { path, .. } => path.display().to_string(),
            Row::Item { path, item } => format!("{}:{}", path.display(), item.line()),
        };

        extra_tooltip(
            tooltip,
            list_row(hovering, false)
                .on_press(move |_| match &pressed {
                    Row::File { path, .. } => folding.toggle(path),
                    // A place opens its file as a source-driven tab, landed on the line
                    // and with the match or the name picked out. [`open_source_place`]
                    // (`agents/Panes.md`) is the arrival every door into a place in a
                    // source file makes, so both panels' rows open one the same way,
                    // down to the tab's assembly side being driven from that line.
                    Row::Item { path, item } => {
                        if !folding.opens(path) {
                            return;
                        }
                        open_source_place(
                            open,
                            visits,
                            marked,
                            landing,
                            plant,
                            driven,
                            path,
                            item.line(),
                            item.columns(),
                            reach(ctrl),
                        );
                    }
                })
                .children(row_children(&row)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// What a row draws: a file row is its fold, its name and its count; a place row is its
/// line number and the line, the matched or named part of it bold and in `match_fg`. A
/// file that would not read leaves the text empty, and the row is the number alone.
fn row_children<T: Place>(row: &Row<T>) -> Vec<Element> {
    match row {
        Row::File {
            name,
            count,
            folded,
            ..
        } => vec![
            chevron(Some(!*folded)).into_element(),
            tree_name(name.clone(), false).into_element(),
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
                .child(
                    paragraph()
                        .width(Size::fill())
                        .max_lines(1)
                        .text_overflow(TextOverflow::Ellipsis)
                        .spans_iter(marked_spans(item.text(), item.spans()).into_iter()),
                )
                .into_element(),
        ],
    }
}
