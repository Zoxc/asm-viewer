# The UI: freya, state, documents and the tabs

The shape of the UI: what freya 0.4 is and is not, the root contexts, what a document is, how the
strip of tabs holds one, what each tab remembers, and how a binary is opened. The worker, the two panes, the
sidebar, the appearance and the scratchpad each have a note of their own beside this one.

## The UI is a directory

The UI is a directory cut out of what was one `ui.rs`; `AGENTS.md`'s layout list says what each file
holds. Two mechanical points carry across all of them and are written out in `src/ui.rs`'s own `//!`
header. The imports there are `pub(crate) use` and every file begins `use super::*;`, so each keeps
the set of names it had as a section. And each `mod x;` is followed by a `pub(crate) use x::*;`, so
a name means what it always meant wherever it is written. Visibility is what the compiler asked for
and no more, so the annotations *are* the list of what crosses a boundary.

Eight of the names are not the obvious one, and each avoids shadowing a crate module the prelude has
already brought in: `ui::source_view` (not `source`), `ui::project_view` (not `project`),
`ui::filter_bar` (not `filter`), `ui::bookmarks_view` (not `bookmarks`), `ui::files_view` (not
`files`), `ui::pad` (not `scratchpad`), `ui::rescued_view` (not `rescue`) and `ui::analyzed` (not
`analysis`, which is the crate `ui/tests.rs` calls into). One name genuinely collides: `freya::prelude` exports a `use_theme` of
its own, so `ui/tests.rs` names ours explicitly. An explicit import wins over a glob, and that line
is the disambiguation rather than a duplicate.

## Framework and state

freya 0.4 is **not** Dioxus-based: no `rsx!`, no `#[component]`, no `use_signal`. It is a builder
API (`rect().width(Size::fill()).child(..)`) over its own `freya-core`. Most freya material online
describes the older API and does not apply.

**A `Component` carries no identity of its own.** `ComponentKey::default_key` hashes
`TypeId::of::<Self>()` and nothing else (`freya-core`'s `element.rs`), and a scope is re-rendered
only when its key or its props changed (`runner.rs:812`). So every sibling of one type answers with
the same key, and a list of them gives the diff nothing to tell two rows apart by. `.key(..)` comes
from `KeyExt`, which the built-in elements implement and a component does not, so a component takes
a key only where it is given somewhere to put one. The twelve keyed components here
(`InstructionRow`, `SourceRow`, `SymbolRow`, `PadRow` and the rest) each hold a `DiffKey` field,
implement `KeyExt::write_key` over it, and answer `render_key` with
`self.key.clone().or(self.default_key())`: `.key(..)` writes the field and the `or` leaves the
type's own key standing where a call site gives none. What goes into it is whatever identifies
*that* row: an `Arc::as_ptr(..).addr()`, an instruction's address, an index.

**A `Writable<T>` compares equal to every other one.** Its `eq` returns `true` outright
(`freya-core`'s `lifecycle/writable.rs`), there being nothing to compare in the four closures it is.
A `Readable<T>` made from a `State` or a `Memo` is the same, and only one made out of a plain value
compares by value (`readable.rs:92,102`). Handing one down as a prop therefore never re-renders the
component holding it, which is what makes a *mapped* one a hazard rather than a convenience:
`into_writable().map(..)` captures its way in (a field of a `Filter`, `dependencies[index]`) and the
component keeps the closures it was first given. A mapping that can change is safe only where the
component holding it is unmounted when it does, which is what the scratchpad's editor and its
dependency rows arrange (`agents/Scratchpad.md`), and never by handing it a new one.

**`prevent_default` cancels the events an event derives; `stop_propagation` stops it bubbling.** One
platform event becomes a queue of tree events, and a handler calling `prevent_default` makes the
executor drop from the rest of that queue everything the emitted event names as cancellable
(`ragnarok`'s `executor.rs:72-90`, over the table at `freya-core`'s `events/name.rs:179-219`). A
`PointerPress`, which is what `on_press` is built on, cancels the `MouseUp` and the
`GlobalPointerPress` beside it; a `MouseDown` cancels its `PointerDown` and `GlobalPointerDown`; a
`KeyDown` its `GlobalKeyDown`. That is the whole of how the filter bar's toggles keep an `Input`'s
keyboard focus (`agents/Sidebar.md`), and it is the derivation `agents/Headless.md` works out for
the `Menu` question read the other way. `stop_propagation` is the other axis and no substitute: it
stops the walk up the ancestors, which for an event that does not bubble (a move, an enter, a leave,
a global, a capture; `name.rs:248-254`) is nothing at all. And the walk starts at the **focused**
node for a key: a node that does not listen itself emits nothing, so an ancestor's `on_key_down`
never runs (`notes/upstream/freya.md`, and the filter panes' Ctrl+F in `agents/Sidebar.md`).

**State** is a handful of `State`s provided at the root with `use_provide_context` and read with
`use_consume`: `Objects`; `Active` (the active tab and the `Document` it shows); `Open` (the open
tabs and the trail behind each); `AsmAt`/`SrcAt` (where each *side* of each place on each of those
trails was left); `CodeAt` (where each code tab's places were left, as addresses); `Visited`
(everywhere the reader has been); `Bookmarked` (the project's bookmarks, in their saved shape);
`Proj` (which project all of that belongs to); `Loading` (the files on their way into `Objects`);
`Marked` (each pane's selected run, and what it owes the other) with `Shift` and `Ctrl`; `MarksAt`
(what each place on each trail had selected in each pane when it was last shown, put back with the
place and never saved); `Land` (a line and an instruction to select the moment a document arrives);
`Plant` (the instruction half of that, left for the listing that draws the document, its rows coming
after it); `CodeRows` (the section view's rows, which the Source pane beside it reads too);
`Analysis` (what the worker has to say about the selected symbol); `Sections`/`Window` (what it has
decoded of the object whose code is on screen, and the stretches the view wants next); `Locations`
(every symbol the line, or the function around it, last asked about was compiled into);
`Pad`/`PadText` (every scratchpad and which is shown, and a buffer per pad); `Talking` (whether a
language server is running, and what would stop it -- `agents/Lsp.md`); `SplitRatio`/`Splits`
(how wide a document's leading side is); plus the memos `Symbols` and `Active`. The eleven that a
project *owns* travel together as a `ProjectStates`, since a project switch closes all of them and
reopens all of them. `MarksAt` is among them for the closing and not the reopening, being the one
that is never saved.

**One strip, three kinds of tab.** A `Document` (`project.rs`) is **a place in a binary or a file**.
`Document::Assembly(Selection)` is an object or a function. `Document::Source(Arc<str>)` is a file
as a string and not a `PathBuf`: the spelling the debug info said, or the project directory joined
with a Files row's entries, which is deliberately the same spelling and is never canonicalised
(`agents/Sidebar.md`). `Document::Code(Arc<Object>)` is **all of one object's code** as one listing
with the symbols drawn as labels inside it (`agents/Panes.md`). A tab has two sides, assembly and
source, and the variant says which side the tab is *about* and therefore which drives the other; an
object's code is assembly-driven like a function's tab. It is one document per object, compared by
the object's pointer, and where the reader was in it is the tab's position and not its identity: a
place in it at an address is that tab landed there, which is also what a call target with no symbol
will open. Pressing an object in the Objects list opens it. `Selection::Object`, the object tab that
draws only "No symbol selected", stays a valid document (restorable, and the shape the file-tab goal
in `notes/Goals.md` will fill) and has no door for now. So opening a file from a directory panel and
opening a function from the symbol list produce the same kind of thing, differing only in which way
the mapping runs. Each tab has one glyph that tells the two apart. One strip rather than one per
kind is what lets the history record a visited file and the session keep the strip's interleaved
order.

**`Active` is a derivation, not a state.** What is open is `Open { strip, docs }`: the `Strip`
(`src/tabs.rs`), whose `tabs` vec *is* the list of open tabs in the reader's own order and which
holds the tab on screen beside it, and `Docs`, the table holding the **trail** behind each document
tab's `DocId`, every place the tab has shown, oldest first, with a cursor on the one it shows now
(`History`, one per tab). A tab is a
trail and not a document: a link followed inside it pushes onto the trail, Back and Forward move its
cursor, and what the tab shows is `Docs::get`, the entry under the cursor. There is no second list.
The active tab is the strip's own, read through the table, which is the whole of `active_tab`, and
`open_ids` is the strip's documents in order. `Docs` holds no order at all; membership is the one
thing the two share, and it is an invariant the closers keep and a test asserts: a tab and its trail
are made together and closed together. One tab may be the **temporal** one (`Docs::temporal`), the
preview a sidebar row opens its place in and the next row reuses. It is a tab like any other with
one flag on it, told apart by its name being italic.

`Active` is a `Memo` over the two, because the strip is written by more than the opening of a
document -- a page brought to the front, a tab moved along the bar -- and none of those changes what
any pane is drawing. `Memo` writes with `set_if_modified`, so a write that changed no document wakes
nothing. It yields the **id and the
document as one pair** (`Entry`), out of one read of both states: the driven line and the viewing
positions are kept per tab *and* place, and an id read a beat apart from the document would pair
another tab's for that beat, which the worker would answer with a re-ask. It is therefore **a beat
behind**, a memo being recomputed by a task woken on a notify. That is right for anything that
*renders* and wrong for anything that must be true inside one event handler, so `open_document`, the
closers and the save observer call `active_tab` on the states directly and never read the memo.
`use_kept_position` asks `Docs` for the same reason: it decides whether to write a row down for a
place that may have just been closed or dropped off its trail, and a memo could still be reporting
it there during exactly that run.

`Active` being `None` means two things and deliberately does not distinguish them: nothing is open,
or **the tab on screen is a page**. Making Settings the tab on screen therefore
means there is no active document. The analysis clears, `session.toml` writes `active = None`, and a
restart with a page on screen restores every tab and shows none of them. That is the price of the
derivation, and it was taken over the alternative, which is remembering the last document that was
active: memory rather than a reading of the strip, and the second source of truth back again.

The first invariant -- the tab on screen is one of the open ones, or `None` -- is `Strip`'s own and
cannot be broken from the UI, every change to what is open going through one of its methods. The
second -- a tab and its trail are made and closed together -- is held by `open_document`, `raise`,
`raise_tab`, `navigate`, `close_tab`, `close_others` and `close_binary`, and nothing else. **Every** site that would *open* a document calls
`open_document` with a `Reach`, which is what the click that opened it says and nothing about the
state can. **`InPlace`** is from inside the tab on screen (a relocation link, the companion header),
pushed onto that tab's trail so the place left is one Back away. **`NewTab`** is beside the tab on
screen, in a tab that stays (Ctrl+click on anything, a menu item, the unified view's Ctrl-press on a
label). **`Preview`** is from outside the panes (a sidebar row), into the one temporal tab, pushed
onto its trail so Back inside it walks the rows clicked, or into a new temporal tab where there is
none. Under every reach a tab already showing the place is **raised** instead, the one on screen
preferred where two show it. `NewTab` promotes the temporal one, since what was asked for is a tab
of this place that stays; `Preview` promotes nothing. Every opening is recorded in `Visited`. **What
promotes** the temporal tab: `NewTab` on the place it shows, a link followed in place inside it (the
reader is reading in it), or a double press on its header. `navigate` never does, walking a trail
not being going somewhere new in it. `raise` is the move between places already open (the strip's
menu, the neighbour a close lands on, a restored session) and records nothing. Pressing a tab needs
none of them: freya's own header wrapper sets the panel's active tab, which *is* the change.
`Selection` itself has **no "nothing" variant**: having none open is an absent one, which is the
only spelling that stays honest once a selection is something a tab can hold. A new tab goes in
**beside the tab on screen** (`Strip::show`), the way a browser opens a link, whatever kind that tab
is: a page has no reserved place at the left of the bar, being a tab like any other.

**Layout** is a toolbar over a `ResizableContainer`: a `PanelSize::px(300.)` sidebar and a
`PanelSize::percent(100.)` content pane, mixing the two sizing modes deliberately so the sidebar
keeps a fixed width and the content takes the rest, with freya's 4px `ResizableHandle` between them.
`ResizableContainer` renders itself `.expanded()`, so it needs a parent already sized;
`Size::flex(..)` only works under a parent with `.content(Content::Flex)`. The content panel holds
`ContentArea` and nothing else: the app's own bar, and under it the tab on screen.

The toolbar holds the pages menu and Open at its left edge and the history chevrons at its
right, held apart by a `Size::flex(1.0)` gap under `Content::Flex`. The gap is measured out of what
the controls left over, where a `Size::fill()` gap would claim the bar and push them off its end.
The pair sits at the corner so it stays under the same one however many neighbours Open grows.
`NavButton` calls the same `navigate` the mouse's side buttons do, since a second spelling of the
step would be a second set of rules about tabs, selection and recording, and both walk **the trail
of the tab on screen** and no other. **It reads `Active` and the table rather than peeking them**,
which is the whole of how the pair stays current: a switch of tab, a push onto any trail, a close
that drops entries and every move of a cursor, the one the button itself just made included, repaint
both. It reads `Active` and not the strip, or the pair would repaint whenever a tab moved along the
bar, which is the whole reason `Active` is a memo. A button with nothing in its direction is **dimmed rather
than hidden**, the first disabled drawing in this app: hiding it would slide the other one under the
pointer, and a reader who has been nowhere yet would never learn the pair is there. Disabled is the
whole of the drawing (no hover wash, no press handler, and the chevron in
`dimmed(icon_fg, pane_bg)`) while the tooltip stays, naming the direction where `entry_text` gives
it nothing to name. `Nav::destination` is the one place the answer is worked out and `Nav::possible`
is it asked as a question, so a live button and a step that does something cannot disagree.
Headless, the runner can be asked whether a button washes under the pointer and whether it kept its
box, and not what colour the chevron came out: an `SvgViewer` rasterises its colour into an image
that is not in the element tree.

The sidebar is a `DockingArea` over a `DockArea` model whose `DockingModel::TabId` is a `Panel`, one
of the seven, so **a document cannot be named there at all** and every rule about where one may go
is a rule the type states. A panel is a **persistent pane**, not a slot the selection drives: each
is a unit `Component` that consumes context and renders off the state it is about, so a selection
change re-renders only the panes that read it and never the root. Adding or removing a panel needs
no migration: the sidebar's layout is not persisted, so a removed one is a compile-time deletion and
an added one starts where the default layout puts it. `Panel` is imported by name as well as through
the glob (`src/ui.rs`), freya's prelude having a `Panel` of its own. The outer split stays a
`ResizableContainer` because docking cannot express a literal 300px.

`tidy` is freya's `close_empty_panels` **written out rather than called**, because that sweep can
leave a tree with no panel at all where this keeps one: an area that loses its last panel keeps an
empty group, so the sidebar stays on screen as somewhere to drop a panel back into. Two behaviours
of freya's are kept: a split left with one child collapses into it, and a lone panel at the root is
never removed.

**The content area is the app's own**, `ContentArea` over `Strip` (`src/ui/strip.rs`): a bar of
chips over the tab on screen, and nothing that can be folded, split or dragged out of. What the dock
used to buy -- one answer to "which document is active", and a strip that closing the last document
cannot fold away -- the strip states outright, where the dock needed a designated panel, a
`tidy` exemption and an `on_drop` that refused a document anywhere else. `Tab` is two-kinded,
`Tab::Document(DocId)` and `Tab::Page(Page)` for Project, Settings and the Scratchpad, because a tab
is `Copy` -- a list's key, a menu row's capture -- and a `Document` is not. A close never walks the
bar through intermediate states: `Strip::close` takes the predicate, works the landing out with
`tabs::landing` before anything is removed, and leaves the tab on screen alone when it survives.

A tab's header is `chip`, hover state and × included, wrapped in a `TabHeader` that owns the hover
and is keyed by its tab. **The chip activates its own tab**, freya's docking having been what did
that before -- it wraps a header in a `DropZone` around a `rect().on_press(set_active)` around a
`DragZone` -- so the press handler calls `raise_tab` and then asks whether it was a **double press**
(`EventsCombos::pressed`, freya's own count of 500 ms and 5 px), which promotes the temporal tab.
The × still has to `stop_propagation`, now so the press does not reach the chip under it and switch
to the tab being closed. The temporal tab is told from one that stays by its name being **italic**
(`font_slant`) and by nothing else, the chip reading the flag out of the table beside the document.
Every tab has a ×, pages included, because there is a way back to one now: the **menu at the top
left of the window** (`PagesButton`), which is the whole of it. It lists all three and marks the
ones that are open rather than listing only the closed ones -- a menu whose rows come and go is one
a reader has to read every time, where a list that is always the same three is one they learn -- and
picking an open one shows it. A page opens **beside the tab on screen**, the way anything else the
reader opens does. What a closed page was showing is state at the root of the app, so closing one
loses nothing: a build or a run it started goes on, and it comes back as it was.

**The × is a control of its own**, `TabClose`, and a component rather than another line of `chip`
for one reason: the hover has to be *its*, freya has no `.hover()` pseudo-state, and the `use_state`
with `on_pointer_over`/`on_pointer_out` around it cannot run in a helper. That is why the × reaches
`chip` as an element already built rather than as an `on_close` handler. Two things follow from its
being a control. It is **a target you hit rather than one you aim at**: a `close_target()` square
centred on the glyph, four pixels of air on every side, capped at the row so the close never decides
how tall the bar is. The square is written as `close_glyph() + 8` rather than as a share of the row,
so the air is what stays fixed when the font or the row moves. The × is drawn a third larger than
the interface font it sits beside: it is a mark and not a letter, and at the text's own size the
multiplication sign looks like a scratch on the tab. And it says under the pointer that it is the ×
and not the tab: `close_hover_bg` behind it and the glyph up from `address_fg` to the interface
text, while the tab under it stays lit. The two are told apart by the wash being the deeper step,
not by the tab going out. It closes the tab itself rather than taking a handler, a `Component` being
`PartialEq` where a closure is not: the `DocId` is the prop and the five states a close needs come
from the contexts, the same ones the header reads a step above it.

A right-click on a chip opens a menu: **Close**, then **Close other tabs** where the tab has
company, and then, for a document, the two rows about the file it is a place in. **Close other
tabs** is `close_others`: the tab it was opened on stays and every other tab goes, a page as
readily as a document, since what the reader pointed at is the bar. It is its own function rather
than `close_tab` in a loop, because each of those would work out a landing of its own and walk the
bar through every intermediate state. **Add bookmark** / **Remove bookmark** is the same `bookmark_item` the
sidebar rows and the instruction rows use (`agents/Sidebar.md`), for the tab's own document, and a
page has neither it nor **Show in file manager**, being no place in a file. The close-others row is
left out when nothing else is open, rather than drawn as a row that would do nothing,
and the chip asks the strip for that at the **press**: whether a tab has company is not something a
chip draws, so subscribing to the strip for it would re-render every tab whenever any one of them
opened.

**Show in file manager** is the third, the same `reveal_item` a Files row's menu ends with
(`agents/Sidebar.md`), on `Document::file`: the binary for the two assembly-driven kinds and the
source file itself for a file. It calls `src/reveal.rs`, which is a list of programs per platform
run in order until one works, **on a thread of its own** -- one of them is a spawn and a wait, and
freya's executor is the UI thread. On the freedesktop desktops the call is on
`org.freedesktop.FileManager1` on the session bus, made through `gdbus` or `dbus-send` rather than
by linking a D-Bus library; macOS is `open`, and Windows `explorer`, whose exit status is 1 either way and so is judged by
whether it started. The path is made absolute and its trailing separator
dropped before the URI is built, a relative one there naming a host rather than a file, and encoded
a byte at a time, which is what lets both D-Bus calls take it unquoted. `explorer` is the one that
does need quoting, its path going inside the switch it parses itself, and the backslashes ending a
path are doubled there, `CommandLineToArgvW` halving a run of them before a quote.

**Showing a folder means opening it, and every rung of every ladder is told which kind it has.**
Each platform has one call that *reveals* -- the window around what it is given, with the item
picked out, which is what showing a file means -- and given a folder that call opens the parent,
which is not what showing a folder means. That was the first version of this and it was wrong on
all three. So each has a second spelling for a folder: the D-Bus interface has `ShowFolders` beside
`ShowItems`, macOS drops `-R`, and Windows drops `/select,`. The freedesktop pair is checked
against a live session bus -- the handler runs as `dolphin --new-window --select <path>` for the
first and `--new-window <path>` for the second. `xdg-open`, the last word when no D-Bus call
answers, picks nothing out and can only open a window, so it opens the window each call above ends
at -- a file's folder, and a folder itself. **When nothing answers the reader gets a box saying so**, the kind a panic uses: they
pressed something, and an item that does nothing at all leaves them wondering whether the app
heard.

The bar scrolls horizontally, because documents are opened by the dozen; a sidebar group's bar is a
plain row, seven panels always fitting. Two things bite there. freya appends one child more than there are tabs, a
`rect().expanded()` drop zone for "past the last tab", and `expanded()` is meaningless inside a
horizontal scroll view, so it is given a width of its own. And a tab's name is elided **by character
count in Rust**, where every other truncation is a width: a `maximum_width` anywhere inside one
makes it shrinkable, and a horizontal scroll view measures children against the space *left*, so
tabs past the edge get no width and draw as a bare ×. Do not "fix" that back into a width.

**A tab is named after the function, not after the whole demangled name** (`src/naming.rs`,
`short_name`). The name a tab is given is its last two path segments and nothing else: generic
arguments go however deep they nest, `<Vec<T> as IntoIterator>::into_iter` is `Vec::into_iter`, a
C++ argument list and the `const` after it go, and rustc's legacy `::h<hash>` suffix goes with them.
The one thing kept beyond two segments is the closure a symbol *is* (`render::{closure#0}` is not
`render`), and only the innermost of them. The character elision above is still there and still the
last word, for the names that are long anyway.

**A name the app made up is not a path and is left whole.** `<entry point>` read as a path is a
`<Type as Trait>` qualifier, which left a tab saying `point`, and `<function 0x140001000>` one
saying `0x140001000`. What is tested is the shape rather than those names: one angle-bracket
group, closed, with nothing on either side of it. A real name opening with `<` is a qualifier, and
the `::` of what it qualifies puts the group's end before the name's, so the two cannot be
confused; and a made-up name added later (the scratchpad's `<pad-3>`, `agents/Scratchpad.md`) is
covered without being named here.

It is real parsing and not a `rsplit("::")`: `::` appears inside generic arguments, `operator<<`
writes an angle bracket that opens no group, `fn(*mut c_void) -> *mut T` writes one that closes
none, and an `extern "C"` puts a quoted run in the middle of a type. So it is a scanner,
framework-free with its own `tests.rs`, written against names taken out of that binary. It lives in
the app rather than beside the demangling in `analysis`, because the crate has no use for it: it
hands out the name the file states and the name the demangler made of it, and *how much of one to
draw* is a question only a view has. `entry_text` is where it is applied and is the one spelling a
document tab and a History row share; `entry_name` beside it is the whole name, which is what the
tooltip says and what the History filter matches, so a generic argument no tab draws is still
something a reader can search for.

**A document's two sides live inside its tab.** `Tab::Document` renders the two panes in a
`ResizableContainer`, not a nested `DockingArea`, which is a great deal of machinery for a two-way
split. The cost is real and was taken deliberately: **the Source pane is no longer independently
arrangeable**, since it is inside a document rather than beside one. Each pane takes its `Document` as
a prop rather than reading `Active`, which is both synchronous and honest: only the active tab's
content is mounted, so a pane is only ever built for the tab it belongs to.

**Which pane comes first is the document's kind**: the side a tab is driven from leads, so
`AssemblyPane` is on the left in an assembly-driven tab and `SourcePane` is on the left in a
source-driven one -- and a tab whose following pane has been put away, by the toggle on either bar
or by its file having no assembly side to show, is the leading pane alone, with no container and no
handle (`following`, `agents/Panes.md`). `DocumentBody` is the only thing that knows this. The
panes themselves are handed no side and read none, so the swap is the order of two `.panel(..)`
calls and nothing else.
Everything the two panes share is keyed by pane *identity* and not by position (`Pane`, `Owed`,
`Marks`, `AsmAt`/`SrcAt`), which is why swapping them moves no selected run, no pair, no owed scroll
and no kept row. The panes are two different component types, so a swap unmounts and remounts both;
their rows come back where `use_kept_position` puts them.

That unmounting is why the split ratio is held at the root (`SplitRatio`, with `Splits` the shared
`ResizableContext` it is read back out of). A `ResizablePanel` registers at its `initial_size` in a
`use_hook` and *removes* its entry in a `use_drop`, so even a shared context comes back holding the
initial sizes under new panel ids. What survives is a number the app keeps, fed in as `initial_size`
and written back out while the split is on screen. It is one number for the app and not one per
document: per-document would be a third `Positions`-shaped map to forget in `close_tab`, for a
number nobody asked to differ per document. That number is **the leading panel's width and not the
assembly pane's**, the one thing here deliberately kept by place rather than by pane. Both readings
are coherent and this one moves nothing on screen: switching from an assembly-driven tab to a
source-driven one leaves the handle exactly where the reader dragged it, where keeping it by pane
would throw the two widths across the split at every switch of kind. The registry agrees with it by
construction, since `apply_resize` speaks positions too: panel 0 is the leading pane in both kinds,
so a drag means the same thing either way round.

**A document is a place in a binary or a file; everything else is a view.** Decide nothing about
this again. A document tab stands for a `Document` and never anything else, and that is what lets five
separate things work without a case each: the Assembly *and* Source panes both render "the active
tab", the record of visits holds it, `SavedDocument::from_document`/`::resolve` write it down and
find it again after a restart, `close_binary` knows which tabs a closing file takes with it, and
`entry_text` knows what to call it. A project view, the settings page and a scratchpad's editor are
none of that: they resolve against no object, they are no file on disk the panes could open, there
is one of each rather than many, and neither pane could draw one. So they are **pages**, the other
arm of `Tab`: a tab like a document's, drawn from state that lives at the root rather than in it. A
`Document` variant for a page was the alternative. It buys a tab in a strip nothing else would put a second
entry in, at the price of five answers nobody wants: what `resolve` does with it after a restart,
what `Document::in_file` says when a binary closes, what the panes draw for it, what the history
means by a "place" that is not one, and what a saved document spells it as. `Document::Code` is not
that: an object's code *is* a place in a binary, the whole of it, and every one of the five has an
answer that is the object's own, which is why it is a document and not a mode of the object's tab.
Persistence follows from the same sentence: a page is not a document, so it is **excluded** from the
saved documents and `SavedDocument` needs no answer for it. What a scratchpad *builds* needs no rule at all: the artifact goes through
`open_files` like any other binary, and its functions are ordinary tabs.

**Each place on each tab's trail remembers where each of its sides was left.** A pane has one
`ScrollController` and shows one tab at a time, so left alone it hands the tab arriving whatever
offset the one leaving had. `AsmAt`/`SrcAt` are two root `Positions` maps beside `Open`, **both
keyed by an `Entry`**, the tab's `DocId` and a `Stop` on its trail -- a document, plus where in it
the tab was: the address for an object's code, the line for a source file, and neither for a
document opened at no place in particular. So an entry means "this side of this place on this tab"
for exactly as long as the tab is open and the place is on its trail, and going Back comes back to
the rows that were left. **A place and not a document** is what makes following a link inside the
unified view, or a name inside a file, a step Back returns from: the two addresses, or the two
lines, are two entries, and a step between them is a switch to every pane, which is the whole of
the behaviour -- there is no second mechanism for moving the view inside one listing. Which is
also why every key is built from the stop the tab is **at** (`place_at`) and never from its
document: a key made of the document alone names a place the trail does not hold, and the two maps
it is the key to answer "never seen" and "closed tab" to that -- a position that reads as the top
of the file and a write that is silently dropped. `use_kept_position` is the whole of the behaviour,
called once by `InstructionList` and once by `SourceList`, each handed its tab's id as a prop from
`DocumentBody`. Which place a listing's row is filed under is `asked_of` the question that listing
answers, never the place the app is showing: while the worker catches up the pane is drawing the one
being left, and for a source-driven tab the question's place is the file and not the resolved
symbol, which is very likely on no trail at all. Navigating in place is not a switch of tab:
`DocumentBody` reads the table, so a push re-renders it and the panes are handed the new document as
a prop with their controllers kept, and the hook's switching arm files the row of the place left
under that place's own entry (still on the trail, so still `contains`) before putting the arriving
one back. Keying the source side by the *file* instead made two functions compiled from one file
share a position they have no reason to share. What is kept is a **row**, clamped to what the tab
holds *now*, so a rebuilt binary or a shortened file cannot come back past the end. A tab nothing is
remembered for opens at an **opening row** the caller hands in: `0` for the Assembly pane, whose
first row is the symbol's own first line, and the symbol's own line for the Source pane
(`opening_row`, off `SymbolLines::line`). A remembered row always wins over it, so it is the first
open this answers and not every one. An opening row of `0` is left alone rather than scrolled to,
since the effect runs a beat after the first render and setting the offset the pane already has
would undo a wheel that got in. Three things are load-bearing. Reading the controller's position
(`<(i32, i32)>::from`) is a `State::read`, which is what **subscribes the effect to the pane's own
scroll**: every position is written down as it happens rather than on the way out, which is what
survives the window merely being closed. The tab the controller is *holding* is tracked in the hook,
in an `Rc<RefCell>` and not a `State`, since nothing renders from it. That is because it is not the
tab the app is showing during the one run that has to move the view, and every write goes under the
held one. And a reveal a run is owed (`Picked::owed`) **wins** over a remembered position because
the same effect makes both: `use_kept_position` is handed the pane's reveal as a closure and asks it
first, applying the remembered row only when no scroll was made. The two *are* owed at once (a
Locations row opens a symbol on a line, so the tab changes and the arriving one is owed a reveal),
and two effects' scrolls land in whichever order the runtime wakes them. With the reveal first, it
had marked itself made by the time the kept row was put over it, which reset both panes to the top.
One effect has one order, and when a reveal scrolls, the effect wakes on that scroll and records
where it landed. **The reveal and the opening row are read out of a cell the pane rewrites every
render**, not passed into the effect: `use_side_effect_with_deps` builds its callback once in a
`use_hook` and refreshes only the deps, and a tab handed another document is not mounted again -- a
link followed in place, a search hit shown in the temporal tab -- so a closure kept from the mount
measured the row it owed against the file that render drew, refused it for ever, and left the pane
to fall back to the opening row. **A tab arriving with a landing on its way goes to the row the
landing names as it draws it**, and holds the move it would otherwise make until the landing has
been spent. `use_land` turns a landing into a run two passes after the switch reaches the hook -- it
runs off `Active`, and that is a memo -- so a pane that waited for it drew the arriving document at
the outgoing place's offset until then, and one that made its move first drew it at the top of the
file. The hook asks the pane to take the landing instead (`coming`), which the source pane does for
a line of the file it is drawing, with the same `reveal_row` the run makes later, so the run finds
the row already there. A landing the pane does not take -- a door that knew only an address, or one
meant for the other pane -- leaves the move held rather than made, since that pass may still plant
this pane a run. Nothing strands the pane on the offset of the tab it left: a landing is only ever
left by a move that changes the place, and that arrival is what spends it.
`close_tab`/`close_others`/`close_binary` forget every position of a tab's entries with the tab, by
id, and `close_binary` forgets those of the entries it takes off the surviving trails too. That is
not tidiness: an `Assembly` entry holds the `Arc<Object>` it points into, and the hook is handed
`Docs::contains` precisely so that the run *after* a close, still holding the place that has gone,
cannot put it straight back. The closers forget the driven lines with them, which *is* tidiness: a
`Source` key holds no object, so nothing is being held up. **Each place remembers what was selected
in each of its panes** the same way: `MarksAt` is a fourth map keyed by the same `Entry`, holding
both panes' runs as they were left (the caret and the selection, no gesture, nothing owed) and, for
an object's code, the place each row of the assembly run stood for. `use_land` (`agents/Panes.md`)
saves under the entry being left and restores for the one arriving, a landing winning over what was
kept and a kept run over a source-driven tab's driven line. It is forgotten in the three closers
with the other three and by `Docs::contains` for the same reason, and it is the one of the four that
`session.toml` never sees.

**A code tab's place is an address, in the same map type.** The listing of an object's whole code is
counted afresh with every answer that lands (`agents/Panes.md`), so a row there means nothing for
long. `CodeAt` is a `Positions<Entry, Spot>`, the map generalised over its value, `row`'s clamp
being the one rows-only answer. It holds the placed address at the top of the pane and how many rows
past that address's own row it was, since the rule over a stretch, the blank under it, its header,
its labels and its first instruction all sit at one address. It is forgotten in the three closers with the other two and travels in
`ProjectStates` as they do. `use_kept_place` in `src/ui/section_view.rs` is its `use_kept_position`,
and the differences are the point. The map is **read** and not peeked, so a place written from
outside while the tab is on top is answered (the run that wakes on its own write finds nothing moved
and writes nothing). The rows the place is re-applied against are produced in the same run, so a
chunk landing above the reader never draws one frame at the old offset. And a move it makes is
re-issued until a run finds the view there, a few times and no more, since a `VirtualScrollView`
clamps a target past its content. "There" is **the row the place names** (`row_of`: `Rows::row_for`
plus the rows past it, which rounds an address inside a row down to the row at or below it) and not
the spot derived from the offset, since a place written from outside can be an address inside a row
(a call's target in the middle of an instruction) that no derived spot ever spells. And when the
rows are rebuilt under the view, the place put back is the map's own where the view was at it as
well as the old rows could tell, and the derived one otherwise: a place written from outside is
exact, a row's share of an undecoded stretch is a guess, and re-applying the guess landed a target
in a stretch the worker had not reached on the row nearest its guess rather than on its own
instruction. The same hook plants the caret a door left for the listing (`Planting`,
`agents/Panes.md`), owing it no scroll: the place is what moves the view there, and is authoritative
in this pane. A place written from outside is answered **once, as a change of the map's value**, and
never as "the map disagrees with the view". A listing of a large binary is millions of rows and tens
of millions of pixels down, past where freya's `f32` scroll offset holds a pixel
(`notes/upstream/freya.md`), so the view cannot always be put exactly where the map says, and a rule
that answered the disagreement moved the view into a run that found the same disagreement, for ever:
the freeze the first scroll through the app's own binary produced. The hook's own arithmetic is
`f64` for the same reason. The list is mounted from the first frame with no rows for the same
reason: the view resets the controller as it mounts, and a move made before that was read back as
the reader's own scroll and written over the place they asked for.

**Opening a binary is the one path in, and it streams.** `open_binaries` is `close_binary`'s
opposite number and the only thing that ever adds to `Objects`. The toolbar's Open, a session
restore and a scratchpad's rebuild all go through it, so they cannot differ about what opening a
file means. It is a `std::thread` and an `async_channel`, `use_analysis`' shape, but the answers
come back one at a time: `Loads::begin` registers the paths **before a byte is read**, so the
sidebar has a row for the whole of the wait rather than from whenever the first answer lands, and
`take_load` writes each batch of objects in as it arrives. The channel is **unbounded and drained in
batches**: unbounded because backpressure is exactly wrong here (the worker is the thing that should
run flat out) and batched because a write per member is a re-render per member, which for an archive
whose members parse in a millisecond is a hundred renders nobody sees. **An object nobody asked for
any more is dropped, not prevented**, which is `use_analysis`' rule in a second place and has to be:
the worker is already parsing when the file is closed. It is checked against `Loads::holds`, the
load *and* the path, since a file closed and reopened while the first parse ran is two loads and
only the second one's objects belong on screen. That is the whole reason a load has an id where an
analysis answer needs none: an answer is about a `Symbol` that already existed, while a load is
about work that has produced nothing to be identified by. `take_load` **returning** is what stops
the worker: it drops the receiver, the next send fails, and the walk breaks. What streaming buys is
not uniform. The 196-member rlib's first member is offered at 102 ms against the 685 ms the whole
file takes (debug build), while the 331 MB binary is one object and gains no object earlier at all;
there the win is the row, on screen from the click instead of an empty list for six seconds.
**Nothing further is opt-in**, and measuring says why: of a file's parse, the whole of what is left
after line info, the DWARF context and the disassembly were already made lazy is reading the bytes
and walking the symbol table, which is what the Objects and Symbols lists *are*. On the 331 MB
binary (release) that is 1.38 s of which 766 ms is the read and 286 ms the demangling; deferring the
demangling is the only lever there is, and it defers work until the first click on the object.


**Identity throughout the UI is `Arc` pointer identity**, not names or indices: list keys are
`Arc::as_ptr(..).addr()` and every prop `PartialEq` is hand-written in terms of `Arc::ptr_eq`. That
matters twice: duplicate symbol names across objects stay distinct, and `#[derive(PartialEq)]` on an
`Arc<T>` field would deep-compare on every parent render.


## Testing the UI

`freya-testing` runs the whole app (components, hooks, effects, layout, events) with no window, no
GPU and no event loop, on the test's own thread. It is a dev-dependency for `src/ui/tests.rs` and
for nothing else, and the binary's whole suite runs in under two seconds, so a test written to
settle one point costs less than a `cargo run` and a look and is worth writing even when it is going
to be deleted again. Keep the ones that pin a mechanism (that a component with no props re-rendered
because it read `palette()`, that a superseded answer was dropped) and delete the ones that only
proved the code just written does what it says.

What it can and cannot be asked is `agents/Headless.md`'s verdict, and `AGENTS.md` has the short
form.

`agents/Headless.md` is the full account, checked against `freya-testing` 0.4.3's sources rather
than its README: the runner's whole surface, the rule that a pass is *either* a render *or* a round
of task polling (which is why a change needs somewhere between *n* and *2n* `sync_and_update`s and
why the tests here loop rather than count), and the house rule that a headless test has to be made
to fail first on the *mechanism* it claims to test.
