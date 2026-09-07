//! The run picked out in each of the two code panes, and what each run means to the
//! other pane: the rows there that are the same place are lit, and a scroll to the first
//! of them is owed once. One run per pane, independent of the other's; nothing here
//! answers to the pointer. A run is the **characters** a sweep over a row's text makes
//! and the keyboard moves, which is what Ctrl+C copies; the rows it touches are what the
//! panes light in each other ([`CharSelection::rows`]).

use super::*;

/// The run a reader has picked out in one pane.
#[derive(Clone, PartialEq)]
pub(crate) struct Picked {
    /// The caret, and the characters picked out: anchored by the press -- at the column
    /// pressed on the text, at the row's start from the gutter or from outside the panes
    /// -- and swept from there. Empty until swept, and then it is the selection. The rows
    /// it touches are the run the two panes point at each other through, so there is no
    /// second copy of them: `chars.rows()` is what is lit.
    pub(crate) chars: CharSelection,
    /// Whether the button is still down, which is what tells a row entered under the
    /// pointer from the pointer merely passing over it.
    pub(crate) dragging: bool,
    /// Whether a sweep of this run goes **by rows**, whole ones from the anchor's to the
    /// pointer's: a run started in the gutter does, as a sweep down an editor's line
    /// numbers does; one started on the text goes by character.
    pub(crate) by_rows: bool,
    /// The file the run is read in: the source pane's own file for its run, and for the
    /// assembly pane's the file the pressed row was compiled from -- which is what the
    /// source pane shows beside an object's code. `None` where the row has no line.
    pub(crate) file: Option<Arc<str>>,
    /// Which panes still owe a scroll to this run: the other pane, for a click made in
    /// this one, and both for a run picked from outside them (a [`Landing`]). Each is
    /// cleared as it is paid, so a repeat click is a second request.
    pub(crate) owed: Owed,
}

impl Picked {
    /// Whether this is a run of the one row `line` of `file`, which is what every door
    /// onto a line makes ([`line_pick`]). The caret it holds on that row is its own: it
    /// may be at a column, where a line alone says only the row.
    pub(crate) fn is_line(&self, file: &Arc<str>, line: u32) -> bool {
        let row = (line as usize).saturating_sub(1);
        self.file.as_ref() == Some(file) && self.chars.rows() == (row..=row)
    }
}

/// Which of the two panes still owe a scroll to a run. A pair of flags and not an
/// `Option<Pane>`: a click in one pane asks the other, but a row in the Locations panel
/// is a click in neither and asks both.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Owed {
    pub(crate) assembly: bool,
    pub(crate) source: bool,
}

impl Owed {
    pub(crate) const BOTH: Owed = Owed {
        assembly: true,
        source: true,
    };

    /// A run neither pane has to scroll to: it is where the listing already is.
    pub(crate) const NEITHER: Owed = Owed {
        assembly: false,
        source: false,
    };

    /// A scroll owed by `pane` alone.
    pub(crate) fn by(pane: Pane) -> Owed {
        match pane {
            Pane::Assembly => Owed {
                assembly: true,
                source: false,
            },
            Pane::Source => Owed {
                assembly: false,
                source: true,
            },
        }
    }

    fn owes(self, pane: Pane) -> bool {
        match pane {
            Pane::Assembly => self.assembly,
            Pane::Source => self.source,
        }
    }

    fn paid(&mut self, pane: Pane) {
        match pane {
            Pane::Assembly => self.assembly = false,
            Pane::Source => self.source = false,
        }
    }
}

/// The two panes' runs. Either is `None` until something is picked out there, and again
/// whenever the listing under it is replaced.
#[derive(Clone, PartialEq, Default)]
pub(crate) struct Marks {
    pub(crate) assembly: Option<Picked>,
    pub(crate) source: Option<Picked>,
}

impl Marks {
    pub(crate) fn of(&self, pane: Pane) -> &Option<Picked> {
        match pane {
            Pane::Assembly => &self.assembly,
            Pane::Source => &self.source,
        }
    }

    fn of_mut(&mut self, pane: Pane) -> &mut Option<Picked> {
        match pane {
            Pane::Assembly => &mut self.assembly,
            Pane::Source => &mut self.source,
        }
    }
}

/// The picked-out rows of both panes, shared through context. The runs each place kept
/// while its tab showed something else are [`Places::marks_at`], forgotten with the place.
#[derive(Clone, Copy)]
pub(crate) struct Marked(pub(crate) State<Marks>);

impl Marks {
    /// The runs as a place keeps them while its tab is elsewhere: no gesture under way,
    /// and no scroll owed -- the kept scroll rows are what put each side back when the
    /// place is shown again, and a reveal owed beside them would fight them.
    pub(crate) fn settled(&self) -> Marks {
        let settle = |picked: &Option<Picked>| {
            picked.as_ref().map(|picked| Picked {
                dragging: false,
                owed: Owed::default(),
                ..picked.clone()
            })
        };
        Marks {
            assembly: settle(&self.assembly),
            source: settle(&self.source),
        }
    }
}

/// What a press on a row's text asked for, from where the pointer went down in it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Press {
    /// A caret at a column: a single press.
    At(usize),
    /// A run of the row's own columns: the word under a double press, the whole text
    /// under a triple.
    Span(usize, usize),
}

/// The other pane from `pane`.
fn other(pane: Pane) -> Pane {
    match pane {
        Pane::Assembly => Pane::Source,
        Pane::Source => Pane::Assembly,
    }
}

/// Change the two runs: the current [`Marks`] cloned, `edit` applied to that, and the
/// result put back only where it differs -- a write that changes nothing would draw both
/// lists again.
///
/// Every writer here goes through this so the guard rule is followed in one place: a
/// `peek` hands back a guard, and an `if let` holds its scrutinee's temporary until the
/// end of its *body*, so a write made inside one would be a mutable borrow taken while
/// the guard was still out, which panics. The clone is a statement of its own, so the
/// guard is gone before `edit` -- which is handed a plain `&mut Marks` and can write it
/// however it likes.
fn update(mut marked: State<Marks>, edit: impl FnOnce(&mut Marks)) {
    let mut marks = marked.peek().clone();
    edit(&mut marks);
    marked.set_if_modified(marks);
}

/// The run picked out in the *other* pane from `pane`, which is what `pane` lights the
/// pair of. Reads, for the same reason [`chars_of`] does.
pub(crate) fn pair_of(marked: State<Marks>, pane: Pane) -> Option<Picked> {
    marked.read().of(other(pane)).clone()
}

/// Whether a sweep is under way in either pane -- the button down on a run -- which is
/// what a control the sweep passes over asks before it answers the pointer: a tooltip
/// armed by a pointer that is dragging a selection past it is a tooltip nobody asked for,
/// and freya's arms on the hover alone (`notes/upstream/freya.md`). A read, so the
/// control re-renders as a sweep starts and ends.
pub(crate) fn sweeping(marked: State<Marks>) -> bool {
    let marks = marked.read();
    let dragging = |picked: &Option<Picked>| picked.as_ref().is_some_and(|p| p.dragging);
    dragging(&marks.assembly) || dragging(&marks.source)
}

/// The run `pane` holds -- the caret, the characters picked out, and so the rows lit --
/// for its rows to draw their part of, and `None` with no run. Reads rather than peeks:
/// this is the subscription that repaints as the run grows.
pub(crate) fn chars_of(marked: State<Marks>, pane: Pane) -> Option<CharSelection> {
    marked.read().of(pane).as_ref().map(|picked| picked.chars)
}

/// Start a run at `row` in `pane`, or -- with Shift held and a run already there -- reach
/// out to it from wherever that run started. `file` is what the pressed row is a row of
/// (see [`Picked::file`]); a reach keeps the file the run began in. `press` is what the
/// row's text answered where the press was on it, and `None` for a press in the gutter,
/// which puts the caret at the row's start and makes the sweep go by rows.
///
/// The other pane's run is left alone: the two are independent.
pub(crate) fn mark_press(
    marked: State<Marks>,
    shift: bool,
    pane: Pane,
    file: Option<Arc<str>>,
    row: usize,
    press: Option<Press>,
) {
    let current = marked.peek().of(pane).clone();
    let picked = match current {
        Some(picked) if shift => Picked {
            // The reach moves the lead to the column pressed on the text, and from the
            // gutter to the row's far end, whole rows being what the gutter reaches. It
            // arms the drag as well, so holding the button after a shift-click and
            // sweeping on carries the run out from there.
            chars: match press {
                Some(Press::At(col)) | Some(Press::Span(_, col)) => {
                    picked.chars.extended(Caret { row, col })
                }
                None => picked.chars.extended(Caret {
                    row,
                    col: if row >= picked.chars.ends().0.row {
                        crate::chars::END
                    } else {
                        0
                    },
                }),
            },
            dragging: true,
            ..picked
        },
        // The one-row run a press starts, which is a drag until the button comes up. The
        // other pane owes it a scroll: a click here asks the other side to show the
        // same place.
        _ => Picked {
            chars: match press {
                Some(Press::At(col)) => CharSelection::at(Caret { row, col }),
                Some(Press::Span(from, to)) => {
                    CharSelection::between(Caret { row, col: from }, Caret { row, col: to })
                }
                None => CharSelection::at(Caret { row, col: 0 }),
            },
            dragging: true,
            by_rows: press.is_none(),
            file,
            owed: Owed::by(other(pane)),
        },
    };

    update(marked, |marks| *marks.of_mut(pane) = Some(picked));
}

/// Pick out `row` of the assembly pane alone, replacing whatever was picked out there.
///
/// What [`mark_press`] does for a click, minus the drag: this is for a control that lands
/// the reader on a row they never pressed -- following a jump -- where the button is back
/// up by the time the answer is known and a sweep from here would be a sweep nobody began.
/// The source pane owes the scroll; the assembly pane has just been given one.
pub(crate) fn mark_row(marked: State<Marks>, file: Option<Arc<str>>, row: usize) {
    update(marked, |marks| {
        marks.assembly = Some(row_pick(file, row, Owed::by(Pane::Source)));
    });
}

/// Put the assembly pane's caret on `row`, at its start, as a [`Planting`] lands: the
/// door that opened the listing named an instruction, and this is the one run in the
/// pane, over whatever was there. `owed` is what the pane still owes it, which is its
/// own reveal in both listings: the tab's place (`Places::code_at`) can say where an object's
/// code sits but not that the rows before an instruction are part of showing it, and a
/// place given that margin would carry it into every restore and every switch.
///
/// The source pane's run, where the same door left one, stops owing this pane a scroll
/// to its pair: the caret **is** that pair, and one scroll to it is this pane's own or
/// its place's.
pub(crate) fn land_row(marked: State<Marks>, file: Option<Arc<str>>, row: usize, owed: Owed) {
    update(marked, |marks| {
        marks.assembly = Some(row_pick(file, row, owed));
        if let Some(source) = marks.source.as_mut() {
            source.owed.paid(Pane::Assembly);
        }
    });
}

/// The one-row run [`mark_row`] and [`land_row`] make of `row`: the row, and a caret at
/// its start.
fn row_pick(file: Option<Arc<str>>, row: usize, owed: Owed) -> Picked {
    Picked {
        chars: CharSelection::at(Caret { row, col: 0 }),
        dragging: false,
        by_rows: false,
        file,
        owed,
    }
}

/// Put a caret at the top of `pane`'s listing: what a pane the keyboard was *handed* does,
/// nothing in it having been clicked.
///
/// A listing with no run has no caret, so the arrows, Home, End and Ctrl+C have nothing to
/// act on and a pane that has just been given the keyboard reads as though it had not. The
/// file is left unsaid: this is a place in the listing and not a line of a file, so it
/// pairs with nothing on the other side and owes no scroll -- the top is where the listing
/// already is.
pub(crate) fn mark_top(marked: State<Marks>, pane: Pane) {
    update(marked, |marks| {
        let picked = row_pick(None, 0, Owed::NEITHER);
        match pane {
            Pane::Assembly => marks.assembly = Some(picked),
            Pane::Source => marks.source = Some(picked),
        }
    });
}

/// Pick out the one row `line` of `file` in the source pane, as a click from outside the
/// panes does: a [`Landing`], or the line a source-driven tab is driven from. `owed`
/// says which panes have yet to scroll to it.
pub(crate) fn mark_line(
    marked: State<Marks>,
    file: Arc<str>,
    line: u32,
    columns: Option<Range<usize>>,
    owed: Owed,
) {
    update(marked, |marks| {
        marks.source = Some(line_pick(file, line, columns, owed));
    });
}

/// The one-row run [`mark_line`] makes of `line`: the row, and a caret at its start --
/// or, where the door named `columns`, that run of the row's characters selected, which
/// is what a search hit lands on. Copying then copies the match and not the line, since
/// characters picked out are what `copy_text` prefers. An empty run is a caret at that
/// column and nothing selected, which is where following a name lands.
pub(crate) fn line_pick(
    file: Arc<str>,
    line: u32,
    columns: Option<Range<usize>>,
    owed: Owed,
) -> Picked {
    let row = (line as usize).saturating_sub(1);
    let chars = match columns {
        Some(columns) => CharSelection::between(
            Caret {
                row,
                col: columns.start,
            },
            Caret {
                row,
                col: columns.end,
            },
        ),
        None => CharSelection::at(Caret { row, col: 0 }),
    };
    Picked {
        chars,
        dragging: false,
        by_rows: false,
        file: Some(file),
        owed,
    }
}

/// Sweep `pane`'s run out to `row`, which does nothing unless a run is already started.
/// `col` is the column under the pointer where the row has text, and the characters
/// follow it; a row with no text, or a gutter, is column 0. A run started in the gutter
/// sweeps by rows instead, whole ones, and the column is not asked.
pub(crate) fn mark_drag(marked: State<Marks>, pane: Pane, row: usize, col: Option<usize>) {
    let Some(picked) = marked.peek().of(pane).clone() else {
        return;
    };
    // Only while the button is down: a row entered with no button held is the pointer
    // merely passing over it.
    if !picked.dragging {
        return;
    }

    update(marked, |marks| {
        *marks.of_mut(pane) = Some(Picked {
            chars: if picked.by_rows {
                picked.chars.by_rows(row)
            } else {
                picked.chars.extended(Caret {
                    row,
                    col: col.unwrap_or(0),
                })
            },
            ..picked
        });
    });
}

/// End the gesture, in whichever pane it was made. The run stays: letting go ends the
/// drag, not the selection.
pub(crate) fn mark_release(marked: State<Marks>) {
    update(marked, |marks| {
        if let Some(picked) = marks.assembly.as_mut() {
            picked.dragging = false;
        }
        if let Some(picked) = marks.source.as_mut() {
            picked.dragging = false;
        }
    });
}

/// Drop `pane`'s run, and leave the other pane's alone.
pub(crate) fn unmark(marked: State<Marks>, pane: Pane) {
    if marked.peek().of(pane).is_none() {
        return;
    }
    update(marked, |marks| *marks.of_mut(pane) = None);
}

/// What `pane` still owes a scroll to.
pub(crate) enum Owing {
    /// Its own run, picked from outside the panes, whose first row it has yet to show.
    Own(RangeInclusive<usize>),
    /// The other pane's run, whose pair here it has yet to bring into view.
    Pair(Picked),
}

/// The scroll `pane` still owes, if it is owed one.
///
/// **A look and not a take.** The click that picks a line out is, in a source-driven
/// tab, the click that asks for the listing, so the run this wakes is still holding the
/// *previous* one, in which no row matches. Consuming the request there would spend it on
/// a listing that cannot answer it and the one that can would arrive to nothing owed. So
/// the flag is left meaning what it says -- the pane owes the scroll until it has made
/// it -- and [`reveal_made`] is what clears it. A request nothing ever matches stays owed
/// until the next click replaces it or the run is dropped with its listing.
pub(crate) fn owed_reveal(marked: State<Marks>, pane: Pane) -> Option<Owing> {
    // `read` and not `peek`: this is the subscription that wakes the caller's effect on
    // the next click, so it has to happen before any early return.
    let marks = marked.read();
    if let Some(own) = marks.of(pane).as_ref().filter(|own| own.owed.owes(pane)) {
        return Some(Owing::Own(own.chars.rows()));
    }
    marks
        .of(other(pane))
        .as_ref()
        .filter(|pair| pair.owed.owes(pane))
        .map(|pair| Owing::Pair(pair.clone()))
}

/// Say that `pane` has made the scroll it was owed. The runs themselves stay, only
/// `pane`'s flag is cleared, so it is answered exactly once and a repeat click is a
/// second request.
pub(crate) fn reveal_made(mut marked: State<Marks>, pane: Pane) {
    let owed = {
        let marks = marked.peek();
        let owes = |picked: &Option<Picked>| picked.as_ref().is_some_and(|p| p.owed.owes(pane));
        owes(&marks.assembly) || owes(&marks.source)
    };
    if !owed {
        return;
    }

    let mut marks = marked.write();
    if let Some(picked) = marks.assembly.as_mut() {
        picked.owed.paid(pane);
    }
    if let Some(picked) = marks.source.as_mut() {
        picked.owed.paid(pane);
    }
}

/// What Ctrl+C takes from `pane`'s run: the characters, where any are selected, and
/// otherwise the caret's row whole -- its own `line`, address and all, as an editor
/// copies the line under a caret with nothing selected. `text` is a row's text as it is
/// drawn, which is what the characters are columns of. `None` with no run at all.
pub(crate) fn copy_text(
    marks: &Marks,
    pane: Pane,
    line: impl Fn(usize) -> String,
    text: impl Fn(usize) -> Line,
) -> Option<String> {
    let picked = marks.of(pane).as_ref()?;
    if picked.chars.is_empty() {
        // Nothing selected is a caret, and the run of a caret is its own row.
        Some(line(picked.chars.lead().row))
    } else {
        Some(picked.chars.copy(text))
    }
}

/// What the keyboard does to a listing's selection: Ctrl+C, Ctrl+A and Escape, and the
/// caret's keys -- the arrows by character and, with Ctrl, by word; Home and End to the
/// row's ends and, with Ctrl, the listing's; Page Up and Page Down by a screen of rows
/// -- each reaching the run out with Shift and collapsing it without ([`move_caret`]).
/// `viewport` is how tall the list is, which is what a page is, and `reveal` is asked to
/// bring the caret's row on screen after each move.
///
/// Goes on the pane's own focusable box and **not** on a global key handler, which would
/// fire while a filter bar had the keyboard and — sorting last (`EventName::cmp`) — would
/// win, turning a copy out of the filter box into a page of disassembly. And it is the
/// pane's own run that is copied: each pane has one, and the keyboard is in one of them.
///
/// `file` is what this listing's rows are rows of -- the source list's own file, and
/// `None` for the two assembly listings, where a run's file is the row's own. It is what
/// a run made from nothing takes ([`Picked::file`]): the keyboard reaches a pane with no
/// press on a row, a press on the tab's chip being enough.
pub(crate) fn on_listing_key(
    marked: State<Marks>,
    pane: Pane,
    file: Option<Arc<str>>,
    rows: usize,
    viewport: State<f32>,
    line: impl Fn(usize) -> String + 'static,
    text: impl Fn(usize) -> Line + 'static,
    mut reveal: impl FnMut(usize) + 'static,
) -> impl FnMut(Event<KeyboardEventData>) + 'static {
    move |e: Event<KeyboardEventData>| {
        let command = e.modifiers.contains(Modifiers::ctrl_or_meta());
        let shift = e.modifiers.contains(Modifiers::SHIFT);

        let motion = match &e.key {
            Key::Named(NamedKey::ArrowLeft) if command => Some(Motion::WordLeft),
            Key::Named(NamedKey::ArrowRight) if command => Some(Motion::WordRight),
            Key::Named(NamedKey::ArrowLeft) => Some(Motion::Left),
            Key::Named(NamedKey::ArrowRight) => Some(Motion::Right),
            Key::Named(NamedKey::ArrowUp) => Some(Motion::Up),
            Key::Named(NamedKey::ArrowDown) => Some(Motion::Down),
            Key::Named(NamedKey::Home) if command => Some(Motion::ListingStart),
            Key::Named(NamedKey::End) if command => Some(Motion::ListingEnd),
            Key::Named(NamedKey::Home) => Some(Motion::RowStart),
            Key::Named(NamedKey::End) => Some(Motion::RowEnd),
            Key::Named(NamedKey::PageUp) => Some(Motion::PageUp),
            Key::Named(NamedKey::PageDown) => Some(Motion::PageDown),
            _ => None,
        };
        if let Some(motion) = motion {
            // A page is the rows the list shows whole; the motion makes one of none.
            let page = (*viewport.peek() / code_row_height()).floor().max(0.0) as usize;
            let moved = move_caret(marked, pane, motion, shift, rows, page, &text);
            if let Some(row) = moved {
                reveal(row);
            }
            return;
        }

        match &e.key {
            Key::Character(character) if command && character == "c" => {
                let copied = copy_text(&marked.peek(), pane, &line, &text);
                if let Some(copied) = copied {
                    // Failing silently: a platform whose display handle gave freya-winit
                    // no clipboard has none, and a listing has nowhere to say so.
                    Clipboard::set(copied).ok();
                }
            }
            Key::Character(character) if command && character == "a" => {
                // Every row of the listing, first row's start to last row's end, and
                // nothing at all for one with no rows. The file stays what the run's was,
                // or the listing's own where there is no run yet, and no scroll is owed:
                // the whole listing names no one place to go to.
                if let Some(last) = rows.checked_sub(1) {
                    let file = marked
                        .peek()
                        .of(pane)
                        .as_ref()
                        .and_then(|picked| picked.file.clone())
                        .or_else(|| file.clone());
                    update(marked, |marks| {
                        *marks.of_mut(pane) = Some(Picked {
                            chars: CharSelection::between(
                                Caret { row: 0, col: 0 },
                                Caret {
                                    row: last,
                                    col: crate::chars::END,
                                },
                            ),
                            dragging: false,
                            by_rows: false,
                            file,
                            owed: Owed::default(),
                        });
                    });
                }
            }
            Key::Named(NamedKey::Escape) => peel(marked, pane),
            _ => {}
        }
    }
}

/// Move `pane`'s caret by `motion`, reaching the run out from its anchor with `extend`
/// (Shift held) and collapsing it to the caret without; the row of the caret it left it
/// at, for the pane to reveal, and `None` where there was nothing to move: a listing
/// with no run at all does nothing with the key.
///
/// The rows follow the caret, since they are the place the panes point at each other
/// through: a one-row run at the caret's row, or with `extend` the run reached out to
/// it. No drag, and **no scroll owed** to the other pane: it would be paid on every
/// repeat of a held key, yanking the other pane about while the reader walks this one.
/// The file stays what the run's was.
fn move_caret(
    marked: State<Marks>,
    pane: Pane,
    motion: Motion,
    extend: bool,
    length: usize,
    page: usize,
    text: impl Fn(usize) -> Line,
) -> Option<usize> {
    let picked = marked.peek().of(pane).clone()?;
    length.checked_sub(1)?;
    let moved = picked.chars.moved(motion, extend, text, length, page);
    let row = moved.lead().row;
    update(marked, |marks| {
        *marks.of_mut(pane) = Some(Picked {
            chars: moved,
            dragging: false,
            by_rows: false,
            owed: Owed::default(),
            ..picked
        });
    });
    Some(row)
}

/// Carry the assembly pane's run to the rows a recount gave it: each end of it put
/// through `map`, which answers a row of the old count with the row of the new; a run
/// either end of which has no row any more is dropped. The columns stay: a row's text is
/// the same text wherever its row is now.
pub(crate) fn carry_assembly(marked: State<Marks>, map: impl Fn(usize) -> Option<usize>) {
    let Some(picked) = marked.peek().assembly.clone() else {
        return;
    };
    set_assembly(marked, carried(&picked, map));
}

/// Put `picked` in the assembly pane, in place of whatever run was there.
pub(crate) fn set_assembly(marked: State<Marks>, picked: Option<Picked>) {
    update(marked, |marks| marks.assembly = picked);
}

/// `picked` with each end put through `map`; `None` where either has no answer. The
/// columns stay: a row's text is the same text wherever its row is now, and so does which
/// end is the caret: a run swept upwards keeps its lead at the top.
pub(crate) fn carried(picked: &Picked, map: impl Fn(usize) -> Option<usize>) -> Option<Picked> {
    Some(Picked {
        chars: picked.chars.mapped(&map)?,
        ..picked.clone()
    })
}

/// Collapse `pane`'s selection to its caret where anything is selected, the rows to the
/// caret's row with it, and otherwise drop the run: Escape peels the selection back a
/// layer at a time, as an editor's does, and the second press takes the place the panes
/// point at each other through.
fn peel(marked: State<Marks>, pane: Pane) {
    let Some(picked) = marked.peek().of(pane).clone() else {
        return;
    };
    update(marked, |marks| {
        *marks.of_mut(pane) = if picked.chars.is_empty() {
            None
        } else {
            Some(Picked {
                chars: picked.chars.collapsed(),
                dragging: false,
                by_rows: false,
                ..picked
            })
        };
    });
}
