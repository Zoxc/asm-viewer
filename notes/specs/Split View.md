# Split View

A document shows a symbol's source beside its assembly. The pane the tab is driven from is on
the left: the assembly in an assembly-driven tab, the source in a source-driven one. The split
between them can be dragged; its width is the left pane's, so it stays where it was when the
kind of tab changes. Both panes, and an object's code, scroll sideways as far as the widest
row drawn.

A tab first opens with the source pane on the symbol's own lines, three rows above its first
line; a symbol the debug info places nowhere opens at the top of the file. A position the tab
was left at wins over both.

## Mapping between the panes

Rows selected on either side light up the rows they map to on the other. Clicking an
instruction scrolls the source pane to the line it was compiled from; clicking a source line
scrolls the assembly to the first instruction compiled from it. Neither opens a document or
adds to the history.

In a source-driven tab, clicking a line finds its symbol across every open object, the symbol
on screen first and then the most recently visited, and puts the assembly side on it. A line
no object holds code from leaves the listing as it is and lights nothing. A source-driven tab
outlives its binary being closed. In an assembly-driven tab the source side only scrolls
within the symbol.

## Selection

A drag selects text; Shift+click extends the selection to the clicked place. A drag in the
gutter selects whole rows. Two clicks select a word, three the row's text. Ctrl+A selects the
whole listing and Escape drops the selection. Each pane has its own. Ctrl+C copies the
selected text of the pane with keyboard focus: the assembly as drawn, address and instruction
with symbol names, and the source as the file's own text.

A drag past any edge of the pane scrolls it towards the pointer while the button is held,
extending the selection to what comes into view.

## Highlighting

Assembly is coloured by span kind; source by its language's grammar, in the assembly's
colours.
