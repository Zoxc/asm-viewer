//! Every colour the window is drawn in, and the read that makes asking for one a
//! subscription to the theme.

use super::*;

/// Every colour the app draws, in one place.
///
/// Two instances, [`Palette::LIGHT`] and [`Palette::DARK`]; [`palette`] is how anything
/// reaches whichever is current. Deliberately not freya's own theming: `ColorsSheet` names
/// none of these roles, and half of this is consumed outside the element tree entirely
/// (see [`Palette::syntax`]). Two tests in `ui/tests.rs` hold the palettes to a contrast
/// floor for every foreground and a visible-step floor for every wash.
pub(crate) struct Palette {
    /// A pane's own body, and the tab header above the active one.
    pub(crate) pane_bg: Color,
    /// The interface text: every label that does not ask for a colour of its own. Set
    /// once on the root and *inherited*, freya resolving an unset `color` from the
    /// parent's.
    pub(crate) text_fg: Color,
    pub(crate) header_bg: Color,
    pub(crate) hairline: Color,
    /// The neutral grey: a row picked out in a list the keyboard is **not** in, the half
    /// of a panel a dragged tab would land in, and the copy of a tab or a panel header
    /// that follows the cursor. A list holding the keyboard draws its pick in
    /// `text_select_bg` instead, so the blue is what the next key acts on and the grey is
    /// a place a list is remembering (`ui/picks.rs`).
    pub(crate) selected_bg: Color,
    /// The wash a row takes under the pointer, in every list in the app. One colour and
    /// not one per pane: a row that lights says the same thing wherever it is. A grey and
    /// a *faint* one -- fainter than either colour a picked row wears, which the palette
    /// test says outright: the pointer passing over a row must not look like the reader
    /// having chosen it.
    pub(crate) row_hover_bg: Color,
    pub(crate) asm_pane_bg: Color,
    /// The pair: the rows of this pane that are the same place as the run picked out in
    /// the other one -- the instructions a selected source line was compiled from, the
    /// line a selected instruction came from. Nothing lights under the pointer; only a
    /// selection lights the other side. Translucent, so it composites with the selection
    /// -- see `blend`.
    pub(crate) pair_bg: Color,
    /// A row that is both: picked out here and the other pane's pair. The same green,
    /// deeper and less see-through, so the two states read as one and the sweep is not
    /// lost in the pair. Its own colour rather than one wash over the other, because a
    /// shadow over so pale a green barely moves it.
    pub(crate) pair_selected_bg: Color,
    /// The rule along the top and the bottom of a run of paired rows: the pair's green a
    /// step deeper, so a block of them is told from the pane around it where the wash
    /// alone is faint. Opaque, a line and not a wash.
    pub(crate) pair_edge: Color,
    /// The wash over the half of a panel a dragged tab would land in.
    pub(crate) drop_preview_bg: Color,
    /// A Lucide glyph in a tab header, or the disclosure triangle a list that folds draws
    /// in its own column, a step lighter than the text beside it.
    pub(crate) icon_fg: Color,
    /// A filter toggle that is on, and one the pointer is over: two shades of the header's
    /// own grey.
    pub(crate) toggle_on_bg: Color,
    pub(crate) toggle_hover_bg: Color,
    /// Behind the language server's control while a server is running: the one place in
    /// the app with a colour of its own, so that a process the reader started is visible
    /// at a glance and tells itself apart from a toggle that happens to be on. **Barely a
    /// colour**: enough purple to be told from the grey of a hover wash, and no more, since
    /// it sits in a bar the reader looks past all day.
    pub(crate) server_bg: Color,
    /// The wash under the × on a tab while the pointer is on the × *itself* rather
    /// than merely on the tab. Translucent, because it sits on either of two grounds --
    /// the active tab's `pane_bg` and a hovered tab's `toggle_hover_bg` -- and has to say
    /// the same thing over both; deeper than the tab's own hover, which is what tells the
    /// two apart on the same surface.
    pub(crate) close_hover_bg: Color,
    /// The wash behind a relocation link the pointer is over, lightening whatever row
    /// background is under it.
    pub(crate) link_hover_bg: Color,
    /// A branch line in the assembly gutter, with its corner and its arrowhead.
    pub(crate) branch_fg: Color,
    /// The same line while a branch of a picked-out row runs down it.
    pub(crate) branch_lit_fg: Color,
    /// The rule across the top of an instruction row a branch lands on, which is what makes
    /// the listing read as the basic blocks it is. Recessive on purpose -- it runs the whole
    /// width of the pane, where the gutter's stroke is a few pixels -- so it is quieter
    /// against the pane than `branch_fg` is, and the palette test says so.
    pub(crate) block_rule: Color,
    /// The selection, wherever there is one: the characters a sweep picked out in either
    /// pane, drawn by the row under its text, and the row picked out in the list holding
    /// the keyboard. A translucent blue-grey, a shade off the pane and a hue off the
    /// pair's green. Being translucent, it is a different colour over each of the two
    /// grounds it lands on -- a list's `pane_bg` and a code pane's `asm_pane_bg` -- and
    /// says the same thing over both.
    pub(crate) text_select_bg: Color,
    /// The row the caret is on, where a press on the text has left one and no sweep has
    /// followed: the selection's colour, faded.
    pub(crate) cursor_row_bg: Color,
    /// The caret itself: a one-pixel stroke, the text colour faded so it marks a place
    /// without reading as a character of the line.
    pub(crate) caret_fg: Color,
    /// The dot at a code row's left edge: a source line that produced code, and an
    /// instruction the debug info places on a source line. Also the mark on the tab the
    /// keyboard is in (`ui/strip.rs`), the one place it is not a dot.
    ///
    /// A drawing and not text, so it is held to a floor of its own and required to stay
    /// quieter than the number or address beside it: a column of dots read at a glance,
    /// where those are read one at a time. A purple of its own, faint, so that the one
    /// thing in the gutter that is not a number does not read as one.
    pub(crate) compiled_fg: Color,

    // The code colours. Which syntactic category takes which is [`Palette::syntax`] on
    // the source side and `kind_color` on the assembly side. Not every one of them is
    // drawn on both panes: a disassembly has no strings, comments, attributes, types or
    // call names, so those five are the source pane's alone.
    /// Where a thing is: the instruction addresses, and the source line-number gutter.
    pub(crate) address_fg: Color,
    /// What is being done: mnemonics and prefixes, source keywords and operators.
    pub(crate) keyword_fg: Color,
    /// A type where it is named. Source-only, and a dim red rather than the keyword's
    /// purple, so `struct Foo` reads as the keyword introducing a name and not as two
    /// halves of one word.
    pub(crate) type_fg: Color,
    /// A function or a method where it is *called* or defined, as against the plain text
    /// around it. Source-only, and a blue with none of the address column's greyness.
    pub(crate) function_fg: Color,
    /// What it is being done to: registers, and source variables, parameters and fields.
    pub(crate) operand_fg: Color,
    /// A value written out: immediates, and source numbers, booleans and constants.
    pub(crate) literal_fg: Color,
    /// A string literal. Source-only.
    pub(crate) string_fg: Color,
    /// A comment. Source-only.
    pub(crate) comment_fg: Color,
    /// The glue between the operands: brackets, commas, and on the assembly side the
    /// operand-size keywords (`qword ptr`) that are glue in exactly the same way.
    pub(crate) punctuation_fg: Color,
    /// An attribute -- `#[derive(..)]` and the rest -- which is scaffolding around the
    /// code rather than code, so it is a plain grey that recedes: quieter than the
    /// keyword it used to be drawn in, and quieter than the punctuation beside it.
    /// Source-only.
    pub(crate) attribute_fg: Color,
    /// A name that names one thing: a relocation target in the assembly, a module or a
    /// constructor in the source. Also the source pane's plain text.
    pub(crate) name_fg: Color,
    /// A link under the pointer, and the rule drawn under it (`link_chrome`).
    pub(crate) name_hover_fg: Color,

    /// What a pattern that will not compile, and the reason it will not, are written in.
    pub(crate) invalid_fg: Color,
    /// Under the file finder's panel, falling on whatever the window was showing: what
    /// lifts it off the app rather than dimming the app to say the same thing. The
    /// reader is choosing a file by what they can see of the window under it, so nothing
    /// there is taken away.
    pub(crate) panel_shadow: Color,
    /// The wash behind what a search or a filter matched: a desaturated green, low enough
    /// to read straight through. A wash and not a colour on the characters, which is what
    /// this was -- a bold orange -- because a match is a *place in* a name and not a thing
    /// of its own, and because it then says the same thing over a picked-out row as over a
    /// plain one. It is the paragraph's own highlight, freya giving a span no background
    /// (`ui/parts.rs`), so a row marks its matches in the one unit skia indexes text by.
    pub(crate) match_bg: Color,
    /// The same for what a find bar matched in a code pane, and a **purple** where that
    /// one is green: green already means "the same place as the run on the other side"
    /// in these two panes (`pair_bg`, which is that green at another alpha), so a match
    /// washed in it would read as a row the other pane had lit. The purple is the
    /// gutter dot's family (`compiled_fg`), at the wash alpha its green counterpart uses.
    ///
    /// A rect the row draws and not the paragraph's highlight, unlike `match_bg`: a code
    /// row's selection is drawn that way already, and the two have to compose -- the
    /// match a step is on wears this and the selection over it.
    pub(crate) find_bg: Color,
}

impl Palette {
    pub(crate) const LIGHT: Palette = Palette {
        pane_bg: Color::WHITE,
        text_fg: Color::BLACK,
        header_bg: Color::from_rgb(245, 245, 245), // WHITE_SMOKE
        hairline: Color::from_rgb(211, 211, 211),  // LIGHT_GRAY
        selected_bg: Color::from_rgb(211, 211, 211),
        row_hover_bg: Color::from_rgb(235, 235, 236),
        asm_pane_bg: Color::from_rgb(248, 248, 248),
        pair_bg: Color::from_argb(160, 228, 237, 216),
        pair_selected_bg: Color::from_argb(190, 197, 214, 184),
        pair_edge: Color::from_rgb(186, 208, 168),
        drop_preview_bg: Color::from_argb(60, 105, 89, 132),
        panel_shadow: Color::from_argb(70, 0, 0, 0),
        icon_fg: Color::from_rgb(90, 90, 90),
        toggle_on_bg: Color::from_rgb(196, 196, 196),
        toggle_hover_bg: Color::from_rgb(225, 225, 225),
        server_bg: Color::from_rgb(233, 229, 243),
        close_hover_bg: Color::from_argb(70, 90, 90, 96),
        link_hover_bg: Color::from_af32rgb(0.6, 255, 255, 255),
        branch_fg: Color::from_rgb(176, 188, 202),
        branch_lit_fg: Color::from_rgb(90, 116, 148),
        block_rule: Color::from_rgb(211, 216, 222),
        text_select_bg: Color::from_argb(44, 40, 70, 130),
        cursor_row_bg: Color::from_argb(20, 40, 70, 130),
        caret_fg: Color::from_argb(150, 0, 0, 0),
        compiled_fg: Color::from_rgb(176, 158, 206),

        address_fg: Color::from_rgb(118, 141, 169),
        keyword_fg: Color::from_rgb(116, 94, 147),
        type_fg: Color::from_rgb(160, 64, 80),
        function_fg: Color::from_rgb(46, 95, 160),
        operand_fg: Color::from_rgb(87, 103, 65),
        literal_fg: Color::from_rgb(80, 107, 135),
        string_fg: Color::from_rgb(148, 98, 74),
        comment_fg: Color::from_rgb(128, 148, 128),
        punctuation_fg: Color::from_rgb(102, 102, 102),
        attribute_fg: Color::from_rgb(132, 132, 140),
        name_fg: Color::from_rgb(50, 50, 50),
        name_hover_fg: Color::from_rgb(105, 89, 132),

        invalid_fg: Color::from_rgb(176, 0, 32),
        match_bg: Color::from_argb(64, 120, 160, 110),
        find_bg: Color::from_argb(64, 150, 118, 190),
    };

    /// The same palette at dark-mode lightness: every value is the one in `LIGHT` turned
    /// through the background, so where a light value is a step *down* from the surface
    /// it sits on, the dark one is the same step *up*.
    pub(crate) const DARK: Palette = Palette {
        pane_bg: Color::from_rgb(30, 30, 32),
        text_fg: Color::from_rgb(232, 232, 232),
        header_bg: Color::from_rgb(40, 40, 43),
        hairline: Color::from_rgb(62, 62, 66),
        selected_bg: Color::from_rgb(66, 66, 72),
        row_hover_bg: Color::from_rgb(46, 46, 49),
        asm_pane_bg: Color::from_rgb(34, 34, 36),
        // The three translucent ones, each stated as what it should come out as over the
        // pane rather than as the light value flipped: `blend` puts 30/30/32 under them.
        pair_bg: Color::from_argb(110, 120, 160, 110),
        pair_selected_bg: Color::from_argb(190, 120, 160, 110),
        pair_edge: Color::from_rgb(104, 140, 96),
        drop_preview_bg: Color::from_argb(90, 150, 130, 190),
        panel_shadow: Color::from_argb(140, 0, 0, 0),
        icon_fg: Color::from_rgb(160, 160, 160),
        toggle_on_bg: Color::from_rgb(88, 88, 92),
        toggle_hover_bg: Color::from_rgb(60, 60, 64),
        server_bg: Color::from_rgb(64, 60, 76),
        // Translucent, and stated the same way as the three above: what it comes out as
        // over a tab, which here means lifting the surface rather than darkening it.
        close_hover_bg: Color::from_argb(75, 200, 200, 210),
        link_hover_bg: Color::from_af32rgb(0.25, 255, 255, 255),
        branch_fg: Color::from_rgb(96, 108, 124),
        branch_lit_fg: Color::from_rgb(150, 178, 210),
        block_rule: Color::from_rgb(66, 72, 80),
        text_select_bg: Color::from_argb(100, 160, 175, 200),
        cursor_row_bg: Color::from_argb(45, 160, 175, 200),
        caret_fg: Color::from_argb(160, 232, 232, 232),
        compiled_fg: Color::from_rgb(104, 92, 134),

        address_fg: Color::from_rgb(132, 156, 186),
        keyword_fg: Color::from_rgb(178, 150, 214),
        type_fg: Color::from_rgb(226, 132, 148),
        function_fg: Color::from_rgb(118, 156, 232),
        operand_fg: Color::from_rgb(158, 180, 120),
        literal_fg: Color::from_rgb(130, 175, 214),
        string_fg: Color::from_rgb(214, 150, 120),
        comment_fg: Color::from_rgb(128, 158, 128),
        punctuation_fg: Color::from_rgb(150, 150, 150),
        attribute_fg: Color::from_rgb(126, 126, 136),
        name_fg: Color::from_rgb(216, 216, 216),
        name_hover_fg: Color::from_rgb(190, 168, 224),

        invalid_fg: Color::from_rgb(240, 110, 120),
        match_bg: Color::from_argb(80, 120, 160, 110),
        find_bg: Color::from_argb(80, 150, 118, 190),
    };

    /// This palette in the shape `freya-code-editor`'s highlighter wants, so the source
    /// pane is coloured from here rather than from `EditorSyntaxTheme::light()`.
    ///
    /// **The trap:** `resolve_capture_color` decides a capture is unmapped by comparing
    /// its colour to `text` and then walks *up* the dotted name for a segment whose colour
    /// differs -- so a child field set to the same colour as `text` silently takes its
    /// *parent's* colour instead. What decides that is which fields share a value, so a
    /// second palette can break it by landing two colours on each other;
    /// `captures_do_not_walk_up` asserts it for both.
    pub(crate) fn syntax(&self) -> EditorSyntaxTheme {
        EditorSyntaxTheme {
            text: self.name_fg,
            // Never actually seen: `SourceRow` draws leading indentation as plain spaces.
            whitespace: self.punctuation_fg,
            attribute: self.attribute_fg,
            boolean: self.literal_fg,
            comment: self.comment_fg,
            constant: self.literal_fg,
            constructor: self.name_fg,
            escape: self.literal_fg,
            function: self.function_fg,
            // A macro is called like a function and is coloured like one -- and it could
            // not be left on the text colour in any case: `resolve_capture_color` walks
            // `function.macro` up to `function` and would paint it in this silently.
            function_macro: self.function_fg,
            function_method: self.function_fg,
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
            type_: self.type_fg,
            variable: self.operand_fg,
            variable_builtin: self.keyword_fg,
            variable_parameter: self.operand_fg,
        }
    }
}

thread_local! {
    /// Which of the two palettes the window is currently drawn in.
    ///
    /// A `State`, and this is the whole of how a theme switch repaints: `State::read`
    /// subscribes the running reactive context, so **asking for a colour is what subscribes
    /// a component to the theme** -- exactly once, wherever in the tree it sits. Thread-local
    /// and global because `State` is `!Send` and it must outlive every scope that reads it;
    /// nothing off the UI thread may ask *this* for a colour, and [`colours`] is what it
    /// asks instead.
    static APPEARANCE: State<Appearance> = State::create_global(Appearance::Light);
}

/// The colours to draw with, and a subscription to the theme for whoever asks.
pub(crate) fn palette() -> &'static Palette {
    colours(appearance())
}

/// The colours of `appearance`, handed the theme rather than asking for it -- and so
/// **the one way to a colour off the UI thread**, where the state above cannot be read
/// at all. The source reader resolves a file's spans against it (`ui/highlight.rs`);
/// everything drawing anything asks [`palette`] and is subscribed by the asking.
pub(crate) fn colours(appearance: Appearance) -> &'static Palette {
    match appearance {
        Appearance::Light => &Palette::LIGHT,
        Appearance::Dark => &Palette::DARK,
    }
}

/// The same subscription, for whatever wants the choice itself rather than a colour.
pub(crate) fn appearance() -> Appearance {
    APPEARANCE.with(|appearance| *appearance.read())
}

/// Draw in this appearance from now on -- the only writer.
///
/// A switch does **not** empty `HIGHLIGHTED`, though its entries hold the palette's
/// colours resolved into them: each says which appearance it was parsed in, and one that
/// says the wrong thing is read again by the source reader off the UI thread. Emptying
/// the cache here would leave the pane with nothing to draw for as long as that takes
/// (`ui/highlight.rs`).
pub(crate) fn set_appearance(next: Appearance) {
    APPEARANCE.with(|appearance| {
        let mut appearance = *appearance;
        appearance.set_if_modified(next);
    });
}

/// Which appearance a stored choice comes to on a windowing system that prefers
/// `preferred`. Pure, and handed the platform's answer rather than asking for it, so the
/// rule is testable with no window anywhere.
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

/// Resolve the appearance from the stored choice and the platform's own, and write it
/// through [`set_appearance`] -- the one function that may change it.
///
/// **Not a `use_hook`**: reading `Platform::preferred_theme` subscribes this scope, so a
/// desktop that goes dark while the app is running repaints. It resolves in the render
/// body rather than in an effect, an effect being a frame late and a frame late on a dark
/// desktop a white flash; the write is idempotent, so that costs nothing.
pub(crate) fn apply_theme(choice: ThemeChoice) {
    let preferred = *Platform::get().preferred_theme.read();

    set_appearance(resolve_appearance(choice, preferred));
}

/// The sheet freya's own components -- the filter boxes, the scrollbars, the resizable
/// handle, the tooltips, the context menu -- read their colours from. The one override is
/// the tooltip's font size, which freya's theme hardcodes and no element can set.
///
/// `ui_size` is an argument and not a read of `fonts()`. What rebuilds the sheet on a font
/// change then lists the same two values as its deps.
pub(crate) fn interface_theme(appearance: Appearance, ui_size: f32) -> Theme {
    let mut theme = match appearance {
        Appearance::Light => light_theme(),
        Appearance::Dark => dark_theme(),
    };

    // The hover box's answer, drawn by freya's markdown viewer, whose own default
    // resolves its colours against freya's colour sheet rather than the palette -- so
    // every field is stated: one left out comes back in freya's colours beside ours. The
    // sizes are the two fonts', prose in the interface font and code in the fixed-width
    // one, which is what the panes draw them in.
    theme.set(
        "markdown_viewer",
        MarkdownViewerThemePreference {
            color: Preference::Specific(palette().text_fg),
            color_link: Preference::Specific(palette().name_hover_fg),
            background_code: Preference::Specific(palette().header_bg),
            color_code: Preference::Specific(palette().name_fg),
            background_blockquote: Preference::Specific(palette().header_bg),
            border_blockquote: Preference::Specific(palette().hairline),
            background_divider: Preference::Specific(palette().hairline),
            // A hover's headings are a doc comment's, inside a box a few lines tall: the
            // steps are small, and the smallest is the prose it sits over.
            heading_h1: Preference::Specific(ui_size + 5.0),
            heading_h2: Preference::Specific(ui_size + 4.0),
            heading_h3: Preference::Specific(ui_size + 3.0),
            heading_h4: Preference::Specific(ui_size + 2.0),
            heading_h5: Preference::Specific(ui_size + 1.0),
            heading_h6: Preference::Specific(ui_size),
            paragraph_size: Preference::Specific(ui_size),
            code_font_size: Preference::Specific(fonts().mono.size()),
            table_font_size: Preference::Specific(ui_size),
        },
    );

    if let Some(tooltip) = theme.get::<TooltipThemePreference>("tooltip").cloned() {
        theme.set(
            "tooltip",
            TooltipThemePreference {
                font_size: Preference::Specific(ui_size),
                ..tooltip
            },
        );
    }

    theme
}

/// `top` composited over `bottom`, both of them translucent. An element has one
/// background, so a row carrying two washes at once is painted with one composite.
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

/// How much of a tab's name survives the fade when its tab is not the one on screen.
///
/// Far more than [`DISABLED_ALPHA`]: a tab off screen is one the reader reads their way
/// along, where a disabled control is one they are being told not to press. It is a step
/// back and not a fade out.
const IDLE_ALPHA: u8 = 150;

/// How much of a disabled control's own colour survives the fade into the surface behind
/// it. One number for both palettes, since what it produces is a composite of two values
/// that are already carried over between them.
const DISABLED_ALPHA: u8 = 100;

/// `color` a step back into `surface`: what a tab that is not the one on screen writes its
/// name in. Derived, as [`dimmed`] is, so it follows the text colour of both palettes with
/// no second value to keep in step.
pub(crate) fn faded(color: Color, surface: Color) -> Color {
    blend(
        Color::from_argb(IDLE_ALPHA, color.r(), color.g(), color.b()),
        surface,
    )
}

/// A control that is drawn but cannot be used: the colour it has when it is live, faded
/// into the surface it sits on.
///
/// Derived rather than a `Palette` field of its own. A disabled drawing follows whatever
/// colour the control uses when it works, in both themes, with no second value per theme
/// to keep in step with the first -- and `blend` is already the rule for "this colour over
/// that ground", so the dimmed state is that rule applied to a foreground.
pub(crate) fn dimmed(color: Color, surface: Color) -> Color {
    blend(
        Color::from_argb(DISABLED_ALPHA, color.r(), color.g(), color.b()),
        surface,
    )
}

/// The wash a code row wears for its own pane's caret.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Wash {
    #[default]
    None,
    /// The caret's row: the run's lead is here and nothing is selected, so the caret is
    /// what shows and this is the line it is on.
    Cursor,
}

/// The background of a code row: the pair -- this row is where the other pane's
/// picked-out run maps to -- the caret's row, or the one over the other. The selection
/// itself is not a wash: the row draws it under its text (`ui/code_row.rs`).
///
/// Nothing here answers to the pointer: a row is lit by a selection, its own pane's or
/// the other's, and by nothing else.
pub(crate) fn row_background(paired: bool, wash: Wash) -> Color {
    // Three colours and not one wash over another: the caret's shadow over the pair's
    // pale green barely moved it, so a row that is both has a green of its own.
    match (paired, wash) {
        (true, Wash::Cursor) => palette().pair_selected_bg,
        (true, Wash::None) => palette().pair_bg,
        (false, Wash::Cursor) => palette().cursor_row_bg,
        (false, Wash::None) => Color::TRANSPARENT,
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
