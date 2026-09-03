# Storage

**A missing or unreadable file counts as empty.**

## Settings

The user's fonts and theme, saved apart from any project.

## Projects

**Project data and session data are saved in separate files.**

- **Project data**, what the user set: name, directory, binaries.
- **Session data**, what the app recorded: tabs and their history, source files, the shown file,
  the selection, the visit history, scroll positions, a hash of each binary.

**Project data is saved as soon as it changes**; session data every thirty seconds and on close.
A change to the binaries saves both, so a session never refers to a binary the project no
longer has. A broken session loses scroll positions, never the binaries.
