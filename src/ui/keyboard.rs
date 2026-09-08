//! Where the platform's keyboard focus is: every box inside the tab on screen it can be
//! in, and the ask a press on a chip makes for it to go there.
//!
//! Nothing here is written when a box takes the focus. A box registers itself for as long
//! as it is mounted, and an ask is kept until there is a box to spend it on:
//! [`use_keyboard_asked`] spends it in an effect and not in the press that made it,
//! because the press is what mounts the pane.

use super::*;

/// Every box the keyboard can be in inside the tab on screen -- the two code panes, the
/// listing of an object's code, the scratchpad's editor -- and whether a press on a chip
/// has asked for it to go there.
///
/// The boxes are a **registration** and not a flag written when one takes the focus: focus
/// is *lost* without an event -- something else asks for it -- so what is asked of the
/// platform has to be asked at the moment the answer is drawn. Each box registers itself
/// while it is mounted ([`use_tab_keyboard`]), and only the tab on screen is mounted, so
/// what this answers is "the keyboard is in the tab and not in the sidebar" -- which is
/// what the mark over the tab on screen says.
#[derive(Default)]
pub(crate) struct Keys {
    /// Every box the keyboard can be in inside the tab on screen, and which pane each is:
    /// [`None`] for the scratchpad's editor, which is a page and not a pane.
    boxes: Vec<(Option<Pane>, AccessibilityId)>,
    /// Asked for by a press on a chip or on a row that opened a tab, and spent by
    /// [`use_keyboard_asked`] once there is a box to spend it on -- which may be several
    /// renders later, a pane with nothing to draw yet registering none.
    wanted: bool,
}

impl Keys {
    /// The box a tab takes the keyboard into: the first registered, which is the pane the
    /// tab is driven from -- `DocumentBody` mounts the leading side first.
    /// The box an ask should be spent on: the **leading pane's**, where the tab on screen
    /// has one drawn, and otherwise whatever box there is.
    ///
    /// By the pane and not by the order they registered in: a pane keeps its box for as
    /// long as it is mounted and the temporal tab's panes outlive the documents they draw,
    /// so the first box registered is the pane whose side led whichever tab opened first --
    /// which is how a reader who opened a file and then a symbol had the keyboard put in
    /// the file beside the listing they had just asked for.
    fn wanted_box(&self, leads: Option<Pane>) -> Option<(Option<Pane>, AccessibilityId)> {
        leads
            .and_then(|pane| self.boxes.iter().find(|(of, _)| *of == Some(pane)))
            .or_else(|| self.boxes.first())
            .copied()
    }
}

/// [`Keys`], shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Keyboard(pub(crate) State<Keys>);

/// Register `a11y` as one of the boxes the keyboard can be in inside the tab on screen,
/// for as long as this scope is mounted, as the box of `pane` -- [`None`] for a box that
/// is not a pane's. See [`Keys`].
pub(crate) fn use_tab_keyboard(pane: Option<Pane>, a11y: AccessibilityId) {
    let mut keyboard = use_consume::<Keyboard>().0;
    use_hook(move || keyboard.write().boxes.push((pane, a11y)));
    use_drop(move || {
        keyboard.write().boxes.retain(|(_, open)| *open != a11y);
    });
}

/// The focusable box `pane` registered, where it has one mounted: what puts the keyboard
/// back in the code after a find bar over it is closed.
pub(crate) fn pane_box(keyboard: State<Keys>, pane: Pane) -> Option<AccessibilityId> {
    keyboard
        .peek()
        .boxes
        .iter()
        .find(|(of, _)| *of == Some(pane))
        .map(|(_, a11y)| *a11y)
}

/// Whether the keyboard is inside the tab on screen. Asking is what subscribes the caller
/// to the focus moving, `AccessibilityId::is_focused` reading the platform's own state.
pub(crate) fn keyboard_in_tab(keyboard: State<Keys>) -> bool {
    keyboard
        .read()
        .boxes
        .iter()
        .any(|(_, a11y)| a11y.is_focused())
}

/// Ask for the keyboard to go into the tab on screen: what pressing a chip does, so that
/// reading follows the tab the reader just chose.
pub(crate) fn ask_for_keyboard(mut keyboard: State<Keys>) {
    keyboard.write().wanted = true;
}

/// Spend that ask, once the tab it was made for has mounted what it has. In an effect and
/// not in the press, because the press is what mounts the panes: the box to focus does not
/// exist until the render it caused has run.
pub(crate) fn use_keyboard_asked(mut keyboard: State<Keys>, open: Open, marked: State<Marks>) {
    use_side_effect(move || {
        // Which side leads the tab on screen, which is the pane the ask is for: the one
        // the reader asked to see, and the one `DocumentBody` draws first.
        let leads = {
            let (strip, docs) = (open.strip.read(), open.docs.read());
            active_document(&strip, &docs).map(|document| document.driven_from())
        };
        // **An ask is kept until there is somewhere to spend it.** A tab opened from a
        // list has nothing to focus in the pass that opened it: its assembly side draws a
        // sentence until the worker answers, and a pane with nothing to show registers no
        // box at all -- so an ask spent on `None` was every ask a row ever made. Both
        // fields are *read*, which is what subscribes this to the boxes as well as to the
        // ask, so the pane arriving is what wakes it. The guard is gone before the write.
        let waiting = {
            let keys = keyboard.read();
            keys.wanted.then(|| keys.wanted_box(leads)).flatten()
        };
        let Some((pane, a11y)) = waiting else {
            return;
        };
        keyboard.write().wanted = false;
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

/// Forget an ask nobody has spent: what a press that puts the keyboard somewhere itself
/// says. An ask now waits for a box, so one made for a tab that never drew a pane would sit
/// there and be spent by whatever pane arrived next -- taking the keyboard out of the list
/// the reader had put it in meanwhile.
pub(crate) fn unask_keyboard(mut keyboard: State<Keys>) {
    if keyboard.peek().wanted {
        keyboard.write().wanted = false;
    }
}
