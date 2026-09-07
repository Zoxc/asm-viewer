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

**One press is one batch of events, against the tree measured before any of them ran.**
Every event a mouse-up produces -- the targeted press, and `GlobalPointerPress` for every
listener -- is emitted in one loop with no re-render between them
(`ragnarok-0.4.3/src/executor.rs:70-101`), so a handler that takes something away is
followed by handlers on nodes that the next render will unmount. A `Writable` mapped by a
key is read *then*, after the render-time guard that justified it. **Cost:** deleting the
shown scratchpad let go of its buffer, and the editor's own `on_global_pointer_press`
(`freya-code-editor-0.4.3/src/editor_ui.rs:255-263`) then indexed the table for it and
crashed the app; `PadBuffers`'s index is total (`ui/pad.rs`), and
`confirming_a_delete_does_not_crash_the_editor_it_takes_the_buffer_from` pins it.

**Two `Writable`s are always equal.** `PartialEq for Writable` returns `true` whatever it is
handed (`lifecycle/writable.rs:60-64`), and props are diffed with `PartialEq`, so a component
holding one mapped onto part of a table (`Writable::map`, `:152-172`) is never told the map
now points at a different part: `run_scope` finds the props unchanged and keeps the old ones
(`runner.rs:812-816`). The component goes on reading the part it mounted with, and there is no
prop to change that would say otherwise. **Cost:** the scratchpad's editor draws its rows from
a `Writable` mapped by the shown pad's id, so switching to a pad already read left the rows
drawing the pad that was left, and deleting *that* pad left them drawing the table's spare
empty buffer -- where `SyntaxBlocks::get_line`'s `self.blocks.get(&line).unwrap()`
(`freya-code-editor-0.4.3/src/syntax.rs:98`) panics for any line at all, inside freya and out
of reach. `SourceEditor` is keyed by its pad so a change remounts the editor and its rows
(`ui/pad_view.rs`); `coming_back_to_a_pad_already_read_draws_its_own_buffer` and
`deleting_a_pad_that_is_not_shown_leaves_the_editor_standing` pin the two halves. Worth
reporting twice over: props holding a `Writable` cannot be diffed, and a line the blocks do
not have should not be an unwrap.

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

## A scroll view corrects an offset past its end and leaves the controller holding it

`ScrollController::scroll_to_y` writes the number it is given and clamps nothing
(`use_scroll_controller.rs:243-245`). The correction is the view's, made as it draws:
`get_corrected_scroll_position` (`shared.rs:38-58`) pins the offset to
`-(content - viewport)`, and it is that corrected number the rows are laid out at
(`virtual_scrollview.rs:546-560`) while the controller keeps what it was told. Four
handlers write a corrected number back -- the wheel (`:384-404`), the content drag
(`:412-450`), the scrollbar (`:455-475`) and the arrow keys (`:483-513`) -- and until the
reader reaches one of them, reading the controller gives a place the rows are not at. The
key handler is itself a producer: `handle_key_event`'s `End` writes `-inner_height` and
not `-(inner_height - viewport)` (`shared.rs:180-186`), so freya over-scrolls its own
controller by a viewport.

What it cost here: `reveal_row` scrolls to `row - CONTEXT_ROWS` and a kept place is put
back at `row * code_row_height()`, both past the end for a row in the last screenful.
Either left the controller up to a screenful below the rows, and the sweep beyond the rows
turns an offset into a row: every point inside the pane then read as a row past the last of
the listing, so a sweep from anywhere in it picked out everything down to the end of the
file with the pointer in the middle of the text (`src/chars.rs`, `beyond`).

**Cost:** `scroll_extent` (`src/ui/list_box.rs`) is the one statement of how far a code
listing goes. Everything that scrolls one clamps to it before writing, everything that
reads a scroll back as a row clamps to it after (`Listing::scrolled`, `reveal_row`,
`reveal_caret`), and `use_kept_position` puts the controller back inside the listing where
it finds it outside -- which covers the writers above this app, `End` included, and the one
restore it makes itself before the pane has been measured and has an extent to speak of.
`a_sweep_reads_the_scroll_the_rows_are_drawn_at` pins the sweep's half. The one writer left
unclamped is the place-keeper for an object's whole code (`src/ui/section_view.rs`), which
works in `f64` for the reason the section below gives and would need an extent of its own;
what it can leave behind is a screenful at the very bottom of a binary, which every reader
now clamps away. Not reported.

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

## A child of a `MenuItem` cannot fill the row, and asking takes the window

A `MenuItem` is `min_width(105)`, `width(fill_minimum)`, `content(fit())`
(`menu.rs:438-440`), inside a `MenuContainer` whose two rects are `content(fit())` as well
(`:248`, `:269`) -- the idiom that makes every row of a menu as wide as its widest. What it
does **not** do is let a row's own child have that width: a `fill` or a `fill_minimum` child
resolves against the available area, which is the overlay the container is drawn in, so the
child comes out the width of the **window** and drags the menu out to it. Measured with an
arrow put at the end of one row: the row came back 452 px wide inside a menu whose other rows
were 233, with the arrow at x=474 in a 500 px window. Nothing in a row can learn what the
widest row made the menu, there being no size to read and no second pass to read it in.

**Cost:** the submenu arrow sits after the name with a gap rather than out at the row's own
end, where a desktop menu puts it (`submenu_label`, `src/ui/strip.rs`; the feature it
substitutes for is under **Wanted**). Not reported yet.

## A `ScrollView` inside a box sized from its content hangs the app

`ScrollView` is `width: fill, height: fill` by default (`scrollview.rs:96-98`), and a `fill`
child of a parent whose height is `Inner` is the two asking each other how tall they are.
The app never settles: the hover box, whose height is the answer it holds, drew nothing at
all with one inside it, and a test around it ran for four minutes without finishing rather
than failing.

Sizing the view from its content instead -- `height(Size::auto())` with a `max_height` --
lays out, and does not scroll: torin clamps what a capped node reports holding, so
`inner_sizes.height` comes back equal to the area's, `get_scroll_position_from_wheel`
(`scrollview.rs:272-277`) sees nothing to scroll past, and the wheel does nothing. Measured
in the tree: every node inside a 174px-tall box reported `inner 174`, with 80 paragraphs in
it.

**Cost:** the hover box measures its own answer (`src/ui/hover_view.rs`). The content is
drawn once at the full height the box may have, a rect around it reports what that came to
through `on_sized`, and the view is given that height or the limit, whichever is less --
two passes, where a scroll view that could size itself would need none.
`a_long_answer_is_capped_and_scrolls_inside_the_box` and `a_short_answer_makes_a_short_box`
pin both directions. Not reported yet.

## The release build's own panic hook is fatal, and catches what the app catches on purpose

`freya-winit 0.4.3`, `src/lib.rs:62`: `launch` installs a panic hook of its own, under
`#[cfg(all(not(debug_assertions), not(target_os = "android")))]`, that shows an `rfd` box
titled "Fatal Error" holding `panic_info.to_string()`, calls the hook it replaced, and then
`std::process::exit(1)`. Three things follow. It is a **release-only** behaviour, so the app
says something in one build and nothing in the other. It **exits**, so nothing the app would
like to save on the way down gets a chance. And it fires for **every** panic, including one
the app catches on purpose: `analysis` guards a demangler let loose on a name out of a string
table (`analysis::guard`), and in a release build that guard used to put up "Fatal Error" and
kill the app over a name it had already decided to do without.

What it cost here: the app's own hook has to be installed from `ui::app`'s first render rather
than from `main`, since a hook set before `launch` becomes the *inner* one and freya's box is
shown before it runs. Installed there it replaces freya's -- `take_hook` and never call it --
so `src/panics.rs` is what the reader sees in both builds, and a guarded panic goes back to
being written down and nothing else. The window between `launch` and that first render is
still freya's. Not reported yet.

**A `Popup` that is closing goes on swallowing presses for the length of its fade.** The
overlay is mounted while `show || background_animation.is_running()` (`popup.rs:207`), and the
background animation is the same 150 ms colour run played in reverse on the way out
(`popup.rs:185`), so the tree survives the close by that long. What survives with it is
`PopupBackground`'s first child: a `Position::new_global()` rect of the whole window, at
`Layer::Overlay`, carrying the `on_press` that asks the popup to close (`popup.rs:56-63`).
Nothing makes it `interactive(false)` while it fades, so for those 150 ms every press lands on
a full-window rect that is on its way to being invisible. In the app, a right-click on a
scratchpad row within about a sixth of a second of dismissing the delete question does nothing
at all. **What it cost:** a headless test that opens the question twice cannot get at the row's
menu the second time, so `confirming_a_delete_does_not_crash_the_editor_it_takes_the_buffer_from`
writes `Pads::confirming` directly and leaves the menu itself to
`a_delete_is_asked_for_before_anything_goes`. There is no workaround in the app: the rect is
inside freya's own component and nothing above it can reach the flag. Not reported yet; the fix
upstream is one `interactive(false)` on the closing frames.

## A dock's group sizes are recomputed on every render and cannot be read

`DockingArea` builds each `DockNode::Split` as a `ResizableContainer` whose panels are an
even `100/n` share, and passes **no controller** (`docking.rs:394-411`), so the context those
panels register into is the anonymous one the container makes for itself
(`resizable_container.rs:284-299`) and nothing outside can reach it. `DockNode::Split` has no
field for a size either, so a group dragged taller is not in the model that *is* readable:
a drag, a tab switch and a split all write straight back into the app's own `State<DockArea>`
(`DockingArea::new` takes it as a `Writable`), and the sizes are the one thing that does not.
**Cost:** the session saves which panels are in which groups, their order and each group's
showing tab, and cannot save how tall the reader made one (`src/ui/dock.rs`,
`agents/Persistence.md`). The two widths the app does save -- the sidebar's and the
document's split -- are its own `ResizableContainer`s, where it passes a controller and can.
A `.controller(..)` on the containers `render_node` builds, or a size on `DockNode::Split`,
would do it.

## A hook-order error names the innocent hook and blames the wrong rule

`use_hook` reads the value at the scope's `current_value` and `downcast_ref`s it to the type
this call expects; where it is a different type the `expect` fails with `HOOKS_ERROR`
(`lifecycle/base.rs:87`), a page of prose about calling hooks conditionally and in for-loops.
Two things about that message are wrong in the common case. **The rule it names need not be
the one broken**: a hook called from inside another hook's closure, or from an event handler,
shifts the order the same way and is a different rule, the third on its own list. And **the
frame it panics in is the hook after the extra ones**, not the one that took the slots -- so
the backtrace names a hook that is correct, in a file nobody has touched, and the offending
call is not in the capture at all: it ran on an *earlier* render.

**Cost:** a crash here reported `use_restore_on_startup` and had nothing to do with it; what
took the slots was `restore_ui` reaching for three contexts through `use_consume` from inside
`use_hook`'s closure, one render earlier, and only where the saved session held an `[ui]`
table. Found by reading which contexts the restore touched, not from the message. The app
consumes contexts in the component and hands the states down (`Arrangement`,
`src/ui/state.rs`), and `AGENTS.md`'s UI gotchas carry the rule. Naming the hook's index, or
the type found against the type expected, would have said it outright.

## An ellipsis costs a text its measured width

A `label` or a `paragraph` of one line is laid out at `f32::MAX` and measures
`longest_line()` -- its natural width -- but **only** with `max_lines(1)`, the default
alignment and no ellipsis; ask for `TextOverflow::Ellipsis` and it is laid out at
`area_size.width + 1.0` instead, so what it measures is the ellipsised line, which is the
box again (`freya-core-0.4.3/src/elements/label.rs:262-280`, `paragraph.rs:306-333`).
Nothing outside torin sees the difference: `inner_sizes` on a text node is its padding and
nothing else, `should_measure_inner_children` being false for a label (`label.rs:288-290`).

So the code panes' horizontal scroll rests on those three conditions holding: a row reports
its content width through `inner_sizes` (`ui/width.rs`), which is the paragraph's natural
width only because the paragraph asks for no ellipsis and no alignment. **A
`.text_overflow(..)` added to a code row would collapse that to the pane's own width**, and
the pane would scroll over nothing, with nothing failing anywhere else.

## Wanted

**A `SubMenu` that says it is one.** It renders a `MenuItem` around `rect().horizontal()`
and the label it was given, and nothing else (`menu.rs:600-603`): no arrow, no marker of any
kind, so a row that opens a list looks exactly like a row that acts. What the app does
instead: `submenu_label` (`src/ui/strip.rs`) draws the arrow itself -- the glyph the Files
tree folds with -- on the live "Open recent" row and on the dim one that stands in for it.
It goes after the name and not at the row's end, which is a second thing the crate lacks:
see **A child of a `MenuItem` cannot fill the row** above.

**A scrollbar that takes its own room instead of lying over the content.** `ScrollBar` is
`Position::new_absolute()` on layer 999, offset back 16 px into the very content it scrolls
(`scrollbar.rs:103-109`, `:69-90`). torin leaves an absolute child out of both the flow and
its parent's content size (`torin/src/measure.rs:913`), so the wrapper rect beside the
content measures zero and the content stays `Size::fill()` of the whole width
(`shared.rs:61-67`): nothing anywhere subtracts the bar's thickness. The last 16 px of every
row is under it. It is invisible while idle and unmounts 800 ms after the last movement, so
nothing looks wrong until the pointer nears that edge and an opaque bar appears over the
text. `show_scrollbar` is the only prop and it is off-or-over, not gutter-or-over; the
thickness is hard-coded (12 idle, 16 hovered) and the `size` field its theme defines is
never read. What the app does instead: the tab bar turns its scrollbar off -- one row cannot
spare 16 px of its height (`src/ui/strip.rs`) -- and every other pane lets the bar cover its
trailing edge. A gutter of the app's own is a goal (`notes/Goals.md`).

**Markdown code blocks that follow the app's own syntax colours.**
`freya-markdown`'s `code-editor` feature draws a fenced block with the `CodeEditor`
component, which is the highlighter this app already colours its source pane with -- but
`CodeBlockEditor` builds a `CodeEditorData` and never calls `set_theme`
(`freya-markdown-0.4.3/src/code_editor.rs:60-70`), and that type starts at
`EditorSyntaxTheme::default()`, which is `light()` (`freya-code-editor-0.4.3/src/editor_data.rs:51`,
`editor_theme.rs:182-186`). The colours are baked into the blocks by the parse, so every
fenced block comes out in freya's light theme whatever the app's appearance is: in dark mode,
light-mode code over a dark ground. Nothing outside the crate can reach it -- `MarkdownViewer`'s
`theme` field is `pub(crate)`, `EditorSyntax` is declared `%[no_ext]` and is not a keyed
component theme, and the one code hook, `code_editor_language`, picks the grammar and not the
colours. What the app does instead: the feature is off, so the hover box's fenced blocks fall
back to plain monospace text, in the one code colour the theme entry *can* set
(`markdown_viewer`, `src/ui/palette.rs`). An `EditorSyntaxTheme` on `CodeBlockEditor`, or a
`CodeEditorData` that read one from the theme sheet, would do it.

**A `Popup` that need not be centred down the window.** `PopupBackground` stacks two
window-sized global rects and `.center()`s the content in the second, and nothing on `Popup`
says otherwise, so a window pinned near the top of the screen -- an editor's quick-open, which
is where a reader typing a path is looking -- cannot use it. The file finder hand-rolls the
overlay layer, the press outside and the Escape key that `Popup` would have given it
(`ui/finder.rs`); `RescuedPopup`, which is content to be centred, still uses `Popup`. An
alignment on the background would do it.

**A `ScrollController` a view is handed from outside only reaches it by luck, and reading one
from an effect is a loop.** `ScrollController::new` keeps the position in states of its own and
hands out a `Callback` to read it (`use_scroll_controller.rs:150-170`); a read through that
callback subscribes nobody the view can be woken by, so `scroll_to_x` from another component
moves nothing until that view re-renders for some other reason -- and `scroll_to` (the only call
that pokes the notifier the view *does* read) can say no more than "the start" or "the end".
Meanwhile a write notifies every reader, and the callback's own `scroll.read()` counts as one, so
an effect that both reads the position and scrolls is woken by its own scroll for ever; the app hit
it as a hang, not as a wrong number. A third: the same view stops answering the wheel entirely
while a `DragZone` has a payload, measured headlessly as the same wheel moving the chips 400px
with no drag and not at all with one. **Cost:** the tab bar scrolls itself -- a clipped box, a row
at an `offset_x`, and the wheel, the reveal and the drag's edge all writing that one number
(`src/ui/strip.rs`). A code pane still uses a controller, where the pane re-renders on the same
change that scrolls it.

**Hit twice.** The second was the code panes, which the sentence above called safe: re-rendering on
the change is not the same as being bounded by it. `use_kept_position` reads the scroll and reveals
a landing's row, and the pane does not spend the landing -- `use_land` does, a pass or more later --
so an unspent landing was revealed again on every wake, and each reveal was the wake. It only ends
where the reveal can satisfy itself, which `reveal_row` cannot in a viewport too short to hold the
row and its `CONTEXT_ROWS`: the source pane of a tab opened from a door froze the app at 100% of a
core with one row drawn. Where the viewport is tall enough the same loop is silent, and shows only
as a pane that will not stay where the reader scrolls it. **Cost:** the hook remembers the landing
it has gone to, for as long as that landing is on its way (`src/ui/focus.rs`), pinned by
`a_landing_is_gone_to_once_and_does_not_drag_the_pane_back`. Anything else here that both reads a
scroll and writes one needs a bound of its own; re-rendering is not one.

**A dock group's tab bar that handles its own overflow.** `DockingArea` hands the bar
renderer the headers and their count and nothing else (`docking.rs:269-275`, `:548`), so a
group narrower than its headers lays them out at their natural widths and past its own right
edge: there is no scrolling, no elision and no overflow list, and a panel drawn out there is
one the reader cannot get at. Measured here: the four names of the sidebar's top group take
376px, and at the 300px it used to open at Locations was laid out from 294 to 368. **Cost:**
the sidebar opens at 380, what the widest default group needs (`src/ui.rs`); a reader who
drags it narrower is on their own. A `ScrollView` around `tab_children` is not the answer it
looks like: it does scroll (measured), but only under Shift or a horizontal wheel, the axes
swapping for `pressing_shift` alone (`scrollview.rs:259-268`), so a plain wheel over a 26px
bar does nothing -- and a `ScrollView` stops answering the wheel entirely while a `DragZone`
holds a payload (above), which is exactly when a panel is being dragged along the bar. The
app's own tab strip scrolls itself for that reason (`src/ui/strip.rs`); a dock's bars would
each need the same again. freya scrolling the headers itself, or a `TabBarContext` saying
which of them did not fit, would do it.

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
a pointer dragging a selection up past either pane's bar arms and shows the tooltips of
what it passed. The app makes them `interactive(false)` while a sweep is under way
(`sweeping`, `ui/marks.rs`), so they are not hit at all.

**A key event is emitted only for a focused node that listens for it.** A keyboard event
becomes one potential event, on the focused node, and `measure_emmitable_events` keeps it only
if that node is listening (`ragnarok/measurement.rs`); bubbling to the ancestors happens after
that, in the runner. So an `on_key_down` on a parent of the focused node never runs unless the
focused node has one too — which reads as the opposite of `does_bubble`. The app puts the
filter panes' Ctrl+F on the focusable rows themselves for this reason (`ui/filter_bar.rs`).

**An `Input` inserts a character it has no chord of its own for.** The editor's `Key::Character`
arm falls through to insertion whatever the modifiers are, so a chord it does not implement —
Ctrl+F — is typed in as an `f`. Declined in the filter bars' `on_pre_key_down` before the edit
(`ui/filter_bar.rs`). That hook replaces the `Input`'s default wholesale rather than composing
with it, so declining one chord means repeating the default for every other key, and a change
to freya's default is missed here.

**`SyntaxHighlighter::tree()`**, so the function spans the source rows' menu needs are not a
second parse of the file (above, and `ui/highlight.rs`).

**A grammar's highlights query compiled once per language.** `set_language` throws away the
configuration it holds and builds another (`syntax.rs:144-150`), and building one is a `Query::new`
over the grammar's whole query text plus a colour resolved for every capture name in it
(`lang_config`, `:427-441`). What it costs has nothing to do with the file: for Rust it is 80 ms of
the 121 ms a 23 KB file takes in a debug build, paid again for every file opened and again for all
of them when the theme changes. Nothing out here can hold the result -- `Query` is not `Clone`
(`tree-sitter-0.26.13/binding_rust/lib.rs:322-326`), and the capture colours are resolved against
the one query, so what would be kept is a config per language *and* appearance, and only the
highlighter can keep it. **Cost:** paid, once per file. The parse is a worker's
(`ui/highlight.rs`), so it is a wait before the file draws and not a freeze; the way round it is to
drop the highlighter for tree-sitter itself, which is writing the component again. A cache inside
it, keyed by language and theme, or a `LangConfig` the caller makes once and hands in, would do it.

**A mark of our own in a `CodeEditor` gutter.** Its gutter is the line number and nothing may
join it: every row is an `EditorLineUI` built inside `CodeEditor::render`
(`editor_ui.rs:279-296`) with `pub(crate)` fields (`editor_line.rs:21-32`), and the gutter it
draws is one label of the number in a box `font_size * 5` wide (`:60`, `:127-140`); the
builder's `gutter(bool)` turns that on and off and says nothing else (`editor_ui.rs:65-114`).
**Cost:** the scratchpad's editor is that component, so a pad's lines carry no dot for the ones
that produced code -- the round mark the Source pane's gutter draws in `compiled_fg`
(`code_mark`, `ui/parts.rs`; `agents/Panes.md`) -- and a reader finds out that a line compiled
to something by putting the cursor on it and watching the listing beside it light. A slot beside
the number, or public fields on `EditorLineUI`, would do it.

**A background for a set of `CodeEditor` lines.** The one background a line gets is
`line_selected_background`, painted for the cursor's row alone and only while nothing is
selected (`editor_line.rs:116-121`); there is no per-line colour on the builder
(`editor_ui.rs:65-114`) and `EditorLineUI`'s fields are `pub(crate)` (`:21-32`). **Cost:** the
scratchpad's editor cannot light the pair -- the lines an instruction was compiled from, green
on the other side of a split (`agents/Panes.md`) -- so the pad's two panes point at each other
one way only: the cursor's line lights the instructions it compiled into, and nothing comes
back (`src/ui/pad_view.rs`). A per-line background, or public fields on `EditorLineUI`, would
do it.

**A `CodeEditor` that can be scrolled to a line.** Its scroll is `CodeEditorData::scrolls`,
`pub(crate)` (`editor_data.rs:33`); the controller is made inside a `use_hook`
(`editor_ui.rs:139-168`) with no `new_controlled` to hand one in; and nothing in the crate
moves it but PageUp and PageDown (`:207-222`), not even to its own cursor. **Cost:** the deferred
diagnostic jump (`notes/Goals.md`): a pressed diagnostic puts the cursor on the line it names
and leaves the pane where it was, so an error below the fold is named and not shown. It is also
half of why the scratchpad's listing follows the editor and nothing goes the other way -- an
instruction that named a line could neither light it nor bring it into view. An overlay of ours
is no way round it: it cannot read the scroll it would have to follow. A `ScrollController` the
editor accepts, or a scroll to its own cursor, would do it.

**A text that says it did not fit.** There is no truncation flag and no event for one:
`SizedEventData` is `area`, `visible_area` and `inner_sizes` and nothing more, and the
natural width is gone before anything outside torin can see it wherever a text asks for an
ellipsis (above). What the app does instead: a row's name is a `paragraph`, whose
`ParagraphHolder` hands back skia's own paragraph, and that answers `did_exceed_max_lines()`
off the layout freya already did -- measured headlessly in a 100px box as `false` for `ab`
and `true` for a name whose `max_intrinsic_width` was 422px (`Fitted`, `ui/parts.rs`).
**Cost:** a name is a paragraph and not a label, which is one `Rc<RefCell<..>>` and one
`on_sized` per row and the accessibility role of a paragraph; and the answer is a render
behind the measurement, so a row is drawn once before it knows. 0.5.0-rc.4 changes none of
it -- the same three fields, and `Label` still has no holder. A flag on `SizedEventData`, or
a `Label::holder`, would do it.

**A paragraph that answers in bytes.** Its hit test and its highlight both speak UTF-16 code
units (`caret_col`, `highlights`), skia's own unit, so a column of a drawn row is a UTF-16
unit and cannot be anything else. **Cost:** the app counts a column in bytes everywhere else
-- a language server is asked in them, a line is indexed by them -- so the source pane
converts each way as it draws and as it is pressed, and a followed definition reads the line
it lands on to count its caret's column (`src/ui/follow.rs`). The conversion is one function
each way (`src/chars.rs`) and the cost is a walk of a short line, but it is a walk that a
byte-offset hit test would remove.
