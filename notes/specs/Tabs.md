# Tabs

One tab per open document: a function, an object or a source file. A tab is assembly-driven,
showing a symbol, or source-driven, showing a file; an icon tells them apart. Clicking a tab
switches to it. Its × closes it and switches to the neighbour; closing the last shows the
placeholder. The × has a square target with four pixels of air around it, and is highlighted
under the pointer. The open tabs are saved with the session in their order.

A control at the right of the tab bar lists every open document, the one on screen marked;
picking one activates it.

A tab's context menu has "Close other tabs", which closes every other document in the panel.
If the tab on screen was closed, the kept tab is shown.

## Back and forward, per tab

Each tab has its own history, which the mouse's side buttons and the toolbar's chevrons move
through. It survives a restart, at most 50 entries per tab. A chevron's tooltip names where
it would go; one with nowhere to go is dimmed, not hidden.

Where a click opens depends on where it was made.

- Inside a tab (a relocation link, the companion header): the same tab. Back returns.
- With Ctrl, or from a menu: a new tab next to the current one.
- Outside a tab (a sidebar item): the temporal tab, below.
- A tab already showing the place is activated instead.

Back restores both panes' rows and selections, and the line a source-driven tab was
following. Selections are kept in memory only.

Closing a binary closes the tabs in it and removes its places from every tab's history.

The History panel is global: every place visited, function or file, across all tabs, newest
first, with no current position. Each item has its tab's icon. Opening a document adds to it,
a place visited again moving to the top; Back, Forward and switching tab do not.

## The temporal tab

A sidebar item opens in the temporal tab, the one preview tab reused by the next item. It
opens next to the current tab, stays in place, and has an italic name. Each item is added to its
history.

It becomes a normal tab on Ctrl+click of the place it shows, a double-click on its header, or
a link followed inside it. Back and Forward do not change it. A tab already showing the place is
activated instead, and the temporal tab is left as it is.

It is saved with the session as the temporal tab.

## A tab's name

A tab is named `module::fn_name`: the last two parts of the demangled name, minus generic
arguments however deeply nested (`<Vec<T> as IntoIterator>::into_iter` becomes
`Vec::into_iter`), a C++ argument list and its `const`, and rustc's legacy `::h<hash>` suffix.
`operator<<` is a name. The innermost closure is kept as a third part. A name that cannot be
shortened is shown as it is.

A name the app made up is shown as it is too. Those are written whole in angle brackets,
`<entry point>` and `<function 0x…>`; no name out of a file is.

History items use the same name. The tooltip shows the whole name and the History filter matches
it. A name still long is cut at 40 characters.
