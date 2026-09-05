# Project View

A tab showing the open project: the directory it is about, where it is kept, and each open
binary with the number of objects it gave. The directory can be typed or chosen with a folder
picker, and a change to it is saved at once.

Where it is kept is the file it is in, or the app's own storage.

## Building

A project whose directory holds a cargo manifest can be built from the tab. With no directory,
or no manifest in it, the tab says so; that is not an error.

Build runs cargo over the workspace. The profile, debug or release, is chosen here and saved with
the project; a project starts on release. One build runs at a time: while it does, the tab says
so and Build cannot be pressed again.

A profile that keeps no debug lines builds binaries with no source side. The tab says so and
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
