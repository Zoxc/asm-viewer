//! The row each list has picked out, and which of two colours it is drawn in.
//!
//! A lit row used to be one fact: the row **is** what the tab on screen shows. Four lists
//! lit a row that way -- Objects, Symbols, History, Locations -- the other four lit none,
//! and there was no way to point at a row without opening what it names. **Alt+press picks
//! a row out and opens nothing**, the same Alt that says a press on a link is not a door
//! this time (`ui/marks.rs`), and that needs a selection the list owns: nothing about the
//! tabs has moved for one to be derived from.
//!
//! So a list draws **its own pick where it has one, and the row the tab shows where it has
//! not**. A press in the list is the only thing that moves that list's pick: a tab
//! switched to, a link followed, a bookmark opened all leave it where the reader put it.
//! The cost is worth writing down -- a list that has been pressed in no longer follows the
//! tabs -- and what it buys is that the lit row is the row last pointed at, in every list
//! and not only in the four with a document behind them. A pick is one per [`Panel`] and
//! held at the root, a panel that is not its dock tab's being unmounted.
//!
//! **The keyboard picks the colour.** A list holding it draws its pick in the selection
//! the code panes use, `text_select_bg`; a list that is not draws it in `selected_bg`'s
//! grey. So a reader looking at three panels at once can tell the list they are typing in
//! from the two remembering where they were, and the blue means the same thing in a list
//! as in the code -- what the next key acts on. The box that answers for a list is the one
//! its pane made focusable ([`RowsBox`]), which is the pane's rows and not its filter box.
//!
//! **The pick is the list's cursor**, which is what makes a list something the keyboard can
//! be used in: Up and Down move it ([`Picking::stepped`]), the list scrolling to keep it in
//! view, and Enter opens it the way pressing its row would ([`Picking::entered`]). What each
//! list answers with is its [`ListKeys`], built by the panel because the panel is the only
//! thing that knows its rows; the keys themselves are answered once, on the box they are
//! drawn in (`ui/filter_bar.rs`).
//!
//! **Opening a tab hands it the keyboard**, however the row was opened: a reader who has
//! put a listing on screen is reading it, so the next key is answered there and not in the
//! list behind it. The pick stays where it was and goes grey, which is what says the list
//! is no longer where a key would land. Which is why acting on a row answers with a
//! [`Pressed`]: a row that only folded a group opened nothing, so it keeps the keyboard --
//! and so does a press with Alt held, which opens nothing at all. Those are what leave the
//! keyboard in a list for its arrows and its Enter to be used.
//!
//! The file finder is not a panel and keeps its own pick: the row its arrows move is
//! already a selection the list owns. It takes only [`chosen`] from here.

use super::*;

/// The row a list has picked out, in the terms that list names its rows by.
///
/// One enum rather than a selection per panel: the lists differ in what a row *is* and
/// agree about everything else, and a pick is compared, cloned and thrown away.
#[derive(Clone)]
pub(crate) enum Pick {
    /// An object, in the Objects tree.
    Object(Arc<Object>),
    /// A symbol: the Symbols list's rows, and the Locations panel's.
    Symbol(Symbol),
    /// A place the reader has been, in the History list.
    Visit(Document),
    /// A bookmark by its place in the list, which is what its own menu names it by.
    Bookmark(usize),
    /// A path: a row of the Files tree, a file the Objects tree read, and the file a run
    /// of hits or of references is grouped under.
    Path(PathBuf),
    /// A place in a file: a search hit, or a reference.
    Place(PathBuf, u32),
}

impl PartialEq for Pick {
    /// Pointer identity for the variant with an `Arc` behind it, as everywhere else in the
    /// UI; the rest are what the rows already compare themselves by.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Pick::Object(ours), Pick::Object(theirs)) => Arc::ptr_eq(ours, theirs),
            (Pick::Symbol(ours), Pick::Symbol(theirs)) => ours == theirs,
            (Pick::Visit(ours), Pick::Visit(theirs)) => ours == theirs,
            (Pick::Bookmark(ours), Pick::Bookmark(theirs)) => ours == theirs,
            (Pick::Path(ours), Pick::Path(theirs)) => ours == theirs,
            (Pick::Place(ours, line), Pick::Place(theirs, other)) => {
                ours == theirs && line == other
            }
            _ => false,
        }
    }
}

/// A list's pick: the row it is, and where that row was in the list when it was picked.
///
/// The row is what a row draws itself from, and the place is what the arrows step. A place
/// is remembered rather than searched for -- finding one again would be a walk of the whole
/// list per keystroke, and the Symbols list is 115k rows -- and it goes stale when the list
/// changes under it, a filter typed or an archive folded. [`ListKeys::held`] is what
/// notices, and the arrows then start from the end they came from.
#[derive(Clone)]
pub(crate) struct PickedRow {
    pub(crate) pick: Pick,
    pub(crate) at: usize,
}

/// The pick each panel has. At the root, because a panel that is not its dock tab's is
/// unmounted and a pick outlives the reader looking at another list.
#[derive(Clone, Copy)]
pub(crate) struct Picks(pub(crate) State<HashMap<Panel, PickedRow>>);

/// The focusable box a list's rows are in, provided by the pane that mints it so that a
/// row can ask whether the keyboard is in the list it is being drawn in.
#[derive(Clone, Copy)]
pub(crate) struct RowsBox(pub(crate) AccessibilityId);

/// How a list row is picked out: what [`list_row`] draws it with.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Chosen {
    /// Not at all: the row answers the pointer and nothing else.
    No,
    /// Picked out in a list the keyboard is not in.
    Idle,
    /// Picked out in the list the keyboard is in.
    Live,
}

/// The box that answers for the list being drawn, where there is one.
///
/// `try_consume_context` and not [`use_consume`]: it is not a hook, so a row outside any
/// list -- which is how a test mounts one on its own -- draws as a list nobody is typing
/// in rather than panicking.
pub(crate) fn rows_box() -> Option<AccessibilityId> {
    try_consume_context::<RowsBox>().map(|rows| rows.0)
}

/// Whether the keyboard is in the list being drawn. Asking is what subscribes the row to
/// the focus moving, `is_focused` reading the platform's own state (`ui/focus.rs`).
pub(crate) fn keyboard_in_list() -> bool {
    rows_box().is_some_and(|rows| rows.is_focused())
}

/// A row that is picked out, drawn for where the keyboard is.
pub(crate) fn chosen(selected: bool, focused: bool) -> Chosen {
    match (selected, focused) {
        (false, _) => Chosen::No,
        (true, false) => Chosen::Idle,
        (true, true) => Chosen::Live,
    }
}

/// What acting on a row did, which is what says where the keyboard goes: a row that opened
/// a tab hands it over when it was Enter that opened it, and one that folded a group keeps
/// it, there being nothing new to read and nowhere for it to go.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Pressed {
    Opened,
    Folded,
}

/// What a list answers the arrows and Enter with: how many rows it is drawing, the row at
/// a place in it, and what opening one does.
///
/// Built by the panel, which is the only thing that knows its rows, and asked by the box
/// they are drawn in (`ListPane`, `ui/filter_bar.rs`). Two closures rather than a
/// `Vec<Pick>`: the Symbols list is 115k rows, and building that vector every render is
/// what this side of the app is written to avoid.
pub(crate) struct ListKeys {
    pub(crate) length: usize,
    /// Which row is drawn at a place, for the arrows to pick out.
    pub(crate) at: Box<dyn Fn(usize) -> Option<Pick>>,
    /// What pressing the row at a place does, which is what Enter does to it. By place and
    /// not by row: a fold is about where a row is in the tree, and this way each panel says
    /// once what its rows do rather than once per kind of pick.
    pub(crate) open: Box<dyn Fn(usize) -> Pressed>,
}

impl ListKeys {
    /// A list that answers no key: what a panel hands over where it has a sentence to say
    /// instead of rows.
    pub(crate) fn none() -> Self {
        Self {
            length: 0,
            at: Box::new(|_| None),
            open: Box::new(|_| Pressed::Folded),
        }
    }

    /// Whether `picked` is still where it says it is.
    fn held(&self, picked: &PickedRow) -> bool {
        self.at(picked.at).as_ref() == Some(&picked.pick)
    }

    fn at(&self, index: usize) -> Option<Pick> {
        (self.at)(index)
    }
}

/// One list's pick, as its rows and its keys read and write it: everything they need that
/// a plain function may not ask for, consumed in the render because a handler runs no hook.
#[derive(Clone, Copy)]
pub(crate) struct Picking {
    picks: State<HashMap<Panel, PickedRow>>,
    alt: State<bool>,
    keyboard: State<Keys>,
    panel: Panel,
    focused: bool,
}

/// The pick of the panel a row is in. Called in the row's own render, as every
/// context-consuming function must be.
pub(crate) fn use_picking(panel: Panel) -> Picking {
    Picking {
        picks: use_consume::<Picks>().0,
        alt: use_consume::<Alt>().0,
        keyboard: use_consume::<Keyboard>().0,
        panel,
        focused: keyboard_in_list(),
    }
}

impl Picking {
    /// How this row is drawn: against the list's own pick where it has one, and against
    /// `shown` -- whether this row is what the tab on screen is showing -- where it has
    /// not. A list with no document behind its rows passes `false`.
    pub(crate) fn drawn(&self, pick: &Pick, shown: bool) -> Chosen {
        let picks = self.picks.read();
        let selected = match picks.get(&self.panel) {
            Some(picked) => &picked.pick == pick,
            None => shown,
        };
        chosen(selected, self.focused)
    }

    /// A press on the row, which is `at` in the list as it is drawn: pick it out, and then
    /// do what the row does -- unless Alt is held, which is what says this press is not a
    /// door. A row that opened a tab hands it the keyboard.
    ///
    /// The write's guard is gone before `opens` runs, which opens documents and folds
    /// trees and must not meet a borrow of ours.
    pub(crate) fn press(self, pick: Pick, at: usize, opens: impl FnOnce() -> Pressed) {
        self.pick(PickedRow { pick, at });
        if *self.alt.peek() {
            return;
        }
        self.went(opens());
    }

    /// The arrows: the pick moved `by` rows of `keys`, and where it landed, for the list
    /// to scroll to.
    ///
    /// Both ends stop at the list, as the finder's do and for its reason: unclamped, Down
    /// held counts on past the last row and every Up after it is spent coming back. A list
    /// with no pick, or one whose pick it has moved under, starts at the end the key came
    /// from.
    pub(crate) fn stepped(self, keys: &ListKeys, by: isize) -> Option<usize> {
        if keys.length == 0 {
            return None;
        }
        let last = keys.length - 1;
        // Bound in a statement of its own, so no read guard is alive at the write below.
        let held = self.picks.peek().get(&self.panel).cloned();
        let at = match held.filter(|picked| keys.held(picked)) {
            Some(picked) => picked.at.saturating_add_signed(by).min(last),
            None if by > 0 => 0,
            None => last,
        };
        let pick = keys.at(at)?;
        self.pick(PickedRow { pick, at });
        Some(at)
    }

    /// Enter: open what the pick names, exactly as pressing its row would, the keyboard
    /// included.
    ///
    /// A pick the list has moved under opens nothing: what is in its place now is not what
    /// the reader picked, and the next arrow puts them back on a row they can see.
    pub(crate) fn entered(self, keys: &ListKeys) {
        let held = self.picks.peek().get(&self.panel).cloned();
        let Some(picked) = held.filter(|picked| keys.held(picked)) else {
            return;
        };
        self.went((keys.open)(picked.at));
    }

    /// The reader put the keyboard in this list themselves, so any ask still waiting for a
    /// pane is not theirs and is dropped ([`unask_keyboard`]).
    pub(crate) fn unasked(self) {
        unask_keyboard(self.keyboard);
    }

    /// Where the keyboard goes once a row has been acted on: into the tab a row opened,
    /// and nowhere at all for a row that only folded a group -- there being nothing new to
    /// read, and the list still being what the reader is working down.
    fn went(self, went: Pressed) {
        if went == Pressed::Opened {
            ask_for_keyboard(self.keyboard);
        }
    }

    /// Write the pick, the guard gone before anything reads it back.
    fn pick(mut self, picked: PickedRow) {
        self.picks.write().insert(self.panel, picked);
    }
}
