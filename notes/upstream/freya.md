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

**`on_secondary_down` is `on_pointer_down` under another name** (`extensions.rs:358-372`:
it installs a `pointer_down` handler that forwards the right button), and an element keeps
one handler per event, so an element given both keeps whichever was set last. The two code
panes' rows set both -- the left button's down starts the picked-out run, the right's opens
the menu -- and the row's run silently never started. **Cost:** one `on_pointer_down`
doing both, the right button mapped by `secondary` (`ui/marks.rs`);
`picking_out_a_row_below_a_separator_lights_that_rows_own_branch` presses a row and would
have caught it.

**A bubbling event is measured once, against its deepest listener.** `pointer_down`, the
press and the other events that bubble are emitted once (`ragnarok-0.4.3/src/measurement.rs:170`)
with `element_location` taken from the deepest listening node, and every ancestor's handler is
re-dispatched that same data (`freya-core-0.4.3/src/runner.rs:320-354`), so an ancestor's
`element_location()` is relative to whichever descendant listened. Non-bubbling events
(`pointer_move`, `pointer_over`, the globals) are measured per listener. **Cost:** nothing
inside a code row listens to `pointer_down` -- the links listen to the press -- so the row's
handler can turn the location into a column (`ui/code_row.rs`);
`a_link_in_the_text_is_one_unit_and_still_opens_its_symbol` presses a link and would catch a
child that started listening.

**A drag goes on outside the window, and the global press that ends one is cancellable.**
Two facts about a held button. The good one: `freya-winit` forwards every `CursorMoved`
and ignores `CursorLeft` while a button is down (`renderer.rs:1024-1046`), winit forwards
the platform's motion unfiltered, and both Wayland and X11 keep reporting the pointer to
the surface a button went down on wherever it goes -- so a `on_global_pointer_move`,
which ragnarok sends to every listener without hit-testing
(`ragnarok-0.4.3/src/measurement.rs:35-47`), sees a sweep leave the rows, the pane and the
window. The bad one: `on_global_pointer_press` is among the events a handler's
`prevent_default` cancels (`name.rs:192-218`, `ragnarok-0.4.3/src/executor.rs:74-81`),
and freya's own scrollbar thumb prevents it unconditionally in its press
(`scrollthumb.rs:64-69`), as does `VirtualScrollView` while its scrollbar is held
(`virtual_scrollview.rs:360-364`) -- so a sweep let go of over the thumb, which appears
under a pointer moving toward the pane's edge, never ended and the run followed the bare
pointer from then on. **Cost:** the sweep beyond the rows is `on_sweep_beyond`
(`ui/code_row.rs`), and the release is the root's `on_capture_global_pointer_press`, the
capture phase running before anything can cancel it (`ui.rs`).

**`pointer_over` fires on entry only.** Its doc says it fires when the pointer is over the
element; `nodes_state.rs:163` dedups it against the hovered set, so it is `pointer_enter`
that also fires for the ancestors. **Cost:** the sweep that picks characters out follows the
pointer with `pointer_move` (`ui/code_row.rs`).

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

## torin sizes an auto-width node from its minimum plus its children

`torin 0.4.3`, `measure.rs`: a node's area starts as `min_max(padding, …, minimum_width, …)`
(`:192`), so under a `min_width` it starts *at the minimum*; a horizontal parent whose width
is `Inner` then adds every child's width to that (`stack_child`, `:1123`: `parent_area.size.width
+= child_area.size.width`), and the `min_max` re-applied afterwards (`:381`) floors a sum that
is already past the floor. A `rect().horizontal().width(Size::auto()).min_width(Size::px(290.))`
holding 107px of labels comes out 397px wide, not 290. A measurer node (a label) is not
affected: its `min_max` runs over the measured size (`:258`). What it cost here: the code
panes' rows, which wanted to be "their content, but never narrower than the pane", are given
that as a `Size::Fn` **width** instead and report their content through `on_sized`'s
`inner_sizes` (`src/ui/width.rs`), which costs a wide row one layout before it is drawn whole.
`a_picked_rows_wash_runs_as_wide_as_the_widest_row` would catch the minimum coming back. Not
reported yet.

## Wanted

**A pointer release nothing can cancel.** There is no `on_global_pointer_up`; a release is
`on_global_pointer_press`, which any handler's `prevent_default` on the way cancels, and
freya's own scrollbar thumb does (above). The app ends a sweep on the capture-phase press
instead (`ui.rs`), which happens to run first; a plain "the button came up" event would say
what is meant.

**A highlight and a caret the size of the line box, on whole pixels.** A paragraph's
`highlights` are painted as the glyphs' tight boxes, stretched by `CursorMode::Expanded` to
the paragraph's area but never wider than the glyphs, and its `cursor_index` is drawn two
pixels wide at the glyph's fractional edge; between one row's highlight and the next's there
is a seam wherever the line's fonts or a placeholder make the line taller than a run. The
code panes draw both marks themselves as rects of the row on the device pixel grid
(`ui/code_row.rs`), reading the columns' x off the `ParagraphHolder` -- which is the one
thing that could not be done without the engine, and which works.

**One inline child is one unit, in writing.** `paragraph().child(..)` reserves a placeholder
that skia counts as one UTF-16 unit of the text (U+FFFC), which is what makes a link inside a
row selectable as a whole; nothing in freya's docs says so, and the registry cannot show
skia's source, so the app pins it with a test
(`a_link_in_the_text_is_one_unit_and_still_opens_its_symbol`).

**A tooltip that does not arm under a held button.** `TooltipContainer` arms its timer on
`pointer_over` and disarms on `pointer_out` and on nothing else (`tooltip.rs:204-216`), so
a pointer dragging a selection up past the companion header or the symbol bar arms and
shows their tooltips. The app makes the two `interactive(false)` while a sweep is under
way (`sweeping`, `ui/marks.rs`), so they are not hit at all.

**`SyntaxHighlighter::tree()`**, so the function spans the source rows' menu needs are not a
second parse of the file (above, and `ui/highlight.rs`).
