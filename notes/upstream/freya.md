# freya 0.4.3

**`MenuContainer` measures itself once and keeps that offset.** A menu that widens after
it is first laid out hangs off the side of the window (`menu.rs:236`). Seen with the
document overflow menu, whose tab list fills in from a worker. **Cost:** the menu is keyed
by its row count so a grown list remounts it (`ui/dock.rs`, `DocumentMenuButton`), and
`a_menu_open_while_the_list_grows_stays_on_the_edge` pins it. Its overflow correction is
also vertical-only and latches, so a `right(0.)` of ours plus the correction lands a whole
menu-width further left; the button is positioned by hand instead
(`the_tab_menu_hangs_from_the_buttons_right_edge`). Both in `agents/Headless.md`.

**`ContextMenuViewer` places the popup at the last global pointer move**, not at the
opening event's point (`context_menu.rs:151-153`), which is invisible in the app and bites
a headless test that never moved the pointer. **Cost:** the test moves it first
(`right_click` in `ui/tests.rs`; `agents/Headless.md`).

**`SyntaxHighlighter` keeps its `Tree` private** (`syntax.rs:120-125`), with no accessor,
so anything else wanted from the parse -- the function spans for C and C++ -- is a second
parse of the file (`ui/highlight.rs`). Not a bug; a gap worth a PR for a `tree()` getter.
