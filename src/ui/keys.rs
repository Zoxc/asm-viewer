//! Whether Shift, Ctrl and Alt are held. Their own states, written from the root's
//! *global* key handlers: a freya pointer event carries no modifiers at all, so a press
//! that means something else under one has to read these rather than the event. Every
//! door reads one -- a row that opens something, a link in a listing -- and so does the
//! pages menu, which offers the Debug page only under Alt. The window losing the focus
//! lets go of all three, a key released over another window sending no key-up here.
//!
//! The three are read off [`ModifierKeys`], which is the whole keyboard: those three and
//! the two states a Caps Lock made into Ctrl is learnt with. One value and not five
//! handles passed about, so the learning and what a door reads cannot come apart.

use super::*;

/// Whether Shift is held, which is what turns a click into "reach to here".
#[derive(Clone, Copy)]
pub(crate) struct Shift(pub(crate) State<bool>);

/// Whether Ctrl is held. It is what turns a press on a symbol's label in the unified view
/// into opening the symbol's tab, where a plain press is a plain press -- picking the row
/// out, and nothing that changes the tab.
#[derive(Clone, Copy)]
pub(crate) struct Ctrl(pub(crate) State<bool>);

/// Whether Alt is held. It is what says a press on a link is not a door this time: every
/// door in a code row acts on a plain press, which leaves no way to put the pointer down
/// on one and sweep, the release following the link instead.
#[derive(Clone, Copy)]
pub(crate) struct Alt(pub(crate) State<bool>);

/// The keyboard as the root's global key handlers keep it: the three modifiers every door
/// reads and the two states it takes to keep them right, made together
/// ([`provide_modifiers`]) because they are only right together.
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
    /// The three every door reads, each provided as a context of its own by
    /// [`provide_modifiers`] so that a row watching Ctrl is not redrawn by Shift.
    pub(crate) shift: State<bool>,
    pub(crate) ctrl: State<bool>,
    pub(crate) alt: State<bool>,
    /// Whether this keyboard's Caps Lock has shown itself to be a Ctrl.
    caps_is_ctrl: State<bool>,
    /// Whether a key *named* Control is down, which is what tells a Caps Lock released
    /// under a real Ctrl from one that is the Ctrl.
    control_held: State<bool>,
}

impl ModifierKeys {
    /// A keyboard with nothing held: its five states, made here and nowhere else. All
    /// five are `State<bool>`, so a constructor taking them in would be five arguments
    /// any two of which could be swapped without a word from the compiler.
    pub(crate) fn create() -> Self {
        Self {
            shift: State::create(false),
            ctrl: State::create(false),
            alt: State::create(false),
            caps_is_ctrl: State::create(false),
            control_held: State::create(false),
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
        // Alt is read by its own name and its own bit alone: no desktop makes another key
        // into it the way Caps Lock is made into Ctrl.
        self.alt.set_if_modified(
            *key == Key::Named(NamedKey::Alt) || modifiers.contains(Modifiers::ALT),
        );
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
        self.alt.set_if_modified(
            *key != Key::Named(NamedKey::Alt) && modifiers.contains(Modifiers::ALT),
        );
    }

    /// Every key let go of: what the window losing the focus means, a key released over
    /// another window sending this one no key-up. What was learnt about Caps Lock stays.
    pub(crate) fn let_go(mut self) {
        self.shift.set_if_modified(false);
        self.ctrl.set_if_modified(false);
        self.alt.set_if_modified(false);
        self.control_held.set_if_modified(false);
    }
}

/// Let go of every key whenever the window loses the focus ([`ModifierKeys::let_go`]),
/// read off freya's `Platform::is_app_focused`: no event says it.
pub(crate) fn use_let_go_on_blur(keys: ModifierKeys) {
    let focused = Platform::get().is_app_focused;
    use_side_effect(move || {
        if !*focused.read() {
            keys.let_go();
        }
    });
}

/// The keyboard, and the three contexts read off it, made and provided together.
///
/// One call and not four, because the five states have to be one keyboard: a row reads
/// Shift for a press that reaches, Ctrl for whether a label is a link now and Alt for
/// whether a press on one is a door at all, and the root's key handler writes those same
/// three off [`ModifierKeys`]. A harness that provided Shift alone mounted rows that
/// panicked on the first link, and one that built a keyboard of its own wrote three
/// states no row was looking at -- so [`roots`] and the bare test harnesses both come
/// here.
pub(crate) fn provide_modifiers() -> ModifierKeys {
    let keys = ModifierKeys::create();
    provide_root_context(keys);
    provide_root_context(Shift(keys.shift));
    provide_root_context(Ctrl(keys.ctrl));
    provide_root_context(Alt(keys.alt));
    keys
}
