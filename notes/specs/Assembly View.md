# Assembly View

## Symbol bar

A bar over the pane shows the demangled name of the symbol shown, over its mangled name, each
on one line, cut with an ellipsis. Clicking a name copies it. For an object's code the bar
names the object.

A triangle opens the bar into the symbol's section, address, declared size and extent; for an
object, its format, symbol count and path. Open or shut is kept per tab and not saved.

## Operands

A symbol named in an operand is a link. Clicking it opens that symbol (`Tabs.md`).
A rip-relative operand keeps its `rip+` before the link: `mov dword ptr [rip+<target>], 7`.
In a linked image, a direct call to the start of a function is a link to it; a call into the
middle of a function keeps its address.

A jump or call target is written without leading zeros: `jle short 4Bh`.
A branch target inside the symbol is a link: clicking it scrolls to the target row and selects
it, opening nothing and adding nothing to the history.
A call or jump target that names no symbol keeps its address. Ctrl+click on it opens the
object's code in a new tab at that address, on the row at or below it, with the caret there;
"Show in unified view" and "Open as symbol" put the caret on their instruction the same way.

## Undecodable architectures

An architecture with no disassembler is named in the pane instead of a listing: "No
disassembler for aarch64".

## Blocks

A separator row sits before each row a branch lands on, so the listing reads as basic blocks.
Branch lines cross it unbroken, and a drag across it is not cut.

## Branch arrows

A gutter left of the addresses draws each branch that stays inside the symbol as a line from
its row to its target's, with an arrowhead at the target, shorter branches nested inside longer
ones. At most five lanes, and only as many as the symbol needs; past five, the outermost lane
is shared. The selected rows' branches are drawn darker. Strokes sit on whole device pixels.

## Object code

Clicking an object in the Objects panel opens its code as one listing, beside the symbol view
and not in place of it: every code section in the object's own layout, a label where each
symbol starts, and the bytes no symbol claims shown as bytes, never decoded. Rows are decoded
in windows around the reader; until a stretch is decoded its rows are estimated from its
bytes. The place the tab keeps is an address.
