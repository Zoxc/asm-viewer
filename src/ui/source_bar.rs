//! The bar over the Source pane naming the file that pane is showing, and
//! [`STALE_SOURCE`], the line drawn under it where that file is not the one the binary was
//! built from.
//!
//! **A companion's name is a door**: pressing it opens that file as a source-driven tab,
//! as pressing a source file's row in the Files view does, and until the source search
//! lands those are the two ways into one. Both go through [`open_source_tab`], which names
//! the document by the spelling an open tab already has for the file: what the bar carries
//! is the debug info's path, which is rarely the reader's own spelling of it. A
//! **subject** is that tab already, so its name is a name and nothing to press. Either way the bar says which file is up, which the
//! tab's own chip only has room for the last part of. At the end of the bar, where this
//! pane is the one the tab is driven from, is [`PaneToggle`]; the Assembly pane's own bar
//! carries it under the same rule.

use super::*;

/// What the Source pane says over a file whose bytes are not the ones the debug info's
/// checksum was taken of: the file is shown, since it is still the best thing to show, but
/// its line numbers are the compiler's and not necessarily this file's.
pub(crate) const STALE_SOURCE: &str = "This file differs from the one the binary was built from";

/// The bar over the Source pane, naming the file the pane is showing and carrying the
/// control that puts the pane beside it away.
///
/// The states come in as arguments because this is a function and not a component: a hook
/// written here would be the pane's own.
pub(crate) fn source_bar(
    side: &SourceSide,
    tab: DocId,
    open: Open,
    visits: State<Visits>,
    ctrl: State<bool>,
    sweeping: bool,
) -> Element {
    let file = side.file().clone();
    let opens = side.opens();
    // The file as a document: what the glyph is drawn from. Not what a press opens --
    // that is the path, which `open_source_tab` names for itself.
    let document = side.as_source();

    rect()
        .width(Size::fill())
        .horizontal()
        // The name takes what the toggle leaves, which torin only works out for a `flex`
        // child of a `Content::Flex` parent.
        .content(Content::Flex)
        .padding(Gaps::new_symmetric(0.0, 8.0))
        .background(palette().header_bg)
        .border(bottom_hairline())
        .child(
            // A box of its own and not the name as the `flex` child directly: a flex child
            // is measured from its content first, so a label placed there takes the width
            // of the whole path and the ellipsis never happens.
            rect()
                .width(Size::flex(1.0))
                .overflow(Overflow::Clip)
                // Not hit while a sweep is under way: the pointer dragging a selection up
                // past the bar would otherwise arm its tooltip, and light it.
                .interactive(!sweeping)
                .child(extra_tooltip(
                    file.to_string(),
                    rect()
                        .horizontal()
                        .cross_align(Alignment::Center)
                        .width(Size::fill())
                        .height(Size::px(list_row_height()))
                        .spacing(GLYPH_GAP)
                        .maybe(opens, |bar| {
                            let file = file.clone();
                            bar.on_press(move |_| {
                                let path = Path::new(&*file);
                                open_source_tab(open, visits, path, Reach::inside(ctrl));
                            })
                        })
                        .child(entry_icon(&document))
                        .child(
                            label()
                                .text(source::name_of(Path::new(&*file)))
                                .width(Size::fill())
                                .max_lines(1)
                                .text_overflow(TextOverflow::Ellipsis),
                        ),
                )),
        )
        // Only where this side leads, and outside the name rather than inside it, so a
        // press on it is a press on the toggle and never a door into the file.
        .maybe(!opens, |bar| {
            bar.child(PaneToggle {
                of: Placing::Tab(tab),
            })
        })
        .into_element()
}
