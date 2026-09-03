# Sidebar

## Dock

Every panel is a tab in one of the two dock areas, with an icon. A panel can be dragged between
the areas, stacked with other panels as tabs, or split off into an area of its own.

## Filter bar

Each list panel has its own filter bar: Objects, Symbols, History, Bookmarks, Locations.
Its three toggles are written as the regex they turn on: `Aa` for case, `\b` for whole word,
`.*` for regex. A pattern that does not compile shows its error under the bar.

## Tooltips

Every list item shows its whole text in a tooltip, with no delay. The filter toggles' tooltips
keep the usual delay.

## Bookmarks

A bookmark is a symbol the user saved, listed in the Bookmarks panel in the order added and
kept with the project. It is added or removed from the context menu of a Symbols item, a History
item, a document's tab, or an instruction in the assembly pane. A bookmark whose binary is closed
is kept, shown dimmed, and works again when the binary is opened.

## Symbols panel

Every loaded object's symbols, one item each. The filter matches the demangled name the item
shows.

## Objects panel

The panel is a tree: one item per file, an archive's members as its children. Each item has
a tag for its format: `ELF`, `PE`, `COFF`, `MACH` or `AR`.

A file still being read is an item already, its name dimmed and `…` in place of the tag.
Objects appear as they are parsed, and can be explored while the rest load.
An archive item shows its object count. In a narrow sidebar the name is cut, never the count.

### Closing a file

A file item's context menu has "Close file"; an archive member's has nothing, since the
file is what closes. Closing drops every object of the file, closes the tabs in it as closing
them by hand would, removes its places from the History panel, and saves the project's binaries
at once.

## Locations panel

"Find all locations" in a source line's or an instruction's context menu lists every symbol
the line was compiled into, across every open object, one item per symbol with its object,
under a heading naming the line. Clicking an item opens the symbol at that line.

Where a function encloses a source line, its context menu also has "N instances of `foo`":
the same list for the whole function, an inlined caller included. Clicking one chooses the
symbol a source-driven tab shows.

## Files panel

The project's directory as a tree, one level read per unfold and read again on a refold.
Clicking a file opens it as a source-driven tab. A file's context menu has "Open file", which
opens it as a binary, and "Close file" once it is loaded.
