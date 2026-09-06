# The Window

## The top bar

Across the top of the window, left to right: the menu, the open project, a gap, the language
server's control, and the back and forward chevrons.

## The menu

At the very left. The ways in and out of a project, and under them Project, Settings and the
Scratchpad.

- "Open a project..." asks for a project file.
- "Open recent" lists the projects the app has been in, most recent first, each by its name;
  one whose file is gone is not listed. It is dim when there is nothing to list.
- "Open a directory as a project..." asks for a directory and starts a project about it.
- "Open a file as a project..." asks for a binary and starts a project holding it.

The last two start an unsaved project. Each of the four leaves the open project as it is.

Under them, "Save as...", which asks for a file and puts the project in it, and "Close
project", which closes whichever is open, saved or not. Save as is absent with no project
open; an unsaved project has Save in the bar instead.

## The open project

The bar names the project the app is in. Hovering says where it is kept. Pressing it shows the
Project view. A close button beside it closes the project; an unsaved project has Save and
Delete there instead.

## With no project open

The window is the top bar and a screen under it: no sidebar, no panes. The screen offers the
menu's ways to open a project, with the recent projects under them.

The bar keeps its menu, without Project, and the rest of it is dim. Settings and the
Scratchpad open as tabs, so the tab bar comes back for them and goes when the last one closes.

## Tooltips

Text too long for where it is drawn is cut with `…`; hovering shows the whole.
Text that fits has no tooltip.
Some tooltips say more than the text — a file's path where its name is drawn, a symbol's whole
name where two parts of it are, where a matched line is — and are always shown.
A button's tooltip says what the button does.
A tooltip over cut text appears at once; every other one waits.
