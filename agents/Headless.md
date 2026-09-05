# Headless testing

`freya-testing` 0.4.3 runs the whole app (components, hooks, effects, layout, events) with no
window, no GPU and no event loop, on the test's own thread. It is a dev-dependency here for the
tests at the bottom of `src/ui.rs`, and it can do much more than those use it for. This note is what
it can and cannot be asked, checked against the sources and against throwaway tests rather than
against the crate's README.

The whole public surface is one file, `freya-testing-0.4.3/src/lib.rs`, 666 lines. Read it before
guessing at anything below.

## The runner

```rust
let (mut test, handles) = TestingRunner::new(
    app,                    // impl Into<AppComponent> -- a `fn() -> impl IntoElement` is one
    (200., 200.).into(),    // Size2D, the window
    |runner| { .. },        // setup, run in the ROOT scope; its return value is `handles`
    1.,                     // scale factor
);
```

`launch_test(app)` is the same thing at 500×500, scale 1, with no setup closure. Nothing here uses
it, because every harness needs contexts.

**The setup closure is how a test reaches the app's state.** It is handed the `Runner` and runs in
the root scope, so `runner.provide_root_context(|| Objects(State::create(Vec::new())))` puts a
context exactly where `app()` would have and hands back a copy the test body keeps. That is the only
door: a `State` cannot be created outside a runtime at all. `State::create(1i32)` outside one
panics, which is why `project_harness` mounts a runner to draw an empty rect.

The closure runs at `lib.rs:242`, *after* freya-testing has provided `ScreenReader`,
`RenderingTicker`, `AnimationClock`, `AssetCacher`, `Platform`, the clipboard and the accessibility
generator, and *before* the `FontCollection` (`lib.rs:252`) and the first `sync_and_update`
(`lib.rs:281`). Two things follow. A test can get at freya's own root states by re-providing them:
`runner.provide_root_context(Platform::get)` consumes the `Platform` already there and hands it
back, which is how `a_desktop_that_changes_its_mind_repaints_the_window` writes `preferred_theme`.
And the app has already rendered once by the time `new` returns, so the first
`test.sync_and_update()` in a test body is the second pass, not the first.

The `Platform` a test can drive this way is `focused_accessibility_id`, `root_size`, `scale_factor`,
`navigation_mode`, `preferred_theme` (mounted `Light`), `is_app_focused` (mounted `true`) and
`accent_color`. Its `sender` swallows `RequestRedraw` and `SetCursorIcon` (`lib.rs:213-228`), so a
cursor-icon change is not observable.

**The window cannot be resized.** `size` is a private field with no setter; a test that needs a
different window makes another runner.

**`scale_factor` scales the layout, not the coordinates you give it.** At scale 2 a `Size::px(100.)`
rect measures 200 in `node.layout().area`, and a press at `(150., 150.)` lands inside it. So the
window size, the measured areas and the cursor points are all in one unit, and `Size::px` is the odd
one out. Every test here uses `1.`, where the two coincide.

## Driving it

`sync_and_update()` is the pass. Everything below queues work and most of the input methods call it
for you; how many *more* you need is the subject of "Passes" below.

**Pointer.** `move_cursor`, `press_cursor`, `release_cursor`, `click_cursor` (a press and a
release), `press_touch`/`move_touch`/`release_touch`, and `scroll(cursor, delta)` for the wheel.
`move_cursor` is the only one that does *not* call `sync_and_update` itself.

**Keyboard.** `press_key(Key)` and `write_text(impl ToString)`. Both hardcode `Modifiers::default()`
and `Code::Unidentified` (`lib.rs:432-451`), so **neither can express Ctrl+C**, which is the app's
copy binding: `on_listing_key` reads `e.modifiers.contains(Modifiers::ctrl_or_meta())`. The escape
hatch is the public `send_event`:

```rust
test.send_event(PlatformEvent::Keyboard {
    name: KeyboardEventName::KeyDown,
    key: Key::Character("c".into()),
    code: Code::KeyC,
    modifiers: Modifiers::CONTROL,
});
test.sync_and_update();
```

Verified: that arrives as `ctrl=true code=KeyC`, where `press_key` arrives as
`ctrl=false code=Unidentified`.

**A right-click is `send_event` too**, since every cursor method hardcodes `MouseButton::Left`.
Three things go with it, all verified by `a_source_row_inside_a_function_offers_its_instances`
(`right_click` in `ui/tests.rs`). The harness must mount `ContextMenuViewer::new()` in an ancestor
scope, as `app()` does on its root: `ContextMenu::get()` panics without one. The popup is placed at
the last **global pointer move** (`context_menu.rs:151-153`), not at the event's point, so
`move_cursor` to the row and `sync_and_update` first. And send the `MouseUp` as well as the
`MouseDown`: the menu opens on the down, and the up of the same gesture is the one global press the
viewer swallows. Left out, the swallow takes the click on an entry instead and the menu stays open.

```rust
test.move_cursor(at);
test.sync_and_update();
for name in [MouseEventName::MouseDown, MouseEventName::MouseUp] {
    test.send_event(PlatformEvent::Mouse { name, cursor: at.into(), button: Some(MouseButton::Right) });
    test.sync_and_update();
}
```

`PlatformEvent` and `MouseEventName` are `freya::prelude::platform`'s, not the prelude's own.

**Keyboard events go to the focused node and nowhere else.** `send_event` passes
`self.accessibility.focused_node_id()` (`lib.rs:417`), and `measure_potential_events` matches a
location-less event against that node alone (`ragnarok-0.4.3/src/measurement.rs:79-92`). A key sent
before anything is focused reaches no handler. Focus is got the way the app gets it: press the pane,
which calls `a11y.request_focus()`, then `sync_and_update`. The request travels through
`UserEvent::FocusAccessibilityNode` into `requested_focus_strategy` and is applied at the top of the
*next* pass (`lib.rs:330-335`).

A **global** key handler is the exception: `measure_source_global_events` emits to every listener
of `GlobalKeyDown` with no focus check at all (`ragnarok-0.4.3/src/measurement.rs:17-47`), so the
root's one handler answers with nothing focused and while an `Input` holds the keyboard. What can
still swallow it is `prevent_default` on the plain `KeyDown` beside it, which cancels the global
one -- the filter boxes decline the chords they must not eat for that reason (`agents/Sidebar.md`).

**Time.** `poll(step, duration)` and `poll_n(step, times)` are `sync_and_update` in a loop with a
real `std::thread::sleep` and a rendering tick between iterations. They are the only thing that ever
sends a tick. `animation_clock()` hands back the `AnimationClock`, which only scales animation
*speed*; there is no virtual clock.

**Painting.** `render()` rasterises to a `SkData` PNG and `render_to_file(path)` writes it. Verified
working here. `launch_doc` is the same thing wrapped for freya's own doc screenshots. The background
is hardcoded `Color::WHITE` (`lib.rs:548`), so a dark-theme page renders over white wherever nothing
paints.

**Fonts.** `set_fonts(HashMap<&str, &[u8]>)` registers typefaces from bytes, and
`set_default_fonts(&[Cow<str>])` replaces the fallback chain and re-measures. Neither is used here;
see "Text" below for why they would matter if a test ever asserted a text width.

## Finding things

`find(matcher) -> Option<T>` and `find_many(matcher) -> Vec<T>` walk the tree pre-order
(`freya-core-0.4.3/src/tree.rs:134-142`), so `find` is the first match in document order, which is
what `painted()`'s "the first background anything paints" relies on. The matcher is handed a
`TestingNode` and a `&dyn ElementExt`.

`TestingNode` has `layout()` (a `LayoutNode`: `area`, `inner_area`, `margin`, `padding`),
`children()`, `is_visible()` and `element()`. `ElementExt` has `style()`, `text_style()`,
`layout()`, `accessibility()`, `layer()`, `effect()` and `events_handlers()`.

**Nothing in that surface reads text.** The repo's tests find a row by its background colour for
that reason. Text is reachable, but only by downcasting past the prelude: `ElementExt: Any`, and the
concrete elements are exported from `freya::elements` rather than `freya::prelude`:

```rust
use freya::elements::{label::LabelElement, paragraph::ParagraphElement};
use std::any::Any;

let labels: Vec<String> = test.find_many(|node, _| {
    (node.element().as_ref() as &dyn Any)
        .downcast_ref::<LabelElement>()
        .map(|l| l.text.to_string())
});
```

Verified: that reads a `label().text(..)`, and the `ParagraphElement` equivalent reads
`p.spans[..].text`, which is what an `InstructionRow` is made of.

**`is_visible()` is the honest answer to "is this on screen".** A plain `ScrollView` keeps every
child in the tree: 40 rows in a 100px viewport are all found by `find_many`, all have real layout
areas, and exactly the five that fit answer `true`. A `VirtualScrollView` is the other case. It only
builds the rows it draws, so a 500-row list in a 100px viewport yields six nodes and `find_many`
cannot see row 400 at all.

## What it genuinely tests

- **Layout, measured.** `node.layout().area` is what torin actually computed, not what the builder
  asked for. That distinction is the whole of
  `a_font_change_repaints_and_resizes_a_component_nothing_else_woke`: a row-height function
  returning a new number proves nothing if the component was never re-rendered.
- **State, effects, hooks, memos**, including the ordering hazards. A `State::read` guard held
  across a write is a runtime panic and nothing else in this repo can catch it, which is why
  `leaving_a_project_leaves_nothing_of_it_behind` mounts a runner to assert about states rather than
  pixels.
- **Reactivity.** That a component with no props, whose parent did not change, re-renders because it
  read a thread-local-backed state: `a_theme_switch_repaints_a_component_nothing_else_woke`. There
  is no other way to assert this.
- **Every pointer-driven interaction that lands on a node**, sweeps included, and, with a hand-built
  `PlatformEvent`, every keyboard one.
- **Scrolling.** `test.scroll` is a real wheel event through a real `ScrollView`. The wheel is
  applied instantly, with no animation (nothing in `scrollviews/scrollview.rs` animates it), so it
  needs no polling. `ScrollController` is plain `State`, readable and writable from the test.
- **Drag and drop, including the dock.** See below; this is the one most likely to be assumed
  impossible.
- **Tree integrity, for free.** freya-testing turns on `freya-core/debug-integrity`, so every
  `sync_and_update` runs `verify_scopes_integrity` in a debug build
  (`freya-core-0.4.3/src/runner.rs:686-687`). The live app never does.

### Drag and drop works, and so does a dock drop

`DragZone`/`DropZone` are built entirely out of ordinary pointer events
(`freya-components-0.4.3/src/drag_drop.rs`): `on_pointer_down` arms the press,
`on_global_pointer_move` promotes it to a drag once the cursor has moved `drag_threshold` (4.0px,
line 65) and writes the payload into `use_drag`, and the drop zone's `on_mouse_up` fires the handler
(lines 212-226). All three are things `TestingRunner` sends.

A real `DockingArea` drop was verified end to end: two panels, tab 1 in panel 0, dragged onto panel
1's content, and `on_drop` was called with `DropTarget::Center(1)` and the tree came back with `[2]`
and `[3, 1]`. The recipe, and each step is load-bearing:

```rust
test.move_cursor(handle);   test.sync_and_update();
test.press_cursor(handle);
test.move_cursor((handle.0 + 10., handle.1 + 10.));  // past the 4px threshold
for _ in 0..3 { test.sync_and_update(); }            // the overlay drop zones mount here
test.move_cursor(target);
for _ in 0..3 { test.sync_and_update(); }
test.release_cursor(target);
for _ in 0..4 { test.sync_and_update(); }
```

The app's own bar is dragged the same way (`a_tab_is_dragged_along_the_bar_to_move_it`), the
drop zones there being the chips themselves; what a drop would land on is read off the chip's
`Border`, since `element.style()` is in the matcher's hands and a colour is not otherwise
observable.

The passes between the two moves are not padding. `DockingArea`'s overlay, the centre zone and the
four edge zones, is rendered only while `use_drag` holds a payload (`docking.rs:560-641`), so it
does not exist to be measured until a pass has run after the threshold was crossed. The overlay is
split 1:2:1 on both axes (`MIDDLE_FLEX = 2.0`, `docking.rs:307`), so the centre is the middle half
of the panel and a `Split` drop means aiming at the outer quarter.

## What it cannot test

**Pixels, in practice.** `render()` gives you a PNG and nothing to compare it against: no golden
images, no diffing, no tolerance. Combined with the text measurement below, a snapshot test would be
pinned to the fonts installed on whoever ran it.

**Text at a fixed width.** Text *is* really shaped: `FontCollection` gets `FontMgr::default()` as
its default manager (`lib.rs:245-248`), which on Linux is skia's fontconfig backend, so real system
fonts resolve. Measured here, twelve `i`s and twelve `M`s in `monospace` came out the same width: a
real monospace advance, not a stub. So **any assertion about a text width is an assertion about the
machine**. The default chain is `Ubuntu, Adwaita Sans, Noto Sans, Arial` on Linux
(`freya-core-0.4.3/src/style/default_fonts.rs`), none of which is guaranteed. `set_fonts` with an
embedded `.ttf` is the way out, and nothing here does it: every existing assertion is about a *row
height*, which is a number the app computes and hands to the layout, not something a font was
measured for.

**Anything the pointer cannot reach.** Hit testing is `is_point_inside`
(`freya-core-0.4.3/src/events/measurer.rs:47-84`): the point must be inside the node's
`visible_area()` *and* inside every inherited clip. A `rect` refines that with its corner radius
(`freya-core-0.4.3/src/elements/rect.rs:467-480`). Verified: a press at `(1., 1.)` on a 100×100 rect
with `corner_radius(20.)` hits nothing at all, while `(21., 21.)` hits. Aim at centres.

**Wall-clock behaviour, except by burning wall clock.** Animations run off `RenderingTicker::tick()`
plus a real `Instant` (`freya-animation-0.4.3/src/hook.rs:271-317`), and `sync_and_update` never
ticks; only `poll`/`poll_n` do, and they `std::thread::sleep` for real. `Tooltip`'s 500ms delay is
an `async_io::Timer` (`freya-components-0.4.3/src/tooltip.rs:209`). Timers do fire under the runner
(async-io has its own reactor thread), but there is no virtual clock anywhere, so a timing test
costs its own duration in real seconds. One here does:
`a_sweep_past_the_edge_scrolls_the_listing_the_pane_is_drawing_now` drives the sweep's 40ms
autoscroll with `poll_n` and takes about 0.4s of them.

**The clipboard, honestly.** `ClipboardContext::new()` is attempted at mount and stored as an
`Option` (`lib.rs:232-238`); on a machine with no display it is `None` and `Clipboard::set` fails
silently, which is exactly what the app already does about it. A test asserting clipboard contents
would pass or fail on the environment.

### The `Menu` question

**The headless runner is not the reason the guard could be removed. The guard was doing nothing, and
worse than nothing.**

`DocumentMenuButton` once carried an `opening` flag on the theory that freya's `Menu` closes on any
global press, the press that opened it is one, and so the first close request had to be swallowed.
Verified by running it:

`Menu` attaches `on_global_pointer_press` and calls `on_close` from it
(`freya-components-0.4.3/src/menu.rs:170-174`). `MouseUp` derives `PointerPress` and globally emits
`GlobalPointerPress` (`freya-core-0.4.3/src/events/name.rs:136-138, 167-169`), so the button's
`on_press` and the menu's close handler come from the *same* source event, and `EventName::Ord` puts
non-capture globals last (`name.rs:61-84`), so `on_press` runs first.

But the listener set for a global event is taken at **measure** time, before any handler runs:
`measure_source_global_events` calls `events_measurer.get_listeners_of(&global_event_name)`
(`ragnarok-0.4.3/src/measurement.rs:35`) inside `EventsMeasurerRunner::run`
(`ragnarok-0.4.3/src/measurer.rs:84-88`), whose whole output is queued and only then executed. The
menu does not exist when the opening press is measured, since `on_press` is what creates it, so its
close handler is not in that batch and **cannot** fire on the press that opened it.

That is not a property of the headless runner. `TestingRunner::send_event`
(`freya-testing-0.4.3/src/lib.rs:409-422`) builds one `EventsMeasurerAdapter` over the current tree,
runs it over a one-element `vec![platform_event]`, and pushes the result down a channel.
`freya-winit-0.4.3/src/renderer.rs:900-928` does character-for-character the same thing for
`WindowEvent::MouseInput`. The live app behaves identically.

Without the guard, one outside click closes it and the button still toggles. With it, an outside
click only cleared the flag and the menu stayed open, costing the reader a second click; the
existing test could not see that because it re-pressed **the button**, where `on_press` closes the
menu before the guard is consulted.


The confusion is inherited from `ContextMenu`, where the swallow is real but is for the *other*
opening gesture. `ContextMenu::open_from_event` (`context_menu.rs:74-88`) sets `Pending` for a
left-button `PressEventData` and `None` otherwise, and `ContextMenuViewer`'s `on_close` treats
`None` as "swallow this one" and `Pending` as "close now" (`context_menu.rs:164-172`). A right-click
menu opens on `on_secondary_down`, a `MouseDown`, and the `MouseUp` that ends the same gesture *is*
measured against a tree that already holds the menu, which is the request that has to be swallowed.
This repo's file-close menu opens exactly that way (`ui.rs`, `on_secondary_down` →
`ContextMenu::open_from_event`) and gets the swallow from freya for free. `DocumentMenuButton` opens
on `on_press`, a `MouseUp`, the last event of its gesture, and needs nothing.

**The rule to take away: a popup opened from `on_press` needs no guard; one opened from a `*_down`
handler does.** And: a test that re-uses the same control to undo an action is not testing the
action's own dismissal path.

## Passes

`TestingRunner::sync_and_update` (`lib.rs:329-384`) drains the processed-event channel and
dispatches the handlers, calls `Runner::sync_and_update`, applies the mutations, measures the layout
and processes accessibility. `Runner::sync_and_update` (`freya-core-0.4.3/src/runner.rs:682-757`)
begins with `handle_events_immediately` (`runner.rs:632-680`), which drains the message queue into
`dirty_scopes` and `dirty_tasks` and then, and this is the whole rule,

> **returns early without polling a single task if any scope is dirty** (`runner.rs:644-646`),

and otherwise polls every queued task once. The pass then renders exactly the scopes that were dirty
when it started.

So a pass is *either* a render *or* a round of task polling, never both, and effects and memos are
tasks: `use_side_effect` is `Effect::create`, a `spawn`ed loop
(`freya-core-0.4.3/src/lifecycle/effect.rs:20-28`, reached from `use_side_effect` at `:98-100`), and
`Memo::create` recomputes in one too (`lifecycle/memo.rs:106-116`). Measured, with an effect that
reads `a` and writes `b`:

| the scope also reads `a` | pass 1 | pass 2 |
|---|---|---|
| yes | renders; `b` unchanged | effect runs; `b == 9` |
| no | effect runs; `b == 9` | — |

**One pass per hop, plus one for every hop whose write also dirties a scope.** A chain of *n* hops
therefore settles somewhere between *n* and *2n* passes, and which it is depends on what the
components happen to read, which is not knowable at the call site. That is the entire reason this
repo writes `for _ in 0..4 { test.sync_and_update(); }` rather than a number it can justify, and why
`pump(&mut test, || ready())` exists for anything with a worker thread behind it: a channel and a
thread make the count genuinely unbounded, so the test loops until the condition holds (and then
four more, for the hops the condition itself starts) and fails loudly at 200 rather than asserting
on whatever happened to have arrived.

**A pass count is not a wait.** `sync_and_update` costs microseconds, so a loop of them gives a
thread on another core no time at all; only `pump`, `poll` and `poll_n` sleep. Six of the
link-following tests pressed a name and then ran 32 passes before reading the state, and under load
one press in ten had not been answered by then. Anything a thread or a `Timer` answers waits on its
condition and never on a number.

**A timer the test starts goes on running between its assertions.** A sweep held past a pane's edge
scrolls the view every `AUTOSCROLL_TICK`, and every pass after that costs real time, so whatever the
scroll moves -- the rows a `VirtualScrollView` has built, the row the sweep reaches -- is a moving
target that settles differently on a loaded machine. Leave it nothing to move (a window that holds
the whole listing) or assert on something it does not touch. Two things go with that. The task
outlives both the render that spawned it and the gesture that started it, since it only notices the
button is up at its next tick, so a second gesture can be driven by the first one's task -- which is
a defect in the app as much as a hazard for a test, and was both here. And `paragraphs` counts
paragraphs: a separator row draws none, so its length is not the listing's row count.

Two corollaries worth knowing. A component that dirties itself every pass would **starve every task
in the app**: the app's `use_theme` writes during its render body and is safe only because the write
is idempotent, and a test that hangs is what says it stopped being. And
`a_desktop_that_changes_its_mind_repaints_the_window`'s "two passes, and the second is not padding"
is this rule: the platform state wakes the scope holding `use_theme`, and the write that scope makes
wakes everything that drew a colour, one pass later.

## Writing one here

The shape every test in `ui.rs` follows:

1. **A harness function**, `fn x_harness() -> impl IntoElement`, that consumes the contexts and
   mounts the smallest thing under test. `project_harness` is `rect().expanded()`, because what is
   under test is the states; `scratchpad_view_harness` mounts the real pane, because what is under
   test is whether its rows survive the list being shortened. Mount the pane only when the pane is
   the question.
2. **Contexts through the setup closure**, returned as a tuple the body keeps. When the set is the
   app's own, use the `project_states!` macro: it provides all ten of `ProjectStates` in `app()`'s
   order, including the `Active` memo derived from the dock and the docs, so a test drives what the
   app drives. It is a macro and not a function because the runner's type is
   `freya_core::integration::Runner`, which freya's prelude does not re-export and this crate does
   not depend on by name.
3. **Substitute the worker, not the work.** `Study`, `Working` and `Feed` are `Arc<dyn Fn>` or a
   channel handed in through a context, so the real `use_analysis_with` / `use_scratchpad_with` /
   `take_load` machinery runs against an answer the test controls. That is what turns "the stale
   answer was dropped" from a race into a fact.
4. **Settle deliberately.** `pump` when a thread is involved; a fixed small loop when it is not; a
   bare `sync_and_update` only when the change is a plain write the scope reads.
5. **Confirm it fails first.** The house rule, and headless tests earn it more than most: the `Menu`
   case above is a test that passed with and without the thing it claimed to be testing. Make the
   assertion discriminate on the *mechanism*. For the menu, that is a press somewhere the menu is
   not, which is the only gesture the guard changes.

A note on process-wide state: `palette()` and `fonts()` read thread-locals, so two runners on two
cargo test threads do not interfere, but `HIGHLIGHTED` is a `static` and does. `SWITCHING` is the
mutex that serialises the tests that switch appearance, and anything new that calls `set_appearance`
needs to take it.

## Verdict

Use a headless test for: **any control that can be pressed**, the popup it opens and the state it
changes; **a drag, including a drop into the dock**; **a scroll and where it lands**; **a keyboard
binding**, modifiers built by hand; **a row height, a width or a position** as laid out rather than
as requested; **anything a worker thread answers**; and **anything about which component
re-rendered**.

Launch the app for: **whether it looks right**. Colour against colour on a real display, a glyph
that turned out to be the wrong size, spacing that is correct and still ugly, a font the desktop
actually resolved, an icon that failed to load, a scrollbar that overlaps the last column. Also for
**anything timed the reader would notice**, a tooltip's delay or an animation's easing, where the
runner can only tell you the value after a real sleep, and for **the platform's own behaviour**,
which is stubbed out here: the cursor icon, the window's theme as the desktop really reports it, the
clipboard, the file dialog.

The line is not "logic versus UI". It is: does the assertion have a number or a state behind it, or
does it only have an opinion. The first is a test. The second is a screenshot.
