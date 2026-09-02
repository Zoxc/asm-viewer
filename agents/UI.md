# The UI: freya, state, documents and the dock

The shape of the UI: what freya 0.4 is and is not, the root contexts, what a document is, how the
dock holds one, what each tab remembers, and how a binary is opened. The worker, the two panes,
the sidebar, the appearance and the scratchpad each have a note of their own beside this one.

## The UI is a directory

The UI is a directory, and each of its files is a **cut out of what was one 8 700-line
`ui.rs`** rather than a boundary designed from scratch — what each holds is what that file's
section banners and `AGENTS.md`'s layout list already said belonged together. Two mechanical points
carry across all of them and are written out in `src/ui.rs`'s own `//!` header: the imports
there are `pub(crate) use` and every file begins `use super::*;`, so each keeps the set of
names it had as a section; and each `mod x;` is followed by a `pub(crate) use x::*;`, so a
name means what it always meant wherever it is written. Visibility is what the compiler
asked for and no more, so the annotations *are* the list of what crosses a boundary.

Five of the names are not the obvious one, and each avoids shadowing a crate module the
prelude has already brought in: `ui::source_view` (not `source`), `ui::project_view` (not
`project`), `ui::filter_bar` (not `filter`), `ui::pad` (not `scratchpad`) and `ui::analyzed`
(not `analysis`, which is the crate `ui/tests.rs` calls into). One name genuinely collides:
`freya::prelude` exports a `use_theme` of its own, so `ui/tests.rs` names ours explicitly —
an explicit import wins over a glob, and that line is the disambiguation rather than a
duplicate.

## Framework and state

freya 0.4 is **not** Dioxus-based: no `rsx!`, no `#[component]`, no `use_signal`. It is a builder
API (`rect().width(Size::fill()).child(..)`) over its own `freya-core`. Most freya material online
describes the older API and does not apply.

**State** is a handful of `State`s provided at the root with `use_provide_context` and read with
`use_consume`: `Objects`, `Active` (the active `Document`), `Open` (the open tabs),
`AsmAt`/`SrcAt` (where each *side* of each of those tabs was left), `Hist`, `Proj` (which project
all of that belongs to), `Loading` (the files on their way into `Objects`), `Focused`, `Pinned`,
`Marked`/`Shift`, `Land` (a line to pin the moment a document arrives), `Analysis` (what
the worker has to say about the selected symbol), `Locations` (every symbol the line last
asked about was compiled into), `Pad`/`PadText` (every scratchpad and which is shown, and a buffer per pad),
`SplitRatio`/`Splits` (how wide a document's assembly side is), plus the memos `Symbols` and
`Active`. The seven that a project *owns* travel together as a `ProjectStates`, since a project
switch closes all of them and reopens all of them.

**One strip, two kinds of tab.** A `Document` (`project.rs`) is **a place in a binary or a file**:
`Document::Assembly(Selection)` — an object or a function — or `Document::Source(Arc<str>)`, the
string the debug info said and never a path this filesystem was asked about. A tab has two sides,
assembly and source, and the variant says which side the tab is *about* and therefore which drives
the other. So opening a file from a directory panel and opening a function from the symbol list
produce the same kind of thing, differing only in which way the mapping runs. Each tab wears the
one glyph that tells the two apart — the same pair the Assembly and Source views wore before those
two became a document's left and right halves. Until Step 1 this was two strips with two notions of what was open — `Open`/`Sel` for
functions and `Files`/`Shown` for files — and the merge is what made the history able to record a
visited file and the session able to keep the strip's interleaved order.

**`Active` is a derivation, not a state.** What is open is `Open { dock, docs }`: the content
area's dock, whose **document panel**'s `tabs` vec *is* the list of open documents in the reader's
own order, and `Docs`, the table saying which `Document` each tab's `DocId` stands for. There is no
second list and no cursor — the active document is that panel's active tab read through the table,
which is the whole of `active_document`, and `open_documents` is the same walk for the list. `Docs`
holds no order at all; membership is the one thing the two share, and it is an invariant the three
functions keep and a test asserts: a document's tab and its table entry are made together and
closed together.

`Active` is a `Memo` over the two, because the dock notifies on every layout change and a reader
dragging a split must not re-render every pane that draws a document — `Memo` writes with
`set_if_modified`, so a drag that changed no document wakes nothing. It is therefore **a beat
behind**, a memo being recomputed by a task woken on a notify. That is right for anything that
*renders* and wrong for anything that must be true inside one event handler, so `activate`,
`close_tab`, `close_others`, `close_binary` and the save observer call `active_document` on the
states directly and never read the memo. `use_kept_position` asks `Docs` for the same reason: it decides whether to
write a row down for a tab that may have just been closed, and a memo could still be reporting it
open during exactly that run.

`Active` being `None` means two things and deliberately does not distinguish them: nothing is open,
or **the tab on top of the document panel is a view**. Making Settings the active tab therefore
means there is no active document — the analysis clears, `session.toml` writes `active = None`, and
a restart with a view on top restores every tab and shows none of them. That is the price of the
derivation, and it was taken over the alternative, which is remembering the last document that was
active there: memory rather than a reading of the dock, and the second source of truth back again.

The invariant — the active document is one of the open tabs, or `None` — is held by four functions
and nothing else: `activate`, `close_tab`, `close_others`, `close_binary`. **Every** site that would *open* a
document calls `activate`, `navigate` included, because the history keeps an entry long after its
tab was closed; pressing a tab needs none of them, freya's own header wrapper setting the panel's
active tab, which *is* the change. `Selection` itself has **no "nothing" variant**: having none open
is an absent one, which is the only spelling that stays honest once a selection is something a tab
can hold.

**Layout** is a toolbar over a `ResizableContainer`: a `PanelSize::px(300.)` sidebar and a
`PanelSize::percent(100.)` content pane, mixing the two sizing modes deliberately so the sidebar
keeps a fixed width and the content takes the rest, with freya's 4px `ResizableHandle` between
them. `ResizableContainer` renders itself `.expanded()`, so it needs a parent already sized —
`Size::flex(..)` only works under a parent with `.content(Content::Flex)`. The content panel holds
the `DockingArea` and nothing else: the open documents are tabs *in* it, so the bar over them is
the document panel's own tab bar rather than a strip of the app's.

The toolbar itself holds three controls: the two history chevrons and Open. `NavButton` calls the
same `navigate` the mouse's side buttons do — a second spelling of the step would be a second set
of rules about tabs, selection and recording — and **it reads `Hist` rather than peeking it**, which
is the whole of how the pair stays current: a visit pushed anywhere, a close that drops entries and
every move of the cursor, the one the button itself just made included, repaints both. A button
with nothing in its direction is **dimmed rather than hidden**, the first disabled drawing in this
app: hiding it would slide the other one under the pointer, and a reader who has been nowhere yet
would never learn the pair is there. Disabled is the whole of the drawing — no hover wash, no press
handler, and the chevron in `dimmed(icon_fg, pane_bg)` — while the tooltip stays, naming the
direction where `entry_text` gives it nothing to name. `Nav::destination` is the one place the
answer is worked out and `Nav::possible` is it asked as a question, so a live button and a step that
does something cannot disagree. Headless, the runner can be asked whether a button washes under the
pointer and whether it kept its box, and not what colour the chevron came out: an `SvgViewer`
rasterises its colour into an image that is not in the element tree.

Inside each panel is a `DockingArea` over a `DockArea` model. A `Tab` is two-kinded —
`Tab::View(View)` for one of the seven views, `Tab::Document(DocId)` for an open document — because
`DockingModel::TabId` is `Copy + PartialEq + Hash` and a `Document` is none of the three. Both areas
use `Tab` as the payload and `use_drag` keeps one `DockDrag<Tab>` at the root. The outer split stays
a `ResizableContainer` because docking cannot express a literal 300px. A drag carries only the tab,
so the area receiving a drop evicts it from the other through a wired-up
`other: Option<State<DockArea>>`. A view is a **persistent pane**, not a slot the selection drives:
each is a unit `Component` that consumes context and renders off the state it is about, so a
selection change re-renders only the panes that read it and never the root.

**One panel is designated, and the reason is the opening rather than the placeholder.** A click in
the symbol list opens a document, and that document has to land *somewhere*; a dock has many panels
and freya has no notion of "the panel documents belong to", so `DockArea::documents` names one.
Three rules follow. `on_drop` refuses a document into any other panel — one visible document is what
lets `Analysis`, `Marked`, `Focused` and `Pinned` each hold one answer for the window — and refuses
a `DocId` the table no longer knows, which is a drag that outlived its document and is the whole
payoff of **ids never being reused**. A **view**, by contrast, may go anywhere, that panel included:
Project, Settings and the Scratchpad start tabbed in it, to the left of the documents, where they
are always visible. And `tidy` exempts it from the folding sweep, so closing the last document
cannot fold the content area away.

`tidy` is freya's `close_empty_panels` **written out rather than called**, because that sweep retains
every non-empty child with no exemption and has to be replaced rather than followed — a panel
re-created after it would come back somewhere else in the tree. The two behaviours of freya's that
are kept: a split left with one child collapses into it, and a lone panel at the root is never
removed. Likewise a close never goes through `DockNode::remove_tab_except`, which sets a panel's
active tab to `tabs.first()`; landing on the **neighbour** is a rule of this app, so `close_tab`
removes the tab by hand and chooses with `tabs::landing`.

**Open documents *are* dock tabs.** This supersedes 6c's "the tab strip is not the dock's tabs",
which argued three things. Two are answered by the designated panel: there is one answer to "which
document is active" — that panel's active tab — and closing the last document folds nothing away,
the panel being exempt from `tidy`. The third stands and is the price: the layout and the list of
open documents are no longer separable, and the arrangement survives a close because a rule says so
rather than because the shape makes it impossible to break. What it buys is that a reader arranges
their documents the way they already arrange the views, and that Steps 9, 12, 19 and 33 each have
one kind of tab to change instead of two.

A document's header is `chip` — the same element the content area's own strip drew, hover state and
× included. **Nothing in it activates the tab**: freya wraps a header in a `DropZone` around a
`rect().on_press(set_active)` around a `DragZone`, so pressing it makes it the panel's active tab
and therefore the active document. That is also why the × must `stop_propagation`, or a close would
first switch to the tab it is closing. The × is drawn for documents only — the views are furniture,
one of a kind, with no way back once closed, where a document is always reachable again from the
symbol list or the history.

**The × is a control of its own**, `TabClose`, and a component rather than another line of
`chip` for one reason: the hover has to be *its*, freya has no `.hover()` pseudo-state, and the
`use_state` with `on_pointer_over`/`on_pointer_out` around it cannot run in a helper — which is
why the × reaches `chip` as an element already built rather than as an `on_close` handler. Two
things follow from its being a control. It is **a target you hit rather than one you aim at**: a
`close_target()` square — a row less the air above and below, the shape `toggle_size()` is —
centred on the glyph, so what grew is the padding and not the ×, which keeps the interface font's
own size. And it says under the pointer that it is the × and not the tab: `close_hover_bg` behind
it and the glyph up from `address_fg` to the interface text, while the tab under it stays lit —
the two are told apart by the wash being the deeper step, not by the tab going out. It closes the
tab itself rather than taking a handler, a `Component` being `PartialEq` where a closure is not:
the `DocId` is the prop and the five states a close needs come from the contexts, the same ones
the header reads a step above it. The headless pair is
`a_press_beside_the_glyph_still_closes_the_tab`, which presses inside the target and nowhere near
the glyph, and `the_close_target_lights_under_the_pointer`.

A right-click on a document's header opens a menu of one item, **Close other tabs**, which is
`close_others`: the tab it was opened on stays, every other *document* in the panel goes, and a view
sharing the panel is left where it is — it is not a document, and the × it has no place for is the
same argument. Its own function rather than `close_tab` in a loop, because each of those would work
out a landing of its own and walk the panel through every intermediate state, where the landing here
is known before anything is removed: the kept tab, and only when the tab on screen is one of the
ones closing. The header opens no menu at all when nothing else is open, rather than a menu whose
one row would do nothing, and it asks the panel for that at the **press** — whether a tab has
company is not something a header draws, so subscribing to the panel for it would re-render every
tab whenever any one of them opened.

The document panel's tab bar is the horizontally scrolling one the strip used to be (`chip_strip`),
because documents are opened by the dozen; a view panel's stays a plain row, seven views always
fitting. Two things bite there. freya appends one child more than there are tabs — a
`rect().expanded()` drop zone for "past the last tab" — and `expanded()` is meaningless inside a
horizontal scroll view, so it is given a width of its own. And a tab's name is elided **by character
count in Rust**, where every other truncation is a width: a `maximum_width` anywhere inside one makes
it shrinkable, and a horizontal scroll view measures children against the space *left*, so tabs past
the edge get no width and draw as a bare ×. Do not "fix" that back into a width.

**A document's two sides live inside its tab.** `Tab::Document` renders `AssemblyPane` beside
`SourcePane` in a `ResizableContainer` — not a nested `DockingArea`, which is a great deal of
machinery for a two-way split. The cost is real and was taken deliberately: **the Source pane is no
longer independently dockable**, since it is inside a document rather than beside one. Each pane
takes its `Document` as a prop rather than reading `Active`, which is both synchronous and honest —
only the active tab's content is mounted, so a pane is only ever built for the tab it belongs to.

That unmounting is why the split ratio is held at the root (`SplitRatio`, with `Splits` the shared
`ResizableContext` it is read back out of). A `ResizablePanel` registers at its `initial_size` in a
`use_hook` and *removes* its entry in a `use_drop`, so even a shared context comes back holding the
initial sizes under new panel ids; what survives is a number the app keeps, fed in as `initial_size`
and written back out while the split is on screen. One number for the app and not one per document:
per-document would be a third `Positions`-shaped map to forget in `close_tab`, for a number nobody
asked to differ per document.

**A document is a place in a binary or a file; everything else is a view.** This is 8e's rule with
Step 1's amendment — it used to end at "in a binary" — and everything below it is unchanged, so
decide nothing about it again. The document panel holds `Document`s and never anything else, and
that is what lets five separate things work without a case each: the Assembly *and* Source panes both
render "the active tab", the history records it, `SavedDocument::from_document`/`::resolve` write
it down and find it again after a restart, `close_binary` knows which tabs a closing file takes
with it, and `entry_text` knows what to call it. A project view, the settings page and a
scratchpad's editor are none of that: they resolve against no object, they are no file on disk the
panes could open, there is one of each rather than many, and neither pane could draw one. So they
are **dockable views** — a `Tab` — which is the mechanism the app already has for "a pane with its
own state that the reader can put where they like", and which `InfoTab` was already an instance of.
A third `Document` variant was the alternative and buys a tab in a strip nothing else would put a
second entry in, at the price of five answers nobody wants: what `resolve` does with it after a
restart, what `Document::in_file` says when a binary closes, what the panes draw for it, what the
history means by a "place" that is not one, and what the session file spells it as. Persistence
follows from the same sentence: a `Tab` is layout, and the dock layout is deliberately not
persisted, so a view is **explicitly excluded** from the saved tabs and `SavedDocument` needs no
answer for it. What a scratchpad *builds* needs no rule at all — the artifact goes through
`open_files` like any other binary, and its functions are ordinary tabs.

**Each tab remembers where each of its sides was left.** A pane has one `ScrollController` and
shows one tab at a time, so left alone it hands the tab arriving whatever offset the one leaving
had. `AsmAt`/`SrcAt` are two root `Positions` maps beside `Open`, **both keyed by the `Document`**
— so an entry means "this side of this tab" for exactly as long as the tab is open — and
`use_kept_position` is the whole of the behaviour, called once by `InstructionList` and once by
`SourceList`. Which tab a listing's row is filed under is `asked_of` the question that listing
answers, never the tab the app is showing: while the worker catches up the pane is drawing the one
being left, and for a source-driven tab the question's tab is the file's and not the resolved
symbol's, which is very likely not open at all. Keying the source side by the *file*, which is what the Source pane's own strip did,
made two functions compiled from one file share a position they have no reason to share. What is
kept is a **row**, clamped to what the tab holds *now*, so a rebuilt binary or a shortened file
cannot come back past the end. Three things are
load-bearing. Reading the controller's position (`<(i32, i32)>::from`) is a `State::read`, which is
what **subscribes the effect to the pane's own scroll**: every position is written down as it
happens rather than on the way out, which is what survives the window merely being closed. The tab
the controller is *holding* is tracked in the hook — an `Rc<RefCell>`, not a `State`, since nothing
renders from it — because it is not the tab the app is showing during the one run that has to move
the view, and every write goes under the held one. And a `Pin::reveal` **wins** over a remembered
position because the same effect makes both: `use_kept_position` is handed the pane's reveal as a
closure and asks it first, applying the remembered row only when no scroll was made. The two *are*
owed at once — a Locations row opens a symbol on a line, so the tab changes and the arriving one
is owed a reveal — and two effects' scrolls land in whichever order the runtime wakes them; with
the reveal first, it had marked itself made by the time the kept row was put over it, which reset
both panes to the top. One effect has one order, and when a reveal scrolls, the effect wakes on
that scroll and records where it landed. `close_tab`/`close_others`/`close_binary` forget both of a tab's positions with the tab, which is
not tidiness: a `Document::Assembly` key holds the `Arc<Object>` it points into — and the hook is
handed the tab list precisely so that the run *after* a close, still holding the tab that has gone,
cannot put it straight back. `close_tab` forgets the tab's driven line with them, which *is*
tidiness: a `Document::Source` key holds no object, so nothing is being held up.

**Opening a binary is the one path in, and it streams.** `open_binaries` is `close_binary`'s
opposite number and the only thing that ever adds to `Objects` — the toolbar's Open, a session
restore and a scratchpad's rebuild all go through it, so they cannot differ about what opening a
file means. A `std::thread` and an `async_channel`, `use_analysis`' shape, but the answers come back
one at a time: `Loads::begin` registers the paths **before a byte is read**, so the sidebar has a
row for the whole of the wait rather than from whenever the first answer lands, and `take_load`
writes each batch of objects in as it arrives. The channel is **unbounded and drained in batches** —
unbounded because backpressure is exactly wrong here (the worker is the thing that should run flat
out) and batched because a write per member is a re-render per member, which for an archive whose
members parse in a millisecond is a hundred renders nobody sees. **An object nobody asked for any
more is dropped, not prevented**, which is `use_analysis`' rule in a second place and has to be: the
worker is already parsing when the file is closed. It is checked against `Loads::holds` — the load
*and* the path, since a file closed and reopened while the first parse ran is two loads and only the
second one's objects belong on screen, which is the whole reason a load has an id where an analysis
answer needs none (an answer is about a `Symbol` that already existed; a load is about work that has
produced nothing to be identified by). `take_load` **returning** is what stops the worker: it drops
the receiver, the next send fails, and the walk breaks. What streaming buys is not uniform:
the 196-member rlib's first member is offered at 102 ms against the 685 ms the whole file
takes (debug build), while the 331 MB binary is one object and gains no object earlier at all — there
the win is the row, on screen from the click instead of an empty list for six seconds. **Nothing
further is opt-in**, and measuring says why: of a file's parse, the whole of what is left after
line info, the DWARF context and the disassembly were already made lazy is reading the bytes and
walking the symbol table, which is what the Objects and Symbols lists *are*. On the 331 MB binary
(release) that is 1.38 s of which 766 ms is the read and 286 ms the demangling; deferring the
demangling is the only lever there is, and it defers work until the first click on the object.


**Identity throughout the UI is `Arc` pointer identity**, not names or indices: list keys are
`Arc::as_ptr(..).addr()` and every prop `PartialEq` is hand-written in terms of `Arc::ptr_eq`. That
matters twice — duplicate symbol names across objects stay distinct, and `#[derive(PartialEq)]` on
an `Arc<T>` field would deep-compare on every parent render.


## Testing the UI

`freya-testing` runs the whole app — components, hooks, effects, layout, events — with no window,
no GPU and no event loop, on the test's own thread. It is a dev-dependency for `src/ui/tests.rs`
and for nothing else, and the binary's whole suite runs in under two seconds, so
a test written to settle one point costs less than a `cargo run` and a look and is worth writing
even when it is going to be deleted again. Keep the ones that pin a mechanism — that a component
with no props re-rendered because it read `palette()`, that a superseded answer was dropped — and
delete the ones that only proved the code just written does what it says.

What it can be asked: **any control that can be pressed** and the state it changes, **a drag,
including a drop into the dock**, **a scroll and where it lands**, **a keyboard binding** with the
modifiers built by hand (`press_key` hardcodes `Modifiers::default()` and cannot express Ctrl),
**a row height, width or position as laid out** rather than as requested, **anything a worker
thread answers**, and **which component re-rendered** — that last has no other test shape at all.
What it cannot: whether it *looks* right; any text width, which is really shaped by the machine's
own fonts and so is an assertion about the machine; the platform's own behaviour, which is stubbed
(the cursor icon, the desktop's real theme, the clipboard, the file dialog); and anything timed,
there being no virtual clock, so a timing test costs its own duration in real seconds. Those are
what launching the app is for.

`agents/Headless.md` is the full account, checked against `freya-testing` 0.4.3's sources rather
than its README: the runner's whole surface, the rule that a pass is *either* a render *or* a
round of task polling (which is why a change needs somewhere between *n* and *2n*
`sync_and_update`s and why the tests here loop rather than count), and the house rule that a
headless test has to be made to fail first on the *mechanism* it claims to test.

