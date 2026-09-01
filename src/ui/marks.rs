//! A run of rows picked out to be copied, in either of the two code panes. One selection
//! for the window, so Ctrl+C has one answer, which is why the pane is part of it.

use super::*;

/// The run of rows a reader has picked out to be copied, and which pane it is in. Picking
/// a row in one pane drops whatever the other had.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Marks {
    pane: Pane,
    pub(crate) rows: RowSelection,
}

/// The picked-out rows, shared through context. `None` until something is picked, and
/// again whenever the listing under it is replaced.
#[derive(Clone, Copy)]
pub(crate) struct Marked(pub(crate) State<Option<Marks>>);

/// Whether Shift is held, which is what turns a click into "reach to here". Its own state,
/// written from the root's *global* key handlers, because a freya pointer event carries no
/// modifiers at all.
#[derive(Clone, Copy)]
pub(crate) struct Shift(pub(crate) State<bool>);

/// The rows picked out in `pane`, and nothing when the selection is the other pane's.
/// Reads rather than peeks: this is the subscription that repaints as the run grows.
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
        // The one-row run a press starts, which is a drag until the button comes up.
        _ => RowSelection {
            anchor: row,
            lead: row,
            dragging: true,
        },
    };

    marked.set_if_modified(Some(Marks { pane, rows }));
}

/// Sweep the run out to `row`, which does nothing unless a run is already started.
pub(crate) fn mark_drag(mut marked: State<Option<Marks>>, pane: Pane, row: usize) {
    let Some(marks) = *marked.peek() else {
        return;
    };
    if marks.pane != pane {
        return;
    }

    // Only while the button is down: a row entered with no button held is the pointer
    // merely passing over it.
    if !marks.rows.dragging {
        return;
    }

    marked.set_if_modified(Some(Marks {
        rows: marks.rows.extended(row),
        ..marks
    }));
}

/// End the gesture. The run stays: letting go ends the drag, not the selection.
///
/// The read is a `let` of its own and **not** the scrutinee of an `if let`: an `if let`
/// holds its temporary until the end of its *body*, so the write inside would be a
/// mutable borrow taken while the `peek` guard was still out, which panics.
pub(crate) fn mark_release(mut marked: State<Option<Marks>>) {
    let current = *marked.peek();

    if let Some(marks) = current {
        // The rows it holds are untouched: letting go is the end of the gesture and not
        // the end of the selection.
        marked.set_if_modified(Some(Marks {
            rows: RowSelection {
                dragging: false,
                ..marks.rows
            },
            ..marks
        }));
    }
}

/// Drop `pane`'s selection, and leave the other pane's alone.
fn unmark(mut marked: State<Option<Marks>>, pane: Pane) {
    if marked.peek().is_some_and(|marks| marks.pane == pane) {
        marked.set(None);
    }
}

/// What Ctrl+C, Ctrl+A and Escape do to a listing's selection.
///
/// Goes on the pane's own focusable box and **not** on a global key handler, which would
/// fire while a filter bar had the keyboard and — sorting last (`EventName::cmp`) — would
/// win, turning a copy out of the filter box into a page of disassembly.
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
                    // Failing silently: a platform whose display handle gave freya-winit
                    // no clipboard has none, and a listing has nowhere to say so.
                    // Each row's own line, in listing order, newline-separated — so a
                    // drag that went upwards copies what one that went down does.
                    let text = picked.rows.rows().map(&line).collect::<Vec<_>>().join("\n");
                    Clipboard::set(text).ok();
                }
            }
            Key::Character(character) if command && character == "a" => {
                // Every row of the listing, and nothing at all for one with no rows.
                if let Some(last) = rows.checked_sub(1) {
                    marked.set(Some(Marks {
                        pane,
                        rows: RowSelection {
                            anchor: 0,
                            lead: last,
                            dragging: false,
                        },
                    }));
                }
            }
            Key::Named(NamedKey::Escape) => unmark(marked, pane),
            _ => {}
        }
    }
}

/// Drop a pane's picked-out rows when the listing they index into is replaced: the
/// assembly pane's when the selection moves to another symbol, the source pane's when
/// another file is shown.
///
/// At the root and keyed on the states that say *which listing*, **never on the listings
/// themselves**: `AsmData` carries an `Arc<Lanes>` rebuilt every render, so an effect
/// inside each list would fire on every render and wipe the run the press just started.
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
    // and not a `State`: nothing renders from it.
    let showing = use_hook(|| Rc::new(RefCell::new(None::<Arc<str>>)));
    use_side_effect(move || {
        // The *file the Source pane is drawing*, which is not the active document: two
        // functions from one file leave the same lines on screen. Compared against what
        // it last was rather than answered to directly, since reading the analysis
        // subscribes this to writes -- a request, the slow flag -- that change no listing.
        let file =
            source_side(active.read().as_ref(), &analysis.read()).map(|side| side.file().clone());
        // Cloned out of the borrow before the `borrow_mut`.
        let was = showing.borrow().clone();
        if was == file {
            return;
        }
        *showing.borrow_mut() = file;

        unmark(marked, Pane::Source);
    });
}
