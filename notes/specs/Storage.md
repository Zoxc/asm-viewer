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

A project is kept in a file the user chose or in the app's own storage. One in a file is named
by it; one in app storage is called an unsaved project, with a number. Changing the directory a
project is about does not move it.

Opening files with no project open makes an unsaved project, and there can be many.

Project data and session data are saved in separate files.

- **Project data**, what the user set: the id, directory, binaries, bookmarks, the build
  profile, and the language server to read it with.
- **Session data**, what the app recorded: tabs and their history, source files, the shown file,
  the visit history, a hash of each binary, whether the directory has been agreed to, and the
  UI state.

Session data, and anything else the app keeps about a project, sits beside the project data and
carries the project's id: a large random number, never shown. One with another id is ignored.

Project data is saved as soon as it changes; session data every thirty seconds and on close.
A change to the binaries saves both, so a session never refers to a binary the project no
longer has. A broken session loses scroll positions, never the binaries.

The session is restored when the project is opened. On startup that is the project the app was
last in, or none if it was left with none, unless it was given a project file to open. Saved
scroll positions are clamped to what the tab holds now. A place in a binary that no longer
resolves is dropped. A source file no longer on disk still comes back, its pane saying so.

## A project in a file

A project kept in a file can sit beside the code it is about and be shared or checked in. The
file ends in `.avproj`, and the app's files for that project are named after it, so one ignore
rule covers them.

Save as puts a project in a file: it is copied there with the app's files for it, under a new
id. The app is then in the copy, and the original is left as it was.

An unsaved project has Save instead, which moves rather than copies: the id stays the same, and
nothing is left in app storage.

Only the project file holds what the user set, so it changes only when they do. The files
beside it are the app's, found by its name.

A path in the project file is relative where it is under the file's directory and absolute
where it is not, so a project checked in beside its code opens on another machine.

The project file is the user's and is never moved aside: one that will not parse does not open,
and the app says so rather than writing over it. A file beside it that will not parse is moved
aside like anything else the app stores.

Nothing is locked: two apps open on one project both write it, and the last write wins. A
change made to the file while the app holds the project is lost the next time it writes.

### Opening one

A project file is opened from the menu at the top left, from the context menu of a Files item,
or by naming it on the command line. One project is open at a time, so opening another closes
the one before it. A path on the command line that is not a project file is reported there, and
the app closes.

## Closing a project

Closing a project saves it, closes its binaries and tabs, and stops the language server,
leaving the app with none. The project is left where it is and stays in the recent list, saved
or not.

An unsaved project can also be deleted, which closes it and removes it from app storage. The
app asks first.

## Restoring a binary

A hash of each binary file is saved. If it differs on restore, a saved place is found by
symbol name, the address only breaking a tie between two symbols of that name, and the tab's
scroll position is dropped. Nothing is refused or asked. A binary with no saved hash counts as
unchanged.
