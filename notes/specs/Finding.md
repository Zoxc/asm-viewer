# Finding

## Find a file

Ctrl+P opens the file finder over the app, at the top and centred across it: a box, and under
it the files of the project's directory. Escape closes it, as does a click outside. With no
project directory it says so and points at the Project view.

The characters typed have to appear in the path in order, not next to each other: `srcuivw`
finds `src/ui/files_view.rs`. Case is ignored and the matched characters are marked. The best
come first: the file's name before a directory above it, a run before the same characters spread
out, the start of a name before inside it, the shorter path on a tie. A row is the file's name
with its directory after it, dimmed. With nothing typed it lists the project's files opened
recently, newest first.

The list is moved through and opened from the keyboard, or a row is clicked. Opening one is
pressing a Files item, with Ctrl a new tab. The finder closes and keeps nothing of what was
typed.

It lists what the source pane can show, skipping what git is told to ignore, hidden files and
files too big, as the Search panel does. The first walk fills the list; after that the finder
opens on what it has and walks again behind it, files arriving as it runs. The walk is shared
with anything else reading the directory under the same rules.
