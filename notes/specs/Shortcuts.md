# Shortcuts

The page listing every key and every mouse gesture the app answers to.
It is one of the app's pages, and is there with no project open.

## The list

A section per place a gesture applies, headed by that place.
The gestures that work anywhere come first.

A row is what the gesture does on the left, and the gesture on the right.
A gesture is written as it is pressed: `Ctrl+P`, `Shift+click`, `Double-click`, `Wheel`.
A key and a click that do the same thing are one row.
A motion key's row says that Shift extends the selection, rather than a row for each.
A gesture that means different things in two of the places says both.

## The filter

A box at the top matches the gesture and what it does.
A section with no row left is not drawn.

## What is not listed

Ordinary text editing in a box: typing, the caret keys, copy and paste.
The gesture that reveals the Debug page.

## Implementation notes

The list is written by hand; nothing checks it against the handlers.
