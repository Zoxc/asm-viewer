//! Every colour the window is drawn in, and the read that makes asking for one a
//! subscription to the theme.
//!
//! The two palettes and the mechanism reaching them are one file because the mechanism is
//! the only way in: `palette()` is a read of the appearance, so a colour taken any other
//! way would be a patch of the window that a switch never repaints.
//!
//! The three helpers at the end are here for the same reason, though they sat far from the
//! palette while this was one file. Compositing a wash over the row under it and saying
//! which colour a span's kind takes are colour *decisions* -- they read the palette and
//! answer with another colour -- so they belong beside the values they compose rather than
//! beside the rects they are eventually drawn into.

use super::*;

/// Every colour the app draws, in one place.
///
/// There are two instances, [`Palette::LIGHT`] and [`Palette::DARK`], and [`palette`] is
/// how anything reaches whichever is current. The indirection was more of the point than
/// the struct was: the dark mode `Goals.md` asks for under *UI* is the same design at
/// dark-mode lightness rather than a second one, so it is one more `const` beside the
/// first and no edit at all to the call sites that name a colour.
///
/// **The dark values are the light ones carried over, not chosen again.** Every
/// relationship in the light palette is a relationship in the dark one: the header sits
/// one step off the pane on both (below it in light, above it in dark, which is the same
/// step through the same background), the pin is the focus at more alpha, the branch
/// hover is the branch line brightened towards the address column's hue, and each of the
/// eight code colours keeps its hue and its place in the ordering. What could *not* be
/// carried over literally are the translucent washes: `blend` composites them over the
/// pane, so the same alpha over a dark ground is a fraction of the contrast it had over
/// white, and each of them was re-judged against what it comes out as rather than
/// inverted. The tests below are what hold both halves of that -- a contrast floor for
/// every foreground on the surface it is drawn on, and a visible-difference floor for
/// every wash over the row under it.
///
/// It is also not freya's own theming (`Theme` / `ColorsSheet` / `define_theme!`).
/// `ColorsSheet` has a fixed set of fields naming none of these roles, a `define_theme!`
/// per row component would be styling machinery for elements nothing outside this file
/// ever styles, and -- the part that settles it -- the half of the palette this step is
/// about cannot be read from the element tree at all: the source pane's colours are baked
/// into a `SyntaxBlocks` by a highlighter that runs when a file is *loaded*, so they have
/// to be plain values available outside any component. See [`Palette::syntax`]. freya's
/// own theme is still given the matching *sheet* (`interface_theme`), because the handful
/// of freya components in the window -- the filter boxes, the scrollbars, the tooltips --
/// read their colours from it and nowhere else.
pub(crate) struct Palette {
    // Surfaces and chrome, carried over from the original floem styling.
    /// A pane's own body, and the tab header above the active one, which is white so
    /// that it reads as the top edge of that body rather than as part of the tab bar.
    pub(crate) pane_bg: Color,
    /// The interface text: every label that does not ask for a colour of its own. Set
    /// once on the root and *inherited* -- freya resolves an unset `color` from the
    /// parent's (`freya-core`'s `TextStyleState::from_data`), so the whole chrome follows
    /// from one call there. It is black in the light palette, which is exactly what the
    /// default was, so the light theme is unchanged by this field existing.
    pub(crate) text_fg: Color,
    pub(crate) header_bg: Color,
    pub(crate) hairline: Color,
    pub(crate) selected_bg: Color,
    pub(crate) object_hover_bg: Color,
    pub(crate) symbol_pane_bg: Color,
    pub(crate) symbol_hover_bg: Color,
    pub(crate) asm_pane_bg: Color,
    /// The pointer's own hover, on an instruction row and on a source line alike: both
    /// panes show code, and one colour for "the pointer is here" reads across them as one
    /// gesture.
    pub(crate) code_row_hover_bg: Color,
    /// The cross-view highlight: this row is what the row the pointer is on maps to on the
    /// other side. Weaker than the hover, and translucent like it, so a row carrying both
    /// comes out as the hover *over* this rather than as one or the other -- see `blend`.
    pub(crate) line_focus_bg: Color,
    /// The same highlight, made to stay by a click. The one colour in two strengths rather
    /// than two colours, because it is the one relationship: a pin is the position the
    /// reader asked to keep, and the pointer wandering off to a second one must not make
    /// the two indistinguishable.
    pub(crate) line_pin_bg: Color,
    /// The wash over the half of a panel a dragged tab would land in.
    pub(crate) drop_preview_bg: Color,
    /// A Lucide glyph in a dock tab header. A step lighter than the title beside it,
    /// because the icon is what the eye finds the tab by and the word is what tells it
    /// apart: an icon as dark as its label competes with it at the same size. It is the
    /// palette's one chrome *foreground* -- every other colour in this group is a surface,
    /// the interface text itself being freya's theme colour and inherited.
    pub(crate) icon_fg: Color,
    /// A filter toggle that is on, and one the pointer is over. Two shades of the header's
    /// own grey rather than a colour of their own: a 22px square is small enough that "this
    /// one is pressed" has to be read from how dark it is, and against `header_bg` these are
    /// the two steps that are legible without looking like a third kind of thing.
    pub(crate) toggle_on_bg: Color,
    pub(crate) toggle_hover_bg: Color,
    /// The wash behind a relocation link the pointer is over, lightening whatever row
    /// background is under it.
    pub(crate) link_hover_bg: Color,
    /// A branch line in the assembly gutter, with its corner and its arrowhead. Quieter
    /// than anything it runs beside: the gutter is a diagram of the listing and must not
    /// compete with it for the eye.
    pub(crate) branch_fg: Color,
    /// The same line while a branch of the row under the pointer runs down it. One hue in
    /// two strengths, the way `line_focus_bg` and `line_pin_bg` are and for the same
    /// reason -- it is one relationship, drawn twice over -- and that hue is the address
    /// column's own, since a branch names a place in the listing exactly as an address
    /// does.
    pub(crate) branch_hover_fg: Color,
    /// The run of rows a reader has picked out to copy. Translucent like the two washes
    /// above and composited with them by `blend`, since a row can be selected, pointed at
    /// and pinned at once -- and a *grey* where those two are blue, because it answers a
    /// different question: the blues say what this row maps to on the other side, and this
    /// says what would land on the clipboard.
    pub(crate) row_select_bg: Color,

    // The code colours, shared by both panes. Which syntactic category takes which of
    // them is [`Palette::syntax`]; these names are the category rather than either pane's
    // own vocabulary, because they answer for both.
    /// Where a thing is: the instruction addresses, and the source line-number gutter.
    pub(crate) address_fg: Color,
    /// What is being done: mnemonics and prefixes, source keywords, operators and types.
    pub(crate) keyword_fg: Color,
    /// What it is being done to: registers, and source variables, parameters and fields.
    pub(crate) operand_fg: Color,
    /// A value written out: immediates, and source numbers, booleans and constants.
    pub(crate) literal_fg: Color,
    /// A string literal. Source-only, and one of the two colours the assembly side has no
    /// equivalent for at all.
    pub(crate) string_fg: Color,
    /// A comment. Source-only, and the other one.
    pub(crate) comment_fg: Color,
    /// The glue between the operands: brackets, commas, and on the assembly side the
    /// operand-size keywords (`qword ptr`) that are glue in exactly the same way.
    pub(crate) punctuation_fg: Color,
    /// A name that names one thing: a relocation target in the assembly, a function,
    /// method or module in the source. Also the source pane's plain text, which is what
    /// most of a line is.
    pub(crate) name_fg: Color,
    /// A relocation link under the pointer, and the underline drawn beneath it.
    pub(crate) name_hover_fg: Color,

    /// What a pattern that will not compile, and the reason it will not, are written in.
    pub(crate) invalid_fg: Color,
}

impl Palette {
    pub(crate) const LIGHT: Palette = Palette {
        pane_bg: Color::WHITE,
        text_fg: Color::BLACK,
        header_bg: Color::from_rgb(245, 245, 245), // WHITE_SMOKE
        hairline: Color::from_rgb(211, 211, 211),  // LIGHT_GRAY
        selected_bg: Color::from_rgb(211, 211, 211),
        object_hover_bg: Color::from_rgb(144, 238, 144), // LIGHT_GREEN
        symbol_pane_bg: Color::from_rgb(243, 243, 228),
        symbol_hover_bg: Color::from_rgb(226, 226, 205),
        asm_pane_bg: Color::from_rgb(248, 248, 248),
        code_row_hover_bg: Color::from_argb(160, 228, 237, 216),
        line_focus_bg: Color::from_argb(70, 120, 160, 220),
        line_pin_bg: Color::from_argb(120, 120, 160, 220),
        drop_preview_bg: Color::from_argb(60, 105, 89, 132),
        icon_fg: Color::from_rgb(90, 90, 90),
        toggle_on_bg: Color::from_rgb(196, 196, 196),
        toggle_hover_bg: Color::from_rgb(225, 225, 225),
        link_hover_bg: Color::from_af32rgb(0.6, 255, 255, 255),
        branch_fg: Color::from_rgb(176, 188, 202),
        branch_hover_fg: Color::from_rgb(90, 116, 148),
        row_select_bg: Color::from_argb(80, 96, 110, 128),

        address_fg: Color::from_rgb(118, 141, 169),
        keyword_fg: Color::from_rgb(116, 94, 147),
        operand_fg: Color::from_rgb(87, 103, 65),
        literal_fg: Color::from_rgb(80, 107, 135),
        // The two source-only ones, picked to sit at the same lightness and the same low
        // saturation as the five above rather than at a highlighter's usual brightness:
        // a terracotta, which is the one warm hue nothing else here uses, and a sage
        // grey-green well clear of the operand olive.
        string_fg: Color::from_rgb(148, 98, 74),
        comment_fg: Color::from_rgb(128, 148, 128),
        punctuation_fg: Color::from_rgb(102, 102, 102),
        name_fg: Color::from_rgb(50, 50, 50),
        name_hover_fg: Color::from_rgb(105, 89, 132),

        invalid_fg: Color::from_rgb(176, 0, 32),
    };

    /// The same palette at dark-mode lightness.
    ///
    /// Read it beside `LIGHT` rather than on its own: every value here is the one above it
    /// turned through the background, and where a light value is a step *down* from the
    /// surface it sits on, the dark one is the same step *up*. The ground is 30/30/32, a
    /// hair cooler than neutral the way white is not, and the pane relationships are the
    /// light ones reflected -- the header a step off the body, the assembly pane a step
    /// off the header, the symbol pane the one faintly warm surface it already was.
    ///
    /// The eight code colours keep their hue and their ordering: purple for what is being
    /// done, olive for what it is done to, blue for a value written out, terracotta for a
    /// string, sage for a comment, grey for the glue, near-white for a name and the slate
    /// blue for an address. They move to where a dark ground can carry them -- lightness
    /// up, saturation down a little, since a colour that is legible on white at 40%
    /// lightness is a glare at 40% on black -- and the *relative* order of their weights
    /// is unchanged, which is what keeps an instruction and a statement reading at the
    /// same density (5e).
    pub(crate) const DARK: Palette = Palette {
        pane_bg: Color::from_rgb(30, 30, 32),
        text_fg: Color::from_rgb(232, 232, 232),
        header_bg: Color::from_rgb(40, 40, 43),
        hairline: Color::from_rgb(62, 62, 66),
        selected_bg: Color::from_rgb(66, 66, 72),
        // The one hue in the chrome, and the one the light palette states loudest: a row
        // the pointer is over in the objects tree. Carried over means the same green at
        // the same distance from its surface, which on a dark ground is a deep one.
        object_hover_bg: Color::from_rgb(48, 92, 52),
        symbol_pane_bg: Color::from_rgb(38, 38, 33),
        symbol_hover_bg: Color::from_rgb(52, 52, 44),
        asm_pane_bg: Color::from_rgb(34, 34, 36),
        // The four translucent ones. Each is stated as what it should come out as over
        // the pane rather than as the light value with its channels flipped: `blend` puts
        // 30/30/32 under them, so an alpha that lightened white by a little darkens a dark
        // ground by nothing at all. The colour under the alpha is therefore *lighter* than
        // the ground here where it is darker than white there, and the alphas are up.
        code_row_hover_bg: Color::from_argb(110, 120, 160, 110),
        line_focus_bg: Color::from_argb(80, 130, 170, 230),
        line_pin_bg: Color::from_argb(140, 130, 170, 230),
        drop_preview_bg: Color::from_argb(90, 150, 130, 190),
        icon_fg: Color::from_rgb(160, 160, 160),
        toggle_on_bg: Color::from_rgb(88, 88, 92),
        toggle_hover_bg: Color::from_rgb(60, 60, 64),
        // A wash that *lightens* the row under it, exactly as in the light palette --
        // "raised" is lighter on both grounds -- but a quarter of the way to white rather
        // than three fifths of it, which over a row background of 30 is the same step of
        // about fifty levels that 0.6 over white was.
        link_hover_bg: Color::from_af32rgb(0.25, 255, 255, 255),
        branch_fg: Color::from_rgb(96, 108, 124),
        branch_hover_fg: Color::from_rgb(150, 178, 210),
        row_select_bg: Color::from_argb(90, 150, 165, 185),

        address_fg: Color::from_rgb(132, 156, 186),
        keyword_fg: Color::from_rgb(178, 150, 214),
        operand_fg: Color::from_rgb(158, 180, 120),
        literal_fg: Color::from_rgb(130, 175, 214),
        string_fg: Color::from_rgb(214, 150, 120),
        comment_fg: Color::from_rgb(128, 158, 128),
        punctuation_fg: Color::from_rgb(150, 150, 150),
        // A step *below* the interface text, as the light `name_fg` is a step above black:
        // the code panes are what the eye rests on and the chrome around them names
        // itself once.
        name_fg: Color::from_rgb(216, 216, 216),
        name_hover_fg: Color::from_rgb(190, 168, 224),

        invalid_fg: Color::from_rgb(240, 110, 120),
    };

    /// This palette in the shape `freya-code-editor`'s highlighter wants, so that the
    /// source pane is coloured from here rather than by `EditorSyntaxTheme::light()` --
    /// a GitHub-ish theme with no relationship to the colours the assembly pane beside it
    /// is drawn in.
    ///
    /// Thirty-four capture fields onto seven colours. The buckets are the assembly side's
    /// own meanings, which four of its five turn out to have a source equivalent for --
    /// `address_fg` is the fifth, and it is shared already, being the instruction addresses
    /// and the source line-number gutter rather than anything a capture names:
    ///
    /// - `keyword_fg`, the mnemonic's purple, takes keywords and operators, and types
    ///   with them: in C a type name *is* a keyword, and in Rust a builtin one reads like
    ///   one. Attributes and labels are language directives and go here too.
    /// - `operand_fg`, the register's olive, takes variables, parameters and fields,
    ///   because a register is the assembly's variable. This is the bucket that does the
    ///   most for the two panes reading as one: an instruction is mostly registers and a
    ///   statement is mostly identifiers, so both sides end up with olive at the same
    ///   density, in the same place in the line.
    /// - `literal_fg`, the immediate's blue, takes numbers, booleans and constants -- and
    ///   the escapes inside a string, which are a character written another way.
    /// - `name_fg`, the relocation target's near-black, takes functions, methods, macros,
    ///   modules and constructors. That colour already *is* a function's name on the
    ///   assembly side, so `call helper` and `helper(x)` name it identically. It is also
    ///   the colour of text no capture claims, which is most of a line.
    /// - `punctuation_fg`, the `Other` span's grey, takes brackets and delimiters.
    ///
    /// One trap in how this is read back: `syntax.rs`'s `resolve_capture_color` decides a
    /// capture is unmapped by comparing its colour to `text` and then walks up the dotted
    /// name looking for a segment whose colour differs, so a child field set to the same
    /// colour as `text` inherits its *parent's* colour instead of the text colour.
    /// Nothing below is caught by it -- every field equal to `name_fg`, which is `text`
    /// here, has a parent equal to it as well, so the walk ends where it started -- but
    /// giving, say, `punctuation_bracket` the text colour while `punctuation` keeps the
    /// grey would silently paint brackets grey. What decides that is which fields *share*
    /// a value and not what the values are, so it is a property a second palette can
    /// break by accident -- a dark `punctuation_fg` that happened to land on the dark
    /// `name_fg` would do it -- and `captures_do_not_walk_up` asserts it for both.
    pub(crate) fn syntax(&self) -> EditorSyntaxTheme {
        EditorSyntaxTheme {
            text: self.name_fg,
            // A run of leading indentation, which the highlighter colours rather than
            // leaving to the text: `SourceRow` draws it as plain spaces, so this is the
            // one field here that is never actually seen.
            whitespace: self.punctuation_fg,
            attribute: self.keyword_fg,
            boolean: self.literal_fg,
            comment: self.comment_fg,
            constant: self.literal_fg,
            constructor: self.name_fg,
            escape: self.literal_fg,
            function: self.name_fg,
            function_macro: self.name_fg,
            function_method: self.name_fg,
            keyword: self.keyword_fg,
            label: self.keyword_fg,
            module: self.name_fg,
            number: self.literal_fg,
            operator: self.keyword_fg,
            property: self.operand_fg,
            punctuation: self.punctuation_fg,
            punctuation_bracket: self.punctuation_fg,
            punctuation_delimiter: self.punctuation_fg,
            punctuation_special: self.keyword_fg,
            string: self.string_fg,
            string_escape: self.literal_fg,
            string_special: self.string_fg,
            tag: self.keyword_fg,
            text_literal: self.string_fg,
            text_reference: self.name_fg,
            text_title: self.keyword_fg,
            text_uri: self.string_fg,
            text_emphasis: self.name_fg,
            type_: self.keyword_fg,
            variable: self.operand_fg,
            variable_builtin: self.keyword_fg,
            variable_parameter: self.operand_fg,
        }
    }
}

thread_local! {
    /// Which of the two palettes the window is currently drawn in.
    ///
    /// A `State` and not a plain value, and this is the whole of how a theme switch repaints:
    /// `State::read` subscribes the reactive context that is running, so **asking for a
    /// colour is what subscribes a component to the theme**. A scope that draws nothing
    /// coloured is not woken; every scope that does is, exactly once, wherever in the tree it
    /// sits and whatever built it -- a row a `VirtualScrollView` recycles as much as a tab.
    ///
    /// The two answers that were weighed against it, and why they lose:
    ///
    /// - **A context read threaded through the call sites.** freya's own components do this
    ///   (`use_applied_theme!` is a `use_consume` of its `State<Theme>`), and it is the
    ///   idiomatic answer -- but a hook must run unconditionally in a component body, and
    ///   `palette()` is called from `row_background`, `kind_color`, `hairline_border` and a
    ///   dozen other free functions, from inside `if` arms and render callbacks, and from
    ///   `Highlighted::new`, which is not a component at all. So it would be a hook line in
    ///   each of the twenty-one components *plus* the free functions left reading a static
    ///   anyway, and a component whose line was forgotten would be a patch of the old theme
    ///   that nothing points at.
    /// - **Re-rendering the tree from the root.** This does not work: freya marks a child
    ///   scope dirty only when its props change (`freya-core`'s `runner.rs`), and every view
    ///   here is a unit `Component` whose props never do -- that is the same memoisation that
    ///   makes a selection change cost only the tabs that read it. Forcing it with a `key`
    ///   that changes with the theme would work by *remounting*, which resets the scope
    ///   storage below it: the three filters, the objects tree's folds, every hover and every
    ///   scroll controller. A theme switch is not a reason to lose the reader's search.
    ///
    /// What it costs, honestly: `palette()` was a `&'static` and is now a thread-local
    /// lookup, a subscribe and a generational-box read -- some tens of nanoseconds, against
    /// perhaps a thousand calls in a full render of both panes. The reference it hands back is
    /// still `'static`, because the two palettes are `const`s and only the *choice* is state.
    ///
    /// It is thread-local because a `State` is `!Send` (freya's storage is `Rc`-based) and
    /// created global because it must outlive every scope that reads it -- including, in the
    /// tests, a `TestingRunner`'s whole tree. Only the UI thread draws, so only the UI thread
    /// ever touches this one; a second thread would silently get a second copy of it, which is
    /// why nothing off the UI thread may ask for a colour.
    static APPEARANCE: State<Appearance> = State::create_global(Appearance::Light);
}

/// The colours to draw with, and a subscription to the theme for whoever asks.
pub(crate) fn palette() -> &'static Palette {
    match appearance() {
        Appearance::Light => &Palette::LIGHT,
        Appearance::Dark => &Palette::DARK,
    }
}

/// The same subscription, for the two things that want the choice itself rather than a
/// colour: freya's own theme sheet, and the effect that keeps it in step.
pub(crate) fn appearance() -> Appearance {
    APPEARANCE.with(|appearance| *appearance.read())
}

/// Draw in this appearance from now on. **The only way to change it**, deliberately: the
/// source pane's spans are cached with the palette's colours already resolved into them,
/// so a switch has to empty `HIGHLIGHTED` as well, and that clear lives here rather than
/// at the call site that happens to switch the theme today. `set_if_modified_and_then` is
/// what makes the pair one step -- the cache is emptied exactly when the value really
/// changed, so setting the appearance it is already in costs nothing and re-highlights
/// nothing.
pub(crate) fn set_appearance(next: Appearance) {
    APPEARANCE.with(|appearance| {
        let mut appearance = *appearance;
        appearance.set_if_modified_and_then(next, || highlighted().clear());
    });
}

/// Which appearance a stored choice comes to on a windowing system that prefers
/// `preferred`.
///
/// The two enums are one distinction twice over -- a *choice* has three answers and a
/// *preference* has two -- so this is a rule and not a lookup: a reader who named a theme
/// is answered by their own answer, and `Desktop` is the only one of the three that is a
/// question at all.
///
/// Pure, and handed the platform's answer rather than asking for it, exactly as
/// `fonts.rs`'s `resolve_font` is handed the desktop's: the rule is then testable with no
/// window anywhere, which is the whole reason the read and the rule are two functions.
pub(crate) fn resolve_appearance(choice: ThemeChoice, preferred: PreferredTheme) -> Appearance {
    match choice {
        ThemeChoice::Light => Appearance::Light,
        ThemeChoice::Dark => Appearance::Dark,
        ThemeChoice::Desktop => match preferred {
            PreferredTheme::Light => Appearance::Light,
            PreferredTheme::Dark => Appearance::Dark,
        },
    }
}

/// The sheet freya's own components read their colours from.
///
/// The palette above is deliberately *not* moved into freya's theming -- `ColorsSheet`
/// names none of these roles, and half the palette is consumed outside the element tree
/// entirely. But the window does hold a few freya components (the filter boxes, the
/// scrollbars, the resizable handle, the tooltips, the context menu), they read their
/// colours from this and from nothing else, and a white text box on a dark pane is not a
/// theme switch. So the base sheet follows the appearance, and the one override this app
/// has always made -- the tooltip's font size, which its theme hardcodes and no element
/// can set -- is applied on top of whichever it is.
pub(crate) fn interface_theme(appearance: Appearance) -> Theme {
    let mut theme = match appearance {
        Appearance::Light => light_theme(),
        Appearance::Dark => dark_theme(),
    };

    if let Some(tooltip) = theme.get::<TooltipThemePreference>("tooltip").cloned() {
        theme.set(
            "tooltip",
            TooltipThemePreference {
                font_size: Preference::Specific(fonts().ui.size()),
                ..tooltip
            },
        );
    }

    theme
}

/// `top` composited over `bottom`, both of them translucent.
///
/// An element has one background, so a row that is both hovered and pointed at from the
/// other pane would need a second rect inside it purely to carry the second colour.
/// Compositing the two here paints the pixels those two rects would have, since what lies
/// under both is the pane's own background either way.
pub(crate) fn blend(top: Color, bottom: Color) -> Color {
    let (top_alpha, bottom_alpha) = (top.a() as f32 / 255.0, bottom.a() as f32 / 255.0);
    let alpha = top_alpha + bottom_alpha * (1.0 - top_alpha);
    if alpha == 0.0 {
        return Color::TRANSPARENT;
    }

    let channel = |top: u8, bottom: u8| {
        ((top as f32 * top_alpha + bottom as f32 * bottom_alpha * (1.0 - top_alpha)) / alpha)
            .round() as u8
    };

    Color::from_argb(
        (alpha * 255.0).round() as u8,
        channel(top.r(), bottom.r()),
        channel(top.g(), bottom.g()),
        channel(top.b(), bottom.b()),
    )
}

/// The background of a code row: the pointer's own hover, the run of rows picked out to be
/// copied, the cross-view highlight it got from the other pane, the stronger one a click
/// pinned there, or any of them over any of the others.
///
/// A row that is both pinned and pointed at is painted as pinned, the stronger of the two
/// saying everything the weaker would. A selection is not one of that pair and does not
/// replace either: the two say what this row maps to and this says what would be copied,
/// so the three stack.
pub(crate) fn row_background(hovering: bool, focused: bool, pinned: bool, selected: bool) -> Color {
    let cross = match (pinned, focused) {
        (true, _) => palette().line_pin_bg,
        (false, true) => palette().line_focus_bg,
        (false, false) => Color::TRANSPARENT,
    };

    // `blend` over a transparent bottom is the top colour unchanged, so a hovered row that
    // is neither selected nor lit comes out as the hover alone without a case of its own.
    let hovered = if hovering {
        blend(palette().code_row_hover_bg, cross)
    } else {
        cross
    };

    // The selection goes on top of the hover and not under it, the other way round from
    // every other pair here. It is the only one of the three that says what a keystroke
    // is about to act on, and a row swept over by the pointer -- which is every row of a
    // drag, one after another -- would otherwise show the hover and almost none of it.
    if selected {
        blend(palette().row_select_bg, hovered)
    } else {
        hovered
    }
}

pub(crate) fn kind_color(kind: SpanKind) -> Color {
    match kind {
        SpanKind::Mnemonic | SpanKind::Prefix => palette().keyword_fg,
        SpanKind::Register => palette().operand_fg,
        SpanKind::Number => palette().literal_fg,
        SpanKind::Address => palette().address_fg,
        SpanKind::Other => palette().punctuation_fg,
    }
}
