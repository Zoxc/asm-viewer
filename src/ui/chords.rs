//! The chords the window answers wherever the keyboard is, and what a text box does with
//! a key before it edits anything.

use super::*;

/// One of the window's chords: Ctrl (or Meta) and a letter, with Shift where the chord
/// takes it and never Alt. `F` as well as `f`, which is what Caps Lock makes of it.
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
}

impl Chord {
    /// Every one of them, which is what a text box declines.
    pub(crate) const ALL: [Chord; 3] = [Chord::Find, Chord::Search, Chord::Finder];

    fn letter(self) -> &'static str {
        match self {
            Chord::Find | Chord::Search => "f",
            Chord::Finder => "p",
        }
    }

    /// Whether Shift is part of the chord, which is the whole of what tells Ctrl+F from
    /// Ctrl+Shift+F: each is answered for the modifier the other declines.
    fn shifted(self) -> bool {
        matches!(self, Chord::Search)
    }

    /// Whether `key` is this chord.
    pub(crate) fn is(self, key: &Key, modifiers: Modifiers) -> bool {
        modifiers.contains(Modifiers::ctrl_or_meta())
            && modifiers.contains(Modifiers::SHIFT) == self.shifted()
            && !modifiers.contains(Modifiers::ALT)
            && matches!(key, Key::Character(character) if character.eq_ignore_ascii_case(self.letter()))
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
/// a chord the box kept would reach nothing either. Named keys are declined for the
/// second reason alone: they belong to a handler beside the box, as the finder's arrows
/// move a list its box does not hold.
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
