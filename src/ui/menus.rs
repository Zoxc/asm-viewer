//! The menus a right-click opens over a tab and over a file row, and the two items more
//! than one menu is built of.
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
                .on_press(move |_| match keep {
                    Tab::Document(id) => close_tab(open, places, id),
                    Tab::Page(page) => close_page(open, page),
                })
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
/// reads as bookmarked -- and the name a new one gets is the whole `entry_name`, what the
/// row's tooltip says. `add` is what the item says when there is none yet: a sidebar row
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

/// The write every bookmark gesture makes: a bookmark of `document` added, or the one
/// pointing at it taken off. The item above presses it and so does the window's key
/// (`Chord::Bookmark`), so the two cannot come to mean different things.
///
/// Which of the two happens is [`Bookmarks::toggle`]'s own question, asked by resolving
/// each entry against what is loaded; the name a new one is made under is the whole
/// [`entry_name`].
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
        .toggle(document, entry_name(document), &loaded);
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
