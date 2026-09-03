# Scratchpad

A scratchpad is one Rust source file, built with cargo as a package of its own. Its functions
open like any other binary's.

There are many scratchpads, apart from any project, each saved in a directory of its own. A
scratchpad has a name the user can change; it may be empty or the same as another's, and an
unnamed one is shown as `<pad-3>`. The Scratchpad view's side panel lists them, and the one
last opened opens first. A new scratchpad is made at once. Each has its own editor state and
its own run, so switching pads stops nothing. Scratchpads cannot be deleted.

## Crates

A scratchpad's crates.io dependencies are a list in the UI, a name and a version per row. A
wildcard version is refused. A bad row is marked in place, on the half that is wrong, with the
reason under it. A build cargo rejects before compiling is shown against the rows.

## Building

The compiler output wraps. A diagnostic's place in the scratchpad's own file is a link:
clicking it puts the editor's cursor on that line and column, clamped to the text as it is
now. A place in another file is plain text.

## Running

Run starts the built program. Its output streams line by line, stdout and stderr told apart by
colour, and scrolls sideways rather than wrapping. Stop kills it and everything it started; a
rebuild and closing the window also end a run; an edit does not. Past a cap the oldest lines
are dropped and the pane says how many; a line with no newline is cut.

The output stays at the bottom while the reader is there. Scrolling away releases the follow,
and scrolling back to the bottom arms it again.
