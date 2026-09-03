# Appearance: palette, theme, fonts and the settings page

Every colour and every measurement, how a theme or font change repaints, where the desktop's
fonts come from, and the page the reader edits both on.

**One palette, one place.** Every colour is a field of `Palette` in `ui/palette.rs`, there are two
instances (`Palette::LIGHT` and `Palette::DARK`), and `palette()` is how anything reaches
whichever is current — no call site names a colour, and none of them changed when the second
palette arrived, which is what the indirection was for. The dark values are the light ones
**carried over**, not designed again: every relationship holds on both sides (the header a step
off the pane, each code colour keeping its hue and its place in the ordering), and the only ones that could not be flipped literally are the translucent washes,
which `blend` composites over the pane — the same alpha over a dark ground is a fraction of the
step it was over white, so each was judged as what it *comes out as*. Two tests hold that: a
contrast floor for every foreground on the surface it is really drawn on (3.0, not WCAG's 4.5 —
the light palette's address column and its comments are meant to recede and sit between 3 and
3.5), the × on a dock tab included, whose surface is its own wash composited over whichever of two
grounds the tab is on; and a visible-step floor for every wash over the row under it — a code
row's three being the pair's green (`pair_bg`, the other pane's selection mapped here), the
selection's grey (`row_select_bg`), and the deeper green a row that is both takes
(`pair_selected_bg`, held to a step well past the pair's, since a shadow over so pale a green
barely moved it), none answering to the pointer (`agents/Panes.md`) — and
the × required to move the tab it sits on further than
the tab's own hover moves the bar — the one wash here that has to be told apart from *another
highlight* rather than from a plain surface, since the pointer on the × lights the tab under it
too. Two colours are drawings rather than text and are held inside the contrast test to floors of
their own instead of to the 3.0: the gutter's branch line, which must not vanish into the pane and
must be lit louder than it is at rest, and `block_rule`, the hairline starting a basic block,
which must read against the pane and stay *quieter* than that branch line — it runs the whole
width of the listing where the stroke beside it is a few pixels long. The code colours are named for what they mean, not for the pane they came from, and
`Palette::syntax` maps `freya-code-editor`'s ~33 capture fields onto them. Beware
`resolve_capture_color`: it treats a capture whose colour equals `text` as unmapped and walks *up*
the dotted name, so giving a child field the text colour while its parent holds another silently
paints the child in the parent's colour — a property of which fields *share* a value, so a second
palette can break it by landing two colours on each other, and `captures_do_not_walk_up` asserts it
for both. Three of those fields used to point at colours the assembly side had brought with it, and
the result was that a Rust file read as two colours: `attribute` and `type` were both `keyword_fg`,
and `function` and `function.method` were `name_fg`, which is also the plain text, so a call site was
the colour of everything around it. Each has an entry of its own now — `attribute_fg` a plain grey
that recedes, `#[derive(..)]` being scaffolding around the code rather than code; `type_fg` a dim red,
so `struct Foo` reads as a keyword introducing a name and not as two halves of one word; `function_fg`
a blue with none of the address column's greyness — each written light-first and turned through the
background for dark like every other pair. `function.macro` went with them although the goal did not
name it: by the trap above, a child left on the text colour is painted in its parent's, so leaving it
alone would have made it `function_fg` silently instead of saying so. **The assembly side keeps the
five it had**, which was the open question and is a decision and not an omission: none of the three
has anything to name over there. `SpanKind` is a mnemonic, a prefix, a register, a number, an address
and glue — a listing holds no attribute, no type, and no call site that is not already a relocation
target, which is as often data as it is a function, is the one name in a row of registers, and has
`name_fg`/`name_hover_fg` and an underline to be told apart by. The split was for a *file* reading as
two colours; a listing never had that problem, and repainting the mnemonic to keep the two sides from
sharing would cost them the one vocabulary they are read in. So the three are source-only, and the
contrast test holds them on `pane_bg` alone beside the strings and the comments, with `attribute_fg`
additionally required to land *quieter* than the keyword it left, the punctuation beside it and the
plain text — a relationship rather than a value, receding being the whole of what it is for.
This is deliberately **not** freya's own theming — `ColorsSheet` names none of these
roles, and the source pane's colours cannot be read from the element tree at all, being baked into
a `SyntaxBlocks` when a file is *loaded*.

**A disabled control is derived rather than a field.** `dimmed(color, surface)` is the colour the
control has when it is live, faded into the ground it sits on at `DISABLED_ALPHA` through the same
`blend` the washes use — so a dimmed drawing follows whatever colour the live one is given, in both
themes, and there is no second value per theme to keep in step with the first. The toolbar's two
history buttons are the only caller so far. Its test is a floor rather than a value, the way the
branch gutter's is: `dimmed` must land strictly quieter than the live colour and no closer to the
surface than 1.5, in both palettes.

**A theme switch repaints by being asked for a colour.** `palette()` reads a thread-local
`State<Appearance>` and hands back a `&'static` to one of the two `const`s, so `State::read`
subscribes whichever scope is rendering: *asking for a colour is what subscribes a component to the
theme*, exactly once, wherever it sits and whatever built it. The two alternatives were weighed and
lost. Threading a context read through the call sites is freya's own idiom but impossible here — a
hook must run unconditionally in a component body, and `palette()` is called from free functions,
from `if` arms, from render callbacks and from `Highlighted::new`, which is not a component; it
would be a line in each of the twenty-one components with the free functions still on a static, and
a forgotten line would be a patch of the old theme. Re-rendering from the root does not work at
all: freya marks a child dirty only when its props change (`freya-core`'s `runner.rs`) and every
view here is a unit `Component`, so forcing it means a `key` that remounts the tree and throws away
the three filters, the objects tree's folds and every scroll controller. The cost of what was
chosen is that `palette()` is a thread-local lookup and a subscribe rather than a constant — tens
of nanoseconds against perhaps a thousand calls per full render. **`set_appearance` is the only way
to change it**, because the switch also has to `HIGHLIGHTED.clear()`: that cache holds
`SyntaxBlocks` with colours already resolved into them, so its entries are not stale but the wrong
theme, and nothing a re-render does would repaint them. The clear is inside the setter
(`set_if_modified_and_then`) rather than at a call site, so it cannot be routed around. The
appearance is resolved by `use_theme` at the root of `app()` from two inputs — the stored choice
(`settings.rs`, read once: it is a file) and `Platform::preferred_theme`, which freya keeps from
winit's `Window::theme()` and re-sets on the OS's `ThemeChanged` event — through the pure
`resolve_appearance`, where only `Theme::Desktop` is a question at all. **Not a `use_hook`**: the
preference is a `State`, so *reading* it subscribes the root and a desktop that goes dark while the
app is running repaints, which the subprocess this replaced could never do. It resolves in the
render body rather than in an effect, because an effect lands a frame late and a frame late on a
dark desktop is a white window flashing; the write is idempotent, so the frame it costs is the one
after an actual change, and the two-hop path (the platform wakes the root, the root's write wakes
everything that drew a colour) is what the headless test spells out. The control for the choice is
the settings page's three buttons, which write the choice and nothing else — `set_appearance` stays
the one writer. The one thing `text_fg` adds is the
interface text: set once on the root rect and *inherited*, since freya resolves an unset `color`
from the parent's, and it is `BLACK` in the light palette because that was already the default.

**Fonts.** `fonts.rs` asks the desktop for its interface and fixed-width fonts.
**Which desktop to ask is a runtime question**, not a compile-time one — one Linux build
runs on both — so `XDG_CURRENT_DESKTOP` only *sorts* `kreadconfig6`/`kreadconfig5` (KDE's `font`
and `fixed`, a comma-separated spec) against `gsettings` (Gnome's `font-name` and
`monospace-font-name`, a quoted Pango `Family Size` whose family can hold spaces and trailing style
words), and the other is tried anyway: a tool that is not installed is already a `None` here.
Gnome's `text-scaling-factor` multiplies the point size, because it is *how* Gnome says "make text
bigger" — `font-name` keeps its nominal size and the accessibility slider moves this instead;
winit's own display scale is separate and multiplying both would compound. Windows is `SystemParametersInfoW(SPI_GETNONCLIENTMETRICS)`
for `lfMessageFont`, over a `windows-sys` pinned to the 0.61 the lock already holds transitively
so no fourth copy of it appears (`cargo tree -d`). Its `lfHeight` is divided by the screen DC's
`LOGPIXELSY` rather than by `SystemParametersInfoForDpi`, deliberately: that function and
`GetDpiForSystem` only exist from Windows 10 1607, and `windows-sys` links its imports statically,
so naming one would turn "no font setting" into a process that will not start — winit itself
`GetProcAddress`es that family for the same reason. The pairing is also what makes it *correct*:
both the metrics and the DC's DPI are virtualised into whatever DPI space the process is in, so
they agree without this file knowing which that is. Windows stores no desktop-wide monospace font
at all, so that half stays `Consolas`. Each font is then a *chain*:
the desktop's answer in front of the platform's own (`Segoe UI`/`Consolas`,
`.AppleSystemUIFont`/`Menlo`, else the generic `sans-serif`/`monospace` that skia resolves through
fontconfig). A family named with no usable size keeps the family and takes the app's default size.
The platform font must be named — freya's global fallbacks are all proportional, so a
chain resolving to nothing silently takes the assembly view out of a monospaced face — and must
equally not name *another* platform's families, which had a Windows box rendering in DejaVu. The
one font freya will not let an element set is the tooltip's, hardcoded in its theme, so
`interface_theme` provides a `Theme` with `tooltip.font_size` at the interface size — on top of
freya's own `light_theme()`/`dark_theme()` sheet, chosen by the appearance, which is the one place
freya's theming is used for colour: the filter boxes, scrollbars, resizable handle, tooltips and
context menu read their colours from it and from nothing else, and a white text box on a dark pane
is not a theme switch. Overriding anything in that sheet is `Theme::set` with a whole
`*ThemePreference` under its string key, each field a `Preference` that is either `Specific(v)` or
`Reference("name")`, and a reference resolves against freya's own `ColorsSheet` and nothing else:
it cannot be pointed at a `Palette` colour, a name that sheet does not know comes out silently as
`primary`, and a reference on a field that is not a `Color` panics where it is resolved
(`freya-components`' `theming/macros.rs:288-395`). Hence the tooltip's size, an `f32`, is a
`Preference::Specific`.

**A font change repaints the same way a theme change does, and moves the rows with it.** `fonts()`
in `ui/metrics.rs` reads a thread-local `State<Arc<Fonts>>` exactly as `palette()` reads the appearance, so
*asking for a font is what subscribes a scope to it*; `set_fonts` is the one writer, and unlike
`set_appearance` it has nothing to invalidate beside it, a cached `SyntaxBlocks` carrying colours
and no font. The readers are the two row heights, `icon_size`,
`FontExt::assembly_font`, the root rect's own `.font(&fonts().ui)` and the
tooltip's `font_size` in the root's `Theme` — that last one is the only place a change has to be
*carried* rather than picked up, freya's theme sheet being a value, so the root's effect has the
interface size in its deps beside the appearance. `ROW_HEIGHT` went the same way and became a
function: one font's size plus `ROW_LEADING` (12, which is exactly what the old constant's 26 was
over the 14px fixed-width default). That was 9c's real
decision, and the alternative — a page offering a 20pt assembly font and drawing it clipped inside
a 26px row — was worse than the work. It is safe because the scroll view's `item_size` and its
rows' own height are read in the **same render pass**, so they cannot see different numbers, and
because the per-tab positions 8b saves are *rows* rather than pixel offsets. The floor
(`MIN_ROW_HEIGHT`) is against a hand-edited `settings.toml`, where a size of 0.1 is positive enough
to pass `FontSetting::size` and would make `item_size` a fraction of a pixel.

**One more number the fonts drag in is the device pixel grid.** A row height is a function of a
font and the branch gutter's strokes are drawn at fractions of a row, so where they land is
whatever the font left behind — and freya rounds nothing between the layout and Skia.
`pixel_grid()` sits beside the row heights in `ui/metrics.rs` and answers a `Grid`
(`src/pixels.rs`) off freya's `Platform::scale_factor`, which is a root context and so subscribes
whoever reads it, exactly as `fonts()` and `palette()` do. Only the gutter asks so far; anything
else drawing a hairline should (`agents/Panes.md`).

**And it is two functions, because no row mixes the two fonts.** `list_row_height` follows
`fonts().ui` and `code_row_height` follows `fonts().mono`; both are `row_height_for`, so they can
differ only in which font they ask about. It was one number — the *larger* of the two sizes — and
the `max` read as a constraint while being nothing of the kind: every row in the code panes sets
`assembly_font()` on itself and on each of its spans, every sidebar row sets nothing and inherits
the interface font from the root, and no row anywhere draws in both. So the one number was two
lists sharing an answer, and raising either font padded the rows drawn in the other — an 18pt
assembly font made the objects tree, the symbol list, the tab bars and the chips 36px tall for a
12px font. Which height a site takes is decided by **the font its rows are actually drawn in**, and
getting one wrong is a misalignment that reads as a rendering glitch: the code height goes to the
instruction and source rows, the editor's line height, a run's output rows and the `item_size` of
those views; the list height to everything else, the filter bar's own height, `toggle_size` and
`icon_size`'s cap included, since a filter bar sits over a sidebar list and there is no filter over a code pane.
`row_at`/`row_offset` are the **code** panes' conversion alone — `use_kept_position` and
`reveal_row` are called by `InstructionList` and `SourceList` and by nothing else — so the old
"one conversion for every pane" argument for a single height went with the `max`. One thing did
move: at the app's own defaults (9pt interface, 10.5pt fixed-width) a sidebar row is 24px where it
was 26, because 26 was the *mono* font's number and had never been anything else. No floor holds it
at 26; that would be the same coupling under another name.

**The Settings page** (`Tab::Settings`) is where the theme choice and the two font overrides are
edited. `Prefs` holds an `EditedSettings` — `OpenProject`'s shape, and for its reason: a family is
a `String` here and an `Option<String>` in the file, an empty box **is** how a reader says "I have
not said", and `EditedSettings::settings` is the one place the two spellings meet. A *size* gets no
such treatment: it is a stepper and not a text box, so there is no half-typed state and no third
answer for text that is not a number — which also keeps a reader from spending a keystroke at 1pt
on the way to typing 12. `use_settings` is the whole of the wiring, and the write it makes is
compared against **what the file currently says** rather than against what was loaded, `Saves`'
rule: a fixed baseline would leave the file holding the middle answer when a reader changes a
setting and changes it back, and comparing at all is what stops a run that never opened the page
from creating `settings.toml`. `use_settings_with` takes the write as an argument, since the real
one edits the settings of whoever runs the tests.

**An override is drawn differently from the value it would replace**, which is the goal's own
words and the reason `settings.rs` keeps `None` as a real third state. Three cues, deliberately
more than one: the field's *name* is interface text when the reader set it and `address_fg` when
they did not; the *value* is real text in the box against a placeholder showing what is being
inherited (`fonts::inherited`, so what is shown is by construction what would be used, the
platform's own family and the app's own size included); and the **Clear** button is there only when
there is something to clear, which is also the only way back to unspecified — a family box can be
emptied, a stepper cannot.

