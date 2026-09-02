//! The bar over the Assembly pane naming what that pane is drawing.
//!
//! Two rows -- the demangled name over the mangled original -- because the only other
//! place a symbol is named is its tab, which is cut to `CHIP_NAME_CHARS` by `short_name`,
//! and the mangled spelling appeared nowhere at all.

use super::*;

/// What the bar names.
///
/// A tab that is a whole object is asked of no worker ([`ask`] answers `None` for one, and
/// the hook then resets [`Analyzed`]), so there is never an analysis of one and the bar has
/// to fall back to the document for it. Everything else is the symbol the pane is drawing.
#[derive(Clone)]
pub(crate) enum Named {
    Symbol(Symbol),
    Object(Arc<Object>),
}

impl PartialEq for Named {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Named::Symbol(a), Named::Symbol(b)) => a == b,
            (Named::Object(a), Named::Object(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// One name in the bar: one line however long the name is, and **copied when it is
/// pressed**.
///
/// The row is an ellipsis over most of a name that runs long -- the samples in this repo
/// reach 1038 bytes mangled -- so the tooltip and the clipboard are between them the whole
/// of how a reader gets at the rest of it. Wrapping was the alternative and is what the
/// pane has no room for: a bar tall enough for the worst name is a bar that is that tall
/// for every other one.
#[derive(Clone, PartialEq)]
struct NameRow {
    text: String,
    /// The mangled spelling, which recedes under the demangled one above it.
    dim: bool,
}

impl Component for NameRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let copying = self.text.clone();

        row_tooltip(
            self.text.clone(),
            CursorArea::new().child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(list_row_height()))
                    // The row's own direction is vertical, so it is the *main* axis the
                    // label is centred on.
                    .main_align(Alignment::Center)
                    .overflow(Overflow::Clip)
                    // The grey a chrome control takes under the pointer, and not the
                    // relocation label's `link_hover_bg`: that one is a translucent white,
                    // which over the header's own grey is six levels and says nothing.
                    .maybe(hovering(), |row| row.background(palette().toggle_hover_bg))
                    .on_pointer_over(move |_| hovering.set_if_modified(true))
                    .on_pointer_out(move |_| hovering.set_if_modified(false))
                    // Failing silently, the way the listing's own copy does: a platform
                    // whose display handle gave freya-winit no clipboard has none, and a
                    // header has nowhere to say so.
                    .on_press(move |_| {
                        Clipboard::set(copying.clone()).ok();
                    })
                    .child(
                        label()
                            .text(self.text.clone())
                            .width(Size::fill())
                            .max_lines(1)
                            .text_overflow(TextOverflow::Ellipsis)
                            // Unset rather than `text_fg` when it is not dimmed, so the
                            // name goes on inheriting the interface colour from the root.
                            .maybe(self.dim, |name| name.color(palette().address_fg)),
                    ),
            ),
        )
    }
}

/// The bar over the Assembly pane, naming what that pane is drawing.
///
/// **The drawn symbol and never the selected one.** It is handed a [`Named`] worked out
/// from the same [`Analyzed::showing`] the listing under it is built from, rather than
/// reading `Active` the way the Info pane it replaces did: the two disagree for as long as
/// the worker takes, and a bar naming a function the rows below it are not of is worse than
/// no bar.
#[derive(Clone, PartialEq)]
pub(crate) struct SymbolBar {
    pub(crate) named: Named,
}

impl Component for SymbolBar {
    fn render(&self) -> impl IntoElement {
        // The mangled row only where there is a demangling: `display()` falls back to the
        // mangled name, so a symbol that was never mangled would otherwise be named twice.
        let names: Vec<Element> = match &self.named {
            Named::Symbol(symbol) => {
                let data = &symbol.data;
                std::iter::once(
                    NameRow {
                        text: data.display().to_owned(),
                        dim: false,
                    }
                    .into(),
                )
                .chain(data.demangled.is_some().then(|| {
                    NameRow {
                        text: data.name.clone(),
                        dim: true,
                    }
                    .into()
                }))
                .collect()
            }
            Named::Object(object) => vec![NameRow {
                text: object.name.clone(),
                dim: false,
            }
            .into()],
        };

        rect()
            .width(Size::fill())
            .horizontal()
            // The names take what the disclosure column leaves, which torin only works out
            // for a `flex` child of a `Content::Flex` parent.
            .content(Content::Flex)
            .padding(Gaps::new_symmetric(0.0, 8.0))
            .background(palette().header_bg)
            .border(bottom_hairline())
            // The column the disclosure triangle will sit in, kept from the start so the
            // names line up with themselves whether or not there is one.
            .child(rect().width(Size::px(CHEVRON_WIDTH)))
            .child(
                // A box of its own and not the names as the `flex` child directly: a flex
                // child is measured from its content first, so a label placed there takes
                // the width of its whole name and the ellipsis never happens.
                rect()
                    .width(Size::flex(1.0))
                    .overflow(Overflow::Clip)
                    .children(names),
            )
    }
}
