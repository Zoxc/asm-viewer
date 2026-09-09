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

Each pane has its own selection, in grey. The rows on the other side that are the same place
are marked in green. Nothing responds to the pointer alone. Clicking an
instruction scrolls the source pane to the line it was compiled from; clicking a source line
scrolls the assembly to the first instruction compiled from it. Neither opens a document or
adds to the history.

In a source-driven tab, clicking a line finds its symbol across every open object, the symbol
on screen first and then the most recently visited, and puts the assembly side on it. A line
no object holds code from leaves the listing as it is and lights nothing. A source-driven tab
outlives its binary being closed. In an assembly-driven tab the source side only scrolls
within the symbol.

## Gutter marks

A line that produced code, in any open object, is marked with a dot at its left edge. So is an
instruction the debug info places on a source line, in any file.

## Opening the other pane

In the pane the tab is not driven from, a context menu opens that pane as a tab of its own, on
the instruction or line it was over.

## Putting a pane away

The bar over the leading pane has a control that toggles the other pane, as Ctrl+\ does;
whether it is away is kept for the tab.

## Following a name

While a language server is running, every name it can place is a link: a call, a type or a
trait, a field, a variant, a module, a local. A name where one is defined is not, nor a
built-in type, which it places nowhere. An item in a trait `impl` is the exception: it links
to the trait's own declaration. Under the pointer a link is underlined and lit, and the
pointer becomes a hand. With no server there are no links.

Clicking one asks the server where the name is defined and goes to the file and line it
names. The source pane lands on that line and the assembly side follows it, as it does a
clicked line. The tab shows that file, so one that was assembly-driven becomes source-driven.
Back returns. Ctrl+click opens the definition in a new tab.

Clicking a link does not select its line. One that answers nothing, or answers its own line,
does nothing.

A name's context menu — a link, or the name where it is defined — has three questions for the
server. "Go to definition" does what clicking a link does. "Find references to `foo`" and
"Find implementations" answer in the Locations panel. A question with no answer says so.

F12, Shift+F12 and Ctrl+F12 ask the same three about the name the cursor is on, and Alt+F12
lists every symbol its line was compiled into. A cursor on no name asks nothing, and only the
source pane answers.

## What a name is

While a language server is running, resting the pointer on a name in the source pane asks
what it is and shows the answer in a box.

The answer is the server's own: the name's path and signature as code, and the doc comment
above it.
Code in it is coloured as the source pane is, the rest as written, headings, lists, emphasis
and rules included.

The box sits at the name's left edge above its row, under it where it does not fit, and
inside the window.
The pointer can move into it, where an answer too tall for it scrolls.
It goes when the pointer leaves both it and the name, on a press or a key, and when the code
under it scrolls.
Nothing is shown while a menu is open or text is being selected.

## Selection

A drag selects text; Shift+click extends the selection to the clicked place. A drag in the
gutter selects whole rows.
With Alt held no link lights, the pointer over one stays the I-beam, and a press on it
selects instead of following it.
Two clicks select a word, three the row's text. Ctrl+A selects the whole listing and Escape
drops the selection. Each pane has its own. Ctrl+C copies the selected text of the pane with
keyboard focus: the assembly as drawn, address and instruction with symbol names, and the
source as the file's own text.

A drag past any edge of the pane scrolls it towards the pointer while the button is held,
extending the selection to what comes into view.

## Find in the pane

Ctrl+F opens a find bar along the bottom of the code pane the keyboard is in. It does not
overlay code. Each pane has its own. Selected text within one line becomes the search term; a
selection crossing lines leaves the box as it was.

The bar is a box, the filter bars' three toggles, a count like `3 of 128`, and a button each
way. A pattern that does not compile shows its error under the box, and one that matches
nothing says so.

Every match is marked. Enter goes to the next and Shift+Enter to the one before, wrapping at
the ends; the buttons do the same. Each scrolls the pane to the match and selects it, so Ctrl+C
copies it. F3 and Shift+F3 do the same with the focus still in the code.

Escape closes the bar and puts the focus back in the pane. What was typed lasts as long as the
tab is open.

### What is searched

What the pane draws, which is what copying from it copies. The source pane searches the file it
is showing, an assembly pane the symbol it is drawing.

An object's code is read a piece at a time, so it has no count. A step searches from where the
pane is until it finds a match, and wraps once before saying there is nothing to find. The bar
says how far it has got, and the app stays responsive. A new pattern gives up the search.

## Highlighting

Assembly is coloured by span kind; source by its language's grammar, in the assembly's
colours, with colours of their own for attributes, types, function names, comments and
strings.
