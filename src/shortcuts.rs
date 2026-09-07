//! Every key and every mouse gesture the app answers to, written out.
//!
//! **The list is written by hand and nothing checks it against the handlers.** The other
//! answer was to make bindings data the handlers read rather than matches they are written
//! as, which is a refactor of every handler in the app; it is worth doing when something
//! else wants it, and until then the list is honest today and wrong the first time someone
//! adds a binding without touching it.
//!
//! A row is a gesture and what it does, and the rules for what gets a row of its own are
//! `notes/specs/Shortcuts.md`'s: a key and a click that do the same thing share one, one
//! row carries the Shift rule for every motion key rather than each doubling, and a gesture
//! that means different things in two places says both.

use crate::filter::Matcher;

/// One gesture: how it is pressed, and what it does.
#[derive(PartialEq)]
pub struct Gesture {
    /// Written as it is pressed -- `Ctrl+P`, `Shift+click`, `Wheel`.
    pub keys: &'static str,
    pub does: &'static str,
}

/// The gestures that apply in one place, under the name of that place.
pub struct Section {
    pub place: &'static str,
    pub gestures: &'static [Gesture],
}

/// A section and the rows of it a filter kept.
pub struct Listed {
    pub section: &'static Section,
    pub gestures: Vec<&'static Gesture>,
}

/// Shorthand, so the table below is the list and not the punctuation around it.
const fn gesture(keys: &'static str, does: &'static str) -> Gesture {
    Gesture { keys, does }
}

/// Every section, the ones that work anywhere first.
///
/// A `static` and not a `const`: the rows are handed to the view as `&'static Gesture`,
/// and a `const` is copied into each place it is named rather than having an address.
pub static SECTIONS: &[Section] = &[
    Section {
        place: "Anywhere in the window",
        gestures: &[
            gesture("Ctrl+P", "Open the file finder."),
            gesture(
                "Ctrl+Shift+F",
                "Open the Search panel and put the caret in its box.",
            ),
            gesture("Back button", "Go back in this tab's history."),
            gesture("Forward button", "Go forward in it."),
            gesture(
                "Any key or press",
                "Put away the box the language server's answer is in.",
            ),
        ],
    },
    Section {
        place: "The code panes",
        gestures: &[
            gesture(
                "Click",
                "Put the caret where the pointer is; in the gutter, select the whole row.",
            ),
            gesture(
                "Drag",
                "Select as far as the pointer. Past the pane's edge it scrolls and goes on \
                 selecting.",
            ),
            gesture("Shift+click", "Reach the selection out to here."),
            gesture("Double-click", "Select the word."),
            gesture("Triple-click", "Select the row's text."),
            gesture("Left, Right", "Move the caret a character."),
            gesture("Ctrl+Left, Ctrl+Right", "Move it a word."),
            gesture("Up, Down", "Move it a row."),
            gesture("Home, End", "Move it to the start or the end of the row."),
            gesture(
                "Ctrl+Home, Ctrl+End",
                "Move it to the start or the end of the listing.",
            ),
            gesture("Page Up, Page Down", "Move it a screen of rows."),
            gesture(
                "Shift with any of those",
                "Reach the selection out as the caret moves.",
            ),
            gesture(
                "Ctrl+C",
                "Copy what is selected, or the caret's row where nothing is.",
            ),
            gesture("Ctrl+A", "Select the whole listing."),
            gesture(
                "Escape",
                "Drop the selection, the caret first and then the run.",
            ),
            gesture("Click a link", "Open what it names, in this tab."),
            gesture("Ctrl+click a link", "Open it in a new tab."),
            gesture("Ctrl+click an address", "Open the object's code there."),
            gesture(
                "Alt+click a link",
                "Select over it instead of following it.",
            ),
            gesture("Click a branch arrow", "Go to the row it points at."),
            gesture("Right-click", "The row's menu."),
            gesture("Rest on a name", "Ask the language server what it is."),
        ],
    },
    Section {
        place: "The sidebar",
        gestures: &[
            gesture(
                "Click a row",
                "Open it in the temporal tab, the one the next row reuses, and put the \
                 keyboard in that tab. An archive or a folder folds instead.",
            ),
            gesture("Ctrl+click a row", "Open it in a tab that stays."),
            gesture(
                "Alt+click a row",
                "Pick it out and open nothing, so the keyboard stays on the list. A folder \
                 or an archive does not fold either.",
            ),
            gesture("Right-click a row", "The row's menu."),
            gesture("Up, Down", "Move the pick to another row."),
            gesture(
                "Enter",
                "Open the row the pick is on, exactly as pressing it would.",
            ),
            gesture("Ctrl+F", "Put the caret in the filter box over the list."),
        ],
    },
    Section {
        place: "The tab bar",
        gestures: &[
            gesture("Click a tab", "Show it."),
            gesture("Double-click a tab", "Keep a temporal tab."),
            gesture("Right-click a tab", "The tab's menu."),
            gesture("Drag a tab", "Move it along the bar."),
            gesture("Click the ×", "Close the tab."),
            gesture("Wheel", "Scroll the bar."),
        ],
    },
    Section {
        place: "The file finder",
        gestures: &[
            gesture("Up, Down", "Move to another file."),
            gesture("Enter", "Open the file."),
            gesture("Click a row", "Open that file."),
            gesture("Alt+click a row", "Move to it and open nothing."),
            gesture("Escape, or click outside", "Close the finder."),
        ],
    },
    Section {
        place: "A text box",
        gestures: &[
            gesture("Enter", "In the Search panel, run the search."),
            gesture("Escape", "Leave the box."),
            gesture("Tab", "Nothing. It does not move the keyboard on."),
        ],
    },
    Section {
        place: "The scratchpad",
        gestures: &[
            gesture("Tab", "Indent."),
            gesture("Click a diagnostic", "Go to what it is about."),
        ],
    },
    Section {
        place: "Menus and windows",
        gestures: &[gesture(
            "Escape, or click outside",
            "Close the menu, or the window that is asking.",
        )],
    },
];

/// The sections a filter left, each with the rows of it that matched.
///
/// A row matches on either half, so a reader who knows the key and a reader who knows what
/// they want both find it. **A section with nothing left is dropped** rather than drawn
/// empty under its heading.
pub fn matching(matcher: &Matcher) -> Vec<Listed> {
    SECTIONS
        .iter()
        .filter_map(|section| {
            let gestures: Vec<&Gesture> = section
                .gestures
                .iter()
                .filter(|gesture| matcher.matches(gesture.keys) || matcher.matches(gesture.does))
                .collect();

            (!gestures.is_empty()).then_some(Listed { section, gestures })
        })
        .collect()
}

#[cfg(test)]
mod tests;
