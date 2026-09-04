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

### Uses of a name

A name's context menu has "Find uses of `foo`", which asks the language server where the name
is used. It is there whether the name is called there or defined there, and only while a
server runs. The heading is the name, and the uses are grouped under the file each is in, the
files by path and the uses by line. A file row says how many uses it holds and folds them away
when pressed; under it each use is its line number and the line, with the name marked in it,
as a Search match is. A long line is cut. Where the name is defined is not listed; clicking a
link goes there. Clicking a use opens the file on that line with the name selected, the assembly
side following it as it does a clicked line; Ctrl+click opens it in a new tab.

A name the server answers nothing for says so, as does one asked while it is still reading the
project; asking again once it has finished answers.

## Files panel

The project's directory as a tree, one level read per unfold and read again on a refold.
Clicking a file opens it as a source-driven tab. A file's context menu has "Open file", which
opens it as a binary, and "Close file" once it is loaded.

## Search panel

Searches the text of the project's directory. The box takes a pattern and Enter runs it. Its
three toggles are the filter bars' — `Aa` for case, `\b` for whole word, `.*` for regex — and a
pattern that does not compile shows its error under the box. A new search stops the one before
it. With no project directory the panel says so and points at the Project view.

Matches arrive while the search runs, grouped under the file they are in, the files in the order
the search reaches them: a directory's own files before the directories under it, each sorted by
name, so the list only grows at its end. A file row says how many matches it holds, and folds
them away when pressed. A match row is its line number and the line's text, with the matched
part marked; a long line is cut. Above the list the panel says how many matches in how many
files, and says while it is still searching.

Pressing a match opens its file as a source-driven tab on that line, as pressing a Files item
opens one. The matched text is selected there, so copying copies the match. Nothing else moves.

The search reads only what the app can show. It skips what git is told to ignore, hidden files,
files it finds to be binary, and files too big for the source pane. It does not follow symbolic
links. The Files panel is different: it shows every entry it reads.

The search stops at 10,000 matches and says there are more. What was searched for is not kept
with the session, and leaving the project clears it.

### Reaching it

Ctrl+Shift+F opens the panel and puts the caret in its box, from anywhere in the window. Ctrl+F
still reaches the filter box of the list it is pressed in.
