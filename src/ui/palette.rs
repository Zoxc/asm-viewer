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
    pub(crate) selected_bg: Color,
    pub(crate) object_hover_bg: Color,
    pub(crate) symbol_pane_bg: Color,
    pub(crate) symbol_hover_bg: Color,
    pub(crate) asm_pane_bg: Color,
    /// The pointer's own hover, on an instruction row and on a source line alike.
    pub(crate) code_row_hover_bg: Color,
    /// The cross-view highlight: this row is what the row the pointer is on maps to on the
    /// other side. Translucent, so it composites with the hover -- see `blend`.
    pub(crate) line_focus_bg: Color,
    /// The same highlight, made to stay by a click: one colour in two strengths.
    pub(crate) line_pin_bg: Color,
    /// The wash over the half of a panel a dragged tab would land in.
    pub(crate) drop_preview_bg: Color,
    /// A Lucide glyph in a dock tab header, a step lighter than the title beside it.
    pub(crate) icon_fg: Color,
    /// A filter toggle that is on, and one the pointer is over: two shades of the header's
    /// own grey.
    pub(crate) toggle_on_bg: Color,
    pub(crate) toggle_hover_bg: Color,
    /// The wash behind a relocation link the pointer is over, lightening whatever row
    /// background is under it.
    pub(crate) link_hover_bg: Color,
    /// A branch line in the assembly gutter, with its corner and its arrowhead.
    pub(crate) branch_fg: Color,
    /// The same line while a branch of the row under the pointer runs down it.
    pub(crate) branch_hover_fg: Color,
    /// The run of rows a reader has picked out to copy. Translucent, and composited with
    /// the two washes above by `blend`.
    pub(crate) row_select_bg: Color,

    // The code colours, shared by both panes. Which syntactic category takes which is
    // [`Palette::syntax`].
    /// Where a thing is: the instruction addresses, and the source line-number gutter.
    pub(crate) address_fg: Color,
    /// What is being done: mnemonics and prefixes, source keywords, operators and types.
    pub(crate) keyword_fg: Color,
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
    /// A name that names one thing: a relocation target in the assembly, a function,
    /// method or module in the source. Also the source pane's plain text.
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
        string_fg: Color::from_rgb(148, 98, 74),
        comment_fg: Color::from_rgb(128, 148, 128),
        punctuation_fg: Color::from_rgb(102, 102, 102),
        name_fg: Color::from_rgb(50, 50, 50),
        name_hover_fg: Color::from_rgb(105, 89, 132),

        invalid_fg: Color::from_rgb(176, 0, 32),
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
        object_hover_bg: Color::from_rgb(48, 92, 52),
        symbol_pane_bg: Color::from_rgb(38, 38, 33),
        symbol_hover_bg: Color::from_rgb(52, 52, 44),
        asm_pane_bg: Color::from_rgb(34, 34, 36),
        // The four translucent ones, each stated as what it should come out as over the
        // pane rather than as the light value flipped: `blend` puts 30/30/32 under them.
        code_row_hover_bg: Color::from_argb(110, 120, 160, 110),
        line_focus_bg: Color::from_argb(80, 130, 170, 230),
        line_pin_bg: Color::from_argb(140, 130, 170, 230),
        drop_preview_bg: Color::from_argb(90, 150, 130, 190),
        icon_fg: Color::from_rgb(160, 160, 160),
        toggle_on_bg: Color::from_rgb(88, 88, 92),
        toggle_hover_bg: Color::from_rgb(60, 60, 64),
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
        name_fg: Color::from_rgb(216, 216, 216),
        name_hover_fg: Color::from_rgb(190, 168, 224),

        invalid_fg: Color::from_rgb(240, 110, 120),
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
    /// A `State`, and this is the whole of how a theme switch repaints: `State::read`
    /// subscribes the running reactive context, so **asking for a colour is what subscribes
    /// a component to the theme** -- exactly once, wherever in the tree it sits. Thread-local
    /// and global because `State` is `!Send` and it must outlive every scope that reads it;
    /// nothing off the UI thread may ask for a colour.
    static APPEARANCE: State<Appearance> = State::create_global(Appearance::Light);
}

/// The colours to draw with, and a subscription to the theme for whoever asks.
pub(crate) fn palette() -> &'static Palette {
    match appearance() {
        Appearance::Light => &Palette::LIGHT,
        Appearance::Dark => &Palette::DARK,
    }
}

/// The same subscription, for whatever wants the choice itself rather than a colour.
pub(crate) fn appearance() -> Appearance {
    APPEARANCE.with(|appearance| *appearance.read())
}

/// Draw in this appearance from now on -- **the only way to change it**: the source pane's
/// spans are cached with the palette's colours resolved into them, so a switch has to empty
/// `HIGHLIGHTED` too, and that clear lives here rather than at a call site.
pub(crate) fn set_appearance(next: Appearance) {
    APPEARANCE.with(|appearance| {
        let mut appearance = *appearance;
        appearance.set_if_modified_and_then(next, || highlighted().clear());
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

/// The sheet freya's own components -- the filter boxes, the scrollbars, the resizable
/// handle, the tooltips, the context menu -- read their colours from. The one override is
/// the tooltip's font size, which freya's theme hardcodes and no element can set.
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

/// How much of a disabled control's own colour survives the fade into the surface behind
/// it. One number for both palettes, since what it produces is a composite of two values
/// that are already carried over between them.
const DISABLED_ALPHA: u8 = 100;

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

/// The background of a code row: the pointer's own hover, the run of rows picked out to be
/// copied, the cross-view highlight from the other pane, the stronger one a click pinned
/// there, or any of them over any of the others.
pub(crate) fn row_background(hovering: bool, focused: bool, pinned: bool, selected: bool) -> Color {
    let cross = match (pinned, focused) {
        (true, _) => palette().line_pin_bg,
        (false, true) => palette().line_focus_bg,
        (false, false) => Color::TRANSPARENT,
    };

    // `blend` over a transparent bottom is the top colour unchanged, so a hovered row
    // that is neither selected nor lit needs no case of its own.
    let hovered = if hovering {
        blend(palette().code_row_hover_bg, cross)
    } else {
        cross
    };

    // The selection goes *on top of* the hover, the other way round from every other pair
    // here: a row swept over by the pointer would otherwise show almost none of it.
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
