# Project View

A dockable view showing the open project: its name and directory, both editable, the id it is
stored under, and each open binary with the number of objects it gave. The directory has a
folder picker. A change to the name or directory is saved at once.

## Building

A project whose directory holds a cargo manifest can be built from the view. With no directory,
or no manifest in it, the view says so; that is not an error.

Build runs cargo over the workspace. The profile, debug or release, is chosen here and saved with
the project; a project starts on release. One build runs at a time: while it does, the view says
so and Build cannot be pressed again.

A profile that keeps no debug lines builds binaries with no source side. The view says so and
offers to add them: the button writes line tables into that profile in the workspace's manifest,
and the offer goes once they are there.

What the build produced is listed, one row per artifact cargo named, with the target it came from.
Clicking a row opens it as a binary, unless it is open already. A build replaces the artifacts of
the build before it that are still open; a binary opened any other way is left alone.

The compiler's output wraps, as the scratchpad's does. A build that succeeded keeps its warnings.
A build cargo refused before compiling shows what cargo said. A diagnostic's place in a file under
the project's directory is a link: clicking it opens that file at that line. A place elsewhere is
plain text.

The compiler's output is the last build's and is not saved. Which paths a build produced is
remembered, so a build after a restart still replaces them.

## Recent projects

A section of the view lists the other projects, most recent first; a project whose directory
is gone is not listed. Clicking one saves the open project, closes its binaries and tabs, and
opens the clicked one as it was left.
