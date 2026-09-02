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

**A scope reused under a different component type panics on the downcast.** `From<T> for
Element` stores a render closure that downcasts the scope's props to `T`
(`element.rs:407`, `downcast_ref::<T>().unwrap()`), and `Runner::run_scope`
(`runner.rs:806-822`) swaps the props of an existing scope when the key or the props changed
but keeps the closure -- so props of another component type arriving at a scope's path panic
inside freya rather than remounting it. It arrives there because siblings are matched by key
alone (`path_element.rs:203-222`, first-in-first-out among equal keys) and a same-key child
whose index did not change is recorded as unmoved, while the removals, insertions and moves
around it are applied in that order: the node graph and the element tree then disagree at
that slot. Hit by the assembly listing's `SeparatorRow`s, which all carried the type's
default key, whenever the listing scrolled by at least a separator's distance -- three rows
or so, one wheel notch. **Cost:** every separator is keyed by the address of the row it opens
and tagged apart from the instruction rows' keys (`ui/assembly.rs`;
`scrolling_past_a_separator_keeps_every_row_its_own`). Worth reporting: `run_scope` should
remount on a key change rather than swap props under a stale closure, and the debug
duplicate-key check (`path_element.rs:180-197`) exempts exactly the default keys that cause
it.

## The scroll offset is an `f32`

`ScrollController`'s position is read back as an `i32` but is held and laid out as an `f32`, so
an offset past 2^24 pixels -- about sixteen million, some 670 000 rows of `code_row_height()` --
is rounded to the nearest few pixels. A listing of an object's whole code is estimated at a row
per four bytes before it is decoded, so a large binary's `.text` is millions of rows and its far
end is not addressable to the row. What it cost here: the place-keeping effect of
`src/ui/section_view.rs` once re-issued a move whenever the map and the view disagreed, and past
that point they always did, which was an effect waking itself for ever (`agents/UI.md`). The
effect now answers a written place once and does its own arithmetic in `f64`; the view itself
may still land a row off down there, which nothing above the framework can mend. Not reported:
it is an `f32` by design, and a virtual list this long is unusual.

## A key event's modifiers are the mask before the key, and the change itself never arrives

`freya-winit` stores `WindowEvent::ModifiersChanged` (`renderer.rs:709`) and hands the stored
mask to the next `KeyboardInput` (`:947`); nothing forwards the change to the app, and no mouse
or pointer event carries modifiers (`events/data.rs`). On Wayland the compositor sends a key and
then the modifiers, so a modifier's own press and release arrive over the mask as it was before
them. What it cost here: a Caps Lock KDE has made into Ctrl (`caps:ctrl_modifier`, which keeps
the keysym and adds a Control *action*, unlike `ctrl:nocaps`) names itself Caps Lock over a mask
without Ctrl on the way down and with Ctrl on the way up, so the app's Ctrl was never set by the
press and left set by the release. `ModifierKeys` (`src/ui/marks.rs`) learns such a Caps Lock from
its first release. The fix upstream is either forwarding `ModifiersChanged` as a global event or
carrying the current modifiers on pointer events. Not reported yet.
