//! The run of rows picked out in each of the two code panes, and what each run means to
//! the other pane: the rows there that are the same place are lit, and a scroll to the
//! first of them is owed once. One run per pane, independent of the other's; nothing
//! here answers to the pointer. Beside the rows, the run of **characters** a sweep over a
//! row's text makes, which is what Ctrl+C copies when there is one.

use freya::engine::prelude::{RectHeightStyle, RectWidthStyle};

use super::*;

/// The run of rows a reader has picked out in one pane.
#[derive(Clone, PartialEq)]
pub(crate) struct Picked {
    pub(crate) rows: RowSelection,
    /// The characters picked out, where the run was started on a row's text and not in
    /// its gutter: anchored by the press, swept with the rows. Empty until swept, and
    /// `None` for a run of rows alone.
    pub(crate) chars: Option<CharSelection>,
    /// The file the run is read in: the source pane's own file for its run, and for the
    /// assembly pane's the file the pressed row was compiled from -- which is what the
    /// source pane shows beside an object's code. `None` where the row has no line.
    pub(crate) file: Option<Arc<str>>,
    /// Which panes still owe a scroll to this run: the other pane, for a click made in
    /// this one, and both for a run picked from outside them (a [`Landing`]). Each is
    /// cleared as it is paid, so a repeat click is a second request.
    pub(crate) owed: Owed,
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

/// The picked-out rows of both panes, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Marked(pub(crate) State<Marks>);

/// Whether Shift is held, which is what turns a click into "reach to here". Its own state,
/// written from the root's *global* key handlers, because a freya pointer event carries no
/// modifiers at all.
#[derive(Clone, Copy)]
pub(crate) struct Shift(pub(crate) State<bool>);

/// Whether Ctrl is held, tracked as [`Shift`] is and for the same reason: it is what turns a
/// press on a symbol's label in the unified view into opening the symbol's tab, where a
/// plain press is a plain press -- picking the row out, and nothing that changes the tab.
#[derive(Clone, Copy)]
pub(crate) struct Ctrl(pub(crate) State<bool>);

/// The two modifiers as the root's global key handlers keep them, and what it takes to keep
/// them right.
///
/// A key event carries the key's own name and the modifier mask **as it was before the
/// key**: on Wayland the compositor sends the key and then the modifiers, and freya keeps
/// only the mask, handing it to the next key event and never forwarding the change. So a
/// modifier's own press is known by its *name* and its release by its name too, and the
/// mask is what recovers when a key event was missed. That is what a Caps Lock made into
/// Ctrl by the desktop breaks: KDE's `caps:ctrl_modifier` keeps the key's name and adds the
/// Control action, so its press names Caps Lock over a mask without Ctrl, and its release
/// names Caps Lock over a mask with Ctrl still in it -- which read as a press missed and
/// left Ctrl stuck on. Nothing freya exposes says what the mask became, so the keyboard
/// is **learnt**: a Caps Lock coming up with Ctrl in the mask while no Control key is down
/// acts as Ctrl, and from then on its press counts and its release clears
/// (`notes/upstream/freya.md`).
#[derive(Clone, Copy)]
pub(crate) struct ModifierKeys {
    shift: State<bool>,
    ctrl: State<bool>,
    /// Whether this keyboard's Caps Lock has shown itself to be a Ctrl.
    caps_is_ctrl: State<bool>,
    /// Whether a key *named* Control is down, which is what tells a Caps Lock released
    /// under a real Ctrl from one that is the Ctrl.
    control_held: State<bool>,
}

impl ModifierKeys {
    pub(crate) fn new(
        shift: State<bool>,
        ctrl: State<bool>,
        caps_is_ctrl: State<bool>,
        control_held: State<bool>,
    ) -> Self {
        Self {
            shift,
            ctrl,
            caps_is_ctrl,
            control_held,
        }
    }

    /// A key went down: `key` under `modifiers`, the mask as it was before it.
    pub(crate) fn down(mut self, key: &Key, modifiers: Modifiers) {
        self.shift.set_if_modified(
            *key == Key::Named(NamedKey::Shift) || modifiers.contains(Modifiers::SHIFT),
        );
        let control = *key == Key::Named(NamedKey::Control);
        if control {
            self.control_held.set_if_modified(true);
        }
        let caps = *key == Key::Named(NamedKey::CapsLock) && *self.caps_is_ctrl.peek();
        self.ctrl
            .set_if_modified(control || caps || modifiers.contains(Modifiers::CONTROL));
    }

    /// A key came up: `key` under `modifiers`, the mask as it was before it.
    pub(crate) fn up(mut self, key: &Key, modifiers: Modifiers) {
        self.shift.set_if_modified(
            *key != Key::Named(NamedKey::Shift) && modifiers.contains(Modifiers::SHIFT),
        );
        let control = *key == Key::Named(NamedKey::Control);
        if control {
            self.control_held.set_if_modified(false);
        }
        let caps = *key == Key::Named(NamedKey::CapsLock);
        // The one event that shows a Caps Lock for the Ctrl it is: up, with Ctrl in the
        // mask, and no key named Control down to account for it.
        let learnt = caps && modifiers.contains(Modifiers::CONTROL) && !*self.control_held.peek();
        if learnt {
            self.caps_is_ctrl.set_if_modified(true);
        }
        let caps_is_ctrl = learnt || *self.caps_is_ctrl.peek();
        self.ctrl.set_if_modified(
            !control && !(caps && caps_is_ctrl) && modifiers.contains(Modifiers::CONTROL),
        );
    }
}

/// The right button's half of a `pointer_down`, as the press a context menu opens from,
/// or `None` for any other button.
///
/// freya's `on_secondary_down` is `on_pointer_down` under another name, and an element
/// keeps one handler per event, so a row that picks itself out on the down and opens a
/// menu on the down has to do both in one handler -- the later of the two would replace
/// the earlier and the press would pick out nothing (`notes/upstream/freya.md`).
pub(crate) fn secondary(e: Event<PointerEventData>) -> Option<Event<PressEventData>> {
    e.try_map(|data| match data {
        PointerEventData::Mouse(mouse) if mouse.button == Some(MouseButton::Right) => {
            Some(PressEventData::Mouse(mouse))
        }
        _ => None,
    })
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

/// The column of a laid-out paragraph under a point `x`, `y` in its own logical
/// coordinates, in the UTF-16 units the text engine counts in. A point left of the text is
/// column 0 and one right of it is the end. `None` before the paragraph has been laid out,
/// which is a holder freya's own code unwraps and which a press cannot reach, the row
/// having nothing to press on until it is drawn.
pub(crate) fn caret_col(holder: &ParagraphHolder, x: f32, y: f32) -> Option<usize> {
    let inner = holder.0.borrow();
    let inner = inner.as_ref()?;
    let scale = inner.scale_factor as f32;
    let at = inner
        .paragraph
        .get_glyph_position_at_coordinate(((x * scale) as i32, (y * scale) as i32));
    Some(at.position.max(0) as usize)
}

/// Where column `col` of a laid-out paragraph is, in logical pixels from its left edge: the
/// left of the character there, or the right of the last one for the column past the end,
/// and 0 for an empty text. `None` before layout, as [`caret_col`] is.
pub(crate) fn caret_x(holder: &ParagraphHolder, col: usize) -> Option<f32> {
    let inner = holder.0.borrow();
    let inner = inner.as_ref()?;
    let scale = inner.scale_factor as f32;
    let rects = |from: usize, to: usize| {
        inner
            .paragraph
            .get_rects_for_range(from..to, RectHeightStyle::Tight, RectWidthStyle::Tight)
    };
    let x = match rects(col, col + 1).first() {
        Some(text) => text.rect.left,
        None => match col
            .checked_sub(1)
            .and_then(|before| rects(before, col).first().map(|text| text.rect.right))
        {
            Some(right) => right,
            None => 0.0,
        },
    };
    Some(x / scale)
}

/// The word around column `col` of a laid-out paragraph, as the text engine divides
/// words; `None` before layout, as [`caret_col`] is.
pub(crate) fn word_at(holder: &ParagraphHolder, col: usize) -> Option<(usize, usize)> {
    let inner = holder.0.borrow();
    let inner = inner.as_ref()?;
    let range = inner
        .paragraph
        .get_word_boundary(col.min(u32::MAX as usize) as u32);
    Some((range.start, range.end))
}

/// The other pane from `pane`.
fn other(pane: Pane) -> Pane {
    match pane {
        Pane::Assembly => Pane::Source,
        Pane::Source => Pane::Assembly,
    }
}

/// The rows picked out in `pane`. Reads rather than peeks: this is the subscription that
/// repaints as the run grows.
pub(crate) fn marked_rows(marked: State<Marks>, pane: Pane) -> Option<RowSelection> {
    marked.read().of(pane).as_ref().map(|picked| picked.rows)
}

/// The run picked out in the *other* pane from `pane`, which is what `pane` lights the
/// pair of. Reads, for the same reason [`marked_rows`] does.
pub(crate) fn pair_of(marked: State<Marks>, pane: Pane) -> Option<Picked> {
    marked.read().of(other(pane)).clone()
}

/// The run of characters `pane`'s run holds, for the rows to draw their part of. Reads,
/// for the reason [`marked_rows`] does.
pub(crate) fn chars_of(marked: State<Marks>, pane: Pane) -> Option<CharSelection> {
    marked
        .read()
        .of(pane)
        .as_ref()
        .and_then(|picked| picked.chars)
}

/// Start a run at `row` in `pane`, or -- with Shift held and a run already there -- reach
/// out to it from wherever that run started. `file` is what the pressed row is a row of
/// (see [`Picked::file`]); a reach keeps the file the run began in. `press` is what the
/// row's text answered where the press was on it, and `None` for a press in the gutter:
/// the rows are picked out either way, and the characters only from the text.
///
/// The other pane's run is left alone: the two are independent.
pub(crate) fn mark_press(
    mut marked: State<Marks>,
    shift: bool,
    pane: Pane,
    file: Option<Arc<str>>,
    row: usize,
    press: Option<Press>,
) {
    let current = marked.peek().of(pane).clone();
    let picked = match current {
        Some(picked) if shift => Picked {
            rows: picked.rows.extended(row),
            // The reach moves the characters' lead with the rows', where both sides have
            // one; a reach from the gutter, or to it, leaves the characters as they were.
            chars: match (picked.chars, press) {
                (Some(chars), Some(Press::At(col))) => Some(chars.extended(Caret { row, col })),
                (Some(chars), Some(Press::Span(_, col))) => {
                    Some(chars.extended(Caret { row, col }))
                }
                (chars, _) => chars,
            },
            ..picked
        },
        // The one-row run a press starts, which is a drag until the button comes up. The
        // other pane owes it a scroll: a click here asks the other side to show the
        // same place.
        _ => Picked {
            rows: RowSelection {
                anchor: row,
                lead: row,
                dragging: true,
            },
            chars: press.map(|press| match press {
                Press::At(col) => CharSelection::at(Caret { row, col }),
                Press::Span(from, to) => {
                    CharSelection::between(Caret { row, col: from }, Caret { row, col: to })
                }
            }),
            file,
            owed: Owed::by(other(pane)),
        },
    };

    let mut marks = marked.peek().clone();
    *marks.of_mut(pane) = Some(picked);
    marked.set_if_modified(marks);
}

/// Pick out `row` of the assembly pane alone, replacing whatever was picked out there.
///
/// What [`mark_press`] does for a click, minus the drag: this is for a control that lands
/// the reader on a row they never pressed -- following a jump -- where the button is back
/// up by the time the answer is known and a sweep from here would be a sweep nobody began.
/// The source pane owes the scroll; the assembly pane has just been given one.
pub(crate) fn mark_row(mut marked: State<Marks>, file: Option<Arc<str>>, row: usize) {
    let mut marks = marked.peek().clone();
    marks.assembly = Some(Picked {
        rows: RowSelection {
            anchor: row,
            lead: row,
            dragging: false,
        },
        chars: None,
        file,
        owed: Owed::by(Pane::Source),
    });
    marked.set_if_modified(marks);
}

/// Pick out the one row `line` of `file` in the source pane, as a click from outside the
/// panes does: a [`Landing`], or the line a source-driven tab is driven from. `owed`
/// says which panes have yet to scroll to it.
pub(crate) fn mark_line(mut marked: State<Marks>, file: Arc<str>, line: u32, owed: Owed) {
    let row = (line as usize).saturating_sub(1);
    let mut marks = marked.peek().clone();
    marks.source = Some(Picked {
        rows: RowSelection {
            anchor: row,
            lead: row,
            dragging: false,
        },
        chars: None,
        file: Some(file),
        owed,
    });
    marked.set_if_modified(marks);
}

/// Sweep `pane`'s run out to `row`, which does nothing unless a run is already started.
/// `col` is the column under the pointer where the row has text, and its characters --
/// where the run has any -- follow it; a row with no text, or a gutter, is column 0.
pub(crate) fn mark_drag(mut marked: State<Marks>, pane: Pane, row: usize, col: Option<usize>) {
    let Some(picked) = marked.peek().of(pane).clone() else {
        return;
    };
    // Only while the button is down: a row entered with no button held is the pointer
    // merely passing over it.
    if !picked.rows.dragging {
        return;
    }

    let mut marks = marked.peek().clone();
    *marks.of_mut(pane) = Some(Picked {
        rows: picked.rows.extended(row),
        chars: picked.chars.map(|chars| {
            chars.extended(Caret {
                row,
                col: col.unwrap_or(0),
            })
        }),
        ..picked
    });
    marked.set_if_modified(marks);
}

/// End the gesture, in whichever pane it was made. The run stays: letting go ends the
/// drag, not the selection.
///
/// The read is a `let` of its own and **not** the scrutinee of an `if let`: an `if let`
/// holds its temporary until the end of its *body*, so the write inside would be a
/// mutable borrow taken while the `peek` guard was still out, which panics.
pub(crate) fn mark_release(mut marked: State<Marks>) {
    let mut marks = marked.peek().clone();
    if let Some(picked) = marks.assembly.as_mut() {
        picked.rows.dragging = false;
    }
    if let Some(picked) = marks.source.as_mut() {
        picked.rows.dragging = false;
    }
    marked.set_if_modified(marks);
}

/// Drop `pane`'s run, and leave the other pane's alone.
fn unmark(mut marked: State<Marks>, pane: Pane) {
    if marked.peek().of(pane).is_none() {
        return;
    }
    let mut marks = marked.peek().clone();
    *marks.of_mut(pane) = None;
    marked.set(marks);
}

/// What `pane` still owes a scroll to.
pub(crate) enum Owing {
    /// Its own run, picked from outside the panes, whose first row it has yet to show.
    Own(RowSelection),
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
        return Some(Owing::Own(own.rows));
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

/// What Ctrl+C takes from `pane`'s run: the characters, where a sweep picked any out, and
/// otherwise the rows -- each row's own `line`, in listing order, newline-separated, so a
/// sweep that went upwards copies what one that went down does. `text` is a row's text as
/// it is drawn, which is what the characters are columns of. `None` with no run at all.
pub(crate) fn copy_text(
    marks: &Marks,
    pane: Pane,
    line: impl Fn(usize) -> String,
    text: impl Fn(usize) -> Line,
) -> Option<String> {
    let picked = marks.of(pane).as_ref()?;
    match picked.chars.filter(|chars| !chars.is_empty()) {
        Some(chars) => Some(chars.copy(text)),
        None => Some(picked.rows.rows().map(line).collect::<Vec<_>>().join("\n")),
    }
}

/// What Ctrl+C, Ctrl+A and Escape do to a listing's selection.
///
/// Goes on the pane's own focusable box and **not** on a global key handler, which would
/// fire while a filter bar had the keyboard and — sorting last (`EventName::cmp`) — would
/// win, turning a copy out of the filter box into a page of disassembly. And it is the
/// pane's own run that is copied: each pane has one, and the keyboard is in one of them.
pub(crate) fn on_listing_key(
    marked: State<Marks>,
    pane: Pane,
    rows: usize,
    line: impl Fn(usize) -> String + 'static,
    text: impl Fn(usize) -> Line + 'static,
) -> impl FnMut(Event<KeyboardEventData>) + 'static {
    let mut marked = marked;

    move |e: Event<KeyboardEventData>| {
        let command = e.modifiers.contains(Modifiers::ctrl_or_meta());

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
                // Every row of the listing, and nothing at all for one with no rows. The
                // file stays what the run's was, and no scroll is owed: the whole
                // listing names no one place to go to.
                if let Some(last) = rows.checked_sub(1) {
                    let mut marks = marked.peek().clone();
                    let file = marks
                        .of(pane)
                        .as_ref()
                        .and_then(|picked| picked.file.clone());
                    *marks.of_mut(pane) = Some(Picked {
                        rows: RowSelection {
                            anchor: 0,
                            lead: last,
                            dragging: false,
                        },
                        chars: None,
                        file,
                        owed: Owed::default(),
                    });
                    marked.set(marks);
                }
            }
            Key::Named(NamedKey::Escape) => peel(marked, pane),
            _ => {}
        }
    }
}

/// Drop `pane`'s picked-out characters where a sweep made any, and otherwise the run
/// itself: Escape peels the selection back a layer at a time, since the rows are a place
/// the panes point at each other through and the characters are only what is copied.
fn peel(mut marked: State<Marks>, pane: Pane) {
    let Some(picked) = marked.peek().of(pane).clone() else {
        return;
    };
    let mut marks = marked.peek().clone();
    *marks.of_mut(pane) = match picked.chars.filter(|chars| !chars.is_empty()) {
        Some(_) => Some(Picked {
            chars: None,
            ..picked
        }),
        None => None,
    };
    marked.set(marks);
}

/// Drop a pane's picked-out rows when the listing they index into is replaced: the
/// assembly pane's when another question is asked, the source pane's when the pane moves
/// off the run's file.
///
/// At the root and keyed on the states that say *which listing*, **never on the listings
/// themselves**: `AsmData` carries an `Arc<Lanes>` rebuilt every render, so an effect
/// inside each list would fire on every render and wipe the run the press just started.
pub(crate) fn use_clear_marks(
    active: Memo<Option<Document>>,
    asked: Asked,
    analysis: State<Analyzed>,
    reading: State<Reading>,
    marked: State<Marks>,
) {
    // The **question** and not the active document: a source-driven tab's listing is
    // replaced when a line in it is clicked, which changes no document, and a run picked
    // out of the last line's function would survive into the next one's as raw row
    // indices. Every document change that can have a run under it changes the question
    // too, so this is a superset of what it was.
    use_side_effect(move || {
        let _ = asked.read_ask();
        unmark(marked, Pane::Assembly);
    });
    // And the rows of an object's code, which are counted afresh with every answer that
    // lands: a run of them is listing rows too, and would survive the rows shifting under
    // it. Keyed on the reading's generation and compared against what it last was, since
    // reading the state subscribes this to the asks the view makes as it scrolls, which
    // change no row.
    let counted = use_hook(|| Rc::new(RefCell::new(None::<u64>)));
    use_side_effect(move || {
        let generation = reading.read().generation;
        let was = *counted.borrow();
        if was == Some(generation) {
            return;
        }
        *counted.borrow_mut() = Some(generation);
        if was.is_some() {
            unmark(marked, Pane::Assembly);
        }
    });
    // Which file the Source pane was drawing the last time this ran. An `Rc<RefCell>`
    // and not a `State`: nothing renders from it.
    let showing = use_hook(|| Rc::new(RefCell::new(None::<Arc<str>>)));
    use_side_effect(move || {
        // The *file the Source pane is drawing*, which is not the active document: two
        // functions from one file leave the same lines on screen. Compared against what
        // it last was rather than answered to directly, since reading the analysis
        // subscribes this to writes -- a request, the slow flag -- that change no listing.
        let file = source_side(active.read().as_ref(), &analysis.read(), &marked.read())
            .map(|side| side.file().clone());
        // Cloned out of the borrow before the `borrow_mut`.
        let was = showing.borrow().clone();
        if was == file {
            return;
        }
        *showing.borrow_mut() = file;

        // Dropped only when the pane moves **off the run's file**, and not whenever the
        // file changes: a run a landing plants is in the file the pane is about to show,
        // and the switch it causes -- from the listing being left to the one arriving --
        // must not be what drops it. A run in a file the pane never reaches stays,
        // undrawn, until the next question replaces it.
        let picked = marked
            .peek()
            .source
            .as_ref()
            .and_then(|picked| picked.file.clone());
        if picked.is_some() && was == picked {
            unmark(marked, Pane::Source);
        }
    });
}

/// Start each document afresh: drop both runs whenever the active document changes, and
/// plant the one the arrival asks for -- a [`Landing`] naming this document, picked out
/// in the source pane with both panes owed the scroll; or, for a source-driven tab, the
/// line it is driven from, with none owed, so coming back to a tab whose assembly side
/// is a listing of one line shows which line and why.
///
/// A landing is spent by whichever document arrives, the one it named or another: it is
/// for the next arrival only, and one left lying would pick a line out in a document
/// opened for some other reason later.
pub(crate) fn use_land(
    active: Memo<Option<Document>>,
    marked: State<Marks>,
    landing: State<Option<Landing>>,
    driven: State<Driven>,
) {
    use_side_effect(move || {
        // Subscribes the effect to the active document, which is all it wants from it;
        // the landing is peeked, so setting one wakes nothing until the document does.
        let active = active.read().clone();

        let (mut marked, mut landing) = (marked, landing);
        let asked = landing.peek().clone();
        if asked.is_some() {
            landing.set(None);
        }
        let landed = asked
            .filter(|landing| Some(&landing.tab) == active.as_ref())
            .map(|landing| (landing.at.file, landing.at.line, Owed::BOTH));
        let planted = landed.or_else(|| match &active {
            Some(document @ Document::Source(file)) => driven
                .peek()
                .line(document)
                .map(|line| (file.clone(), line, Owed::default())),
            _ => None,
        });

        marked.set_if_modified(Marks::default());
        if let Some((file, line, owed)) = planted {
            mark_line(marked, file, line, owed);
        }
    });
}
