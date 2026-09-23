//! Where the platform's keyboard focus is: every box it can be in, inside the tab on
//! screen and in the sidebar's panels, and the ask that puts it in one of them.
//!
//! Nothing here is written when a box takes the focus. A box registers itself for as long
//! as it is mounted, and an ask is kept until there is a box to spend it on:
//! [`use_keyboard_asked`] spends it in an effect and not in the press that made it,
//! because the press is what mounts the pane -- and, for a panel, the chord is what
//! raises it.

use super::*;

/// Every box the keyboard can be put in -- the two code panes, the listing of an object's
/// code, the scratchpad's editor, and the list of each sidebar panel.
///
/// The boxes are a **registration** and not a flag written when one takes the focus: focus
/// is *lost* without an event -- something else asks for it -- so what is asked of the
/// platform has to be asked at the moment the answer is drawn. Each box registers itself
/// while it is mounted ([`use_tab_keyboard`], [`use_panel_keyboard`]).
///
/// **Two lists and not one**, because the two answer different questions. Only the tab on
/// screen is mounted, so `boxes` answers "the keyboard is in the tab and not in the
/// sidebar" -- which is what the mark over the tab on screen says -- and a panel's box
/// must not count as being in the tab. `panels` is keyed by the panel instead of by a
/// pane, a panel being at most one box: only the panel on top in its group is mounted.
#[derive(Default)]
pub(crate) struct Keys {
    /// Every box the keyboard can be in inside the tab on screen, and which pane each is:
    /// [`None`] for the scratchpad's editor, which is a page and not a pane.
    boxes: Vec<(Option<Pane>, AccessibilityId)>,
    /// The box each mounted panel is reached by: its filter box where it has one, and its
    /// rows where it has not (`ui/filter_bar.rs`).
    panels: Vec<(Panel, AccessibilityId)>,
}

/// What the keyboard has been asked into. **One ask and not one per kind**: the reader
/// has one keyboard, so a chord that reaches a panel cancels the ask an opened row left
/// behind rather than racing it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wanted {
    /// The tab on screen: what a press on a chip, and a row that opened a tab, ask for.
    /// `back` for an ask that returns the reader to the tab -- a chip, Escape out of a
    /// list -- which goes to the pane the tab last had the keyboard in rather than the
    /// one that leads it.
    Tab { back: bool },
    /// One of the sidebar's panels: what that panel's chord asks for ([`reach_panel`]).
    Panel(Panel),
}

impl Keys {
    /// The box an ask should be spent on: `left`'s where it has one drawn, then the
    /// **leading pane's**, and otherwise whatever box there is.
    ///
    /// By the pane and not by the order they registered in: a pane keeps its box for as
    /// long as it is mounted and the temporal tab's panes outlive the documents they draw,
    /// so the first box registered is the pane whose side led whichever tab opened first --
    /// which is how a reader who opened a file and then a symbol had the keyboard put in
    /// the file beside the listing they had just asked for.
    fn wanted_box(
        &self,
        left: Option<Pane>,
        leads: Option<Pane>,
    ) -> Option<(Option<Pane>, AccessibilityId)> {
        let drawn = |pane: Option<Pane>| {
            pane.and_then(|pane| self.pane_box(pane).map(|a11y| (Some(pane), a11y)))
        };
        drawn(left)
            .or_else(|| drawn(leads))
            .or_else(|| self.boxes.first().copied())
    }

    /// The box `pane` registered, where it has one mounted: what puts the keyboard back
    /// in the code after a find bar over it is closed.
    pub(crate) fn pane_box(&self, pane: Pane) -> Option<AccessibilityId> {
        self.boxes
            .iter()
            .find(|(of, _)| *of == Some(pane))
            .map(|(_, a11y)| *a11y)
    }

    /// The box `panel` registered, where it is mounted at all: what a chord that reaches
    /// the panel puts the keyboard in, and what says whether the keyboard is in it.
    pub(crate) fn panel_box(&self, panel: Panel) -> Option<AccessibilityId> {
        self.panels
            .iter()
            .find(|(of, _)| *of == panel)
            .map(|(_, a11y)| *a11y)
    }
}

/// [`Keys`] and the ask, shared through context.
///
/// **Two states and not one**: the boxes change as a pane or a panel mounts, and the ask
/// with every chip pressed and every row that opens a tab. The chip of the tab on screen
/// reads the boxes ([`keyboard_in_tab`]), and must not be drawn again for an ask made and
/// spent.
#[derive(Clone, Copy)]
pub(crate) struct Keyboard {
    pub(crate) keys: State<Keys>,
    /// Where the keyboard has been asked to go, and [`None`] once it is there. Spent by
    /// [`use_keyboard_asked`] once there is a box to spend it on -- which may be several
    /// renders later, a pane with nothing to draw yet registering none.
    asked: State<Option<Wanted>>,
    /// The pane each document tab last had the keyboard in: where a press on its chip,
    /// or Escape out of a list, puts it back. Written by [`use_keyboard_left`].
    left: State<HashMap<DocId, Pane>>,
}

impl Keyboard {
    pub(crate) fn create() -> Self {
        Keyboard {
            keys: State::create(Keys::default()),
            asked: State::create(None),
            left: State::create(HashMap::new()),
        }
    }
}

/// Register `a11y` as one of the boxes the keyboard can be in inside the tab on screen,
/// for as long as this scope is mounted, as the box of `pane` -- [`None`] for a box that
/// is not a pane's. See [`Keys`].
pub(crate) fn use_tab_keyboard(pane: Option<Pane>, a11y: AccessibilityId) {
    let mut keyboard = use_consume::<Keyboard>().keys;
    use_hook(move || keyboard.write().boxes.push((pane, a11y)));
    use_drop(move || {
        keyboard.write().boxes.retain(|(_, open)| *open != a11y);
    });
}

/// The same for a sidebar panel: `a11y` is the box a chord that reaches `panel` puts the
/// keyboard in, for as long as the panel is mounted. See [`Keys`].
pub(crate) fn use_panel_keyboard(panel: Panel, a11y: AccessibilityId) {
    let mut keyboard = use_consume::<Keyboard>().keys;
    use_hook(move || keyboard.write().panels.push((panel, a11y)));
    use_drop(move || {
        keyboard.write().panels.retain(|(_, open)| *open != a11y);
    });
}

/// Whether the keyboard is inside the tab on screen. Asking is what subscribes the caller
/// to the focus moving, `AccessibilityId::is_focused` reading the platform's own state.
pub(crate) fn keyboard_in_tab(keyboard: Keyboard) -> bool {
    keyboard
        .keys
        .read()
        .boxes
        .iter()
        .any(|(_, a11y)| a11y.is_focused())
}

/// Ask for the keyboard to go into the tab on screen, in the pane that leads it: what a
/// row that opened a tab asks, so that reading follows what the reader just chose.
pub(crate) fn ask_for_keyboard(mut keyboard: Keyboard) {
    keyboard
        .asked
        .set_if_modified(Some(Wanted::Tab { back: false }));
}

/// Ask for the keyboard to go back into the tab on screen, in the pane it was last in
/// there: what pressing a chip does, and what Escape on a list does.
pub(crate) fn return_keyboard(mut keyboard: Keyboard) {
    keyboard
        .asked
        .set_if_modified(Some(Wanted::Tab { back: true }));
}

/// Ask for it to go into `panel` instead: what a panel's chord does, once it has raised
/// the panel ([`reach_panel`]).
pub(crate) fn ask_for_panel(mut keyboard: Keyboard, panel: Panel) {
    keyboard.asked.set_if_modified(Some(Wanted::Panel(panel)));
}

/// Spend that ask, once whatever it was made for has mounted a box. In an effect and not
/// in the press or the chord, because the press is what mounts the panes and the chord is
/// what raises the panel: the box to focus does not exist until the render it caused has
/// run.
///
/// Also where the pane each tab last had the keyboard in is written down
/// ([`use_keyboard_left`]), which is what a `back` ask reads.
pub(crate) fn use_keyboard_asked(keyboard: Keyboard, open: Open, marked: State<Marks>) {
    use_keyboard_left(keyboard, open);
    use_side_effect(move || {
        // **An ask is kept until there is somewhere to spend it.** A tab opened from a
        // list has nothing to focus in the pass that opened it: its assembly side draws a
        // sentence until the worker answers, and a pane with nothing to show registers no
        // box at all -- so an ask spent on `None` was every ask a row ever made. A panel
        // asked for is the same the other way round: the chord that named it may have
        // raised it from behind another, and its box arrives with the render that does.
        // While an ask waits the boxes are *read*, which is what subscribes this to them,
        // so a box arriving is what wakes it. With no ask it reads nothing but the ask, and
        // is not woken by every tab moved or trail pushed.
        let wanted = *keyboard.asked.read();
        let waiting = match wanted {
            None => return,
            Some(Wanted::Tab { back }) => {
                // Which side leads the tab on screen, which is the pane the ask is for:
                // the one the reader asked to see, and the one `DocumentBody` draws first.
                // A reader coming back to the tab goes to where they left it instead.
                let (left, leads) = {
                    let (strip, docs) = (open.strip.read(), open.docs.read());
                    match active_tab(&strip, &docs) {
                        Some((id, at)) => (
                            back.then(|| keyboard.left.peek().get(&id).copied())
                                .flatten(),
                            Some(at.document.driven_from()),
                        ),
                        None => (None, None),
                    }
                };
                keyboard.keys.read().wanted_box(left, leads)
            }
            // No pane, so no caret: a list's own pick is its cursor, and a panel handed
            // the keyboard has one already or takes it from the first arrow.
            Some(Wanted::Panel(panel)) => keyboard
                .keys
                .read()
                .panel_box(panel)
                .map(|a11y| (None, a11y)),
        };
        let Some((pane, a11y)) = waiting else {
            return;
        };
        let mut asked = keyboard.asked;
        asked.set(None);
        a11y.request_focus();
        // **A pane handed the keyboard has a caret put in it**, where it has no run of its
        // own. Nothing was clicked in it -- a row of a list opened this tab, or a chip was
        // pressed -- and a listing with no run draws no caret, so the arrows, Home, End
        // and Ctrl+C would have nothing to act on and the pane would read as though the
        // keyboard were somewhere else. Only an ask does this, and never a press: a press
        // in a pane says where the caret goes, including the press under the last row that
        // deliberately picks nothing out.
        if let Some(pane) = pane {
            if marked.peek().of(pane).is_none() {
                mark_top(marked, pane);
            }
        }
    });
}

/// Write down which pane of the tab on screen the keyboard is in, whenever it moves into
/// one.
///
/// **Only a move is written**, and only into a pane: the keyboard going into the sidebar
/// leaves the tab's pane as it was, and a switch of tab that keeps the same pane focused --
/// the panes are not mounted again for every tab -- writes nothing over the new tab's. The
/// move a switch does make, the ask spending itself, happens after the strip has changed,
/// so it is written under the tab it was made for.
fn use_keyboard_left(keyboard: Keyboard, open: Open) {
    let focused = use_memo(move || {
        keyboard
            .keys
            .read()
            .boxes
            .iter()
            .find(|(_, a11y)| a11y.is_focused())
            .and_then(|(pane, _)| *pane)
    });
    use_side_effect(move || {
        let Some(pane) = *focused.read() else {
            return;
        };
        let Some(Tab::Document(id)) = open.strip.peek().active() else {
            return;
        };
        let docs = open.docs.peek();
        let mut left = keyboard.left;
        let mut left = left.write();
        // A closed tab's is dropped here: ids are never given out twice, so it would
        // only sit there.
        left.retain(|id, _| docs.get(*id).is_some());
        left.insert(id, pane);
    });
}

/// Forget an ask nobody has spent: what a press that puts the keyboard somewhere itself
/// says. An ask now waits for a box, so one made for a tab that never drew a pane would sit
/// there and be spent by whatever pane arrived next -- taking the keyboard out of the list
/// the reader had put it in meanwhile.
pub(crate) fn unask_keyboard(mut keyboard: Keyboard) {
    keyboard.asked.set_if_modified(None);
}
