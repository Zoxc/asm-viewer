//! A run of rows picked out to be copied, in either of the two code panes.
//!
//! One selection for the window and not one per pane, because Ctrl+C must have one answer
//! and the keyboard focus -- which nothing draws -- deciding between two lit runs is a coin
//! flip. Which pane a run is in is therefore part of the selection rather than a second
//! state beside it.
//!
//! Dropping a run when the listing under it is replaced happens at the root, keyed on the
//! states that say which listing, and never in an effect inside either list.

use super::*;

/// The run of rows a reader has picked out to be copied, and which pane it is in.
///
/// One selection for the whole app rather than one per pane, and that is what the `pane`
/// is for. Ctrl+C has to have exactly one answer, and the pane it belongs to is not
/// something a reader can see: two runs lit at once in two panes, with the keyboard focus
/// -- which nothing draws -- deciding which of them lands on the clipboard, is a coin
/// flip dressed up as a feature. Picking a row in one pane therefore drops whatever the
/// other had, the way selecting in one text field drops the selection in the next.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Marks {
    pane: Pane,
    pub(crate) rows: RowSelection,
}

/// The picked-out rows, shared through context: written by the row the pointer is on and
/// read by the list that draws it and copies it. `None` until something is picked, and
/// again whenever the listing under it is replaced.
#[derive(Clone, Copy)]
pub(crate) struct Marked(pub(crate) State<Option<Marks>>);

/// Whether Shift is held, which is what turns a click into "reach to here".
///
/// Its own state, and written from the root's *global* key handlers, because a pointer
/// event carries no modifiers at all: `MouseEventData` is a location and a button
/// (freya-core `events/data.rs`), so the only way to know what the keyboard was doing
/// when a row was clicked is to have been watching it. freya-edit does the same thing for
/// the same reason -- `TextDragging::shift`, fed by `EditableEvent::KeyDown` -- but from
/// the focused editor's own handlers; global ones here so that the first shift-click
/// after a pane is reached works, rather than only the ones after it has the focus.
#[derive(Clone, Copy)]
pub(crate) struct Shift(pub(crate) State<bool>);

/// The rows picked out in `pane`, and nothing when the selection is the other pane's.
///
/// Reads rather than peeks: this is what a list calls to work out what its rows draw, so
/// it is the subscription that repaints them as the run grows.
pub(crate) fn marked_rows(marked: State<Option<Marks>>, pane: Pane) -> Option<RowSelection> {
    (*marked.read())
        .filter(|marks| marks.pane == pane)
        .map(|marks| marks.rows)
}

/// Start a run at `row`, or -- with Shift held, in the pane the run is already in --
/// reach out to it from wherever that run started.
pub(crate) fn mark_press(mut marked: State<Option<Marks>>, shift: bool, pane: Pane, row: usize) {
    let rows = match *marked.peek() {
        Some(marks) if shift && marks.pane == pane => marks.rows.extended(row),
        _ => RowSelection::at(row),
    };

    marked.set_if_modified(Some(Marks { pane, rows }));
}

/// Sweep the run out to `row`, which does nothing at all unless the button is still down
/// on it -- the pointer crossing a row is the hover, and the hover is answered elsewhere.
pub(crate) fn mark_drag(mut marked: State<Option<Marks>>, pane: Pane, row: usize) {
    let Some(marks) = *marked.peek() else {
        return;
    };
    if marks.pane != pane {
        return;
    }

    marked.set_if_modified(Some(Marks {
        rows: marks.rows.dragged_to(row),
        ..marks
    }));
}

/// End the gesture, wherever in the window the button came up. The run stays: letting go
/// is the end of the drag and not the end of the selection.
///
/// The read is a `let` of its own and not the scrutinee of an `if let`, which is the shape
/// this was written in first and which panicked on every mouse-up: a `State`'s `peek`
/// hands back a guard borrowing the state, and the temporary holding an `if let`'s
/// scrutinee lives until the end of its *body*, so the write inside was a mutable borrow
/// taken while that one was still out (`writable_utils.rs:96`). `mark_drag`'s `let ...
/// else` and `mark_press`'s `match` end their temporaries with the statement, which is
/// why the same code was fine there and why nothing about it is visible at the call site.
/// `Marks` is `Copy`, so binding it first costs nothing at all.
pub(crate) fn mark_release(mut marked: State<Option<Marks>>) {
    let current = *marked.peek();

    if let Some(marks) = current {
        marked.set_if_modified(Some(Marks {
            rows: marks.rows.released(),
            ..marks
        }));
    }
}

/// Drop `pane`'s selection, and leave the other pane's alone.
///
/// Called when the listing itself is replaced -- another symbol, another file -- because
/// the run is a range of row *indices*, and rows 40 to 60 of the function the reader just
/// left are not a thing to keep highlighted in the one they arrived at.
fn unmark(mut marked: State<Option<Marks>>, pane: Pane) {
    if marked.peek().is_some_and(|marks| marks.pane == pane) {
        marked.set(None);
    }
}

/// What Ctrl+C, Ctrl+A and Escape do to a listing's selection.
///
/// One handler for both panes, differing in the pane it answers for and in how a row of
/// it reads as text. It goes on the pane's own focusable box rather than on a global key
/// handler, which would fire while a filter bar had the keyboard: two things writing the
/// clipboard from one Ctrl+C, with the global one sorting last (`EventName::cmp`) and so
/// winning, would take a copy out of the filter box and give back a page of disassembly.
pub(crate) fn on_listing_key(
    marked: State<Option<Marks>>,
    pane: Pane,
    rows: usize,
    line: impl Fn(usize) -> String + 'static,
) -> impl FnMut(Event<KeyboardEventData>) + 'static {
    let mut marked = marked;

    move |e: Event<KeyboardEventData>| {
        let command = e.modifiers.contains(Modifiers::ctrl_or_meta());

        match &e.key {
            Key::Character(character) if command && character == "c" => {
                let picked = (*marked.peek()).filter(|marks| marks.pane == pane);
                if let Some(picked) = picked {
                    // Failing silently is the only answer there is: the clipboard is a
                    // root context freya-winit fills in from the window's display handle,
                    // so a platform that gave it none has none, and there is nowhere in a
                    // listing to say so.
                    Clipboard::set(picked.rows.text(&line)).ok();
                }
            }
            Key::Character(character) if command && character == "a" => {
                if let Some(rows) = RowSelection::all(rows) {
                    marked.set(Some(Marks { pane, rows }));
                }
            }
            Key::Named(NamedKey::Escape) => unmark(marked, pane),
            _ => {}
        }
    }
}

/// Drop a pane's picked-out rows when the listing they index into is replaced: the
/// assembly pane's when the selection moves to another symbol, the source pane's when
/// another file is shown. Rows 40 to 60 of the function just left are not rows 40 to 60
/// of the one arrived at.
///
/// Here, at the root, and keyed on the two states that say *which listing* -- and
/// deliberately not on the listings themselves. The obvious version is a
/// `use_side_effect_with_deps` inside each list, and it is wrong twice over: `AsmData`
/// carries an `Arc<Lanes>` built fresh on every render (7b), so it compares unequal to
/// itself and the effect would fire on every render, wiping the run the press had just
/// started -- which is exactly what the headless check caught -- and a dep compared by
/// pointer can be fooled by a new allocation landing where the old one was.
///
/// Its own effect rather than a third line in `use_clear_focus`, because the two answer
/// to different things: a focus and a pin are positions in the selected symbol's line
/// info and go when *it* does, while the source pane's run is a range of lines in a file
/// that a change of symbol very often leaves open.
pub(crate) fn use_clear_marks(
    active: Memo<Option<Document>>,
    analysis: State<Analyzed>,
    marked: State<Option<Marks>>,
) {
    use_side_effect(move || {
        let _ = active.read();
        unmark(marked, Pane::Assembly);
    });
    // Which file the Source pane was drawing the last time this ran. An `Rc<RefCell>`
    // and not a `State` for `use_kept_position`'s reason: nothing renders from it, and a
    // state here would cost the root a second render every time the pane changed file.
    let showing = use_hook(|| Rc::new(RefCell::new(None::<Arc<str>>)));
    use_side_effect(move || {
        // The *file the Source pane is drawing*, which is what its rows index into, and
        // which is not the active document: an assembly-driven tab draws its companion,
        // so switching from one function to another compiled from the same file leaves
        // the same lines on screen and the run picked out in them still means something.
        // `source_side` is the one place either pane works that out, so this cannot
        // disagree with what is drawn.
        //
        // Compared against what it last was rather than answered to directly, because
        // reading the analysis subscribes this to all of it — a request going out and the
        // slow flag turning over are writes to it that change no listing, and dropping a
        // run of rows on one of those would take it away under the reader's hand.
        let file =
            source_side(active.read().as_ref(), &analysis.read()).map(|side| side.file().clone());
        // Cloned out of the borrow before the `borrow_mut`, which panics exactly the way
        // a `State` guard held across a write does.
        let was = showing.borrow().clone();
        if was == file {
            return;
        }
        *showing.borrow_mut() = file;

        unmark(marked, Pane::Source);
    });
}
