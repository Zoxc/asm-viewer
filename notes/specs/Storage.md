# Storage

A missing or unreadable file counts as empty.

## A file that will not parse

A file that will not parse is moved aside before the app writes over it: the settings, the
recent projects, and a project's two files when it is opened. It keeps the path it had,
under an `incompatible` directory beside the settings. Nothing there is ever overwritten;
a name already taken gets a number in front of it. A window then names every path written.

## A panic

Every panic is written down, in a `panics` directory beside the settings: one file per run, a
record per panic, with what panicked, where, the message and the backtrace.

A panic the app is hardened against — a name it cannot demangle, debug information it cannot
read — is written down and nothing more; the app carries on. Any other panic is shown in a
window naming what panicked and where it was saved, with the backtrace a button away. The app
then saves what is open and closes.

## Settings

The user's fonts and theme, saved apart from any project.

## Projects

A project is identified by the directory it is stored in. Renaming it or changing its
directory does not move it. A project without a name is an anonymous project: opening files
with no project open makes one, and there can be many. A project in which nothing was opened
is not written.

Project data and session data are saved in separate files.

- **Project data**, what the user set: name, directory, binaries, bookmarks.
- **Session data**, what the app recorded: tabs and their history, source files, the shown file,
  the selection, the visit history, scroll positions, a hash of each binary.

Project data is saved as soon as it changes; session data every thirty seconds and on close.
A change to the binaries saves both, so a session never refers to a binary the project no
longer has. A broken session loses scroll positions, never the binaries.

The session is restored when the project is opened; on startup, the project last open is. Saved
scroll positions are clamped to what the tab holds now. A place in a binary that no longer
resolves is dropped. A source file no longer on disk still comes back, its pane saying so.

## Restoring a binary

A hash of each binary file is saved. If it differs on restore, a saved place is found by
symbol name, the address only breaking a tie between two symbols of that name, and the tab's
scroll position is dropped. Nothing is refused or asked. A binary with no saved hash counts as
unchanged.
