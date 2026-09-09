//! The chords the window answers wherever the keyboard is, and what a text box does with
//! a key before it edits anything.

use super::*;

/// The four modifiers a gesture is spelt in: the platform's command key -- Ctrl, or Cmd
/// on a Mac -- Shift and Alt. A lock is none of them, and is masked off here: Caps Lock
/// and Num Lock arrive in the same set as the four, so a chord compared against the set
/// whole would go unanswered on a keyboard with either of them on.
pub(crate) fn held(modifiers: Modifiers) -> Modifiers {
    modifiers & (Modifiers::CONTROL | Modifiers::META | Modifiers::SHIFT | Modifiers::ALT)
}

/// The one key a chord is pressed on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stroke {
    /// A character, in either case: `F` as well as `f`, which is what Caps Lock makes of
    /// it. The digits and the punctuation have no case to differ in.
    Typed(&'static str),
    /// A key with a name of its own: the function keys, Tab and the arrows.
    Named(NamedKey),
}

impl Stroke {
    /// Whether `key` is this one.
    fn is(self, key: &Key) -> bool {
        match (self, key) {
            (Stroke::Typed(text), Key::Character(character)) => {
                !text.is_empty() && character.eq_ignore_ascii_case(text)
            }
            (Stroke::Named(named), Key::Named(pressed)) => named == *pressed,
            _ => false,
        }
    }
}

/// One of the window's chords: **a key and the modifiers it wants**, which is all a chord
/// is. The key is a letter, a digit, a piece of punctuation or a named key, and the
/// modifiers are exactly those the chord names -- a chord with Ctrl alone is not answered
/// with Alt held as well.
///
/// **Only the window's own keys are here.** A key a list, a box, a pane or the finder
/// answers where it is -- the arrows, Home, End, Page Up, Page Down, Enter and Escape --
/// belongs to that handler and must never be a chord: [`Chord::ALL`] is declined by every
/// text box, and declining those would take the keys a box is typed with.
///
/// Most of these are **named and not yet answered**. The name is what a text box declines
/// and what the handler that grows the binding asks for, and naming them all at once is
/// what keeps the declining whole: a chord added one at a time is a chord that types a
/// letter into a box until the day its binding lands.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Chord {
    /// Ctrl+F: the box over the list the keyboard is in, or the find bar over the code
    /// pane it is in. Answered on a list's rows (`filter_bar.rs`) and on a pane's own box
    /// (`find_bar.rs`), so the bar a reader opens is the one they were reading.
    Find,
    /// Ctrl+Shift+F: the Search panel, exactly the chord [`Chord::Find`] leaves free.
    /// Answered at the root ([`root_key_down`]), since it works from anywhere.
    Search,
    /// Ctrl+P: the file finder. Answered at the root, wherever the keyboard is.
    Finder,
    /// Ctrl+W: close the tab on screen, page or document.
    CloseTab,
    /// Ctrl+F4: the second spelling of [`Chord::CloseTab`], which is what a reader who
    /// learnt the key in a Windows editor reaches for. One door, two keys.
    CloseTabF4,
    /// Ctrl+Tab: the next tab along the bar.
    NextTab,
    /// Ctrl+Shift+Tab: the one before.
    PreviousTab,
    /// Ctrl+1 to Ctrl+9: the nth tab along the bar, 9 being the last however many there
    /// are.
    ///
    /// One variant carrying the number, and not nine of its own: the nine differ in
    /// nothing but the digit, the digit is the argument the answer takes, and nine
    /// variants would be nine `match` arms of the same line. A number no chord was built
    /// with is a chord no key is ([`Chord::digit`]).
    NthTab(u8),
    /// Alt+Left: a step back along this tab's trail.
    Back,
    /// Alt+Right: a step forward along it.
    Forward,
    /// Ctrl+O: open a project.
    OpenProject,
    /// Ctrl+,: the Settings page.
    Settings,
    /// F1: the Shortcuts page.
    Shortcuts,
    /// Ctrl+Shift+L: start the language server, or stop it.
    Server,
    /// Ctrl+Shift+E: raise the Files panel and put the keyboard in it.
    Files,
    /// Ctrl+Shift+O: raise Objects.
    Objects,
    /// Ctrl+T: raise Symbols.
    Symbols,
    /// Alt+C: the match case toggle of the filter bar the keyboard is in.
    MatchCase,
    /// Alt+W: its whole word toggle.
    WholeWord,
    /// Alt+R: its regular expression toggle.
    Regex,
    /// F3: the next match of the find bar over the pane the keyboard is in.
    FindNext,
    /// Shift+F3: the match before it.
    FindPrevious,
    /// F12: go to what the name under the caret names.
    Definition,
    /// Shift+F12: find references to it.
    References,
    /// Ctrl+F12: find implementations of it.
    Implementations,
    /// Alt+F12: every symbol this line was compiled into.
    AllLocations,
    /// Ctrl+D: bookmark the place the tab is showing, or take the bookmark off it.
    Bookmark,
    /// Ctrl+\: put the other pane away, or bring it back.
    OtherPane,
    /// Ctrl+B: build the scratchpad's pad.
    Build,
    /// F5: run it.
    Run,
    /// Shift+F5: stop the run.
    StopRun,
    /// Ctrl+N: a new scratchpad.
    NewPad,
}

impl Chord {
    /// Every one of them, which is what a text box declines. Every variant is named here,
    /// [`Chord::NthTab`] digit by digit: a chord left out is a chord a box would keep.
    pub(crate) const ALL: [Chord; 40] = [
        Chord::Find,
        Chord::Search,
        Chord::Finder,
        Chord::CloseTab,
        Chord::CloseTabF4,
        Chord::NextTab,
        Chord::PreviousTab,
        Chord::NthTab(1),
        Chord::NthTab(2),
        Chord::NthTab(3),
        Chord::NthTab(4),
        Chord::NthTab(5),
        Chord::NthTab(6),
        Chord::NthTab(7),
        Chord::NthTab(8),
        Chord::NthTab(9),
        Chord::Back,
        Chord::Forward,
        Chord::OpenProject,
        Chord::Settings,
        Chord::Shortcuts,
        Chord::Server,
        Chord::Files,
        Chord::Objects,
        Chord::Symbols,
        Chord::MatchCase,
        Chord::WholeWord,
        Chord::Regex,
        Chord::FindNext,
        Chord::FindPrevious,
        Chord::Definition,
        Chord::References,
        Chord::Implementations,
        Chord::AllLocations,
        Chord::Bookmark,
        Chord::OtherPane,
        Chord::Build,
        Chord::Run,
        Chord::StopRun,
        Chord::NewPad,
    ];

    /// The character [`Chord::NthTab`] is pressed on, and nothing at all outside 1 to 9,
    /// so a chord built with a number the bar has no key for matches no key either.
    fn digit(nth: u8) -> &'static str {
        ["1", "2", "3", "4", "5", "6", "7", "8", "9"]
            .get(nth.wrapping_sub(1) as usize)
            .copied()
            .unwrap_or("")
    }

    /// The key this chord is pressed on and the modifiers it wants beside it. One table,
    /// so a chord that gains a variant has to say both.
    fn spelling(self) -> (Stroke, Modifiers) {
        let command = Modifiers::ctrl_or_meta();
        let shift = Modifiers::SHIFT;
        let alt = Modifiers::ALT;
        let alone = Modifiers::empty();
        match self {
            Chord::Find => (Stroke::Typed("f"), command),
            Chord::Search => (Stroke::Typed("f"), command | shift),
            Chord::Finder => (Stroke::Typed("p"), command),
            Chord::CloseTab => (Stroke::Typed("w"), command),
            Chord::CloseTabF4 => (Stroke::Named(NamedKey::F4), command),
            Chord::NextTab => (Stroke::Named(NamedKey::Tab), command),
            Chord::PreviousTab => (Stroke::Named(NamedKey::Tab), command | shift),
            Chord::NthTab(nth) => (Stroke::Typed(Chord::digit(nth)), command),
            Chord::Back => (Stroke::Named(NamedKey::ArrowLeft), alt),
            Chord::Forward => (Stroke::Named(NamedKey::ArrowRight), alt),
            Chord::OpenProject => (Stroke::Typed("o"), command),
            Chord::Settings => (Stroke::Typed(","), command),
            Chord::Shortcuts => (Stroke::Named(NamedKey::F1), alone),
            Chord::Server => (Stroke::Typed("l"), command | shift),
            Chord::Files => (Stroke::Typed("e"), command | shift),
            Chord::Objects => (Stroke::Typed("o"), command | shift),
            Chord::Symbols => (Stroke::Typed("t"), command),
            Chord::MatchCase => (Stroke::Typed("c"), alt),
            Chord::WholeWord => (Stroke::Typed("w"), alt),
            Chord::Regex => (Stroke::Typed("r"), alt),
            Chord::FindNext => (Stroke::Named(NamedKey::F3), alone),
            Chord::FindPrevious => (Stroke::Named(NamedKey::F3), shift),
            Chord::Definition => (Stroke::Named(NamedKey::F12), alone),
            Chord::References => (Stroke::Named(NamedKey::F12), shift),
            Chord::Implementations => (Stroke::Named(NamedKey::F12), command),
            Chord::AllLocations => (Stroke::Named(NamedKey::F12), alt),
            Chord::Bookmark => (Stroke::Typed("d"), command),
            Chord::OtherPane => (Stroke::Typed("\\"), command),
            Chord::Build => (Stroke::Typed("b"), command),
            Chord::Run => (Stroke::Named(NamedKey::F5), alone),
            Chord::StopRun => (Stroke::Named(NamedKey::F5), shift),
            Chord::NewPad => (Stroke::Typed("n"), command),
        }
    }

    /// Whether `key` is this chord: its own key, under its own modifiers and no others.
    /// The modifiers are exact, which is the whole of what tells Ctrl+F from Ctrl+Shift+F
    /// and F12 from its three neighbours -- each is answered for the modifier the others
    /// decline.
    pub(crate) fn is(self, key: &Key, modifiers: Modifiers) -> bool {
        let (stroke, wanted) = self.spelling();
        stroke.is(key) && held(modifiers) == wanted
    }

    /// The key event this chord is, as a test presses one. Only the tests want it: the
    /// app asks the other way round, a key arriving and [`Chord::is`] saying whether it
    /// was a chord.
    #[cfg(test)]
    pub(crate) fn pressed(self) -> (Key, Modifiers) {
        let (stroke, modifiers) = self.spelling();
        let key = match stroke {
            Stroke::Typed(text) => Key::Character(text.into()),
            Stroke::Named(named) => Key::Named(named),
        };
        (key, modifiers)
    }

    /// Whether `key` is any of them.
    fn any(key: &Key, modifiers: Modifiers) -> bool {
        Chord::ALL.iter().any(|chord| chord.is(key, modifiers))
    }
}

/// Which of freya's two text boxes the keys are for. Their defaults differ and each keeps
/// its own: an `Input` leaves Tab to move the keyboard on and cancels every ordinary key,
/// where the editor types the Tab and leaves the key to the handlers beside it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Boxed {
    /// freya's `Input`: every filter bar, the find bar, the finder's box.
    Input,
    /// freya's `CodeEditor`: the scratchpad's (`pad_view.rs`).
    Editor,
}

impl Boxed {
    /// What the box itself does with a key nothing above it declined: freya's own hook,
    /// written out once (`notes/upstream/freya.md`).
    fn tail(self, e: &Event<KeyboardEventData>) -> bool {
        match self {
            Boxed::Input => match &e.key {
                Key::Named(NamedKey::Enter)
                | Key::Named(NamedKey::Escape)
                | Key::Named(NamedKey::Shift) => true,
                Key::Named(NamedKey::Tab) => false,
                _ => {
                    e.stop_propagation();
                    e.prevent_default();
                    true
                }
            },
            Boxed::Editor => {
                e.stop_propagation();
                if let Key::Named(NamedKey::Tab) = &e.key {
                    e.prevent_default();
                }
                true
            }
        }
    }
}

/// The hook every text box is given: the window's chords and the `declined` keys left to
/// the handlers beside the box, `answer` run for the keys the box itself acts on, and
/// freya's own tail for the rest.
///
/// **Declining is what makes a chord work at all.** A box inserts any character it has no
/// chord of its own for, Ctrl held or not, so Ctrl+P in one would type a `p`; and the
/// tail's `prevent_default` cancels the global key event the root answers a chord by, so
/// a chord the box kept would reach nothing either. The second reason is the whole of why
/// a chord on a **named** key -- F1, F3, Alt+Left -- has to be declined as well, nothing
/// being typed by one: the tail cancels it exactly as it cancels a letter's.
///
/// `declined` is the box's own list, the named keys that belong to a handler beside it --
/// the finder's arrows move a list its box does not hold. A named key in neither list is
/// the box's to swallow.
pub(crate) fn box_keys(
    boxed: Boxed,
    declined: &'static [NamedKey],
    answer: impl Fn(&Key, Modifiers) + 'static,
) -> Callback<Event<KeyboardEventData>, bool> {
    Callback::new(move |e: Event<KeyboardEventData>| {
        if Chord::any(&e.key, e.modifiers) {
            return false;
        }
        if matches!(&e.key, Key::Named(named) if declined.contains(named)) {
            return false;
        }
        answer(&e.key, e.modifiers);
        boxed.tail(&e)
    })
}
