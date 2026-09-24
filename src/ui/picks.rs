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
//! The one other writer is a close, which drops the picks into the closed binary
//! ([`Pick::in_file`]).
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
//! its pane made focusable, which is the pane's rows and not its filter box, and which the
//! list's [`Picking`] carries to every row.
//!
//! **The pick is the list's cursor**, which is what makes a list something the keyboard can
//! be used in: Up and Down move it ([`Picking::stepped`]), Home, End and the two page keys
//! move it further ([`Picking::jumped`]), the list scrolling to keep it in view, Left and
//! Right fold the row it is on ([`Picking::folded`]), Enter opens it the way pressing its
//! row would ([`Picking::entered`]), and Escape hands the keyboard back to the tab on
//! screen ([`Picking::to_the_tab`]). What each list answers with is its [`ListKeys`],
//! built by the panel because the panel is the only thing that knows its rows; the keys
//! themselves are answered once, on the box they are drawn in (`ui/filter_bar.rs`).
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
    /// A bookmark: the entry itself, which is what its row draws. Not its place in the
    /// list, which its menu removes by and which the row below a removed one inherits.
    Bookmark(Bookmark),
    /// A path: a row of the Files tree, a file the Objects tree read, and the file a run
    /// of hits or of references is grouped under.
    Path(PathBuf),
    /// A place in a file: a search hit, or a reference. The line, and the column the
    /// place starts at where it names one: two references on one line are two rows, and
    /// each is its own pick.
    Place(PathBuf, u32, Option<usize>),
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
            (Pick::Place(ours, line, column), Pick::Place(theirs, other, at)) => {
                ours == theirs && line == other && column == at
            }
            _ => false,
        }
    }
}

impl Pick {
    /// Whether this names something in the binary at `path`, which a close lets go of: an
    /// object, a symbol and a visit each hold the whole `Object`. A path, a place and a
    /// bookmark hold none.
    pub(crate) fn in_file(&self, path: &Path) -> bool {
        match self {
            Pick::Object(object) => object.path == path,
            Pick::Symbol(symbol) => symbol.object.path == path,
            Pick::Visit(document) => document.in_file(path),
            Pick::Bookmark(_) | Pick::Path(_) | Pick::Place(..) => false,
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
///
/// `PartialEq` because a panel's own entry is read through a [`Memo`] ([`use_picking`]),
/// which is what keeps a press in one list off every other list's rows.
#[derive(Clone, PartialEq)]
pub(crate) struct PickedRow {
    pub(crate) pick: Pick,
    pub(crate) at: usize,
}

/// The pick each panel has. At the root, because a panel that is not its dock tab's is
/// unmounted and a pick outlives the reader looking at another list.
#[derive(Clone, Copy)]
pub(crate) struct Picks(pub(crate) State<HashMap<Panel, PickedRow>>);

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

/// What a list row does when it opens a document: it goes through the door every row
/// outside the panes takes ([`Reach::outside`]) and answers with the keyboard handed over.
///
/// Two halves of one contract, so they are written together and once. A row that opens a
/// document is a preview, or a tab of its own with Ctrl, and no row is anything else: a
/// rule stated on the enum and then applied at nine call sites is a rule the tenth row
/// can be written without.
///
/// A **source** document goes through [`open_source_tab`], which names it by the spelling
/// an open tab already has for the file. A bookmark is saved to disk, so the spelling it
/// kept is a tab's from a session that may have spelled the project directory another way;
/// a [`Document::Source`] is never canonicalised, so without the rule it would open a second
/// tab of a file already open.
///
/// Not a hook, and it consumes nothing: the row's render has already taken [`Doors`] and
/// the [`Ctrl`] state, so a press handler and a [`ListKeys::open`] closure can both call
/// this.
pub(crate) fn opened(doors: Doors, ctrl: State<bool>, document: Document) -> Pressed {
    let (open, visits, reach) = (doors.open, doors.visits, Reach::outside(ctrl));
    match &document {
        Document::Source(file) => {
            open_source_tab(open, visits, file, reach);
        }
        _ => {
            open_document(open, visits, document, reach);
        }
    }
    Pressed::Opened
}

/// A list of rows as the keys step it: how many there are, and the row at a place.
///
/// The three shapes a panel's rows come in -- a [`Shared`] slice, a [`Filtered`] and a
/// plain `Vec` -- so that [`ListKeys::over`] writes the past-the-end answer once instead
/// of each panel writing its own.
///
/// `count` and `row` rather than `len` and `get`: a trait method wins over an inherent
/// one reached through `Deref`, so a `len` here would quietly become what `rows.len()`
/// means on every `Shared` and every `Vec` in the UI.
pub(crate) trait Stepped {
    type Row;

    fn count(&self) -> usize;

    /// The row at `at`, or nothing past the end -- which is where a keyboard step off the
    /// last row asks.
    fn row(&self, at: usize) -> Option<&Self::Row>;
}

impl<T> Stepped for Shared<T> {
    type Row = T;

    fn count(&self) -> usize {
        self.len()
    }

    fn row(&self, at: usize) -> Option<&T> {
        self.get(at)
    }
}

impl<T> Stepped for Filtered<T> {
    type Row = T;

    fn count(&self) -> usize {
        self.len()
    }

    fn row(&self, at: usize) -> Option<&T> {
        self.at(at)
    }
}

impl<T> Stepped for Vec<T> {
    type Row = T;

    fn count(&self) -> usize {
        self.len()
    }

    fn row(&self, at: usize) -> Option<&T> {
        self.get(at)
    }
}

/// What a list answers the arrows and Enter with: how many rows it is drawing, the row at
/// a place in it, and what opening one does.
///
/// Built by the panel, which is the only thing that knows its rows, and asked by the box
/// they are drawn in (`ListPane`, `ui/filter_bar.rs`). Closures rather than a `Vec<Pick>`:
/// the Symbols list is 115k rows, and building that vector every render is what this side
/// of the app is written to avoid.
pub(crate) struct ListKeys {
    pub(crate) length: usize,
    /// Which row is drawn at a place, for the arrows to pick out.
    pub(crate) at: Box<dyn Fn(usize) -> Option<Pick>>,
    /// What pressing the row at a place does, which is what Enter does to it. By place and
    /// not by row: a fold is about where a row is in the tree, and this way each panel says
    /// once what its rows do rather than once per kind of pick.
    pub(crate) open: Box<dyn Fn(usize) -> Pressed>,
    /// What Left and Right do to the row at a place: `false` folds the group under it
    /// away and `true` opens it. A row already the way the key asks, and a row with
    /// nothing under it at all, is left alone -- which is the two keys in a flat list,
    /// where every row is one ([`ListKeys::over`]).
    ///
    /// Its own closure and not [`ListKeys::open`] under another name: a press *toggles*,
    /// and these two say which way, so what a panel writes here is the same fold with
    /// the direction asked for rather than assumed.
    pub(crate) fold: Box<dyn Fn(usize, bool)>,
}

impl ListKeys {
    /// A list that answers no key: what a panel hands over where it has a sentence to say
    /// instead of rows.
    pub(crate) fn none() -> Self {
        Self {
            length: 0,
            at: Box::new(|_| None),
            open: Box::new(|_| Pressed::Folded),
            fold: ListKeys::flat(),
        }
    }

    /// The keys over a flat list: `pick` says what a row is and `open` what pressing one
    /// does, each asked of the row itself. Which list the three closures index is settled
    /// here, and so is what they answer past its end.
    ///
    /// One [`Rc`] of the rows, shared by all three: the arrows ask what a row is, Enter
    /// asks what pressing one does, and Left and Right ask which way it folds, and all
    /// three are the list the panel is drawing rather than one worked out again.
    pub(crate) fn over<L: Stepped + 'static>(
        rows: L,
        pick: impl Fn(&L::Row) -> Pick + 'static,
        open: impl Fn(&L::Row) -> Pressed + 'static,
    ) -> Self {
        Self::keys(Rc::new(rows), pick, open, ListKeys::flat())
    }

    /// The same over a tree: `fold` is what Left and Right do to a row, and it is called
    /// only for a row that is there.
    pub(crate) fn folding<L: Stepped + 'static>(
        rows: L,
        pick: impl Fn(&L::Row) -> Pick + 'static,
        open: impl Fn(&L::Row) -> Pressed + 'static,
        fold: impl Fn(&L::Row, bool) + 'static,
    ) -> Self {
        let rows = Rc::new(rows);
        let folded = rows.clone();
        Self::keys(
            rows,
            pick,
            open,
            Box::new(move |at, unfold| {
                if let Some(row) = folded.row(at) {
                    fold(row, unfold);
                }
            }),
        )
    }

    /// The bounds check both constructors share, written once.
    fn keys<L: Stepped + 'static>(
        rows: Rc<L>,
        pick: impl Fn(&L::Row) -> Pick + 'static,
        open: impl Fn(&L::Row) -> Pressed + 'static,
        fold: Box<dyn Fn(usize, bool)>,
    ) -> Self {
        let stepped = rows.clone();
        ListKeys {
            length: rows.count(),
            at: Box::new(move |at| stepped.row(at).map(&pick)),
            open: Box::new(move |at| match rows.row(at) {
                Some(row) => open(row),
                None => Pressed::Folded,
            }),
            fold,
        }
    }

    /// A list with nothing to fold: every list but the Objects tree and the Files tree.
    fn flat() -> Box<dyn Fn(usize, bool)> {
        Box::new(|_, _| {})
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
    /// This panel's own entry in that table, as a row asks for it: see [`use_picking`].
    picked: Memo<Option<PickedRow>>,
    alt: State<bool>,
    keyboard: Keyboard,
    panel: Panel,
    /// The focusable box the list's rows are in, which is what answers for whether the
    /// keyboard is in the list.
    rows: AccessibilityId,
}

/// The pick of the panel a row is in, minted once on the pane every panel draws its list
/// in ([`use_list_pane`]) and carried to the rows in [`ListStates`].
///
/// The states are consumed and not read, so the pane subscribes to none of them. Whether
/// the keyboard is in the list is asked in [`Picking::drawn`] for that reason.
///
/// **The pick itself is a [`Memo`] over this panel's entry** and not the table. Every row
/// of every list reads it as it draws itself, so a read of the table put every mounted
/// row of every panel on the list of what a press or an arrow step in one of them wakes.
/// A memo notifies only when this panel's own entry changes. It is made here because the
/// panel is the pane's and not the row's, so there is one per list and not one per row.
pub(crate) fn use_picking(panel: Panel, rows: AccessibilityId) -> Picking {
    let picks = use_consume::<Picks>().0;
    Picking {
        picks,
        picked: use_memo(move || picks.read().get(&panel).cloned()),
        alt: use_consume::<Alt>().0,
        keyboard: use_consume::<Keyboard>(),
        panel,
        rows,
    }
}

impl Picking {
    /// How this row is drawn: against the list's own pick where it has one, and against
    /// `shown` -- whether this row is what the tab on screen is showing -- where it has
    /// not. A list with no document behind its rows passes `false`.
    ///
    /// Called only while a row renders, which is why whether the keyboard is in the list
    /// is asked here and not in [`use_picking`]: `is_focused` reads the platform's own
    /// state, so the read subscribes the scope asking, and the pane draws nothing from the
    /// answer. Asked there, a focus move anywhere in the app would re-render every mounted
    /// panel for rows that wake on their own.
    pub(crate) fn drawn(&self, pick: &Pick, shown: bool) -> Chosen {
        let picked = self.picked.read();
        let selected = match picked.as_ref() {
            Some(picked) => &picked.pick == pick,
            None => shown,
        };
        chosen(selected, self.rows.is_focused())
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

    /// Home, End and the two page keys: the pick put on `at`, clamped to the list, and
    /// where it landed for the list to scroll to.
    ///
    /// [`Picking::stepped`]'s counterpart for the keys that name a row rather than a
    /// distance: nothing is read of the pick that was there, so neither key cares whether
    /// the list has moved under one.
    pub(crate) fn jumped(self, keys: &ListKeys, at: usize) -> Option<usize> {
        let last = keys.length.checked_sub(1)?;
        let at = at.min(last);
        let pick = keys.at(at)?;
        self.pick(PickedRow { pick, at });
        Some(at)
    }

    /// Left and Right: the group under the picked row folded away, or opened. The pick
    /// stays where it is -- a fold is about the row the reader is on -- and a row with
    /// nothing under it does nothing at all.
    ///
    /// A pick the list has moved under folds nothing, [`Picking::entered`]'s rule: what is
    /// in its place now is not what the reader picked.
    pub(crate) fn folded(self, keys: &ListKeys, unfold: bool) {
        // Bound in a statement of its own, so no read guard is alive while the fold below
        // writes the tree.
        let held = self.picks.peek().get(&self.panel).cloned();
        let Some(picked) = held.filter(|picked| keys.held(picked)) else {
            return;
        };
        (keys.fold)(picked.at, unfold);
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

    /// Escape: the keyboard back in the tab on screen, in the pane it was last in there.
    /// The same ask a pressed chip makes ([`return_keyboard`]) and not a focus taken
    /// here, a list knowing nothing about which box the tab has -- and it is the ask that
    /// puts a caret in a pane that has none, which a reader arriving from a list needs as
    /// much as one arriving from a chip.
    pub(crate) fn to_the_tab(self) {
        return_keyboard(self.keyboard);
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
