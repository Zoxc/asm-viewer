//! The menus a right-click opens over a tab, over a file row and over a sidebar row, the
//! items more than one menu is built of, and the pieces they are all drawn from: a row,
//! what its text says, the mark after it, the line between two groups, and the button a
//! menu hangs under.
//!
//! A menu is **built per press**, closing over whatever was under the pointer, and its
//! states come in as arguments because it is made in an event handler, where no hook may
//! run.

use super::*;

/// The menu a tab opens on a right-click: **Close**, **Close other tabs** where the tab
/// has company, and then, for a document, the bookmark item and **Show in file manager**
/// for the file it is a place in.
///
/// The chip says whether there is another tab to close, so the one row that would do
/// nothing is left out rather than drawn dead.
///
/// **Only the keys that would do what the row does.** Ctrl+W and Ctrl+D are answered for
/// the tab **on screen** (`root_key_down`), and this menu opens on whichever chip was
/// under the pointer, so a menu on any other chip says neither: the rows do what they
/// say, and a key beside one would be closing or bookmarking somebody else. `showing` is
/// whether this chip is that tab.
pub(crate) fn tab_menu(
    states: ProjectStates,
    keep: Tab,
    others: bool,
    showing: bool,
    document: Option<Document>,
) -> Menu {
    let ProjectStates {
        open,
        places,
        bookmarks,
        objects,
        ..
    } = states;

    Menu::new()
        .child(
            MenuButton::new()
                .on_press(move |_| close(open, places, keep))
                .child(menu_label(
                    "Close",
                    showing.then_some(shortcuts::key!(CloseTab)),
                )),
        )
        .maybe_child(others.then(|| {
            MenuButton::new()
                .on_press(move |_| close_others(open, places, keep))
                // "tabs" and not "documents": the bar is what the reader is pointing at,
                // and a page in it goes the way a document does.
                .child("Close other tabs")
        }))
        // The file the tab is a place in: the binary for an assembly tab, the source file
        // for a file's. A page is neither, and has neither row.
        .maybe_child(document.clone().map(|document| {
            bookmark_item(
                bookmarks,
                objects,
                document,
                "Add bookmark",
                showing.then_some(shortcuts::key!(Bookmark)),
            )
        }))
        .maybe_child(document.map(|document| reveal_item(document.file().to_path_buf())))
}

/// The menu a Files row over an object that is not loaded opens on a right-click: one
/// item, opening it the way the toolbar's Open does.
pub(crate) fn open_menu(
    objects: State<Vec<Arc<Object>>>,
    loading: State<Loads>,
    path: PathBuf,
) -> Menu {
    Menu::new().child(
        MenuButton::new()
            .on_press(move |_| {
                let path = path.clone();
                // `spawn_forever`, not `spawn`: a task belongs to the scope that spawned
                // it, and this one's is the menu's button, which the press closes -- the
                // load would be dropped before its first poll.
                spawn_forever(async move {
                    open_binaries(objects, loading, vec![path]).await;
                });
            })
            // The opposite of the Objects row's "Close file", in the same word.
            .child("Open file"),
    )
}

/// The menu a file row opens on a right-click: one item, letting go of the binary.
pub(crate) fn close_menu(states: ProjectStates, path: PathBuf) -> Menu {
    Menu::new().child(
        MenuButton::new()
            .on_press(move |_| close_binary(states, &path))
            // "file" and not "object": the row may be one object of a file or the archive
            // above 196 of them, and the word has to be true of both.
            .child("Close file"),
    )
}

/// The menu a Files row over a file opens on a right-click: **Close file** when the app
/// holds the path already ([`ProjectStates::holds_path`]), and **Open file** when it does
/// not. Never both, since opening a path twice puts a second copy of each of its objects
/// in the list.
///
/// Whatever the row adds -- the file manager's item, and a project file's own -- goes on
/// after this, so the Objects rows that share `close_menu` keep the one item they had.
pub(crate) fn file_menu(states: ProjectStates, path: PathBuf) -> Menu {
    match states.holds_path(&path) {
        true => close_menu(states, path),
        false => open_menu(states.objects, states.loading, path),
    }
}

/// The one menu item every bookmark gesture is: adding a bookmark of `document`, or
/// removing the one that points at it, whichever is true at the press. Which it is comes
/// from `Bookmarks::matching` -- by resolution, so a symbol that moved under a rebuild still
/// reads as bookmarked -- and the name a new one gets is [`Names::whole`], what the row's
/// tooltip says. `add` is what the item says when there is none yet: a sidebar row
/// and a tab say "Add bookmark", an instruction row "Bookmark symbol", since the row is not
/// the symbol and has to say what it would bookmark.
///
/// `key` is Ctrl+D where this menu was opened somewhere that key means this very
/// document, which is the tab on screen and nowhere else: the key is asked of that tab
/// (`root_key_down`) and not of the row under the pointer.
pub(crate) fn bookmark_item(
    bookmarked: State<Bookmarks>,
    objects: State<Vec<Arc<Object>>>,
    document: Document,
    add: &'static str,
    key: Option<&'static str>,
) -> MenuButton {
    let bookmarked_already = bookmarked
        .peek()
        .matching(&document, &objects.peek())
        .is_some();
    let text = match bookmarked_already {
        true => "Remove bookmark",
        false => add,
    };
    MenuButton::new()
        .on_press(move |_| toggle_bookmark(bookmarked, objects, &document))
        .child(menu_label(text, key))
}

/// The menu a Symbols or History row opens on a right-click: [`bookmark_item`] and
/// nothing else.
///
/// No key beside it: Ctrl+D is about the tab on screen and this row is not it.
pub(crate) fn bookmark_menu(
    bookmarked: State<Bookmarks>,
    objects: State<Vec<Arc<Object>>>,
    document: Document,
) -> Menu {
    Menu::new().child(bookmark_item(
        bookmarked,
        objects,
        document,
        "Add bookmark",
        None,
    ))
}

/// The write every bookmark gesture makes: a bookmark of `document` added, or the one
/// pointing at it taken off. [`bookmark_item`] presses it and so does the window's key
/// (`Chord::Bookmark`), so the two cannot come to mean different things.
///
/// Which of the two happens is [`Bookmarks::toggle`]'s own question, asked by resolving
/// each entry against what is loaded; the name a new one is made under is
/// [`Names::whole`].
pub(crate) fn toggle_bookmark(
    mut bookmarked: State<Bookmarks>,
    objects: State<Vec<Arc<Object>>>,
    document: &Document,
) {
    // The objects are peeked before the list is written: two different states, and the
    // write wakes the panel.
    let loaded = objects.peek().clone();
    bookmarked
        .write()
        .toggle(document, Names::of(document).whole, &loaded);
}

/// The item that shows a file, or a folder, where the rest of the reader's tools are: on
/// a document's tab, and on a Files row. One item everywhere, since the path is all it is
/// about; which of the two it names is worked out where the call is made.
///
/// The call is a subprocess and is made on a thread of its own (`crate::reveal`), so
/// there is no task here for the press that closes the menu to drop.
pub(crate) fn reveal_item(path: PathBuf) -> MenuButton {
    MenuButton::new()
        .on_press(move |_| reveal::reveal(path.clone()))
        .child("Show in file manager")
}

/// One row of a menu: a word, the key it has where it has one, and what pressing it
/// does. A helper and not a component, the hover being `MenuItem`'s own.
pub(crate) fn menu_row(
    text: &str,
    key: Option<&'static str>,
    mut close: State<bool>,
    mut act: impl FnMut() + 'static,
) -> MenuButton {
    MenuButton::new()
        .on_press(move |_| {
            act();
            close.set(false);
        })
        .child(menu_label(text, key))
}

/// **What every menu item's text is drawn as**: what the item does, and -- where the
/// gesture has a key where the menu was opened -- how that key is pressed, after the name
/// and a step back from it.
///
/// The spelling is never written here. It comes from `shortcuts::key!`, the list the
/// Shortcuts page draws (`src/shortcuts.rs`), so a menu and that page cannot come to say
/// different things. An item with no key is the bare label it always was.
pub(crate) fn menu_label(text: impl Into<String>, key: Option<&'static str>) -> Element {
    match key {
        None => item_name(text.into(), None).into_element(),
        Some(key) => marked_label(text.into(), key, None),
    }
}

/// The name of a menu row, in `colour` where it is not the menu's own.
fn item_name(text: String, colour: Option<Color>) -> Label {
    label()
        .text(text)
        .max_lines(1)
        .map(colour, |name, colour| name.color(colour))
}

/// A menu row's name with a mark after it: the arrow on a row that opens a submenu, or
/// the key on a row that has one. One treatment for the two, so they sit alike.
///
/// **After the name and not out at the row's own end**, which is where a desktop menu puts
/// it. A row here is a `MenuItem` -- `fill_minimum` inside a container that fits its
/// content -- so a child asking to fill takes the *window* and drags the menu out to it,
/// and nothing in the row can learn how wide the widest row made the menu
/// (`notes/upstream/freya.md`). [`MENU_MARK_GAP`] is what keeps the mark from reading as
/// part of the word.
///
/// `colour` is the dim row's, which is drawn in place of a live one and has to look like
/// it; a live row inherits the menu's own and is handed `None`. The mark is a step back
/// from the name either way, being about the row rather than part of what it says.
fn marked_label(text: String, mark: &str, colour: Option<Color>) -> Element {
    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(MENU_MARK_GAP)
        .child(item_name(text, colour))
        .child(
            label()
                .text(mark.to_owned())
                .max_lines(1)
                .color(colour.unwrap_or_else(|| palette().address_fg)),
        )
        .into_element()
}

/// What freya lays a `MenuItem` out at, so a row of the app's own beside them lines up.
/// Neither is reachable from the theme, so both are written here and pinned by a test.
pub(crate) const MENU_ROW_WIDTH: f32 = 105.0;
pub(crate) const MENU_ROW_PADDING: (f32, f32) = (6.0, 12.0);

/// The mark on a row that opens a submenu: freya's `SubMenu` draws none, so such a row is
/// otherwise the twin of one that acts. The glyph the Files tree folds with, so the app
/// points one way everywhere.
const SUBMENU_ARROW: &str = "\u{25b8}";

/// One of those rows: the name, and the arrow after it -- [`marked_label`] with the arrow
/// as its mark, the treatment a key beside an item is drawn with too.
pub(crate) fn submenu_label(text: &str, colour: Option<Color>) -> Element {
    marked_label(text.to_owned(), SUBMENU_ARROW, colour)
}

/// A line between two groups of the menu. freya has no separator, and a `Menu` takes any
/// child, so it is a rect a pixel high in the colour the panes are divided by.
pub(crate) fn menu_rule() -> Element {
    rect()
        .width(Size::fill())
        .height(Size::px(1.0))
        .margin(Gaps::new_symmetric(4.0, 0.0))
        .background(palette().hairline)
        .into_element()
}

/// A button that opens a menu under itself: [`bar_button`] lit while the menu is up or the
/// pointer is on it, and the menu hung from the button's bottom edge, closed by `Menu`'s
/// own press-outside. The position is **vertical only**; which way the menu opens is
/// `MenuContainer`'s own overflow correction, so a button at either end of the bar opens
/// its menu into the window.
///
/// **The popup is positioned by hand** rather than through `ContextMenu`, which pins a
/// menu's top-left corner to the pointer and clamps to nothing -- opened from a button at
/// the right-hand edge of the bar it would draw off the side of the window.
///
/// **No guard against `Menu`'s own close-on-any-global-press**, and none is needed: global
/// listeners are snapshotted when the event is measured, before any handler runs, so the
/// menu this press opens is not in that batch. A popup opened from a `*_down` handler is
/// the case that does need the swallow; copying it here ate the first press outside the
/// menu.
///
/// A helper and not a component: `hovering` and `showing` are the caller's, a hook running
/// only where one renders. `press` is what the press does with `showing`, so a caller that
/// reads a modifier at that moment ([`PagesButton`]) can. The box is the caller's too --
/// the tab list's is as wide as the chips' close column and as tall as the bar it is
/// pinned to the end of, where the pages menu is a toolbar square.
pub(crate) fn dropdown(
    size: (f32, f32),
    tooltip: &str,
    icon: Element,
    hovering: State<bool>,
    showing: State<bool>,
    press: impl FnMut(Event<PressEventData>) + 'static,
    menu: impl FnOnce() -> Element,
) -> Element {
    let (width, height) = size;
    let glow = match showing() {
        true => Glow::Open,
        false => Glow::No,
    };
    let button = extra_tooltip(
        tooltip.to_owned(),
        bar_button(hovering, true, glow)
            .width(Size::px(width))
            .height(Size::px(height))
            .on_press(press)
            .child(icon),
    );

    rect()
        .width(Size::px(width))
        .height(Size::px(height))
        .child(button)
        .maybe_child(showing().then(|| {
            rect()
                .position(Position::new_absolute().top(height))
                .child(menu())
                .into_element()
        }))
        .into_element()
}
